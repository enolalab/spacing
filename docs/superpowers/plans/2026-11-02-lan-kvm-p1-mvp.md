# spacing P1 — Linux↔Linux Hotkey-Only MVP Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Một daemon Rust cho phép máy "server" (có phím/chuột vật lý) forward input qua LAN sang máy "client" Linux bằng hotkey, với kết nối TLS + fingerprint pairing.

**Architecture:** Star topology 1 server ↔ N client. Capture = evdev grab, inject = uinput virtual device (kernel-level, chạy được trên cả Wayland lẫn X11). Transport = TCP + TLS1.3 (rustls, cert tự ký, mutual auth qua fingerprint pinning). Canonical wire key = USB HID usage ID. Clipboard, portal backend, edge-switch, Windows/macOS KHÔNG nằm trong plan này (P2+).

**Tech Stack:** Rust 2021, tokio, evdev 0.12 (capture + uinput), rustls 0.23 + tokio-rustls, rcgen (self-signed certs), serde + postcard (framing), clap, toml, tracing.

Spec nguồn: `docs/superpowers/specs/2026-11-02-lan-kvm-design.md`

## Global Constraints

- Edition `2021`, `cargo test --workspace` phải pass ở mọi task (`default-members=["."]` nên `cargo test` trần chỉ chạy test của bin `spacing` — luôn dùng `--workspace` hoặc `-p <crate>`); `cargo clippy` không warning mới.
- Wire: TCP + TLS1.3, frame = `u32 len` (LE) + `postcard` payload, max frame 2 MiB.
- Canonical key trên wire = **USB HID usage ID** (keyboard page 0x07), `u16`.
- Heartbeat 250ms khi remote active; client watchdog 500ms → `release_all()`.
- Nguyên tắc vàng: lỗi/mất kết nối → server về `Local` + ungrab ngay, không bao giờ kẹt input.
- Keystroke KHÔNG được log. Log fingerprint, hostname, transitions — không log key data.
- deps (version tối thiểu, đã >7 ngày): serde 1, postcard 1 (alloc), tokio 1 (full), evdev 0.12 (tokio feature), rustls 0.23, tokio-rustls 0.26, rustls-pemfile 2, rcgen 0.13, sha2 0.10, futures 0.3, clap 4 (derive), toml 0.8, dirs 5, anyhow 1, thiserror 2, tracing 0.1, tracing-subscriber 0.3 (env-filter). Dev: proptest 1, tempfile 3.
- Chỉ hỗ trợ phím thuộc keyboard page 0x07; media/consumer keys (volume...) bị drop ở capture — giới hạn đã biết của P1.

---

### Task 1: Workspace scaffold + `spacing-core` protocol types + HID map

**Files:**
- Create: `Cargo.toml` (workspace), `rust-toolchain.toml`
- Create: `crates/spacing-core/Cargo.toml`
- Create: `crates/spacing-core/src/lib.rs`
- Create: `crates/spacing-core/src/event.rs`
- Create: `crates/spacing-core/src/protocol.rs`
- Create: `crates/spacing-core/src/hidmap.rs`
- Create: `crates/spacing-net/Cargo.toml` + `crates/spacing-net/src/lib.rs` (skeleton rỗng — impl ở Task 4)
- Create: `crates/spacing-capture/Cargo.toml` + `crates/spacing-capture/src/lib.rs` (skeleton — Task 5)
- Create: `crates/spacing-inject/Cargo.toml` + `crates/spacing-inject/src/lib.rs` (skeleton — Task 6)
- Test: `crates/spacing-core/src/lib.rs` (module tests inline)

> Skeleton crates là BẮT BUỘC: root `[dependencies]` trỏ path tới cả 4 crate — thiếu manifest nào workspace không parse, `cargo test -p spacing-core` cũng fail.

**Interfaces:**
- Produces (cả plan dùng): `spacing_core::event::{InputEvent, MouseButton}`; `spacing_core::protocol::{Message, PROTOCOL_VERSION, CLIPBOARD_MAX}`; `spacing_core::hidmap::{evdev_to_hid, hid_to_evdev, KEY_NAMES, key_name_to_hid}`.
- `InputEvent::KeyDown(u16)/KeyUp(u16)` mang HID usage; `Wheel{dx,dy}` đơn vị detent.

- [ ] **Step 1: Tạo workspace + test thất bại**

`Cargo.toml` (root):

```toml
[workspace]
resolver = "2"
members = ["crates/*", "."]
default-members = ["."]

[workspace.package]
edition = "2021"
license = "MIT"
```

`rust-toolchain.toml`:

```toml
[toolchain]
channel = "stable"
components = ["rustfmt", "clippy"]
```

Root `Cargo.toml` (package `spacing`, bin ở `src/main.rs` — tạo stub `src/main.rs` = `fn main() {}`):

```toml
[package]
name = "spacing"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
spacing-core = { path = "crates/spacing-core" }
spacing-net = { path = "crates/spacing-net" }
spacing-capture = { path = "crates/spacing-capture" }
spacing-inject = { path = "crates/spacing-inject" }
clap = { version = "4", features = ["derive"] }
toml = "0.8"
dirs = "5"
anyhow = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tokio = { version = "1", features = ["full"] }
futures = "0.3"
serde = { version = "1", features = ["derive"] }
gethostname = "0.5"
```

`crates/spacing-core/Cargo.toml`:

```toml
[package]
name = "spacing-core"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
serde = { version = "1", features = ["derive"] }
postcard = { version = "1", features = ["alloc"] }

[dev-dependencies]
proptest = "1"
```

Skeleton manifests (giống nhau cho `spacing-net`, `spacing-capture`, `spacing-inject` — đổi `name`; `[dependencies]` để trống, điền ở task tương ứng):

```toml
[package]
name = "spacing-net"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
```

Mỗi `src/lib.rs` skeleton chỉ chứa một dòng comment `// filled in Task N` để file parse.

Test đầu tiên — `crates/spacing-core/src/hidmap.rs` phần test (tạo file với chỉ test, compile fail vì thiếu impl):

```rust
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
}
```

- [ ] **Step 2: Chạy test, verify fail**

Run: `cargo test -p spacing-core`
Expected: FAIL — `cannot find function evdev_to_hid` (file chưa có impl).

- [ ] **Step 3: Viết implementation đầy đủ**

`crates/spacing-core/src/lib.rs`:

```rust
pub mod event;
pub mod hidmap;
pub mod protocol;
```

`crates/spacing-core/src/event.rs`:

```rust
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
```

`crates/spacing-core/src/protocol.rs`:

```rust
use crate::event::InputEvent;
use serde::{Deserialize, Serialize};

pub const PROTOCOL_VERSION: u16 = 1;
pub const CLIPBOARD_MAX: usize = 1 << 20; // 1 MiB
pub const MAX_FRAME: usize = 2 << 20;     // 2 MiB hard cap per frame

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Message {
    Hello { version: u16, hostname: String },
    HelloAck { version: u16 },
    PairRequest { hostname: String },
    PairAccept,
    PairReject,
    Input(InputEvent),
    ReleaseControl,
    Heartbeat,
    ClipboardSet { text: String }, // defined for v1; unused in P1
    Error { code: u16, message: String },
}
```

`crates/spacing-core/src/hidmap.rs` — bảng map evdev `KEY_*` → USB HID usage (Linux input-event-codes, keyboard page). Nguồn mapping: USB HID Usage Tables §10 + kernel hid-input:

```rust
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
    (80, 0x5A), (81, 0x5B), (82, 0x58), (83, 0x62), (87, 0x44), (88, 0x45),
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
    ("kpenter", 0x58), ("kpdot", 0x62), ("kpplus", 0x57), ("kpminus", 0x56),
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
```

`protocol.rs` — thêm proptest roundtrip ở cuối file (spec §7 yêu cầu):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{InputEvent, MouseButton};
    use proptest::prelude::*;

    fn arb_event() -> impl Strategy<Value = InputEvent> {
        prop_oneof![
            any::<u16>().prop_map(InputEvent::KeyDown),
            any::<u16>().prop_map(InputEvent::KeyUp),
            (any::<i32>(), any::<i32>())
                .prop_map(|(dx, dy)| InputEvent::MouseMoveRel { dx, dy }),
            prop_oneof![
                Just(MouseButton::Left), Just(MouseButton::Right),
                Just(MouseButton::Middle), Just(MouseButton::Side),
                Just(MouseButton::Extra),
            ]
            .prop_flat_map(|b| {
                prop_oneof![
                    Just(InputEvent::ButtonDown(b)),
                    Just(InputEvent::ButtonUp(b)),
                ]
            }),
            (any::<i32>(), any::<i32>())
                .prop_map(|(dx, dy)| InputEvent::Wheel { dx, dy }),
        ]
    }

    fn arb_message() -> impl Strategy<Value = Message> {
        prop_oneof![
            (any::<u16>(), "\\PC{0,64}")
                .prop_map(|(v, h)| Message::Hello { version: v, hostname: h }),
            any::<u16>().prop_map(|v| Message::HelloAck { version: v }),
            "\\PC{0,64}".prop_map(|h| Message::PairRequest { hostname: h }),
            Just(Message::PairAccept),
            Just(Message::PairReject),
            arb_event().prop_map(Message::Input),
            Just(Message::ReleaseControl),
            Just(Message::Heartbeat),
            "\\PC{0,1024}".prop_map(|t| Message::ClipboardSet { text: t }),
            (any::<u16>(), "\\PC{0,256}")
                .prop_map(|(c, m)| Message::Error { code: c, message: m }),
        ]
    }

    proptest! {
        #[test]
        fn postcard_roundtrip(msg in arb_message()) {
            let bytes = postcard::to_allocvec(&msg).unwrap();
            let back: Message = postcard::from_bytes(&bytes).unwrap();
            prop_assert_eq!(back, msg);
        }
    }
}
```

Test module thêm vào cuối `hidmap.rs` (giữ test ở Step 1):

```rust
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
        fn any_mapped_key_roundtrips(i in proptest::sample::Index::from(0..EV_TO_HID.len())) {
            let (ev, hid) = EV_TO_HID[i.index(EV_TO_HID.len())];
            proptest::prop_assert_eq!(evdev_to_hid(ev), Some(hid));
            proptest::prop_assert_eq!(hid_to_evdev(hid), Some(ev));
        }
    }
}
```

- [ ] **Step 4: Chạy test, verify pass**

Run: `cargo test -p spacing-core`
Expected: PASS — 4 tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(core): protocol types + evdev<->HID map"
```

