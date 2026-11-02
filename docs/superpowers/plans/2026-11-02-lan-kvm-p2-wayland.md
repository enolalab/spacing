# P2 Implementation Plan — Wayland-native (portal + clipboard + edge switch)

**Spec:** `docs/superpowers/specs/2026-11-02-lan-kvm-design.md` §P2
**Prereqs verified:** host runs GNOME Shell 50.1 on Wayland,
`xdg-desktop-portal-gnome 50` → InputCapture + RemoteDesktop(ConnectToEIS)
both available for real testing. wlr portal implements neither — VMs keep
the evdev path (unchanged, regression-only).

## Decisions

1. **Backend abstraction:** `#[async_trait]` traits `CaptureBackend` /
   `InjectionBackend` (spec §Traits) + `Box<dyn>` selected by config
   `backend = "auto"|"evdev"|"portal"` (default `auto` → portal when
   `WAYLAND_DISPLAY` set + portal answers, else evdev/uinput).
2. **ei codec:** `reis` (feature `tokio`) for BOTH directions —
   `ei::Context` on the capture side (InputCapture hands us an ei fd),
   `eis::Context` on the inject side (RemoteDesktop ConnectToEIS makes us
   the emulated-input server). Key events use evdev keycodes ↔ existing
   `hidmap` — same canonical HID wire format as evdev path.
3. **CaptureEvent stream:** `next_event()` yields
   `enum CaptureEvent { Input(InputEvent), Activated{zone}, Deactivated }`
   so portal barrier crossings reach the router; evdev impl only emits
   `Input`.
4. **Edge switch:** config `[server.layout.edges] right = "laptop"` —
   barriers placed on the desktop bounding-box edges; `Activated{zone}`
   → `SwitchToRemote`, `Deactivated`/`Released` → `SwitchToLocal`.
   On portal path, *forwarding hotkeys do not exist* (portal only
   delivers events while captured) — `Ctrl+Alt+0` still works because the
   ei stream carries keys during capture. Documented.
5. **grab()/ungrab() on portal:** grab = no-op (compositor already
   captures on activation); ungrab = `release()` re-arming barriers.
6. **Clipboard:** `arboard` (Wayland + X11). Poll ~300 ms (GNOME has no
   ext-data-control). Latest-wins with **echo suppression**: remember the
   last text we applied remotely; watcher fires only when content differs
   from both last-seen and last-applied. Enforce `CLIPBOARD_MAX` (1 MiB)
   on send AND recv.
7. **RemoteDesktop inject:** `reis::eis` context — handshake binds the
   compositor-created seat/devices; emit evdev keycodes (compositor maps
   to its own layout — Synergy semantics). `MouseMoveAbs` only when the
   device advertises abs pointer; `release_all` keys-up the held set.

## Deps (pinned ≥7d-old releases)

- `ashpd = { version = "0.13", features = ["input_capture", "remote_desktop", "wayland"] }`
- `reis = { version = "0.7", features = ["tokio"] }`
- `arboard = "3"`
- `async-trait = "0.1"`

## Tasks (TDD)

### T1 — Backend traits + selection (refactor, all tests stay green)

- `spacing-capture`: `trait CaptureBackend { grab, ungrab, next_event ->
  Option<CaptureEvent> }`; `EvdevCapture` implements it (Input only).
  `backend::select_capture(&cfg) -> Box<dyn CaptureBackend>` (auto-detect).
- `spacing-inject`: `trait InjectionBackend { inject, release_all,
  set_clipboard }`; `UinputInject` implements it (`set_clipboard` →
  clipboard module from T2).
- server.rs/client.rs take `Box<dyn>` — no behaviour change.
- Tests: existing 25 must pass unchanged; +select picks evdev when no
  WAYLAND_DISPLAY.

### T2 — Clipboard plumbing

- `spacing-inject::clipboard`: `trait Clipboard { get_text, set_text }` +
  `ArboardClipboard` (auto Wayland/X11) + `NullClipboard` (headless warn).
