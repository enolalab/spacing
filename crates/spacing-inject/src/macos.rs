//! macOS injection backend — `CGEventPost` at the HID event tap.
//!
//! Keys are emitted as `CGKeyCode`s (layout-independent). Mouse motion
//! tracks a virtual cursor position and posts absolute `MouseMoved`
//! events (CoreGraphics applies the delta itself); scroll uses
//! line-unit wheel events.
//!
//! Permissions (TCC): "Accessibility" for event posting
//! (System Settings → Privacy & Security).

use spacing_core::event::{InputEvent as Ev, MouseButton};
use spacing_core::hidmap::hid_to_evdev;
use spacing_core::keymap_mac::evdev_to_cgkey;
use std::collections::HashSet;
use std::ffi::c_void;
use std::io;

// --- minimal CoreGraphics FFI -------------------------------------------

type CGEventRef = *mut c_void;
type CFTypeRef = *const c_void;
type CGEventSourceRef = *const c_void;

#[repr(C)]
#[derive(Clone, Copy)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[link(name = "CoreGraphics", kind = "framework")]
#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CGEventCreateKeyboardEvent(
        src: CGEventSourceRef,
        key: u16,
        down: bool,
    ) -> CGEventRef;
    fn CGEventCreateMouseEvent(
        src: CGEventSourceRef,
        type_: u32,
        point: CGPoint,
        button: u32,
    ) -> CGEventRef;
    fn CGEventCreateScrollWheelEvent(
        src: CGEventSourceRef,
        units: u32,
        count: u32,
        w1: i32,
        w2: i32,
    ) -> CGEventRef;
    fn CGEventCreate(src: CGEventSourceRef) -> CGEventRef;
    fn CGEventGetLocation(ev: CGEventRef) -> CGPoint;
    fn CGEventPost(tap: u32, ev: CGEventRef);
    fn CFRelease(cf: CFTypeRef);
}

const T_MOVED: u32 = 5;
const T_LDRAG: u32 = 6;
const T_RDRAG: u32 = 7;
const T_ODRAG: u32 = 27;
const T_LDOWN: u32 = 1;
const T_LUP: u32 = 2;
const T_RDOWN: u32 = 3;
const T_RUP: u32 = 4;
const T_ODOWN: u32 = 25;
const T_OUP: u32 = 26;

const UNIT_LINE: u32 = 1;
const HID_TAP: u32 = 0;

/// macOS inject backend. `pos` = our virtual cursor position — starts
/// at the real cursor, moves with each injected relative motion.
pub struct CgInject {
    pos: CGPoint,
    held: HashSet<u16>, // HID codes held; buttons as 0x1000+enum
}

impl CgInject {
    pub fn create() -> io::Result<Self> {
        // Seed the virtual cursor at the real pointer position.
        let pos = unsafe {
            let ev = CGEventCreate(std::ptr::null());
            let p = CGEventGetLocation(ev);
            CFRelease(ev as CFTypeRef);
            p
        };
        Ok(Self {
            pos,
            held: HashSet::new(),
        })
    }

    fn post(ev: CGEventRef) {
        unsafe {
            CGEventPost(HID_TAP, ev);
            CFRelease(ev as CFTypeRef);
        }
    }

    fn post_key(cg: u16, down: bool) {
        let ev = unsafe { CGEventCreateKeyboardEvent(std::ptr::null(), cg, down) };
        if !ev.is_null() {
            Self::post(ev);
        }
    }

    fn post_mouse(type_: u32, pt: CGPoint, btn: u32) {
        let ev = unsafe { CGEventCreateMouseEvent(std::ptr::null(), type_, pt, btn) };
        if !ev.is_null() {
            Self::post(ev);
        }
    }

    fn post_scroll(dx: i32, dy: i32) {
        let ev = unsafe { CGEventCreateScrollWheelEvent(std::ptr::null(), UNIT_LINE, 2, dy, dx) };
        if !ev.is_null() {
            Self::post(ev);
        }
    }

    fn btn_args(b: MouseButton) -> (u32, u32, u32) {
        // (down type, up type, CGMouseButton)
        match b {
            MouseButton::Left => (T_LDOWN, T_LUP, 0),
            MouseButton::Right => (T_RDOWN, T_RUP, 1),
            MouseButton::Middle | MouseButton::Side | MouseButton::Extra => {
                (T_ODOWN, T_OUP, 2)
            }
        }
    }

    fn mark(&mut self, code: u16, down: bool) {
        if down {
            self.held.insert(code);
        } else {
            self.held.remove(&code);
        }
    }

    pub fn inject(&mut self, ev: &Ev) -> io::Result<()> {
        match *ev {
            Ev::KeyDown(hid) | Ev::KeyUp(hid) => {
                let down = matches!(ev, Ev::KeyDown(_));
                if let Some(cg) = hid_to_evdev(hid).and_then(evdev_to_cgkey) {
                    Self::post_key(cg, down);
                    self.mark(hid, down);
                }
            }
            Ev::ButtonDown(b) | Ev::ButtonUp(b) => {
                let down = matches!(ev, Ev::ButtonDown(_));
                let (td, tu, btn) = Self::btn_args(b);
                Self::post_mouse(if down { td } else { tu }, self.pos, btn);
                self.mark(0x1000 + b as u16, down);
            }
            Ev::MouseMoveRel { dx, dy } => {
                self.pos.x += dx as f64;
                self.pos.y += dy as f64;
                // Post a drag event while a button is held so drags work.
                let (t, btn) = if self.held.contains(&(0x1000 + MouseButton::Left as u16)) {
                    (T_LDRAG, 0)
                } else if self.held.contains(&(0x1000 + MouseButton::Right as u16)) {
                    (T_RDRAG, 1)
                } else if self.held.iter().any(|c| *c >= 0x1000) {
                    (T_ODRAG, 2)
                } else {
                    (T_MOVED, 0)
                };
                Self::post_mouse(t, self.pos, btn);
            }
            Ev::Wheel { dx, dy } => Self::post_scroll(dx, dy),
        }
        Ok(())
    }

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
                let (_, tu, btn) = Self::btn_args(b);
                Self::post_mouse(tu, self.pos, btn);
            } else if let Some(cg) = hid_to_evdev(code).and_then(evdev_to_cgkey) {
                Self::post_key(cg, false);
            }
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl crate::InjectionBackend for CgInject {
    async fn inject(&mut self, ev: &Ev) -> io::Result<()> {
        CgInject::inject(self, ev)
    }

    async fn release_all(&mut self) -> io::Result<()> {
        CgInject::release_all(self)
    }
}