---

### Task 2: Router state machine + hotkey logic

**Files:**
- Create: `crates/spacing-core/src/combo.rs`
- Create: `crates/spacing-core/src/router.rs`
- Modify: `crates/spacing-core/src/lib.rs` (add `pub mod combo; pub mod router;`)

**Interfaces:**
- Consumes: `InputEvent`, `key_name_to_hid` (Task 1).
- Produces: `combo::{ModSet, Combo, parse_combo(&str) -> Result<Combo, ComboError>}`; `router::{Router, Target, Decision}`. `Router::new(hotkey_local: Combo, hotkey_switch: Vec<(Combo, String)>)`; `on_event(&mut self, InputEvent) -> Decision`; `on_disconnected(&mut self)`.

- [ ] **Step 1: Test thất bại**

`crates/spacing-core/src/router.rs` (chỉ test trước):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::InputEvent::*;

    fn router() -> Router {
        Router::new(
            parse_combo("Ctrl+Alt+0").unwrap(),
            vec![
                (parse_combo("Ctrl+Alt+1").unwrap(), "laptop".into()),
            ],
        )
    }

    const CTRL: u16 = 0xE0;
    const ALT: u16 = 0xE2;
    const K0: u16 = 0x27;
    const K1: u16 = 0x1E;
    const KA: u16 = 0x04;

    #[test]
    fn hotkey_switches_to_remote() {
        let mut r = router();
        r.on_event(KeyDown(CTRL));
        r.on_event(KeyDown(ALT));
        match r.on_event(KeyDown(K1)) {
            Decision::SwitchToRemote(name) => assert_eq!(name, "laptop"),
            d => panic!("expected SwitchToRemote, got {d:?}"),
        }
        // sau khi remote: mọi key forward, kể cả keyup của chính combo
        assert_eq!(r.on_event(KeyUp(K1)), Decision::Forward(KeyUp(K1)));
    }

    #[test]
    fn local_hotkey_returns() {
        let mut r = router();
        r.on_event(KeyDown(CTRL));
        r.on_event(KeyDown(ALT));
        r.on_event(KeyDown(K1));
        r.on_event(KeyUp(K1)); // vẫn giữ ctrl+alt
        match r.on_event(KeyDown(K0)) {
            Decision::SwitchToLocal => {}
            d => panic!("expected SwitchToLocal, got {d:?}"),
        }
    }

    #[test]
    fn local_mode_drops_events() {
        let mut r = router();
        assert_eq!(r.on_event(KeyDown(KA)), Decision::Drop);
    }

    #[test]
    fn remote_mode_forwards_everything() {
        let mut r = router();
        r.on_event(KeyDown(CTRL));
        r.on_event(KeyDown(ALT));
        r.on_event(KeyDown(K1));
        assert_eq!(r.on_event(KeyDown(KA)), Decision::Forward(KeyDown(KA)));
        assert_eq!(
            r.on_event(MouseMoveRel { dx: 3, dy: -2 }),
            Decision::Forward(MouseMoveRel { dx: 3, dy: -2 })
        );
    }

    #[test]
    fn disconnect_resets_to_local() {
        let mut r = router();
        r.on_event(KeyDown(CTRL));
        r.on_event(KeyDown(ALT));
        r.on_event(KeyDown(K1));
        assert!(matches!(r.target(), Target::Remote(_)));
        r.on_disconnected();
        assert_eq!(r.target(), &Target::Local);
        assert_eq!(r.on_event(KeyDown(KA)), Decision::Drop);
    }
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p spacing-core router`
Expected: FAIL — unresolved `parse_combo`, `Router`.

- [ ] **Step 3: Implement**

`crates/spacing-core/src/combo.rs`:

```rust
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
                if i != s.split('+').count() - 1 {
                    return Err(ComboError::UnknownKey(other.into()));
                }
                key = Some(key_name_to_hid(other).ok_or_else(|| ComboError::UnknownKey(other.into()))?);
            }
        }
    }
    match key {
        Some(k) => Ok(Combo { mods, key: k }),
        None => Err(ComboError::NoKey),
    }
}
```

`crates/spacing-core/src/router.rs` — đặt impl TRƯỚC `#[cfg(test)]`:

```rust
use crate::combo::{Combo, ModSet};
use crate::event::InputEvent;
use std::collections::HashSet;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Local,
    Remote(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    /// Local mode: event already reached the OS — ignore.
    Drop,
    /// Remote mode: forward to active client.
    Forward(InputEvent),
    /// Intercepted hotkey → return control locally (ungrab + ReleaseControl).
    SwitchToLocal,
    /// Intercepted hotkey → grab + forward to named client.
    SwitchToRemote(String),
}

/// Pure state machine. Owns no I/O: daemon maps Decisions to
/// grab/ungrab/network side effects.
pub struct Router {
    target: Target,
    hotkey_local: Combo,
    hotkey_switch: Vec<(Combo, String)>,
    held: HashSet<u16>,
}

impl Router {
    pub fn new(hotkey_local: Combo, hotkey_switch: Vec<(Combo, String)>) -> Self {
        Self {
            target: Target::Local,
            hotkey_local,
            hotkey_switch,
            held: HashSet::new(),
        }
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    /// Called when the active client connection drops; daemon must ungrab.
    pub fn on_disconnected(&mut self) {
        self.target = Target::Local;
        self.held.clear();
    }

    fn mods(&self) -> ModSet {
        let mut m = ModSet::default();
        for &h in &self.held {
            if let Some(mm) = ModSet::from_hid(h) {
                if mm.ctrl {
                    m.ctrl = true;
                }
                if mm.alt {
                    m.alt = true;
                }
                if mm.shift {
                    m.shift = true;
                }
                if mm.meta {
                    m.meta = true;
                }
            }
        }
        m
    }

    fn matches(&self, c: &Combo, key: u16) -> bool {
        c.key == key && self.mods() == c.mods
    }

    pub fn on_event(&mut self, ev: InputEvent) -> Decision {
        match ev {
            InputEvent::KeyDown(hid) => {
                // hotkey check BEFORE recording/forwarding.
                // NOTE: `held` is NOT cleared on switch — modifiers remain
                // physically held, and hotkeys must keep matching (e.g.
                // Ctrl+Alt+1 → Ctrl+Alt+0 without re-pressing modifiers).
                if self.matches(&self.hotkey_local, hid) {
                    self.target = Target::Local;
                    return Decision::SwitchToLocal;
                }
                for (combo, name) in &self.hotkey_switch {
                    if self.matches(combo, hid) {
                        self.target = Target::Remote(name.clone());
                        return Decision::SwitchToRemote(name.clone());
                    }
                }
                self.held.insert(hid);
                self.dispatch(ev)
            }
            InputEvent::KeyUp(hid) => {
                self.held.remove(&hid);
                self.dispatch(ev)
            }
            _ => self.dispatch(ev),
        }
    }

    fn dispatch(&self, ev: InputEvent) -> Decision {
        match self.target {
            Target::Local => Decision::Drop,
            Target::Remote(_) => Decision::Forward(ev),
        }
    }
}
```

Thêm dep `thiserror` vào `crates/spacing-core/Cargo.toml` `[dependencies]`:

```toml
thiserror = "2"
```

Update `lib.rs`:

```rust
pub mod combo;
pub mod event;
pub mod hidmap;
pub mod protocol;
pub mod router;
```

- [ ] **Step 4: Verify pass**

