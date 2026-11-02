use spacing_core::event::{InputEvent as Ev, MouseButton};
use spacing_core::hidmap::evdev_to_hid;

/// Map a raw evdev event to the canonical wire event.
/// Returns None for events we intentionally drop (autorepeat, sync,
/// unmapped/media keys, absolute axes).
pub fn map_event(ev: evdev::InputEvent) -> Option<Ev> {
    match (ev.event_type(), ev.code(), ev.value()) {
        (evdev::EventType::KEY, code, value) => key(code, value),
        (evdev::EventType::RELATIVE, code, value) => rel(code, value),
        _ => None,
    }
}

fn key(code: u16, value: i32) -> Option<Ev> {
    const BTN_LEFT: u16 = 0x110;
    const BTN_RIGHT: u16 = 0x111;
    const BTN_MIDDLE: u16 = 0x112;
    const BTN_SIDE: u16 = 0x113;
    const BTN_EXTRA: u16 = 0x114;

    let btn = |code: u16| -> Option<MouseButton> {
        Some(match code {
            BTN_LEFT => MouseButton::Left,
            BTN_RIGHT => MouseButton::Right,
            BTN_MIDDLE => MouseButton::Middle,
            BTN_SIDE => MouseButton::Side,
            BTN_EXTRA => MouseButton::Extra,
            _ => return None,
        })
    };

    match (btn(code), value) {
        (Some(b), 1) => return Some(Ev::ButtonDown(b)),
        (Some(b), 0) => return Some(Ev::ButtonUp(b)),
        (Some(_), _) => return None, // repeat
        (None, _) => {}
    }
    match (evdev_to_hid(code), value) {
        (Some(hid), 1) => Some(Ev::KeyDown(hid)),
        (Some(hid), 0) => Some(Ev::KeyUp(hid)),
        _ => None, // repeat(2) or unmapped key
    }
}

fn rel(code: u16, value: i32) -> Option<Ev> {
    match code {
        0x00 => Some(Ev::MouseMoveRel { dx: value, dy: 0 }), // REL_X
        0x01 => Some(Ev::MouseMoveRel { dx: 0, dy: value }), // REL_Y
        0x08 => Some(Ev::Wheel { dx: 0, dy: value }),        // REL_WHEEL
        0x06 => Some(Ev::Wheel { dx: value, dy: 0 }),        // REL_HWHEEL
        _ => None,                                         // REL_WHEEL_HI_RES etc.
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use spacing_core::event::{InputEvent as Ev, MouseButton};

    fn ev(type_: u16, code: u16, value: i32) -> evdev::InputEvent {
        evdev::InputEvent::new(evdev::EventType(type_), code, value)
    }

    #[test]
    fn key_a_maps_to_hid() {
        assert_eq!(
            map_event(ev(evdev::EventType::KEY.0, 30, 1)),
            Some(Ev::KeyDown(0x04))
        );
        assert_eq!(
            map_event(ev(evdev::EventType::KEY.0, 30, 0)),
            Some(Ev::KeyUp(0x04))
        );
    }

    #[test]
    fn key_repeat_is_dropped() {
        assert_eq!(map_event(ev(evdev::EventType::KEY.0, 30, 2)), None);
    }

    #[test]
    fn mouse_buttons_map() {
        assert_eq!(
            map_event(ev(evdev::EventType::KEY.0, 0x110, 1)),
            Some(Ev::ButtonDown(MouseButton::Left))
        );
        assert_eq!(
            map_event(ev(evdev::EventType::KEY.0, 0x113, 0)),
            Some(Ev::ButtonUp(MouseButton::Side))
        );
    }

    #[test]
    fn rel_motion_maps() {
        assert_eq!(
            map_event(ev(evdev::EventType::RELATIVE.0, 0x00, 5)),
            Some(Ev::MouseMoveRel { dx: 5, dy: 0 })
        );
        assert_eq!(
            map_event(ev(evdev::EventType::RELATIVE.0, 0x08, 1)),
            Some(Ev::Wheel { dx: 0, dy: 1 })
        );
    }
}
