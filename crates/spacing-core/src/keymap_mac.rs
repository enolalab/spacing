//! evdev ↔ macOS `CGKeyCode` mapping.
//!
//! `CGKeyCode` is Apple's hardware key numbering (virtual key codes
//! from `Events.h`) — layout-independent, like evdev. Table covers the
//! alphanumeric block, modifiers, function keys, nav cluster and the
//! main keypad keys. Unmapped keys (brightness, media, etc.) return
//! `None` and are dropped.

/// `(evdev code, CGKeyCode)` — CG codes from `Events.h`/`HIToolbox`.
const TABLE: &[(u16, u16)] = &[
    (30, 0x00),  // KEY_A          → kVK_ANSI_A
    (48, 0x0B),  // KEY_B          → kVK_ANSI_B
    (46, 0x08),  // KEY_C          → kVK_ANSI_C
    (32, 0x02),  // KEY_D          → kVK_ANSI_D
    (18, 0x0E),  // KEY_E          → kVK_ANSI_E
    (33, 0x03),  // KEY_F          → kVK_ANSI_F
    (34, 0x05),  // KEY_G          → kVK_ANSI_G
    (35, 0x04),  // KEY_H          → kVK_ANSI_H
    (23, 0x22),  // KEY_I          → kVK_ANSI_I
    (36, 0x26),  // KEY_J          → kVK_ANSI_J
    (37, 0x28),  // KEY_K          → kVK_ANSI_K
    (38, 0x25),  // KEY_L          → kVK_ANSI_L
    (50, 0x2E),  // KEY_M          → kVK_ANSI_M
    (49, 0x2D),  // KEY_N          → kVK_ANSI_N
    (24, 0x1F),  // KEY_O          → kVK_ANSI_O
    (25, 0x23),  // KEY_P          → kVK_ANSI_P
    (16, 0x0C),  // KEY_Q          → kVK_ANSI_Q
    (19, 0x0F),  // KEY_R          → kVK_ANSI_R
    (31, 0x01),  // KEY_S          → kVK_ANSI_S
    (20, 0x11),  // KEY_T          → kVK_ANSI_T
    (22, 0x20),  // KEY_U          → kVK_ANSI_U
    (47, 0x09),  // KEY_V          → kVK_ANSI_V
    (17, 0x0D),  // KEY_W          → kVK_ANSI_W
    (45, 0x07),  // KEY_X          → kVK_ANSI_X
    (21, 0x10),  // KEY_Y          → kVK_ANSI_Y
    (44, 0x06),  // KEY_Z          → kVK_ANSI_Z
    // Number row
    (2, 0x12),   // KEY_1          → kVK_ANSI_1
    (3, 0x13),   // KEY_2
    (4, 0x14),   // KEY_3
    (5, 0x15),   // KEY_4
    (6, 0x17),   // KEY_5
    (7, 0x16),   // KEY_6
    (8, 0x1A),   // KEY_7
    (9, 0x1C),   // KEY_8
    (10, 0x19),  // KEY_9
    (11, 0x1D),  // KEY_0
    (12, 0x1B),  // KEY_MINUS      → kVK_ANSI_Minus
    (13, 0x18),  // KEY_EQUAL      → kVK_ANSI_Equal
    // Punctuation
    (26, 0x21),  // KEY_LEFTBRACE  → kVK_ANSI_LeftBracket
    (27, 0x1E),  // KEY_RIGHTBRACE → kVK_ANSI_RightBracket
    (39, 0x29),  // KEY_SEMICOLON  → kVK_ANSI_Semicolon
    (40, 0x27),  // KEY_APOSTROPHE → kVK_ANSI_Quote
    (41, 0x32),  // KEY_GRAVE      → kVK_ANSI_Grave
    (43, 0x2A),  // KEY_BACKSLASH  → kVK_ANSI_Backslash
    (51, 0x2B),  // KEY_COMMA      → kVK_ANSI_Comma
    (52, 0x2F),  // KEY_DOT        → kVK_ANSI_Period
    (53, 0x2C),  // KEY_SLASH      → kVK_ANSI_Slash
    // Whitespace / editing
    (1, 0x35),   // KEY_ESC        → kVK_Escape
    (15, 0x30),  // KEY_TAB        → kVK_Tab
    (57, 0x31),  // KEY_SPACE      → kVK_Space
    (28, 0x24),  // KEY_ENTER      → kVK_Return
    (14, 0x33),  // KEY_BACKSPACE  → kVK_Delete
    (58, 0x39),  // KEY_CAPSLOCK   → kVK_CapsLock
    // Modifiers (CG codes are device-side; FlagsChanged carries them)
    (29, 0x3B),  // KEY_LEFTCTRL   → kVK_Control
    (97, 0x3E),  // KEY_RIGHTCTRL  → kVK_RightControl
    (42, 0x38),  // KEY_LEFTSHIFT  → kVK_Shift
    (54, 0x3C),  // KEY_RIGHTSHIFT → kVK_RightShift
    (56, 0x3A),  // KEY_LEFTALT    → kVK_Option
    (100, 0x3D), // KEY_RIGHTALT   → kVK_RightOption
    (125, 0x37), // KEY_LEFTMETA   → kVK_Command
    (126, 0x36), // KEY_RIGHTMETA  → kVK_RightCommand
    (127, 0x3F), // KEY_COMPOSE    → kVK_Function
    // Function row
    (59, 0x7A),  // KEY_F1
    (60, 0x78),  // KEY_F2
    (61, 0x63),  // KEY_F3
    (62, 0x76),  // KEY_F4
    (63, 0x60),  // KEY_F5
    (64, 0x61),  // KEY_F6
    (65, 0x62),  // KEY_F7
    (66, 0x64),  // KEY_F8
    (67, 0x65),  // KEY_F9
    (68, 0x6D),  // KEY_F10
    (87, 0x67),  // KEY_F11
    (88, 0x6F),  // KEY_F12
    (183, 0x69), // KEY_F13
    (184, 0x6B), // KEY_F14
    (185, 0x71), // KEY_F15
    (186, 0x6A), // KEY_F16
    (187, 0x40), // KEY_F17
    (188, 0x4F), // KEY_F18
    (189, 0x50), // KEY_F19
    (190, 0x5A), // KEY_F20
    // Nav cluster
    (110, 0x72), // KEY_INSERT     → kVK_Help
    (102, 0x73), // KEY_HOME       → kVK_Home
    (107, 0x77), // KEY_END        → kVK_End
    (104, 0x74), // KEY_PAGEUP     → kVK_PageUp
    (109, 0x79), // KEY_PAGEDOWN   → kVK_PageDown
    (111, 0x75), // KEY_DELETE     → kVK_ForwardDelete
    (103, 0x7E), // KEY_UP         → kVK_UpArrow
    (108, 0x7D), // KEY_DOWN       → kVK_DownArrow
    (105, 0x7B), // KEY_LEFT       → kVK_LeftArrow
    (106, 0x7C), // KEY_RIGHT      → kVK_RightArrow
    // KEY_SYSRQ intentionally unmapped — kVK_F13 (0x69) already taken.
    // Keypad
    (69, 0x47),  // KEY_NUMLOCK    → kVK_ANSI_KeypadClear
    (98, 0x4B),  // KEY_KPSLASH    → kVK_ANSI_KeypadDivide
    (55, 0x43),  // KEY_KPASTERISK → kVK_ANSI_KeypadMultiply
    (74, 0x4E),  // KEY_KPMINUS    → kVK_ANSI_KeypadMinus
    (78, 0x45),  // KEY_KPPLUS     → kVK_ANSI_KeypadPlus
    (96, 0x4C),  // KEY_KPENTER    → kVK_ANSI_KeypadEnter
    (83, 0x41),  // KEY_KPDOT      → kVK_ANSI_KeypadDecimal
    (82, 0x52),  // KEY_KP0
    (79, 0x53),  // KEY_KP1
    (80, 0x54),  // KEY_KP2
    (81, 0x55),  // KEY_KP3
    (75, 0x56),  // KEY_KP4
    (76, 0x57),  // KEY_KP5
    (77, 0x58),  // KEY_KP6
    (71, 0x59),  // KEY_KP7
    (72, 0x5B),  // KEY_KP8
    (73, 0x5C),  // KEY_KP9
    (117, 0x51), // KEY_KPEQUAL    → kVK_ANSI_KeypadEquals
];

