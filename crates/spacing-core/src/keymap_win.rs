//! evdev ↔ Windows PS/2 set-1 scancode mapping.
//!
//! evdev key codes for the main block ARE PS/2 set-1 make codes, so the
//! identity mapping holds for most keys. Extended keys (nav cluster,
//! right modifiers, media keys) live behind the E0 prefix on Windows —
//! `SendInput` expresses them via `KEYEVENTF_EXTENDEDKEY`.
//!
//! Unmapped keys return `None` and are dropped, same policy as
//! `hidmap`.

/// Scancode pair: `(code, extended)` — `extended` ⇒ E0 prefix on the
/// wire / `KEYEVENTF_EXTENDEDKEY` for SendInput.
pub fn evdev_to_scan(ev: u16) -> Option<(u8, bool)> {
    if let Some(&(_e, c)) = EV_EXT.iter().find(|(e, _)| *e == ev) {
        return Some((c, true));
    }
    match ev {
        // PS/2 set-1 range — evdev code == scancode byte.
        1..=83 | 84..=88 | 91..=95 => Some((ev as u8, false)),
        _ => None,
    }
}

/// Reverse map. `(code, extended)` → evdev key code.
pub fn scan_to_evdev(code: u8, extended: bool) -> Option<u16> {
    if extended {
        return EV_EXT.iter().find(|(_, c)| *c == code).map(|(e, _)| *e);
    }
    match code as u16 {
        1..=83 | 84..=88 | 91..=95 => Some(code as u16),
        _ => None,
    }
}

/// Extended (E0-prefixed) keys: `(evdev code, E0 scancode byte)`.
///
/// evdev codes: 96=KPENTER 97=RCTRL 98=KPSLASH 99=SYSRQ 100=RALT
/// 102..=111=HOME..DELETE  113..=115 mute/vol 116 power
/// 121..=124 play/stop/prev/next  125/126 = L/R meta  127 menu
/// 163/165/166 next/prev song  172 homepage  173 refresh  128 stop
const EV_EXT: &[(u16, u8)] = &[
    (96, 0x1C),  // KPENTER
    (97, 0x1D),  // RIGHTCTRL
    (98, 0x35),  // KPSLASH
    (99, 0x37),  // SYSRQ / PRINTSCREEN
    (100, 0x38), // RIGHTALT
    (102, 0x47), // HOME
    (103, 0x48), // UP
    (104, 0x49), // PAGEUP
    (105, 0x4B), // LEFT
    (106, 0x4D), // RIGHT
    (107, 0x4F), // END
    (108, 0x50), // DOWN
    (109, 0x51), // PAGEDOWN
    (110, 0x52), // INSERT
    (111, 0x53), // DELETE
    (113, 0x20), // MUTE
    (114, 0x2E), // VOLUMEDOWN
    (115, 0x30), // VOLUMEUP
    (116, 0x5E), // POWER
    (121, 0x22), // PLAYPAUSE
    (122, 0x24), // STOPCD
    (123, 0x19), // NEXTSONG
    (124, 0x10), // PREVIOUSSONG
    (125, 0x5B), // LEFTMETA (Windows key)
    (126, 0x5C), // RIGHTMETA
    (127, 0x5D), // COMPOSE / context menu
    (172, 0x32), // HOMEPAGE
    (173, 0x67), // REFRESH (browser)
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_block_identity() {
        assert_eq!(evdev_to_scan(30), Some((0x1E, false))); // KEY_A
        assert_eq!(evdev_to_scan(28), Some((0x1C, false))); // ENTER
        assert_eq!(evdev_to_scan(1), Some((0x01, false))); // ESC
        assert_eq!(evdev_to_scan(57), Some((0x39, false))); // SPACE
    }

    #[test]
    fn extended_keys() {
        assert_eq!(evdev_to_scan(105), Some((0x4B, true))); // LEFT
        assert_eq!(evdev_to_scan(97), Some((0x1D, true))); // RCTRL
        assert_eq!(evdev_to_scan(125), Some((0x5B, true))); // LMETA
        assert_eq!(evdev_to_scan(111), Some((0x53, true))); // DELETE
    }

    #[test]
    fn roundtrip() {
        for ev in 1u16..=200 {
            if let Some((c, ext)) = evdev_to_scan(ev) {
                assert_eq!(scan_to_evdev(c, ext), Some(ev), "ev={ev}");
            }
        }
    }

    #[test]
    fn unmapped_dropped() {
        assert_eq!(evdev_to_scan(0), None);
        assert_eq!(evdev_to_scan(89), None); // evdev 89 unused
        assert_eq!(scan_to_evdev(0x00, false), None);
    }
}
