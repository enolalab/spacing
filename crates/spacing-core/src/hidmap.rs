/// (evdev code, hid usage) — keyboard page 0x07 only.
/// Unmapped evdev keys (media/consumer page) are intentionally dropped.
pub const EV_TO_HID: &[(u16, u16)] = &[
    (1, 0x29), (2, 0x1E), (3, 0x1F), (4, 0x20), (5, 0x21), (6, 0x22), (7, 0x23),
    (8, 0x24), (9, 0x25), (10, 0x26), (11, 0x27), (12, 0x2D), (13, 0x2E),
    (14, 0x2A), (15, 0x2B), (16, 0x14), (17, 0x1A), (18, 0x08), (19, 0x15),
    (20, 0x17), (21, 0x1C), (22, 0x18), (23, 0x0C), (24, 0x12), (25, 0x13),
    (26, 0x2F), (27, 0x30), (28, 0x28), (29, 0xE0), (30, 0x04), (31, 0x16),
    (32, 0x07), (33, 0x09), (34, 0x0A), (35, 0x0B), (36, 0x0D), (37, 0x0E),
    (38, 0x0F), (39, 0x33), (40, 0x34), (41, 0x35), (42, 0xE1), (43, 0x31),
    (44, 0x1D), (45, 0x1B), (46, 0x06), (47, 0x19), (48, 0x05), (49, 0x11),
    (50, 0x10), (51, 0x36), (52, 0x37), (53, 0x38), (54, 0xE5), (55, 0x55),
    (56, 0xE2), (57, 0x2C), (58, 0x39), (59, 0x3A), (60, 0x3B), (61, 0x3C),
    (62, 0x3D), (63, 0x3E), (64, 0x3F), (65, 0x40), (66, 0x41), (67, 0x42),
    (68, 0x43), (69, 0x53), (70, 0x47), (71, 0x5F), (72, 0x60), (73, 0x61),
    (74, 0x56), (75, 0x5C), (76, 0x5D), (77, 0x5E), (78, 0x57), (79, 0x59),
    (80, 0x5A), (81, 0x5B), (82, 0x62), (83, 0x63), (87, 0x44), (88, 0x45),
    (96, 0x58), (97, 0xE4), (98, 0x54), (99, 0x46), (100, 0xE6),
    (102, 0x4A), (103, 0x52), (104, 0x4B), (105, 0x50), (106, 0x4F),
    (107, 0x4D), (108, 0x51), (109, 0x4E), (110, 0x49), (111, 0x4C),
    (117, 0x67), (119, 0x48), (121, 0x85), (125, 0xE3), (126, 0xE7),
    (127, 0x65), (179, 0xB6), (180, 0xB7),
];

pub fn evdev_to_hid(ev: u16) -> Option<u16> {
    EV_TO_HID.iter().find(|(e, _)| *e == ev).map(|(_, h)| *h)
}

pub fn hid_to_evdev(hid: u16) -> Option<u16> {
    EV_TO_HID.iter().find(|(_, h)| *h == hid).map(|(e, _)| *e)
}

/// (config name, hid usage) — names for hotkey config parsing.
pub const KEY_NAMES: &[(&str, u16)] = &[
    ("esc", 0x29), ("tab", 0x2B), ("enter", 0x28), ("space", 0x2C),
    ("backspace", 0x2A), ("capslock", 0x39), ("delete", 0x4C),
    ("insert", 0x49), ("home", 0x4A), ("end", 0x4D), ("pageup", 0x4B),
    ("pagedown", 0x4E), ("up", 0x52), ("down", 0x51), ("left", 0x50),
    ("right", 0x4F), ("numlock", 0x53), ("scrolllock", 0x47),
    ("printscreen", 0x46), ("pause", 0x48), ("menu", 0x65),
    ("kpenter", 0x58), ("kpdot", 0x63), ("kpplus", 0x57), ("kpminus", 0x56),
    ("kpslash", 0x54), ("kpasterisk", 0x55), ("kpcomma", 0x85),
    ("minus", 0x2D), ("equal", 0x2E), ("leftbrace", 0x2F), ("rightbrace", 0x30),
    ("semicolon", 0x33), ("apostrophe", 0x34), ("grave", 0x35),
    ("backslash", 0x31), ("comma", 0x36), ("dot", 0x37), ("slash", 0x38),
    ("f1", 0x3A), ("f2", 0x3B), ("f3", 0x3C), ("f4", 0x3D), ("f5", 0x3E),
    ("f6", 0x3F), ("f7", 0x40), ("f8", 0x41), ("f9", 0x42), ("f10", 0x43),
    ("f11", 0x44), ("f12", 0x45),
    ("1", 0x1E), ("2", 0x1F), ("3", 0x20), ("4", 0x21), ("5", 0x22),
    ("6", 0x23), ("7", 0x24), ("8", 0x25), ("9", 0x26), ("0", 0x27),
    ("a", 0x04), ("b", 0x05), ("c", 0x06), ("d", 0x07), ("e", 0x08),
    ("f", 0x09), ("g", 0x0A), ("h", 0x0B), ("i", 0x0C), ("j", 0x0D),
    ("k", 0x0E), ("l", 0x0F), ("m", 0x10), ("n", 0x11), ("o", 0x12),
    ("p", 0x13), ("q", 0x14), ("r", 0x15), ("s", 0x16), ("t", 0x17),
    ("u", 0x18), ("v", 0x19), ("w", 0x1A), ("x", 0x1B), ("y", 0x1C),
    ("z", 0x1D),
    ("kp0", 0x62), ("kp1", 0x59), ("kp2", 0x5A), ("kp3", 0x5B),
    ("kp4", 0x5C), ("kp5", 0x5D), ("kp6", 0x5E), ("kp7", 0x5F),
    ("kp8", 0x60), ("kp9", 0x61),
];

pub fn key_name_to_hid(name: &str) -> Option<u16> {
    let n = name.to_ascii_lowercase();
    KEY_NAMES.iter().find(|(k, _)| *k == n).map(|(_, h)| *h)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_common_keys() {
        for ev in [30u16, 2, 28, 42, 103, 59, 87] {
            let hid = evdev_to_hid(ev).expect("mapped");
            assert_eq!(hid_to_evdev(hid), Some(ev));
        }
    }

    #[test]
    fn modifiers_have_expected_usages() {
        assert_eq!(evdev_to_hid(29), Some(0xE0)); // LCTRL
        assert_eq!(evdev_to_hid(56), Some(0xE2)); // LALT
        assert_eq!(evdev_to_hid(42), Some(0xE1)); // LSHIFT
        assert_eq!(evdev_to_hid(125), Some(0xE3)); // LMETA
    }

    #[test]
    fn name_lookup() {
        assert_eq!(key_name_to_hid("F5"), Some(0x3E));
        assert_eq!(key_name_to_hid("1"), Some(0x1E));
        assert_eq!(key_name_to_hid("nope"), None);
    }

    proptest::proptest! {
        #[test]
        fn any_mapped_key_roundtrips(i in 0..EV_TO_HID.len()) {
            let (ev, hid) = EV_TO_HID[i];
            proptest::prop_assert_eq!(evdev_to_hid(ev), Some(hid));
            proptest::prop_assert_eq!(hid_to_evdev(hid), Some(ev));
        }
    }
}
