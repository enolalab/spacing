use serde::{Deserialize, Serialize};

/// USB mouse button index (button page 0x09 is 1-based; we use 0-based slots).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MouseButton {
    Left,
    Right,
    Middle,
    Side,
    Extra,
}

/// Wire-canonical input event. Keys are USB HID usage IDs (page 0x07).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum InputEvent {
    KeyDown(u16),
    KeyUp(u16),
    MouseMoveRel { dx: i32, dy: i32 },
    ButtonDown(MouseButton),
    ButtonUp(MouseButton),
    /// Wheel detents (±1 per notch); injector multiplies to hi-res units.
    Wheel { dx: i32, dy: i32 },
}