- server: poll arm (300 ms) — on change while `Target::Remote` →
  `ClipboardSet` to that conn; on `SwitchToRemote` → push current text
  (post-grab). reader_task extended: `CtrlMsg::Dead(name)` +
  `CtrlMsg::Clipboard(name, text)` — sets server clipboard (suppress echo).
- client: poll arm — local change → `ClipboardSet` to server; inbound
  `ClipboardSet` → `set_text` (suppress echo); `>CLIPBOARD_MAX` → drop +
  `Message::Error` reply.
- Tests (pure logic): `ClipDedup` — change detected once, applied-remote
  not re-sent, >1 MiB rejected, empty→nonempty→empty transitions.

### T3 — ei codec helpers (shared)

- `ei -> InputEvent`: key/button/motion/scroll events → evdev codes →
  `evdev_to_hid`; `InputEvent -> eis emits`: `hid_to_evdev` + per-variant
  emit fns on `reis::eis::Device`.
- In-process loopback: `reis::eis` sender ↔ `reis::ei` receiver over a
  `UnixStream` pair — assert decode(encode(ev)) == ev for all
  `arb_event()` (proptest roundtrip through real ei protocol frames).

### T4 — PortalCapture (server side)

- ashpd InputCapture: `create_session2`+`start` with v1 fallback; caps
  Keyboard|Pointer; `zones()` → build barriers from `layout.edges` on the
  bounding box; `set_pointer_barriers`; `enable`.
- `connect_to_eis` → `ei::Context` → tokio event stream → T3 decode →
  `CaptureEvent::Input`.
- Activated/Deactivated signals → `CaptureEvent` variants → router.
- `grab()` no-op; `ungrab()` → `release()` (reposition cursor).
- Tests: eis-driven fake session feeding Activated + key events; decode
  correctness covered by T3 loopback. Real portal: host manual smoke.

### T5 — PortalInject (client side)

- ashpd RemoteDesktop: session → `select_devices` Keyboard|Pointer →
  `start` → `connect_to_eis` → `eis::Context` → handshake → first device
  with pointer+keyboard caps.
- `inject()` → T3 emits; `release_all()` → key-up held; `set_clipboard`
  → T2 clipboard.
- Tests: reis loopback (eis→ei) assert emitted event stream equals input;
  release_all emits key-up for held only. Host manual smoke: inject
  "hello" into focused window.

### T6 — Config + router

- `Config.server.backend`, `[server.layout.edges]` map; `client.backend`.
- `Router` gains `edges: Vec<(zone,name)>` + `on_edge(zone)` /
  `on_capture_end()` → existing `SwitchToRemote`/`SwitchToLocal`
  decisions (reuse switch/disconnect logic — same invariants).
- Tests: config parse; on_edge known/unknown zone; edge switch while
  remote (remote→remote ReleaseControl ordering unchanged).

### T7 — Integration + docs

- `auto` selection: WAYLAND_DISPLAY + portal probe → portal, else evdev
  with info log.
- Host smoke script: session create, barrier setup, one real edge cross
  (manual), clipboard roundtrip, client inject into a focused window.
- README: matrix rows flip to "supported (GNOME 47+/KDE 6.1+)",
  portal prompt note, edge config docs, new limitations (hotkey→remote
  n/a on portal; GNOME-46 barrier-change bug note).
- VM regression: evdev E2E unchanged (re-run loopback bench once).

## Failure-mode invariants (unchanged golden rules)

- Any portal error while Remote → same `to_local`/`fail_to_local` path.
- `Released`/connection drop → release_all + ungrab-equivalent.
- No keystroke logging (REIS_DEBUG off by default; document env var like
  SPACING_WIRE_DUMP).
- Capture session must never outlive the process without release —
  session Drop impl closes it.

## Out of scope (P3+/P5)

- Cursor warp/position restore on return (portal `release()` takes
  position hints — v1 uses default).
- Multi-monitor barrier geometry beyond bounding-box edges.
- Clipboard images/files (text only, 1 MiB, per spec).
- Input-leap-style edge switching on evdev path (P5).
