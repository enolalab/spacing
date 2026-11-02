//! Windows injection backend — `SendInput`.
//!
//! Keyboard events go out as PS/2 set-1 scan codes
//! (`KEYEVENTF_SCANCODE`, plus `KEYEVENTF_EXTENDEDKEY` for E0 keys) —
//! positional and layout-independent, matching the evdev/HID model.
//! Mouse motion is relative (`MOUSEEVENTF_MOVE` without ABSOLUTE);
//! wheel uses `WHEEL_DELTA` (120/detent).

use spacing_core::event::{InputEvent as Ev, MouseButton};
use spacing_core::hidmap::hid_to_evdev;
use spacing_core::keymap_win::evdev_to_scan;
use std::collections::HashSet;
use std::io;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
use windows_sys::Win32::UI::WindowsAndMessaging::{XBUTTON1, XBUTTON2};

/// Injection backend — stateless except for held-key tracking
/// (`release_all` on disconnect).
pub struct SendInputInject {
    /// (scan, extended) pairs currently held down — keys AND buttons
    /// share the same release path via distinct flag markers below.
    held: HashSet<u16>, // HID codes held (simpler bookkeeping)
}

impl SendInputInject {
    pub fn create() -> io::Result<Self> {
        Ok(Self {
            held: HashSet::new(),
        })
    }

    fn send(inputs: &mut [INPUT]) -> io::Result<()> {
        let n = unsafe { SendInput(inputs.len() as u32, inputs.as_mut_ptr(), 1) };
        if n as usize != inputs.len() {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    fn key_input(scan: u8, extended: bool, down: bool) -> INPUT {
        let mut i: INPUT = unsafe { std::mem::zeroed() };
        i.r#type = INPUT_KEYBOARD;
        {
            i.Anonymous.ki = KEYBDINPUT {
                wVk: 0,
                wScan: scan as u16,
                dwFlags: KEYEVENTF_SCANCODE
                    | if extended { KEYEVENTF_EXTENDEDKEY } else { 0 }
                    | if down { 0 } else { KEYEVENTF_KEYUP },
                time: 0,
                dwExtraInfo: 0,
            };
        }
        i
    }

    fn mouse_input(dx: i32, dy: i32, data: u32, flags: u32) -> INPUT {
        let mut i: INPUT = unsafe { std::mem::zeroed() };
        i.r#type = INPUT_MOUSE;
        {
            i.Anonymous.mi = MOUSEINPUT {
                dx,
                dy,
                mouseData: data,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            };
        }
        i
    }

    fn btn_flags(b: MouseButton, down: bool) -> (u32, u32) {
        let f = match (b, down) {
            (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
            (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
            // Side/Extra arrive as X buttons.
            (MouseButton::Side | MouseButton::Extra, true) => MOUSEEVENTF_XDOWN,
            (MouseButton::Side | MouseButton::Extra, false) => MOUSEEVENTF_XUP,
        };
        let data = match b {
            MouseButton::Side => XBUTTON1 as u32,
            MouseButton::Extra => XBUTTON2 as u32,
            _ => 0,
        };
        (f, data)
    }

    pub fn inject(&mut self, ev: &Ev) -> io::Result<()> {
        match *ev {
            Ev::KeyDown(hid) | Ev::KeyUp(hid) => {
                let down = matches!(ev, Ev::KeyDown(_));
                let Some(ev_code) = hid_to_evdev(hid) else {
                    return Ok(());
                };
                let Some((scan, ext)) = evdev_to_scan(ev_code) else {
                    return Ok(());
                };
                Self::send(&mut [Self::key_input(scan, ext, down)])?;
                if down {
                    self.held.insert(hid);
                } else {
                    self.held.remove(&hid);
                }
            }
            Ev::ButtonDown(b) | Ev::ButtonUp(b) => {
                let down = matches!(ev, Ev::ButtonDown(_));
                let (flags, data) = Self::btn_flags(b, down);
                Self::send(&mut [Self::mouse_input(0, 0, data, flags)])?;
                if down {
                    self.held.insert(0x1000 + b as u16); // pseudo-code
                } else {
                    self.held.remove(&(0x1000 + b as u16));
                }
            }
            Ev::MouseMoveRel { dx, dy } => {
                Self::send(&mut [Self::mouse_input(dx, dy, 0, MOUSEEVENTF_MOVE)])?;
            }
            Ev::Wheel { dx, dy } => {
                let mut ins = Vec::with_capacity(2);
                if dy != 0 {
                    ins.push(Self::mouse_input(
                        0,
                        0,
                        (dy * 120) as u32,
                        MOUSEEVENTF_WHEEL,
                    ));
                }
                if dx != 0 {
                    ins.push(Self::mouse_input(
                        0,
                        0,
                        (dx * 120) as u32,
                        MOUSEEVENTF_HWHEEL,
                    ));
                }
                if !ins.is_empty() {
                    Self::send(&mut ins)?;
                }
            }
        }
        Ok(())
    }

    /// Key-up everything held — disconnect/watchdog path.
    pub fn release_all(&mut self) -> io::Result<()> {
        let held: Vec<u16> = self.held.drain().collect();
        for code in held {
            if code >= 0x1000 {
                let b = match code - 0x1000 {
                    x if x == MouseButton::Left as u16 => MouseButton::Left,
                    x if x == MouseButton::Right as u16 => MouseButton::Right,
                    x if x == MouseButton::Middle as u16 => MouseButton::Middle,
                    x if x == MouseButton::Side as u16 => MouseButton::Side,
                    _ => MouseButton::Extra,
                };
                let (flags, data) = Self::btn_flags(b, false);
                let _ = Self::send(&mut [Self::mouse_input(0, 0, data, flags)]);
            } else if let Some(ev_code) = hid_to_evdev(code) {
                if let Some((scan, ext)) = evdev_to_scan(ev_code) {
                    let _ = Self::send(&mut [Self::key_input(scan, ext, false)]);
                }
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::InjectionBackend for SendInputInject {
    async fn inject(&mut self, ev: &Ev) -> io::Result<()> {
        SendInputInject::inject(self, ev)
    }

    async fn release_all(&mut self) -> io::Result<()> {
        SendInputInject::release_all(self)
    }
}
