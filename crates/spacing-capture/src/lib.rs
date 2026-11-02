pub mod map;

use futures::stream;
use futures::StreamExt as _;
use spacing_core::event::InputEvent;
use std::io;

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
        let mut streams = Vec::new();
        for (path, device) in evdev::enumerate() {
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

    /// Next mapped input event; skips unmapped. The `&mut self` borrow ends
    /// when the future resolves, so the server can grab()/ungrab() inside
    /// the same select! loop.
    pub async fn next_event(&mut self) -> Option<InputEvent> {
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