Run: `cargo test -p spacing-core`
Expected: PASS — tất cả tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(core): router state machine + combo parsing"
```

---

### Task 3: Config + peer store

**Files:**
- Create: `crates/spacing-core/src/config.rs`
- Create: `crates/spacing-core/src/peers.rs`
- Modify: `crates/spacing-core/src/lib.rs` (add `pub mod config; pub mod peers;`)
- Modify: `crates/spacing-core/Cargo.toml` (add `toml`, `anyhow`; `serde` đã có)

**Interfaces:**
- Produces: `config::{Config, Mode, ServerCfg, load_config(path) -> Result<Config>}`; `peers::{PeerStore, PeerInfo}` (file `peers.rs` mới — tạo thêm file `crates/spacing-core/src/peers.rs`). `PeerStore::load(path)`, `save(path)`, `allow(fp)`, `insert(name, fp)`.

- [ ] **Step 1: Test thất bại**

`crates/spacing-core/src/config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    const CFG: &str = r#"
        mode = "server"
        bind = "0.0.0.0:24800"
        [server.hotkeys]
        local = "Ctrl+Alt+0"
        "Ctrl+Alt+1" = "laptop"
        "Ctrl+Alt+2" = "desktop"
    "#;

    #[test]
    fn parses_server_config() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("config.toml");
        std::fs::write(&p, CFG).unwrap();
        let c = load_config(&p).unwrap();
        assert_eq!(c.mode, Mode::Server);
        assert_eq!(c.bind, "0.0.0.0:24800");
        assert_eq!(c.server.hotkeys.local.as_deref(), Some("Ctrl+Alt+0"));
        assert_eq!(c.server.hotkeys.switch["Ctrl+Alt+1"], "laptop");
        assert_eq!(c.server.hotkeys.switch["Ctrl+Alt+2"], "desktop");
    }
}
```

`crates/spacing-core/src/peers.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn peerstore_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("peers.toml");
        let mut s = PeerStore::default();
        s.insert("laptop".into(), "sha256:aa11".into());
        s.save(&p).unwrap();
        let s2 = PeerStore::load(&p).unwrap();
        assert!(s2.allow("sha256:aa11"));
        assert!(!s2.allow("sha256:ffff"));
        assert_eq!(s2.name_for("sha256:aa11"), Some("laptop"));
    }
}
```

Dev-dep `tempfile` vào `crates/spacing-core/Cargo.toml`:

```toml
[dev-dependencies]
proptest = "1"
tempfile = "3"
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p spacing-core`
Expected: FAIL — `load_config`, `PeerStore` missing.

- [ ] **Step 3: Implement**

`crates/spacing-core/src/config.rs`:

```rust
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Server,
    Client,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub mode: Mode,
    #[serde(default = "default_bind")]
    pub bind: String,
    /// Client mode only.
    pub connect_to: Option<String>,
    #[serde(default)]
    pub server: ServerCfg,
}

fn default_bind() -> String {
    "0.0.0.0:24800".into()
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ServerCfg {
    #[serde(default)]
    pub hotkeys: HotkeysCfg,
}

/// `local` is the switch-back combo; every other entry is
/// `"Combo" = "client-name"`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct HotkeysCfg {
    pub local: Option<String>,
    #[serde(flatten)]
    pub switch: BTreeMap<String, String>,
}

