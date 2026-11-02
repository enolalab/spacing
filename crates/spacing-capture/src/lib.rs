#[cfg(target_os = "linux")]
pub mod map;
#[cfg(target_os = "linux")]
pub mod portal;
#[cfg(windows)]
pub mod windows;
#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
use futures::stream;
#[cfg(target_os = "linux")]
use futures::StreamExt as _;
use spacing_core::config::Backend;
use spacing_core::event::InputEvent;
use std::collections::BTreeMap;
use std::io;
#[cfg(target_os = "linux")]
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

/// Screen-edge → client map for portal barrier switching.
pub type EdgeMap = BTreeMap<String, String>;

/// Pick a capture backend.
/// Linux `Auto`: Wayland portal when the session can provide it, else
/// evdev. Windows/macOS always use the native backend regardless of
/// `backend` (the names are Linux-specific).
#[cfg(target_os = "linux")]
pub async fn select_capture(
    backend: Backend,
    edges: &EdgeMap,
) -> io::Result<Box<dyn CaptureBackend>> {
    match backend {
        Backend::Evdev => EvdevCapture::enumerate().map(|c| Box::new(c) as _),
        Backend::Portal => portal::PortalCapture::connect(edges)
            .await
            .map(|c| Box::new(c) as _),
        Backend::Auto => {
            if wayland_session() {
                match portal::PortalCapture::connect(edges).await {
                    Ok(c) => {
                        tracing::info!("Wayland portal backend active (InputCapture/libei)");
                        return Ok(Box::new(c));
                    }
                    Err(e) => {
                        tracing::warn!("portal backend unavailable ({e}) — falling back to evdev");
                    }
                }
            }
            EvdevCapture::enumerate().map(|c| Box::new(c) as _)
        }
    }
}

#[cfg(windows)]
pub async fn select_capture(
    _backend: Backend,
    edges: &EdgeMap,
) -> io::Result<Box<dyn CaptureBackend>> {
    windows::WinHooksCapture::connect(edges).map(|c| Box::new(c) as _)
}

#[cfg(target_os = "macos")]
pub async fn select_capture(
    _backend: Backend,
    edges: &EdgeMap,
) -> io::Result<Box<dyn CaptureBackend>> {
    macos::CgTapCapture::connect(edges).map(|c| Box::new(c) as _)
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
pub async fn select_capture(
    _backend: Backend,
    _edges: &EdgeMap,
) -> io::Result<Box<dyn CaptureBackend>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no capture backend for this platform",
    ))
}

#[cfg(target_os = "linux")]
fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var("XDG_SESSION_TYPE").map_or(true, |v| v == "wayland")
}

/// Owns all physical input devices as event streams.
/// grab() = EVIOCGRAB trên từng fd (OS ngừng nhận input); stream vẫn đọc được.
#[cfg(target_os = "linux")]
pub struct EvdevCapture {
    streams: Vec<evdev::EventStream>,
}

#[cfg(target_os = "linux")]
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

#[cfg(target_os = "linux")]
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use spacing_core::config::Backend;

    #[test]
    fn enumerate_empty_dir_is_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let e = EvdevCapture::enumerate_at(dir.path()).err().unwrap();
        assert_eq!(e.kind(), io::ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn explicit_portal_backend_attempts_without_panic() {
        // On a Wayland host this creates a real portal session; on
        // headless CI it errors — either way it must not panic.
        let _ = select_capture(Backend::Portal, &EdgeMap::new()).await;
    }

    #[tokio::test]
    async fn explicit_evdev_backend_does_not_panic() {
        // Result depends on the machine's /dev/input — only contract is
        // "returns, never panics".
        let _ = select_capture(Backend::Evdev, &EdgeMap::new()).await;
    }
}
