pub mod map;

use futures::stream;
use futures::StreamExt as _;
use spacing_core::config::Backend;
use spacing_core::event::InputEvent;
use std::io;
use std::path::Path;

/// Event from a capture backend. The evdev backend only yields `Input`;
/// the Wayland portal backend additionally reports barrier activations
/// (screen-edge crossings) that drive edge-based switching.
#[derive(Debug)]
pub enum CaptureEvent {
    Input(InputEvent),
    /// Pointer crossed a configured barrier — `zone` names the edge
    /// (e.g. "right") as configured in `[server.layout.edges]`.
    Activated { zone: String },
    /// Capture was released (ESC, session deactivated, …).
    Deactivated,
}

/// Pluggable capture backend (spec §Traits).
/// `&mut self` borrows end when `next_event` resolves, so the server can
/// grab()/ungrab() inside the same select! loop.
#[async_trait::async_trait]
pub trait CaptureBackend: Send {
    /// Grab input exclusively. On the portal backend this is a no-op —
    /// capture starts on barrier activation, not on request.
    async fn grab(&mut self) -> io::Result<()>;
    async fn ungrab(&mut self) -> io::Result<()>;
    /// Next event; `None` when the backend is finished.
    async fn next_event(&mut self) -> Option<CaptureEvent>;
}

/// Pick a capture backend. `Auto`: Wayland portal when implemented and
/// the session supports it, else evdev (kernel-level, works everywhere).
pub fn select_capture(backend: Backend) -> io::Result<Box<dyn CaptureBackend>> {
    match backend {
        Backend::Evdev | Backend::Auto => {
            if matches!(backend, Backend::Auto) && wayland_session() {
                tracing::info!("Wayland session — portal backend pending, using evdev");
            }
            EvdevCapture::enumerate().map(|c| Box::new(c) as _)
        }
        Backend::Portal => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "portal backend not built yet — use backend = \"evdev\" or \"auto\"",
        )),
    }
}

fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var("XDG_SESSION_TYPE").map_or(true, |v| v == "wayland")
}

/// Owns all physical input devices as event streams.
/// grab() = EVIOCGRAB trên từng fd (OS ngừng nhận input); stream vẫn đọc được.
pub struct EvdevCapture {
    streams: Vec<evdev::EventStream>,
}

impl EvdevCapture {
    /// Scan /dev/input, keep devices that look like keyboards or mice,
    /// convert each to an EventStream. Requires read access to
    /// /dev/input/event* (input group or root).
    pub fn enumerate() -> io::Result<Self> {
        Self::enumerate_at(Path::new("/dev/input"))
    }

    /// Same as `enumerate` over an arbitrary directory — testable seam.
    pub fn enumerate_at(dir: &Path) -> io::Result<Self> {
        let mut streams = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let is_event = path
                .file_name()
                .map(|n| n.to_string_lossy().starts_with("event"))
                .unwrap_or(false);
            if !is_event {
                continue;
            }
            let Ok(device) = evdev::Device::open(&path) else {
                continue;
            };
            // Never capture our own injected device — on a machine running
            // both modes it would feed injected events back into the wire
            // (infinite echo loop).
            if device.name() == Some("spacing-virtual-input") {
                continue;
            }
            let is_kb = device
                .supported_keys()
                .map(|k| {
                    (30..=38u16).all(|c| k.contains(evdev::Key::new(c))) // A..L
                })
                .unwrap_or(false);
            let is_mouse = device.supported_relative_axes().map(|r| {
                r.contains(evdev::RelativeAxisType::REL_X)
                    && r.contains(evdev::RelativeAxisType::REL_Y)
            });
            if !(is_kb || is_mouse.unwrap_or(false)) {
                continue;
            }
            match device.into_event_stream() {
                Ok(s) => {
                    tracing::info!(?path, "capturing device");
                    streams.push(s);
                }
                Err(e) => tracing::warn!(?path, "event stream failed: {e}"),
            }
        }
        if streams.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no keyboard/mouse found in /dev/input — check input group permissions",
            ));
        }
        Ok(Self { streams })
    }

    /// Exclusive-grab every device. On failure, releases all grabs so no
    /// device is left hidden from the OS (golden rule).
    pub fn grab(&mut self) -> io::Result<()> {
        for s in &mut self.streams {
            if let Err(e) = s.device_mut().grab() {
                let _ = self.ungrab();
                return Err(e);
            }
        }
        Ok(())
    }

    pub fn ungrab(&mut self) -> io::Result<()> {
        for s in &mut self.streams {
            let _ = s.device_mut().ungrab(); // best-effort: never leave grabbed
        }
        Ok(())
    }

    /// Next mapped input event; skips unmapped.
    pub async fn next_input(&mut self) -> Option<InputEvent> {
        // `&mut EventStream: Stream` (EventStream: Unpin in evdev 0.12).
        let mut merged = stream::select_all(self.streams.iter_mut());
        while let Some(raw) = merged.next().await {
            if let Ok(ev) = raw {
                if let Some(ev) = map::map_event(ev) {
                    return Some(ev);
                }
            }
        }
        None
    }
}

#[async_trait::async_trait]
impl CaptureBackend for EvdevCapture {
    async fn grab(&mut self) -> io::Result<()> {
        EvdevCapture::grab(self)
    }

    async fn ungrab(&mut self) -> io::Result<()> {
        EvdevCapture::ungrab(self)
    }

    async fn next_event(&mut self) -> Option<CaptureEvent> {
        self.next_input().await.map(CaptureEvent::Input)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spacing_core::config::Backend;

    #[test]
    fn enumerate_empty_dir_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let e = EvdevCapture::enumerate_at(dir.path()).err().unwrap();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn explicit_portal_backend_errors_until_built() {
        let e = select_capture(Backend::Portal).err().unwrap();
        assert_eq!(e.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn explicit_evdev_backend_does_not_panic() {
        // Result depends on the machine's /dev/input — only contract is
        // "returns, never panics".
        let _ = select_capture(Backend::Evdev);
    }
}