pub fn load_config(path: &Path) -> Result<Config, anyhow::Error> {
    let raw = std::fs::read_to_string(path)
        .map_err(|e| anyhow::anyhow!("read {}: {e}", path.display()))?;
    let cfg: Config = toml::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("parse {}: {e}", path.display()))?;
    Ok(cfg)
}
```

`crates/spacing-core/src/peers.rs`:

```rust
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerInfo {
    pub fingerprint: String,
    pub last_seen: Option<String>,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct PeerStore {
    #[serde(default)]
    pub peers: BTreeMap<String, PeerInfo>,
}

impl PeerStore {
    pub fn load(path: &Path) -> Result<Self, anyhow::Error> {
        match std::fs::read_to_string(path) {
            Ok(raw) => Ok(toml::from_str(&raw)?),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), anyhow::Error> {
        if let Some(d) = path.parent() {
            std::fs::create_dir_all(d)?;
        }
        std::fs::write(path, toml::to_string_pretty(self)?)?;
        Ok(())
    }

    /// fingerprint is the canonical "sha256:<hex>" form produced by spacing-net.
    pub fn allow(&self, fingerprint: &str) -> bool {
        self.peers.values().any(|p| p.fingerprint == fingerprint)
    }

    pub fn name_for(&self, fingerprint: &str) -> Option<&str> {
        self.peers
            .iter()
            .find(|(_, p)| p.fingerprint == fingerprint)
            .map(|(n, _)| n.as_str())
    }

    pub fn insert(&mut self, name: String, fingerprint: String) {
        self.peers.insert(name, PeerInfo { fingerprint, last_seen: None });
    }

    /// Refresh `last_seen` (unix seconds — tránh thêm datetime dep).
    pub fn seen(&mut self, name: &str) {
        if let Some(p) = self.peers.get_mut(name) {
            p.last_seen = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs().to_string())
                .ok();
        }
    }
}
```

`lib.rs` add `pub mod config; pub mod peers;`. Add `toml = "0.8"`, `anyhow = "1"` to spacing-core deps.

- [ ] **Step 4: Verify pass**

Run: `cargo test -p spacing-core`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(core): config + peer store"
```

---

### Task 4: `spacing-net` — TLS transport, fingerprint pinning, framing

**Files:**
- Create: `crates/spacing-net/Cargo.toml`
- Create: `crates/spacing-net/src/lib.rs`
- Create: `crates/spacing-net/src/cert.rs`
- Create: `crates/spacing-net/src/conn.rs`
- Create: `crates/spacing-net/src/verify.rs`
- Test: `crates/spacing-net/tests/loopback.rs`

**Interfaces:**
- Consumes: `spacing_core::protocol::Message`, `spacing_core::peers::PeerStore`.
- Produces:
  - `cert::{ensure_identity(dir) -> Result<Identity>, fingerprint_of(der) -> String}`; `Identity{certs: Vec<CertificateDer<'static>>, key: PrivateKeyDer<'static>, fingerprint: String}`
  - `conn::{send_msg(&mut WriteHalf, &Message) -> io::Result<()>, recv_msg(&mut ReadHalf) -> io::Result<Message>}`
  - `accept_server(tcp, id, peers: Arc<RwLock<PeerStore>>, pairing_open, peers_path) -> Result<ServerConn>` — peer store được CHIA SẺ với server loop (accept a pair: insert + persist `peers.toml`, client connect lại được ngay không cần restart); `connect_client(tcp, server_name, id, verify: impl Fn(&str)->bool) -> Result<ClientConn>` (type `Rd`/`Wr` = split halves của `TlsStream<TcpStream>`)
  - `verify::PairingVerifier`, `verify::AcceptAnyServerCert`

- [ ] **Step 1: Test thất bại**

`crates/spacing-net/tests/loopback.rs`:

```rust
use spacing_core::protocol::Message;
use spacing_core::peers::PeerStore;
use spacing_net::*;
use std::sync::Arc;
use tokio::net::TcpListener;

#[tokio::test]
async fn paired_handshake_and_echo() {
    let dir = tempfile::tempdir().unwrap();
    let server_id = cert::ensure_identity(&dir.path().join("srv")).unwrap();
    let client_id = cert::ensure_identity(&dir.path().join("cli")).unwrap();

    // client trusts server fp; server trusts client fp
    let mut client_peers = PeerStore::default();
    client_peers.insert("srv".into(), server_id.fingerprint.clone());
    let mut server_peers = PeerStore::default();
    server_peers.insert("cli".into(), client_id.fingerprint.clone());

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let srv_peers = std::sync::Arc::new(std::sync::RwLock::new(server_peers.clone()));
    let pairing_open = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let peers_path = dir.path().join("peers.toml");

    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        let conn =
            accept_server(tcp, server_id.clone(), srv_peers, pairing_open, peers_path)
                .await
                .unwrap();
        conn
    });

    let mut conn = connect_client(tcp_connect(addr).await, "localhost",
                              client_id.clone(),
                              move |fp| client_peers.allow(fp)).await.unwrap();
    conn::send_msg(&mut conn.wr_half, &Message::Hello {
        version: spacing_core::protocol::PROTOCOL_VERSION,
        hostname: "cli".into(),
    }).await.unwrap();

    let mut server_conn = server.await.unwrap();
    conn::send_msg(&mut server_conn.wr, &Message::Heartbeat).await.unwrap();
    let m = conn::recv_msg(&mut conn.rd_half).await.unwrap();
    // first frame back is HelloAck; read again for Heartbeat
    assert!(matches!(m, Message::HelloAck { .. }) || matches!(m, Message::Heartbeat));
}

/// Pairing window: unknown client cert accepted at TLS, PairRequest →
/// PairAccept, fingerprint persisted to peers.toml, window consumed.
#[tokio::test]
async fn pairing_window_registers_client_once() {
    use std::sync::atomic::Ordering;

    let dir = tempfile::tempdir().unwrap();
    let server_id = cert::ensure_identity(&dir.path().join("srv")).unwrap();
    let client_id = cert::ensure_identity(&dir.path().join("cli")).unwrap();

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let peers = std::sync::Arc::new(std::sync::RwLock::new(PeerStore::default()));
    let pairing_open = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let peers_path = dir.path().join("peers.toml");

    let (srv_peers, po) = (peers.clone(), pairing_open.clone());
    let server = tokio::spawn(async move {
        let (tcp, _) = listener.accept().await.unwrap();
        accept_server(tcp, server_id, srv_peers, po, peers_path).await.unwrap()
    });

    let mut conn = connect_client(
        tcp_connect(addr).await, "localhost", client_id.clone(), |_| true,
    )
    .await
    .unwrap();
    conn::send_msg(
        &mut conn.wr_half,
        &Message::PairRequest { hostname: "cli".into() },
    )
    .await
    .unwrap();
    match conn::recv_msg(&mut conn.rd_half).await.unwrap() {
        Message::PairAccept => {}
        other => panic!("expected PairAccept, got {other:?}"),
    }
    server.await.unwrap();

    // in-memory store updated (shared Arc), persisted to disk, window closed
    assert!(peers.read().unwrap().allow(&client_id.fingerprint));
    assert!(!pairing_open.load(Ordering::Relaxed));
    let reloaded = PeerStore::load(&dir.path().join("peers.toml")).unwrap();
    assert!(reloaded.allow(&client_id.fingerprint));
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p spacing-net`
Expected: FAIL — crate/functions don't exist.

- [ ] **Step 3: Implement**

`crates/spacing-net/Cargo.toml`:

```toml
[package]
name = "spacing-net"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
spacing-core = { path = "../spacing-core" }
tokio = { version = "1", features = ["full"] }
tokio-rustls = "0.26"
rustls = { version = "0.23", features = ["ring"] }  # 0.23.17+ default provider là aws_lc_rs — phải bật ring rõ ràng
rustls-pemfile = "2"
rcgen = "0.13"
sha2 = "0.10"
postcard = { version = "1", features = ["alloc"] }
serde = { version = "1", features = ["derive"] }
tracing = "0.1"
anyhow = "1"
gethostname = "0.5"

[dev-dependencies]
tempfile = "3"
```

`crates/spacing-net/src/lib.rs`:

```rust
pub mod cert;
pub mod conn;
pub mod verify;

use anyhow::{anyhow, Context};
use spacing_core::peers::PeerStore;
use spacing_core::protocol::{Message, PROTOCOL_VERSION};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio::io::{ReadHalf, WriteHalf};
use tokio::net::{TcpStream, ToSocketAddrs};
use tokio_rustls::rustls::{ClientConfig, ServerConfig};
use tokio_rustls::{TlsAcceptor, TlsConnector, TlsStream};

pub type Rd = ReadHalf<TlsStream<TcpStream>>;
pub type Wr = WriteHalf<TlsStream<TcpStream>>;

pub struct ServerConn {
    pub rd: Rd,
    pub wr: Wr,
    pub peer_fp: String,
    pub peer_name: String,
}

pub struct ClientConn {
    pub rd_half: Rd,
    pub wr_half: Wr,
    pub peer_fp: String,
}

fn ensure_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        rustls::crypto::ring::default_provider().install_default().ok();
    });
}

pub async fn tcp_connect<A: ToSocketAddrs>(addr: A) -> TcpStream {
    TcpStream::connect(addr).await.expect("tcp connect")
}

fn peer_fp(stream: &TlsStream<TcpStream>) -> anyhow::Result<String> {
    let (_, common) = stream.get_ref();
    let certs = common
        .peer_certificates()
        .ok_or_else(|| anyhow!("peer presented no certificate"))?;
    Ok(cert::fingerprint_of(&certs[0]))
}

/// Server side: accept TCP, run mutual-TLS, verify peer fingerprint.
/// `peers` is SHARED with the server loop — a successful pair must be
/// visible to subsequent accepts without a restart.
/// `pairing_open`: one-shot — unknown fingerprints are accepted at TLS
/// layer; caller must treat next message as PairRequest.
pub async fn accept_server(
    tcp: TcpStream,
    id: cert::Identity,
    peers: Arc<RwLock<PeerStore>>,
    pairing_open: Arc<AtomicBool>,
    peers_path: std::path::PathBuf,
) -> anyhow::Result<ServerConn> {
    ensure_provider();
    let verifier = Arc::new(verify::PairingVerifier {
        peers: peers.clone(),
        pairing_open: pairing_open.clone(),
    });
    let cfg = ServerConfig::builder()
        .with_client_cert_verifier(verifier)
        .with_single_cert(id.certs.clone(), id.key.clone_key())
        .context("server tls config")?;
    // accept()/connect() trả server::/client::TlsStream — .into() sang enum
    // TlsStream chung để dùng chung type Rd/Wr.
    let stream: TlsStream<_> = TlsAcceptor::from(Arc::new(cfg))
        .accept(tcp)
        .await?
        .into();
    let peer_fp = peer_fp(&stream)?;
    let paired = peers.read().unwrap().allow(&peer_fp);
    if !paired && !pairing_open.load(Ordering::Relaxed) {
        anyhow::bail!("untrusted fingerprint {peer_fp}");
    }
    let (mut rd, mut wr) = tokio::io::split(stream);
    // First frame: Hello (paired peer) or PairRequest (new peer).
    let msg = conn::recv_msg(&mut rd).await?;
    let peer_name = match msg {
        Message::Hello { version, hostname } => {
            anyhow::ensure!(version == PROTOCOL_VERSION, "version mismatch {version}");
            anyhow::ensure!(paired, "hello from unpaired client");
            peers.write().unwrap().seen(&hostname);
            conn::send_msg(&mut wr, &Message::HelloAck { version: PROTOCOL_VERSION }).await?;
            hostname
        }
        Message::PairRequest { hostname } => {
            let accepted = !paired
                && {
                    // pairing_open was consumed by the TLS verifier to let
                    // this unknown cert through — grant exactly once.
                    pairing_open.swap(false, Ordering::Relaxed)
                };
            if accepted {
                {
                    let mut g = peers.write().unwrap();
                    g.insert(hostname.clone(), peer_fp.clone());
                    g.save(&peers_path)?; // persist so future connects verify
                }
                conn::send_msg(&mut wr, &Message::PairAccept).await?;
            } else {
                conn::send_msg(&mut wr, &Message::PairReject).await?;
                anyhow::bail!("pairing rejected (not in --pairing mode or already paired)");
            }
            hostname
        }
        other => anyhow::bail!("unexpected first message {other:?}"),
    };
    Ok(ServerConn { rd, wr, peer_fp, peer_name })
}

/// Client side: connect TCP, mutual-TLS. `verify` checks the server
/// fingerprint — pass `|fp| peers.allow(fp)` normally, `|_| true` when
/// pairing (fp is reported to the caller for the trust prompt).
pub async fn connect_client(
    tcp: TcpStream,
    server_name: &str,
    id: cert::Identity,
    verify: impl Fn(&str) -> bool,
) -> anyhow::Result<ClientConn> {
    ensure_provider();
    let cfg = ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(verify::AcceptAnyServerCert))
        .with_client_auth_cert(id.certs.clone(), id.key.clone_key())
        .context("client tls config")?;
    let name = rustls::pki_types::ServerName::try_from(server_name.to_string())
        .map_err(|_| anyhow!("invalid server name {server_name}"))?;
    let stream: TlsStream<_> = TlsConnector::from(Arc::new(cfg))
        .connect(name, tcp)
        .await?
        .into();
    let peer_fp = peer_fp(&stream)?;
    anyhow::ensure!(
        verify(&peer_fp),
        "server fingerprint {peer_fp} not in peers.toml — run `spacing pair` first"
    );
    let (rd_half, wr_half) = tokio::io::split(stream);
    Ok(ClientConn { rd_half, wr_half, peer_fp })
}
```

`crates/spacing-net/src/cert.rs`:

```rust
use anyhow::Context;
use sha2::{Digest, Sha256};
use std::path::Path;
use tokio_rustls::rustls::pki_types::{CertificateDer, PrivateKeyDer};

/// `PrivateKeyDer` does NOT implement `Clone` (only `clone_key()`),
/// so a manual impl is required — `#[derive(Clone)]` will not compile.
pub struct Identity {
    pub certs: Vec<CertificateDer<'static>>,
    pub key: PrivateKeyDer<'static>,
    pub fingerprint: String,
}

impl Clone for Identity {
    fn clone(&self) -> Self {
        Self {
            certs: self.certs.clone(),
            key: self.key.clone_key(),
            fingerprint: self.fingerprint.clone(),
        }
    }
}

/// Load or generate a self-signed identity at `<dir>/{cert,key}.pem`.
pub fn ensure_identity(dir: &Path) -> anyhow::Result<Identity> {
    std::fs::create_dir_all(dir)?;
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    if !cert_path.exists() || !key_path.exists() {
        let hostname = gethostname::gethostname().to_string_lossy().into_owned();
        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec![hostname])?;
        std::fs::write(&cert_path, cert.pem())?;
        std::fs::write(&key_path, key_pair.serialize_pem())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    let mut cr = std::io::BufReader::new(std::fs::File::open(&cert_path)?);
    let certs: Vec<CertificateDer<'static>> = rustls_pemfile::certs(&mut cr)
        .collect::<Result<_, _>>()
        .context("parse cert.pem")?;
    let mut kr = std::io::BufReader::new(std::fs::File::open(&key_path)?);
    let key = rustls_pemfile::private_key(&mut kr)?
        .ok_or_else(|| anyhow::anyhow!("no private key in key.pem"))?;
    let fingerprint = fingerprint_of(&certs[0]);
    Ok(Identity { certs, key, fingerprint })
}

/// Canonical "sha256:<lowercase hex>" fingerprint of a DER certificate.
pub fn fingerprint_of(der: &CertificateDer<'_>) -> String {
    let digest = Sha256::digest(der.as_ref());
    let mut s = String::with_capacity(71);
    s.push_str("sha256:");
    for b in digest {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
```

`crates/spacing-net/src/verify.rs`:

```rust
use spacing_core::peers::PeerStore;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use tokio_rustls::rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use tokio_rustls::rustls::server::danger::{ClientCertVerified, ClientCertVerifier};
use tokio_rustls::rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use tokio_rustls::rustls::{DigitallySignedStruct, DistinguishedName, Error, SignatureScheme};

fn schemes() -> Vec<SignatureScheme> {
    vec![
        SignatureScheme::ED25519,
        SignatureScheme::ECDSA_NISTP256_SHA256,
        SignatureScheme::RSA_PSS_SHA256,
        SignatureScheme::RSA_PKCS1_SHA256,
    ]
}

/// Client-side verifier: accepts any cert at TLS layer; fingerprint is
/// pinned by the app layer after handshake (TOFU model).
#[derive(Debug)]
pub struct AcceptAnyServerCert;

impl ServerCertVerifier for AcceptAnyServerCert {
    fn verify_server_cert(
        &self, _end: &CertificateDer<'_>, _inter: &[CertificateDer<'_>],
        _name: &ServerName<'_>, _ocsp: &[u8], _now: UnixTime,
    ) -> Result<ServerCertVerified, Error> {
        Ok(ServerCertVerified::assertion())
    }
    // required on ServerCertVerifier since rustls 0.23.15
    fn requires_raw_public_keys(&self) -> bool {
        false
    }
    fn verify_tls12_signature(&self, _m: &[u8], _c: &CertificateDer<'_>, _d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(&self, _m: &[u8], _c: &CertificateDer<'_>, _d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        schemes()
    }
}

/// Server-side verifier: accepts client certs whose fingerprint is in
/// the peer store, or any cert while the one-shot pairing window is open.
#[derive(Debug)]
pub struct PairingVerifier {
    pub peers: Arc<RwLock<PeerStore>>,
    pub pairing_open: Arc<AtomicBool>,
}

impl ClientCertVerifier for PairingVerifier {
    fn offer_client_auth(&self) -> bool {
        true
    }
    fn client_auth_mandatory(&self) -> bool {
        true
    }
    fn root_hint_subjects(&self) -> &[DistinguishedName] {
        &[]
    }
    fn verify_client_cert(
        &self, end: &CertificateDer<'_>, _inter: &[CertificateDer<'_>],
        _now: UnixTime,
    ) -> Result<ClientCertVerified, Error> {
        let fp = crate::cert::fingerprint_of(end);
        let trusted = self.peers.read().unwrap().allow(&fp);
        if trusted || self.pairing_open.load(Ordering::Relaxed) {
            Ok(ClientCertVerified::assertion())
        } else {
            Err(Error::General(format!("untrusted client fingerprint {fp}")))
        }
    }
    fn verify_tls12_signature(&self, _m: &[u8], _c: &CertificateDer<'_>, _d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn verify_tls13_signature(&self, _m: &[u8], _c: &CertificateDer<'_>, _d: &DigitallySignedStruct) -> Result<HandshakeSignatureValid, Error> {
        Ok(HandshakeSignatureValid::assertion())
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        schemes()
    }
}
```

`crates/spacing-net/src/conn.rs`:

```rust
use crate::{Rd, Wr};
use spacing_core::protocol::{Message, MAX_FRAME};
use std::io;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

pub async fn send_msg(wr: &mut Wr, msg: &Message) -> io::Result<()> {
    let payload = postcard::to_allocvec(msg)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let len = (payload.len() as u32).to_le_bytes();
    wr.write_all(&len).await?;
    wr.write_all(&payload).await?;
    wr.flush().await
}

pub async fn recv_msg(rd: &mut Rd) -> io::Result<Message> {
    let mut lenb = [0u8; 4];
    rd.read_exact(&mut lenb).await?;
    let len = u32::from_le_bytes(lenb) as usize;
    if len > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too large"));
    }
    let mut buf = vec![0u8; len];
    rd.read_exact(&mut buf).await?;
    postcard::from_bytes(&buf).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}
