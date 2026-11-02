use crate::hidmap::key_name_to_hid;
use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Physical modifier mask (left/right collapsed).
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ModSet {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl ModSet {
    /// HID usages E0..E7 are the 8 modifier keys.
    pub fn from_hid(hid: u16) -> Option<ModSet> {
        let mut m = ModSet::default();
        match hid {
            0xE0 | 0xE4 => m.ctrl = true,
            0xE1 | 0xE5 => m.shift = true,
            0xE2 | 0xE6 => m.alt = true,
            0xE3 | 0xE7 => m.meta = true,
            _ => return None,
        }
        Some(m)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Combo {
    pub mods: ModSet,
    pub key: u16, // HID usage
}

#[derive(Debug, Error, PartialEq)]
pub enum ComboError {
    #[error("empty combo")]
    Empty,
    #[error("unknown key name: {0}")]
    UnknownKey(String),
    #[error("combo has no non-modifier key")]
    NoKey,
}

/// Parses "Ctrl+Alt+1" style combos. Modifier names: ctrl, alt, shift,
/// meta|super|win|cmd. Last token must be a non-modifier key name.
pub fn parse_combo(s: &str) -> Result<Combo, ComboError> {
    let s = s.trim();
    if s.is_empty() {
        return Err(ComboError::Empty);
    }
    let last = s.split('+').count() - 1;
    let mut mods = ModSet::default();
    let mut key = None;
    for (i, tok) in s.split('+').enumerate() {
        let t = tok.trim().to_ascii_lowercase();
        match t.as_str() {
            "ctrl" | "control" => mods.ctrl = true,
            "alt" => mods.alt = true,
            "shift" => mods.shift = true,
            "meta" | "super" | "win" | "cmd" => mods.meta = true,
            other => {
                if i != last {
                    return Err(ComboError::UnknownKey(other.into()));
                }
                key = Some(
                    key_name_to_hid(other).ok_or_else(|| ComboError::UnknownKey(other.into()))?,
                );
            }
        }
    }
    match key {
        Some(k) => Ok(Combo { mods, key: k }),
        None => Err(ComboError::NoKey),
    }
}