pub fn evdev_to_cgkey(ev: u16) -> Option<u16> {
    TABLE.iter().find(|(e, _)| *e == ev).map(|(_, c)| *c)
}

pub fn cgkey_to_evdev(cg: u16) -> Option<u16> {
    TABLE.iter().find(|(_, c)| *c == cg).map(|(e, _)| *e)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_and_mods() {
        assert_eq!(evdev_to_cgkey(30), Some(0x00)); // A
        assert_eq!(evdev_to_cgkey(31), Some(0x01)); // S
        assert_eq!(evdev_to_cgkey(29), Some(0x3B)); // LCTRL → Control
        assert_eq!(evdev_to_cgkey(125), Some(0x37)); // LMETA → Command
    }

    #[test]
    fn nav_and_function() {
        assert_eq!(evdev_to_cgkey(103), Some(0x7E)); // UP
        assert_eq!(evdev_to_cgkey(59), Some(0x7A)); // F1
        assert_eq!(evdev_to_cgkey(111), Some(0x75)); // DELETE
    }

    #[test]
    fn roundtrip() {
        for (e, c) in TABLE {
            assert_eq!(cgkey_to_evdev(*c), Some(*e));
        }
    }

    #[test]
    fn unmapped_dropped() {
        assert_eq!(evdev_to_cgkey(0), None);
        assert_eq!(evdev_to_cgkey(224), None); // brightness etc.
        assert_eq!(cgkey_to_evdev(0x7F), None);
    }
}