```

Note `gethostname` đã khai báo trong Cargo.toml ở trên (cert.rs dùng).

- [ ] **Step 4: Verify pass**

Run: `cargo test -p spacing-net`
Expected: PASS — loopback test green.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(net): mutual-TLS transport with fingerprint pairing + framed protocol"
```

---

### Task 5: `spacing-capture` — evdev backend

**Files:**
- Create: `crates/spacing-capture/Cargo.toml`
- Create: `crates/spacing-capture/src/lib.rs`
- Create: `crates/spacing-capture/src/map.rs` (pure fn evdev→InputEvent)

**Interfaces:**
- Consumes: `spacing_core::{event::*, hidmap::evdev_to_hid}`.
- Produces: `EvdevCapture::enumerate() -> io::Result<Self>`; `grab()`,`ungrab()` (grab fail → ungrab tất cả, không bao giờ để device kẹt grab); `next_event(&mut self) -> Option<InputEvent>` (borrow `&mut self` chỉ trong lúc await — BẮT BUỘC để `grab()` gọi được trong `select!`; `into_stream(self)` consume owner → không dùng được); pure `map::map_event(evdev::InputEvent) -> Option<InputEvent>`.

- [ ] **Step 1: Test thất bại**

`crates/spacing-capture/src/map.rs` (test block):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use spacing_core::event::{InputEvent as Ev, MouseButton};

    fn ev(type_: u16, code: u16, value: i32) -> evdev::InputEvent {
        evdev::InputEvent::new(type_, code, value)
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
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p spacing-capture`
Expected: FAIL — `map_event` missing.

- [ ] **Step 3: Implement**

`crates/spacing-capture/Cargo.toml`:

```toml
[package]
name = "spacing-capture"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
spacing-core = { path = "../spacing-core" }
evdev = { version = "0.12", features = ["tokio"] }
futures = "0.3"
tokio = { version = "1", features = ["full"] }
tracing = "0.1"
```

`crates/spacing-capture/src/map.rs`:

```rust
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
        _ => None,                                          // REL_WHEEL_HI_RES etc.
    }
}
```

`crates/spacing-capture/src/lib.rs` — thiết kế: giữ `Vec<EventStream>` (EventStream own Device; grab qua `device_mut()` — evdev 0.12.2 không có `get_mut()`). KHÔNG dùng `into_stream(self)` — consume owner thì `grab()` không gọi được nữa. `select_all` dùng `iter_mut()` trực tiếp (async-block `filter_map` tạo `!Unpin` → SelectAll bị stuck ở `Unpin` bound):

```rust
pub mod map;

use futures::stream;
use futures::StreamExt as _;
use spacing_core::event::InputEvent;
use std::io;

/// Owns all physical input devices as event streams.
/// grab() = EVIOCGRAB trên từng fd (OS ngừng nhận input); stream vẫn đọc được.
pub struct EvdevCapture {
    streams: Vec<evdev::EventStream>,
}

impl EvdevCapture {
    /// Scan /dev/input, keep devices that look like keyboards or mice,
    /// convert each to an EventStream. Requires read access to
    /// /dev/input/event* (input group or root).
    pub fn enumerate() -> io::Result<Self> {
        let mut streams = Vec::new();
        for (path, device) in evdev::enumerate() {
            let is_kb = device
                .supported_keys()
                .map(|k| {
                    (30..=38u16).all(|c| k.contains(evdev::Key::new(c))) // A..L
                })
                .unwrap_or(false);
            let is_mouse = device.supported_relative_axes().map(|r| {
                r.contains(evdev::RelativeAxisType::REL_X)
                    && r.contains(evdev::RelativeAxisType::REL_Y)
            });
            if !(is_kb || is_mouse.unwrap_or(false)) {
                continue;
            }
            match device.into_event_stream() {
                Ok(s) => {
                    tracing::info!(?path, "capturing device");
                    streams.push(s);
                }
                Err(e) => tracing::warn!(?path, "event stream failed: {e}"),
            }
        }
        if streams.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                "no keyboard/mouse found in /dev/input — check input group permissions",
            ));
        }
        Ok(Self { streams })
    }

    /// Exclusive-grab every device. On failure, releases all grabs so no
    /// device is left hidden from the OS (golden rule).
    pub fn grab(&mut self) -> io::Result<()> {
        for s in &mut self.streams {
            if let Err(e) = s.device_mut().grab() {
                let _ = self.ungrab();
                return Err(e);
            }
        }
        Ok(())
    }

    pub fn ungrab(&mut self) -> io::Result<()> {
        for s in &mut self.streams {
            let _ = s.device_mut().ungrab(); // best-effort: never leave grabbed
        }
        Ok(())
    }

    /// Next mapped input event; skips unmapped. The `&mut self` borrow ends
    /// when the future resolves, so the server can grab()/ungrab() inside
    /// the same select! loop.
    pub async fn next_event(&mut self) -> Option<InputEvent> {
        // `&mut EventStream: Stream` (EventStream: Unpin in evdev 0.12).
        let mut merged = stream::select_all(self.streams.iter_mut());
        while let Some(raw) = merged.next().await {
            if let Ok(ev) = raw {
                if let Some(ev) = map::map_event(ev) {
                    return Some(ev);
                }
            }
        }
        None
    }
}
```

- [ ] **Step 4: Verify pass**

Run: `cargo test -p spacing-capture`
Expected: PASS — 4 map tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(capture): evdev grab backend + event mapping"
```

