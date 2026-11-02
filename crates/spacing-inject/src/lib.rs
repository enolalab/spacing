pub mod clipboard;
#[cfg(target_os = "linux")]
pub mod map;
#[cfg(target_os = "linux")]
pub mod portal;
#[cfg(windows)]
pub mod windows;
#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(target_os = "linux")]
use evdev::uinput::VirtualDeviceBuilder;
#[cfg(target_os = "linux")]
use evdev::{AttributeSet, Key, RelativeAxisType};
use spacing_core::config::Backend;
use spacing_core::event::InputEvent;
#[cfg(target_os = "linux")]
use spacing_core::hidmap::{hid_to_evdev, EV_TO_HID};
#[cfg(target_os = "linux")]
use std::collections::HashSet;
use std::io;

/// Pluggable injection backend (spec §Traits): uinput today, the
/// RemoteDesktop portal (libei) on capable Wayland desktops.
/// Clipboard is intentionally NOT here — the loops own a `Clipboard`
/// directly (server has no injector).
#[async_trait::async_trait]
pub trait InjectionBackend: Send {
    async fn inject(&mut self, ev: &InputEvent) -> io::Result<()>;
    /// Key-up everything currently held — disconnect/watchdog path.
    async fn release_all(&mut self) -> io::Result<()>;
}

/// Pick an injection backend. Linux `Auto`: RemoteDesktop portal on
/// Wayland when available, else uinput. Windows/macOS always use the
/// native backend (`SendInput` / `CGEventPost`).
#[cfg(target_os = "linux")]
pub async fn select_inject(backend: Backend) -> io::Result<Box<dyn InjectionBackend>> {
    match backend {
        Backend::Evdev => UinputInject::create().map(|i| Box::new(i) as _),
        Backend::Portal => portal::PortalInject::connect()
            .await
            .map(|i| Box::new(i) as _),
        Backend::Auto => {
            if wayland_session() {
                match portal::PortalInject::connect().await {
                    Ok(i) => {
                        tracing::info!("Wayland portal backend active (RemoteDesktop/libei)");
                        return Ok(Box::new(i));
                    }
                    Err(e) => {
                        tracing::warn!("portal backend unavailable ({e}) — falling back to uinput");
                    }
                }
            }
            UinputInject::create().map(|i| Box::new(i) as _)
        }
    }
}

#[cfg(windows)]
pub async fn select_inject(_backend: Backend) -> io::Result<Box<dyn InjectionBackend>> {
    windows::SendInputInject::create().map(|i| Box::new(i) as _)
}

#[cfg(target_os = "macos")]
pub async fn select_inject(_backend: Backend) -> io::Result<Box<dyn InjectionBackend>> {
    macos::CgInject::create().map(|i| Box::new(i) as _)
}

#[cfg(not(any(target_os = "linux", windows, target_os = "macos")))]
pub async fn select_inject(_backend: Backend) -> io::Result<Box<dyn InjectionBackend>> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "no injection backend for this platform",
    ))
}

#[cfg(target_os = "linux")]
fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        && std::env::var("XDG_SESSION_TYPE").map_or(true, |v| v == "wayland")
}

/// Virtual keyboard+mouse created via /dev/uinput.
/// Works under Wayland and X11 (kernel-level). Requires write access to
/// /dev/uinput (see README udev rule).
#[cfg(target_os = "linux")]
pub struct UinputInject {
    dev: evdev::uinput::VirtualDevice,
    held: HashSet<u16>, // evdev codes currently down (keys AND BTN_* buttons)
}

#[cfg(target_os = "linux")]
impl UinputInject {
    pub fn create() -> io::Result<Self> {
        let mut keys = AttributeSet::<Key>::new();
        for (code, _) in EV_TO_HID {
            keys.insert(Key::new(*code));
        }
        for b in [0x110u16, 0x111, 0x112, 0x113, 0x114] {
            keys.insert(Key::new(b));
        }
        let mut axes = AttributeSet::<RelativeAxisType>::new();
        axes.insert(RelativeAxisType::REL_X);
        axes.insert(RelativeAxisType::REL_Y);
        axes.insert(RelativeAxisType::REL_WHEEL);
        axes.insert(RelativeAxisType::REL_HWHEEL);
        axes.insert(RelativeAxisType::REL_WHEEL_HI_RES);
        axes.insert(RelativeAxisType::REL_HWHEEL_HI_RES);

        let dev = VirtualDeviceBuilder::new()?
            .name("spacing-virtual-input")
            .with_keys(&keys)?
            .with_relative_axes(&axes)?
            .build()?;
        Ok(Self {
            dev,
            held: HashSet::new(),
        })
    }

    pub fn inject(&mut self, ev: &InputEvent) -> io::Result<()> {
        let out = map::to_evdev(ev);
        if !out.is_empty() {
            self.dev.emit(&out)?; // one write() per wire event
        }
        // track held codes (keys + buttons) for release_all
        match *ev {
            InputEvent::KeyDown(hid) => {
                if let Some(c) = hid_to_evdev(hid) {
                    self.held.insert(c);
                }
            }
            InputEvent::KeyUp(hid) => {
                if let Some(c) = hid_to_evdev(hid) {
                    self.held.remove(&c);
                }
            }
            InputEvent::ButtonDown(b) => {
                self.held.insert(map::btn_code(b));
            }
            InputEvent::ButtonUp(b) => {
                self.held.remove(&map::btn_code(b));
            }
            _ => {}
        }
        Ok(())
    }

    /// Key-up every tracked code — invoked on disconnect/watchdog.
    pub fn release_all(&mut self) -> io::Result<()> {
        let held: Vec<u16> = self.held.drain().collect();
        for code in held {
            self.dev.emit(&[
                evdev::InputEvent::new(evdev::EventType::KEY, code, 0),
                evdev::InputEvent::new(evdev::EventType::SYNCHRONIZATION, 0, 0),
            ])?;
        }
        Ok(())
    }

}

#[cfg(target_os = "linux")]
#[async_trait::async_trait]
impl InjectionBackend for UinputInject {
    async fn inject(&mut self, ev: &InputEvent) -> io::Result<()> {
        UinputInject::inject(self, ev)
    }

    async fn release_all(&mut self) -> io::Result<()> {
        UinputInject::release_all(self)
    }
}

#[cfg(all(test, target_os = "linux"))]
mod select_tests {
    use super::*;

    #[test]
    fn uinput_selection_attempts_without_panic() {
        // Result depends on /dev/uinput access — only contract is
        // "returns, never panics". (Portal needs an async runtime +
        // a live session and is exercised in host smoke tests.)
        let _ = UinputInject::create();
    }
}
