//! libei (ei) wire helpers shared by the portal capture backend
//! (ei *receiver* on an InputCapture socket) and the portal inject
//! backend (ei *sender* on a RemoteDesktop socket).
//!
//! ei key/button codes are evdev `input-event-codes.h` values — our
//! canonical wire format is USB HID usage IDs, so keys convert through
//! `spacing_core::hidmap`. Scroll: `scroll_discrete` units are 1/120 of
//! a wheel detent; `scroll` deltas are logical pixels (~15 px/detent).
//!
//! Set `REIS_DEBUG=1` to dump ei messages (off by default — they would
//! otherwise leak keystrokes to the log, same rule as SPACING_WIRE_DUMP).

use reis::ei;
use reis::event::EiEvent;
use spacing_core::event::{InputEvent, MouseButton};
use spacing_core::hidmap::{evdev_to_hid, hid_to_evdev};

/// libinput px convention for one wheel detent — smooth-scroll deltas
/// arrive in logical pixels; verify against real compositors (GNOME/KDE)
/// before trusting sub-detent fidelity.
const SCROLL_PX_PER_DETENT: f32 = 15.0;

/// Accumulating decoder for ei receiver events. Accumulates sub-detent
/// scroll amounts so slow smooth-scroll still produces wheel detents.
#[derive(Default)]
pub struct EiDecoder {
    discrete_acc: [i32; 2],
    smooth_acc: [f32; 2],
}

impl EiDecoder {
    /// Translate one high-level ei event into a wire `InputEvent`.
    /// Returns `None` for non-input events and sub-detent remnants.
    pub fn decode(&mut self, ev: &EiEvent) -> Option<InputEvent> {
        match ev {
            EiEvent::KeyboardKey(e) => {
                let hid = evdev_to_hid(u16::try_from(e.key).ok()?)?;
                Some(match e.state {
                    ei::keyboard::KeyState::Press => InputEvent::KeyDown(hid),
                    ei::keyboard::KeyState::Released => InputEvent::KeyUp(hid),
                })
            }
            EiEvent::Button(e) => {
                let b = btn_from_code(e.button)?;
                Some(match e.state {
                    ei::button::ButtonState::Press => InputEvent::ButtonDown(b),
                    ei::button::ButtonState::Released => InputEvent::ButtonUp(b),
                })
            }
            EiEvent::PointerMotion(e) => Some(InputEvent::MouseMoveRel {
                dx: e.dx as i32,
                dy: e.dy as i32,
            }),
            EiEvent::ScrollDiscrete(e) => {
                let (dx, dy) =
                    take_detents(&mut self.discrete_acc, e.discrete_dx, e.discrete_dy, 120);
                (dx != 0 || dy != 0).then_some(InputEvent::Wheel { dx, dy })
            }
            EiEvent::ScrollDelta(e) => {
                let (dx, dy) =
                    take_detents_f(&mut self.smooth_acc, e.dx, e.dy, SCROLL_PX_PER_DETENT);
                (dx != 0 || dy != 0).then_some(InputEvent::Wheel { dx, dy })
            }
            _ => None,
        }
    }
}

fn take_detents(acc: &mut [i32; 2], x: i32, y: i32, per: i32) -> (i32, i32) {
    acc[0] += x;
    acc[1] += y;
    let dx = acc[0] / per;
    let dy = acc[1] / per;
    acc[0] -= dx * per;
    acc[1] -= dy * per;
    (dx, dy)
}

fn take_detents_f(acc: &mut [f32; 2], x: f32, y: f32, per: f32) -> (i32, i32) {
    acc[0] += x;
    acc[1] += y;
    let dx = (acc[0] / per).trunc() as i32;
    let dy = (acc[1] / per).trunc() as i32;
    acc[0] -= dx as f32 * per;
    acc[1] -= dy as f32 * per;
    (dx, dy)
}

fn btn_from_code(code: u32) -> Option<MouseButton> {
    Some(match code {
        0x110 => MouseButton::Left,
        0x111 => MouseButton::Right,
        0x112 => MouseButton::Middle,
        0x113 => MouseButton::Side,
        0x114 => MouseButton::Extra,
        _ => return None,
    })
}

/// evdev BTN_* code for a wire mouse button.
pub fn btn_code(b: MouseButton) -> u32 {
    match b {
        MouseButton::Left => 0x110,
        MouseButton::Right => 0x111,
        MouseButton::Middle => 0x112,
        MouseButton::Side => 0x113,
        MouseButton::Extra => 0x114,
    }
}

/// The device interfaces a sender may emit through — bound after the
/// ei handshake when device objects arrive. All are optional: a sender
/// only emits what the remote seat advertised.
#[derive(Clone, Default)]
pub struct EiInterfaces {
    pub keyboard: Option<ei::Keyboard>,
    pub pointer: Option<ei::Pointer>,
    pub button: Option<ei::Button>,
    pub scroll: Option<ei::Scroll>,
}

impl EiInterfaces {
    /// Emit one `InputEvent`. The caller owns `start_emulating`/`frame`
    /// sequencing — batch related events between frames.
    pub fn emit(&self, ev: &InputEvent) {
        match *ev {
            InputEvent::KeyDown(hid) | InputEvent::KeyUp(hid) => {
                let (Some(kb), Some(code)) = (&self.keyboard, hid_to_evdev(hid)) else {
                    return;
                };
                let state = if matches!(ev, InputEvent::KeyDown(_)) {
                    ei::keyboard::KeyState::Press
                } else {
                    ei::keyboard::KeyState::Released
                };
                kb.key(u32::from(code), state);
            }
            InputEvent::ButtonDown(b) | InputEvent::ButtonUp(b) => {
                let Some(btn) = &self.button else { return };
                let state = if matches!(ev, InputEvent::ButtonDown(_)) {
                    ei::button::ButtonState::Press
                } else {
                    ei::button::ButtonState::Released
                };
                btn.button(btn_code(b), state);
            }
            InputEvent::MouseMoveRel { dx, dy } => {
                if let Some(p) = &self.pointer {
                    p.motion_relative(dx as f32, dy as f32);
                }
            }
            InputEvent::Wheel { dx, dy } => {
                // ei protocol forbids mixing scroll + scroll_discrete in
                // one frame — send discrete only; the EIS side emulates
                // smooth deltas. Units are 1/120 per wheel detent.
                if let Some(s) = &self.scroll {
                    s.scroll_discrete(dx * 120, dy * 120);
                }
            }
        }
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn detents_accumulate_sub_unit_remainders() {
        let mut acc = [0f32; 2];
        // 7.5 px = half a detent — nothing emitted yet
        assert_eq!(take_detents_f(&mut acc, 0.0, 7.5, 15.0), (0, 0));
        // second chunk pushes past one detent; remainder carries
        assert_eq!(take_detents_f(&mut acc, 0.0, 8.0, 15.0), (0, 1));
        // negative direction also truncates toward zero correctly
        assert_eq!(take_detents_f(&mut acc, 0.0, -20.0, 15.0), (0, -1));
    }

    #[test]
    fn discrete_accumulates_1_120_units() {
        let mut acc = [0i32; 2];
        assert_eq!(take_detents(&mut acc, 60, 0, 120), (0, 0));
        assert_eq!(take_detents(&mut acc, 60, -240, 120), (1, -2));
    }
}