---

### Task 6: `spacing-inject` — uinput backend

**Files:**
- Create: `crates/spacing-inject/Cargo.toml`
- Create: `crates/spacing-inject/src/lib.rs`
- Create: `crates/spacing-inject/src/map.rs`

**Interfaces:**
- Consumes: `spacing_core::{event::*, hidmap::{hid_to_evdev, EV_TO_HID}}`.
- Produces: `UinputInject::create() -> io::Result<Self>`; `inject(&InputEvent)`; `release_all()`; `set_clipboard(&str)` = `Err` (P1 stub — clipboard not in P1); pure `map::to_evdev(&InputEvent) -> Vec<evdev::InputEvent>`.

- [ ] **Step 1: Test thất bại**

`crates/spacing-inject/src/map.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use spacing_core::event::{InputEvent as Ev, MouseButton};

    fn codes(evs: &[evdev::InputEvent]) -> Vec<(u16, u16, i32)> {
        evs.iter().map(|e| (e.event_type().0, e.code(), e.value())).collect()
    }

    #[test]
    fn keydown_emits_evkey_and_syn() {
        let out = to_evdev(&Ev::KeyDown(0x04)); // 'a' -> evdev 30
        let c = codes(&out);
        assert!(c.contains(&(evdev::EventType::KEY.0, 30, 1)));
        assert!(c.iter().any(|(t, ..)| *t == evdev::EventType::SYNCHRONIZATION.0));
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
        assert!(codes(&out).contains(&(evdev::EventType::RELATIVE.0, 0x08, 1)));   // REL_WHEEL
        assert!(codes(&out).contains(&(evdev::EventType::RELATIVE.0, 0x0B, 120))); // REL_WHEEL_HI_RES
    }

    #[test]
    fn unmapped_key_yields_empty() {
        assert!(to_evdev(&Ev::KeyDown(0xFFFF)).is_empty());
    }
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test -p spacing-inject`
Expected: FAIL — `to_evdev` missing.

- [ ] **Step 3: Implement**

`crates/spacing-inject/Cargo.toml`:

```toml
[package]
name = "spacing-inject"
version = "0.1.0"
edition.workspace = true
license.workspace = true

[dependencies]
spacing-core = { path = "../spacing-core" }
evdev = "0.12"
tracing = "0.1"
```

`crates/spacing-inject/src/map.rs`:

```rust
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
    // evdev 0.12.2: InputEvent::new nhận EventType enum, không phải u16.
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
```

`crates/spacing-inject/src/lib.rs`:

```rust
pub mod map;

use evdev::uinput::VirtualDeviceBuilder;
use evdev::{AttributeSet, Key, RelativeAxisType};
use spacing_core::event::InputEvent;
use spacing_core::hidmap::{hid_to_evdev, EV_TO_HID};
use std::collections::HashSet;
use std::io;

/// Virtual keyboard+mouse created via /dev/uinput.
/// Works under Wayland and X11 (kernel-level). Requires write access to
/// /dev/uinput (see README udev rule).
pub struct UinputInject {
    dev: evdev::uinput::VirtualDevice,
    held: HashSet<u16>, // evdev codes currently down (keys AND BTN_* buttons)
}

impl UinputInject {
    pub fn create() -> io::Result<Self> {
        let mut keys = AttributeSet::<Key>::new();
        for (code, _) in EV_TO_HID {
            keys.insert(Key::new(*code));
        }
        for b in [0x110u16, 0x111, 0x112, 0x113, 0x114] {
            keys.insert(Key::new(b));
        }
        let mut axes = AttributeSet::<RelativeAxisType>::new();
        axes.insert(RelativeAxisType::REL_X);
        axes.insert(RelativeAxisType::REL_Y);
        axes.insert(RelativeAxisType::REL_WHEEL);
        axes.insert(RelativeAxisType::REL_HWHEEL);
        axes.insert(RelativeAxisType::REL_WHEEL_HI_RES);
        axes.insert(RelativeAxisType::REL_HWHEEL_HI_RES);

        let dev = VirtualDeviceBuilder::new()?
            .name("spacing-virtual-input")
            .with_keys(&keys)?
            .with_relative_axes(&axes)?
            .build()?;
        Ok(Self { dev, held: HashSet::new() })
    }

    pub fn inject(&mut self, ev: &InputEvent) -> io::Result<()> {
        let out = map::to_evdev(ev);
        if !out.is_empty() {
            self.dev.emit(&out)?; // one write() per wire event
        }
        // track held codes (keys + buttons) for release_all
        match *ev {
            InputEvent::KeyDown(hid) => {
                if let Some(c) = hid_to_evdev(hid) {
                    self.held.insert(c);
                }
            }
            InputEvent::KeyUp(hid) => {
                if let Some(c) = hid_to_evdev(hid) {
                    self.held.remove(&c);
                }
            }
            InputEvent::ButtonDown(b) => {
                self.held.insert(map::btn_code(b));
            }
            InputEvent::ButtonUp(b) => {
                self.held.remove(&map::btn_code(b));
            }
            _ => {}
        }
        Ok(())
    }

    /// Key-up every tracked code — invoked on disconnect/watchdog.
    pub fn release_all(&mut self) -> io::Result<()> {
        let held: Vec<u16> = self.held.drain().collect();
        for code in held {
            self.dev.emit(&[
                evdev::InputEvent::new(evdev::EventType::KEY, code, 0),
                evdev::InputEvent::new(evdev::EventType::SYNCHRONIZATION, 0, 0),
            ])?;
        }
        Ok(())
    }

    /// P1 stub: clipboard lands in P2.
    pub async fn set_clipboard(&mut self, _text: &str) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "clipboard not in P1"))
    }
}
```

Note: evdev 0.12's `dev.emit(&[events])` — write all at once per event is fine for MVP.

- [ ] **Step 4: Verify pass**

Run: `cargo test -p spacing-inject`
Expected: PASS — 4 map tests.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(inject): uinput virtual device backend"
```

---

### Task 7: Server binary — `spacing serve` + `spacing pair` + `spacing fingerprint`

**Files:**
- Create: `src/cli.rs` (clap definitions)
- Create: `src/main.rs`
- Create: `src/server.rs`
- Create: `src/client.rs` (stub `bail!`, replaced in Task 8)

**Interfaces:**
- Consumes: all crates above. `spacing_net::{accept_server, ServerConn}`; `EvdevCapture`; `Router`, `Decision`; `config`, `peers`.
- Produces: working `spacing serve`, `spacing pair`, `spacing fingerprint`, `spacing client` stub (filled Task 8).

- [ ] **Step 1: CLI structure test (smoke)**

Root `src/main.rs` test via `clap::CommandFactory::debug_assert` — file `src/cli.rs`:

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "spacing", about = "Share keyboard/mouse across LAN")]
pub struct Cli {
    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Run as input owner (captures this machine's devices).
    Serve {
        /// Open a one-shot pairing window for one unknown client cert.
        #[arg(long)]
        pairing: bool,
        /// Config path (default ~/.config/spacing/config.toml).
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Run as controlled machine (injects received input).
    Client {
        /// server addr host:port (overrides config connect_to)
        #[arg(long)]
        to: Option<String>,
        #[arg(long)]
        config: Option<std::path::PathBuf>,
    },
    /// Pair with a server: saves its fingerprint to peers.toml.
    Pair {
        /// server addr host:port
        addr: String,
        /// friendly name to store
        #[arg(long)]
        name: Option<String>,
    },
    /// Print this machine's cert fingerprint.
    Fingerprint,
}
```

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;
    #[test]
    fn cli_debug_assert() {
        Cli::command().debug_assert();
    }
}
```

- [ ] **Step 2: Verify fail**

Run: `cargo test --bin spacing`
Expected: FAIL — main/cli incomplete.

- [ ] **Step 3: Implement**

`src/main.rs`:

```rust
mod cli;
mod client;
mod server;

use anyhow::Context;
use clap::Parser;
use spacing_core::peers::PeerStore;
use std::path::PathBuf;

fn config_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("spacing")
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    let cli = cli::Cli::parse();
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    match cli.cmd {
        cli::Cmd::Serve { pairing, config } => {
            server::run(dir, config, pairing).await
        }
        cli::Cmd::Client { to, config } => client::run(dir, to, config).await,
        cli::Cmd::Pair { addr, name } => pair(dir, &addr, name).await,
        cli::Cmd::Fingerprint => {
            let id = spacing_net::cert::ensure_identity(&dir)?;
            println!("{}", id.fingerprint);
            Ok(())
        }
    }
}

