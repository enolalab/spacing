pub mod map;

use evdev::uinput::VirtualDeviceBuilder;
use evdev::{AttributeSet, Key, RelativeAxisType};
use spacing_core::event::InputEvent;
use spacing_core::hidmap::{hid_to_evdev, EV_TO_HID};
use std::collections::HashSet;
use std::io;

/// Virtual keyboard+mouse created via /dev/uinput.
/// Works under Wayland and X11 (kernel-level). Requires write access to
/// /dev/uinput (see README udev rule).
pub struct UinputInject {
    dev: evdev::uinput::VirtualDevice,
    held: HashSet<u16>, // evdev codes currently down (keys AND BTN_* buttons)
}

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

    /// P1 stub: clipboard lands in P2.
    pub async fn set_clipboard(&mut self, _text: &str) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "clipboard not in P1",
        ))
    }
}
