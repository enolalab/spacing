use spacing_core::event::{InputEvent as Ev, MouseButton};
use spacing_core::hidmap::hid_to_evdev;

/// BTN_* evdev code for a wire button (pub(crate) — injector tracks held
/// buttons via these codes for release_all).
pub(crate) fn btn_code(b: MouseButton) -> u16 {
    match b {
        MouseButton::Left => 0x110,
        MouseButton::Right => 0x111,
        MouseButton::Middle => 0x112,
        MouseButton::Side => 0x113,
        MouseButton::Extra => 0x114,
    }
}

fn ev(type_: evdev::EventType, code: u16, value: i32) -> evdev::InputEvent {
    evdev::InputEvent::new(type_, code, value)
}

/// Expand one wire event into the evdev event sequence to emit
/// (input events + terminating SYN_REPORT).
pub fn to_evdev(wire: &Ev) -> Vec<evdev::InputEvent> {
    let mut out: Vec<evdev::InputEvent> = Vec::new();
    let mut push = |t: evdev::EventType, c: u16, v: i32| out.push(ev(t, c, v));
    match *wire {
        Ev::KeyDown(hid) => match hid_to_evdev(hid) {
            Some(c) => push(evdev::EventType::KEY, c, 1),
            None => return out,
        },
        Ev::KeyUp(hid) => match hid_to_evdev(hid) {
            Some(c) => push(evdev::EventType::KEY, c, 0),
            None => return out,
        },
        Ev::ButtonDown(b) => push(evdev::EventType::KEY, btn_code(b), 1),
        Ev::ButtonUp(b) => push(evdev::EventType::KEY, btn_code(b), 0),
        Ev::MouseMoveRel { dx, dy } => {
            if dx != 0 {
                push(evdev::EventType::RELATIVE, 0x00, dx); // REL_X
            }
            if dy != 0 {
                push(evdev::EventType::RELATIVE, 0x01, dy); // REL_Y
            }
        }
        Ev::Wheel { dx, dy } => {
            // lo-res detents + hi-res 1/120 units (like real mice).
            // REL_WHEEL=0x08, REL_HWHEEL=0x06, REL_WHEEL_HI_RES=0x0B, REL_HWHEEL_HI_RES=0x0C.
            // (0x08 with value*120 = scroll 120 detents — wrong!)
            if dy != 0 {
                push(evdev::EventType::RELATIVE, 0x08, dy);
                push(evdev::EventType::RELATIVE, 0x0B, dy * 120);
            }
            if dx != 0 {
                push(evdev::EventType::RELATIVE, 0x06, dx);
                push(evdev::EventType::RELATIVE, 0x0C, dx * 120);
            }
        }
    }
    if !out.is_empty() {
        out.push(ev(evdev::EventType::SYNCHRONIZATION, 0, 0)); // SYN_REPORT
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use spacing_core::event::{InputEvent as Ev, MouseButton};

    fn codes(evs: &[evdev::InputEvent]) -> Vec<(u16, u16, i32)> {
        evs.iter()
            .map(|e| (e.event_type().0, e.code(), e.value()))
            .collect()
    }

    #[test]
    fn keydown_emits_evkey_and_syn() {
        let out = to_evdev(&Ev::KeyDown(0x04)); // 'a' -> evdev 30
        let c = codes(&out);
        assert!(c.contains(&(evdev::EventType::KEY.0, 30, 1)));
        assert!(c
            .iter()
            .any(|(t, ..)| *t == evdev::EventType::SYNCHRONIZATION.0));
    }

    #[test]
    fn button_maps_to_btn_code() {
        let out = to_evdev(&Ev::ButtonDown(MouseButton::Right));
        assert!(codes(&out).contains(&(evdev::EventType::KEY.0, 0x111, 1)));
    }

    #[test]
    fn wheel_emits_detents_and_hires() {
        // like real mice: lo-res detent (±1) + hi-res 1/120 unit
        let out = to_evdev(&Ev::Wheel { dx: 0, dy: 1 });
        assert!(codes(&out).contains(&(evdev::EventType::RELATIVE.0, 0x08, 1))); // REL_WHEEL
        assert!(codes(&out).contains(&(evdev::EventType::RELATIVE.0, 0x0B, 120))); // REL_WHEEL_HI_RES
    }

    #[test]
    fn unmapped_key_yields_empty() {
        assert!(to_evdev(&Ev::KeyDown(0xFFFF)).is_empty());
    }
}