async fn pair(dir: PathBuf, addr: &str, name: Option<String>) -> anyhow::Result<()> {
    use spacing_core::protocol::Message;
    let id = spacing_net::cert::ensure_identity(&dir)?;
    let tcp = tokio::net::TcpStream::connect(addr).await?;
    let host = addr.split(':').next().unwrap_or("server").to_string();
    // Pairing = trust-on-first-use: don't verify fp yet, show it to user.
    let mut conn =
        spacing_net::connect_client(tcp, &host, id, |_| true).await?;
    println!("server fingerprint: {}", conn.peer_fp);
    print!("trust this fingerprint? [y/N] ");
    use std::io::Write;
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    anyhow::ensure!(line.trim().eq_ignore_ascii_case("y"), "aborted");

    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    spacing_net::conn::send_msg(
        &mut conn.wr_half,
        &Message::PairRequest { hostname },
    )
    .await?;
    match spacing_net::conn::recv_msg(&mut conn.rd_half).await? {
        Message::PairAccept => {}
        Message::PairReject => anyhow::bail!("server rejected pairing (restart serve with --pairing)"),
        other => anyhow::bail!("unexpected reply {other:?}"),
    }

    let mut peers = PeerStore::load(&dir.join("peers.toml"))?;
    let name = name.unwrap_or_else(|| addr.to_string());
    peers.insert(name.clone(), conn.peer_fp.clone());
    peers.save(&dir.join("peers.toml"))?;
    println!("paired with {name}");
    Ok(())
}
```

`src/server.rs`:

```rust
use anyhow::Context;
use spacing_capture::EvdevCapture;
use spacing_core::combo::parse_combo;
use spacing_core::config::{load_config, Mode};
use spacing_core::peers::PeerStore;
use spacing_core::protocol::Message;
use spacing_core::router::{Decision, Router, Target};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const SEND_TIMEOUT: Duration = Duration::from_millis(20);
const HEARTBEAT_MS: u64 = 250;

async fn writer_task(mut wr: spacing_net::Wr, mut rx: mpsc::Receiver<Message>) {
    while let Some(m) = rx.recv().await {
        if spacing_net::conn::send_msg(&mut wr, &m).await.is_err() {
            break;
        }
    }
}

/// Reads client->server control messages; reports peer_name on EOF/Error.
async fn reader_task(mut rd: spacing_net::Rd, name: String, ctrl: mpsc::Sender<String>) {
    loop {
        match spacing_net::conn::recv_msg(&mut rd).await {
            Ok(Message::Error { message, .. }) => {
                tracing::warn!(peer = %name, "client error: {message}");
                let _ = ctrl.send(name).await;
                return;
            }
            Ok(_) => {}
            Err(_) => {
                let _ = ctrl.send(name).await;
                return;
            }
        }
    }
}

pub async fn run(dir: PathBuf, cfg_path: Option<PathBuf>, pairing: bool) -> anyhow::Result<()> {
    let cfg_path = cfg_path.unwrap_or_else(|| dir.join("config.toml"));
    let cfg = load_config(&cfg_path)?;
    anyhow::ensure!(cfg.mode == Mode::Server, "config mode must be \"server\"");
    // Shared with every accept task — pairing inserts are visible immediately.
    let peers = Arc::new(RwLock::new(PeerStore::load(&dir.join("peers.toml"))?));
    let id = spacing_net::cert::ensure_identity(&dir)?;
    tracing::info!(fp = %id.fingerprint, "identity loaded");

    let local = parse_combo(cfg.server.hotkeys.local.as_deref().unwrap_or("Ctrl+Alt+0"))?;
    let mut switch = Vec::new();
    for (combo_s, name) in &cfg.server.hotkeys.switch {
        switch.push((parse_combo(combo_s)?, name.clone()));
    }
    let mut router = Router::new(local, switch);

    let mut cap = EvdevCapture::enumerate().context(
        "cannot capture /dev/input — add user to 'input' group or apply the udev rule in README",
    )?;

    let listener = TcpListener::bind(&cfg.bind).await?;
    tracing::info!(bind = %cfg.bind, pairing, "listening");

    // peer_name -> channel to that client's writer task
    let mut conns: HashMap<String, mpsc::Sender<Message>> = HashMap::new();
    let pairing_open = Arc::new(AtomicBool::new(pairing));
    // accepted conns arrive here from accept tasks
    let (new_tx, mut new_rx) = mpsc::channel::<spacing_net::ServerConn>(8);
    // dead/errored peer_names arrive here from reader tasks
    let (ctrl_tx, mut ctrl_rx) = mpsc::channel::<String>(8);
    let mut hb = tokio::time::interval(Duration::from_millis(HEARTBEAT_MS));
    hb.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            acc = listener.accept() => {
                let (tcp, _) = acc?;
                let (idc, pc, po, ntx, pp) =
                    (id.clone(), peers.clone(), pairing_open.clone(), new_tx.clone(),
                     dir.join("peers.toml"));
                tokio::spawn(async move {
                    match spacing_net::accept_server(tcp, idc, pc, po, pp).await {
                        Ok(conn) => {
                            tracing::info!(peer = %conn.peer_name, fp = %conn.peer_fp, "client connected");
                            let _ = ntx.send(conn).await;
                        }
                        Err(e) => tracing::warn!("connection rejected: {e}"),
                    }
                });
            }
            Some(conn) = new_rx.recv() => {
                let name = conn.peer_name.clone();
                if conns.contains_key(&name) {
                    tracing::warn!(peer = %name, "duplicate peer name — replacing connection");
                }
                let (tx, rx) = mpsc::channel::<Message>(256);
                conns.insert(name.clone(), tx);
                tokio::spawn(writer_task(conn.wr, rx));
                tokio::spawn(reader_task(conn.rd, name, ctrl_tx.clone()));
            }
            Some(dead) = ctrl_rx.recv() => {
                tracing::warn!(peer = %dead, "client lost");
                let prev = router.target().clone();
                conns.remove(&dead);
                if matches!(&prev, Target::Remote(n) if n == &dead) {
                    to_local(&mut cap, &mut router, &conns, &prev).await;
                }
            }
            // cap.next_event() borrows `cap` only while pending — arms can
            // grab()/ungrab() freely after it resolves.
            Some(ev) = cap.next_event() => {
                let prev = router.target().clone();
                match router.on_event(ev) {
                    Decision::Drop => {}
                    Decision::Forward(e) => {
                        if let Target::Remote(name) = router.target() {
                            if let Some(tx) = conns.get(name) {
                                if tx.send_timeout(Message::Input(e), SEND_TIMEOUT).await.is_err() {
                                    fail_to_local(&mut cap, &mut router);
                                }
                            }
                        }
                    }
                    Decision::SwitchToRemote(name) => {
                        if !conns.contains_key(&name) {
                            // hotkey targets a peer that isn't connected —
                            // grabbing here would send input into the void.
                            tracing::warn!(target = %name, "peer not connected — staying local");
                            router.on_disconnected();
                        } else {
                            tracing::info!(target = %name, "switching to remote");
                            // remote->remote: tell the old client to release held keys
                            if let Target::Remote(old) = &prev {
                                if old != &name {
                                    if let Some(tx) = conns.get(old) {
                                        let _ = tx
                                            .send_timeout(Message::ReleaseControl, SEND_TIMEOUT)
                                            .await;
                                    }
                                }
                            }
                            if let Err(e) = cap.grab() {
                                tracing::error!("grab failed: {e} — staying local");
                                router.on_disconnected();
                            }
                        }
                    }
                    Decision::SwitchToLocal => {
                        tracing::info!("back to local");
                        to_local(&mut cap, &mut router, &conns, &prev).await;
                    }
                }
            }
            _ = hb.tick(), if matches!(router.target(), Target::Remote(_)) => {
                if let Target::Remote(name) = router.target() {
                    if let Some(tx) = conns.get(name) {
                        if tx.send_timeout(Message::Heartbeat, SEND_TIMEOUT).await.is_err() {
                            fail_to_local(&mut cap, &mut router);
                        }
                    }
                }
            }
        }
    }
}

/// Spec order: ReleaseControl → (client release_all) → ungrab → Local.
/// `prev` is the target we just left — only it needs ReleaseControl.
async fn to_local(
    cap: &mut EvdevCapture,
    router: &mut Router,
    conns: &HashMap<String, mpsc::Sender<Message>>,
    prev: &Target,
) {
    router.on_disconnected();
    if let Target::Remote(old) = prev {
        if let Some(tx) = conns.get(old) {
            let _ = tx.send_timeout(Message::ReleaseControl, SEND_TIMEOUT).await;
        }
    }
    let _ = cap.ungrab();
}

fn fail_to_local(cap: &mut EvdevCapture, router: &mut Router) {
    tracing::warn!("send failed — returning to local");
    router.on_disconnected();
    let _ = cap.ungrab();
}
```

`src/client.rs` — stub for Task 7, implemented Task 8:

```rust
use std::path::PathBuf;

pub async fn run(_dir: PathBuf, _to: Option<String>, _cfg: Option<PathBuf>) -> anyhow::Result<()> {
    anyhow::bail!("client mode not implemented yet (Task 8)")
}
```

- [ ] **Step 4: Verify**

Run: `cargo build --bin spacing && cargo test --workspace`
Expected: compiles; tất cả tests pass.

- [ ] **Step 5: Commit**

```bash
git add -A
git commit -m "feat(server): serve/pair/fingerprint commands + input routing loop"
```

---

### Task 8: Client binary — connect, inject, watchdog

**Files:**
- Modify: `src/client.rs` (replace stub)
- Modify: `crates/spacing-net/src/lib.rs` if needed (already exposes connect_client with verify closure)

**Interfaces:**
- Consumes: `spacing_net::{connect_client, conn}`, `UinputInject`, `peers`, `config`.
- Produces: working `spacing client --to host:port`.

- [ ] **Step 1: Implement** (no unit-testable new logic — loop wiring; watchdog logic extracted to pure helper tested below)

`src/client.rs`:

```rust
use anyhow::Context;
use spacing_core::config::{load_config, Mode};
use spacing_core::peers::PeerStore;
use spacing_core::protocol::{Message, PROTOCOL_VERSION};
use spacing_inject::UinputInject;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const WATCHDOG_MS: u64 = 500;

pub async fn run(dir: PathBuf, to: Option<String>, cfg_path: Option<PathBuf>) -> anyhow::Result<()> {
    let cfg_path = cfg_path.unwrap_or_else(|| dir.join("config.toml"));
    let cfg = load_config(&cfg_path).ok(); // optional in client mode
    let addr = to
        .or_else(|| cfg.as_ref().and_then(|c| c.connect_to.clone()))
        .context("no server address: pass --to host:port or set connect_to")?;
    if let Some(c) = &cfg {
        anyhow::ensure!(c.mode == Mode::Client, "config mode must be \"client\"");
    }
    let peers = PeerStore::load(&dir.join("peers.toml"))?;
    let id = spacing_net::cert::ensure_identity(&dir)?;
    let tcp = tokio::net::TcpStream::connect(&addr).await?;
    let host = addr.split(':').next().unwrap_or("server");
    let conn = spacing_net::connect_client(tcp, host, id, |fp| peers.allow(fp))
        .await
        .context("TLS connect/fingerprint check failed — run `spacing pair` first")?;
    let (mut rd, mut wr) = (conn.rd_half, conn.wr_half);
    tracing::info!(server = %addr, fp = %conn.peer_fp, "connected");

    let hostname = gethostname::gethostname().to_string_lossy().into_owned();
    spacing_net::conn::send_msg(&mut wr, &Message::Hello {
        version: PROTOCOL_VERSION,
        hostname,
    }).await?;

    let mut inject = UinputInject::create().context(
        "cannot open /dev/uinput — modprobe uinput + apply udev rule in README",
    )?;
    let mut last_rx = Instant::now();
    let mut released = true;
    let mut tick = tokio::time::interval(Duration::from_millis(100));

    loop {
        tokio::select! {
            m = spacing_net::conn::recv_msg(&mut rd) => {
                match m {
                    Ok(Message::Input(ev)) => {
                        match inject.inject(&ev) {
                            Ok(()) => { last_rx = Instant::now(); released = false; }
                            Err(e) => {
                                // spec §5: client báo Error → server về Local;
                                // release phím đang giữ trước khi chết.
                                let _ = spacing_net::conn::send_msg(&mut wr, &Message::Error {
                                    code: 1, message: e.to_string(),
                                }).await;
                                let _ = inject.release_all();
                                anyhow::bail!("inject failed: {e}");
                            }
                        }
                    }
                    Ok(Message::Heartbeat) => { last_rx = Instant::now(); }
                    Ok(Message::ReleaseControl) => { inject.release_all()?; last_rx = Instant::now(); released = true; }
                    Ok(Message::HelloAck{..}) => { last_rx = Instant::now(); }
                    Ok(Message::PairAccept | Message::PairReject) => {}
                    Ok(Message::Error{message, ..}) => tracing::warn!("server error: {message}"),
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("connection lost: {e}");
                        inject.release_all()?;
                        anyhow::bail!("connection lost: {e}");
                    }
                }
            }
            _ = tick.tick() => {
                if !released && last_rx.elapsed() > Duration::from_millis(WATCHDOG_MS) {
                    tracing::warn!("watchdog: no input/heartbeat for {WATCHDOG_MS}ms — releasing keys");
                    inject.release_all()?;
                    released = true;
                }
            }
        }
    }
}
```

`gethostname` is already in root Cargo.toml (Task 1).

- [ ] **Step 2: Verify build**

Run: `cargo build --bin spacing && cargo test --workspace`
Expected: compiles clean; tất cả tests pass.

- [ ] **Step 3: Commit**

```bash
git add -A
git commit -m "feat(client): inject loop with watchdog key release"
```

---

### Task 9: Permissions doc + manual E2E checklist + README

**Files:**
- Create: `README.md`
- Create: `contrib/99-spacing.rules`

**Interfaces:** none (docs).

- [ ] **Step 1: README + udev rule**

`contrib/99-spacing.rules`:

```udev
# Allow users in 'input' group to create virtual devices and read inputs.
KERNEL=="uinput", MODE="0660", GROUP="input", OPTIONS+="static_node=uinput"
SUBSYSTEM=="input", GROUP="input", MODE="0660"
```

`README.md` (phần Quickstart):

```markdown
# spacing

Share one keyboard + mouse across Linux machines on the same LAN.
P1 scope: Linux↔Linux, hotkey switching, TLS + fingerprint pairing.

## Install / permissions

Requires access to `/dev/input/event*` (server) and `/dev/uinput` (client):

    sudo cp contrib/99-spacing.rules /etc/udev/rules.d/
    sudo udevadm control --reload && sudo udevadm trigger
    sudo usermod -aG input $USER   # re-login after this

## Setup (2 machines)

1. On BOTH: `cargo install --path .` (or `cargo build --release`)
2. Server machine: `spacing fingerprint` → note fp; start `spacing serve --pairing` once.
3. Client machine: `spacing pair <server-ip>:24800` → verify fp → `y`.
   Server lưu fingerprint client vào `peers.toml` ngay khi pair — không cần restart.
4. Client: `spacing client --to <server-ip>:24800`.

## config.toml (server)

    mode = "server"
    bind = "0.0.0.0:24800"
    [server.hotkeys]
    local = "Ctrl+Alt+0"
    "Ctrl+Alt+1" = "hostname-of-client"   # = giá trị `hostname` trên máy client
                                        # (client gửi nó trong Hello/PairRequest)

## Known limitations (P1)

- Nhấn hotkey switch giữa lúc modifier (Ctrl/Alt) đang down → modifier có thể
  "kẹt" trên máy server (key-down đã vào OS trước grab, key-up đi remote).
  Workaround: nhấn-thả phím đó một lần. Fix đúng ở P5 (uinput key-up cục bộ).
- IPv6 `host:port` không parse đúng (`host = addr.split(':').next()` — P1 chỉ
  hỗ trợ IPv4/hostname).
- Media/consumer keys (volume...) bị drop ở capture — keyboard page 0x07 only.

## Manual E2E checklist

- [ ] `spacing serve` + `spacing client --to` connect, no errors
- [ ] Type on server → chars appear on client after Ctrl+Alt+1
- [ ] Ctrl+Alt+0 returns control locally instantly
- [ ] Remote→remote (Ctrl+Alt+1 → Ctrl+Alt+2): client cũ không bị kẹt phím (ReleaseControl)
- [ ] Kill client (`kill -9`) mid-typing → server ungrabs, local input works, client releases held keys (watchdog)
- [ ] `spacing pair` from unknown client rejected when not --pairing
- [ ] Scroll wheel trên client đúng tốc độ (không nhanh ×120)
```

- [ ] **Step 2: Commit**

```bash
git add -A
git commit -m "docs: README quickstart + udev rules + E2E checklist"
```

---

## Notes for executor

- `connect_client` takes a verify closure: `|fp| peers.allow(fp)` for
  `client`, `|_| true` for `pair` (already reflected in Tasks 4 & 7).
- `peer_name` = hostname client tự gửi trong `Hello`/`PairRequest` — tên trong
  hotkey config phải khớp hostname đó. Duplicate name on connect: log warn,
  overwrite map entry.
- Wire messages dùng `Hello`/`HelloAck` và `PairAccept` không-mang-fingerprint —
  cố ý đơn giản hoá so với spec (`ClientHello`/`ServerHello`,
  `PairAccept{fingerprint}`): fingerprint đã biết ở TLS layer, không cần gửi lại.
- `EvdevCapture` giữ `Vec<EventStream>`, KHÔNG `into_stream(self)` (consume
  owner thì `grab()` không gọi được — `next_event()` là giải pháp).
- Keystrokes MUST NOT be logged anywhere (Global Constraints).
- If `evdev::uinput` API drifts (crate version), adapt — contract is the
  `to_evdev`/`map_event` map tests, which are version-independent. Tương tự
  với `EventStream::device_mut` (0.12.2 verified — `get_mut` không tồn tại).
- All tasks assume repo root `/home/k0walski/Code/spacing`, git initialized.
