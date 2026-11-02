# P3 — Windows + macOS backends

## Goal

Native capture/inject on Windows and macOS behind the existing
`CaptureBackend`/`InjectionBackend` traits, with screen-edge switching
(the same UX as the portal backend: push the pointer through the
configured edge to take control; `Ctrl+Alt+0` releases).

## Decisions

| Area | Choice | Why |
|---|---|---|
| Windows capture | `WH_KEYBOARD_LL` + `WH_MOUSE_LL` low-level hooks | Same model Barrier/Deskflow use; lets us suppress events while remote (return non-zero) |
| Windows inject | `SendInput` with `KEYEVENTF_SCANCODE` | Scan codes are positional/layout-independent — matches evdev/HID model; VK varies with layout |
| Windows key map | evdev ≈ PS/2 set-1 scancode, plus E0-table for extended keys | evdev codes 1..127 are literally set-1 make codes for the main block |
| macOS capture | `CGEventTapCreate` (session event tap, run-loop thread) | Intercept + drop events while remote; needs Accessibility permission |
| macOS inject | `CGEventPost` (`CGEventCreateKeyboardEvent`/`MouseEvent`) | Standard path (Karabiner/Deskflow equivalent); needs Input Monitoring |
| macOS key map | explicit evdev→`CGKeyCode` table | CGKeyCode is its own numbering |
| Edge detect (both) | own `EdgeDetector`: cursor clamped at edge + N consecutive outward deltas ⇒ `Activated` | Portable equivalent of InputCapture barriers (Barrier/Deskflow technique) |
| Cursor restore | on deactivate, warp cursor back inside the screen (`SetCursorPos` / `CGWarpMouseCursorPosition`) | Leave pointer where it was visually |
| Crate (win) | `windows` (minimal feature set) | Official bindings, names match Win32 docs |
| Crates (mac) | `core-graphics` + `core-foundation` | EventTap/CGEvent + CFRunLoop |
| Testing | `cargo check` per target (msvc/darwin installed); unit tests for keymaps + EdgeDetector on Linux | No Win/mac hardware in this env — mark platform rows "implemented, unverified on hardware" |

## Contracts

- `CaptureBackend` gains no changes; `select_capture` picks `windows`/`macos` under `backend="auto"` per OS. `CaptureEvent::Activated`/`Deactivated` already exist (from portal) — reuse.
- `EdgeDetector` (core or capture crate): `feed(x, y, dx, dy) -> Option<Edge>`; params: screen rect, edge set, threshold (~3 consecutive outward pushes within 1.5 s). Unit-tested.
- Hotkeys still work on both platforms (LL hooks / event tap see keys while local) — `Ctrl+Alt+N` direct-switch + `Ctrl+Alt+0` release identical to evdev path.
- `InjectionBackend` unchanged: evdev `InputEvent` in → platform events out.
- New cfg deps only under `[target.'cfg(...)'.dependencies]` — Linux build unchanged.

## Tasks

1. **T1** `spacing-core`: `keymap::win` (evdev→scancode+E0) + `keymap::mac` (evdev→CGKeyCode) tables + roundtrip/unit tests.
2. **T2** `EdgeDetector` + tests (threshold, timeout, clamp-at-corner ambiguity).
3. **T3** `spacing-capture/src/windows.rs`: message-pump thread, LL hooks, suppress-while-remote flag, edge detect → `Activated`; `release()` unhooks/restores cursor. `cargo check --target x86_64-pc-windows-msvc`.
4. **T4** `spacing-inject/src/windows.rs`: SendInput keyboard (scan codes) + mouse (move/buttons/wheel). `cargo check`.
5. **T5** `spacing-capture/src/macos.rs`: CGEventTap thread + edge detect; `release()` via CGWarpMouseCursorPosition. `cargo check --target aarch64-apple-darwin`.
6. **T6** `spacing-inject/src/macos.rs`: CGEventPost injectors. `cargo check`.
7. **T7** wiring + docs: `select_capture/select_inject` per-OS, `backend="auto"` matrix, README platform rows + permission notes (Win: none beyond user-level hooks; macOS: Accessibility + Input Monitoring TCC), roadmap marks.

## Risks / explicitly out of scope

- No Windows/macOS hardware here ⇒ code is API-correct but **runtime-unverified**; README marks it so.
- Windows: LL hook proc runs on a dedicated thread w/ its own message loop; suppression flag is a simple `AtomicBool`.
- macOS: TCC dialogs are OS-owned (cannot be auto-approved) — document the two toggles users must grant.
- Cursor clamp while remote: we swallow motion but don't fight the user moving the real mouse — same behaviour as InputCapture.

## Status (as-built)

All tasks implemented and cross-checked:

- `spacing-core`: `keymap_win` (evdev↔PS/2 scan+E0), `keymap_mac`
  (evdev↔CGKeyCode), `edge::EdgeDetector` — 36 tests green.
- `spacing-capture/src/windows.rs` — `WH_KEYBOARD_LL`/`WH_MOUSE_LL` on a
  dedicated pump thread; suppress flag; clamped-cursor edge detection.
- `spacing-inject/src/windows.rs` — `SendInput` scan codes + mouse.
- `spacing-capture/src/macos.rs` — `CGEventTap` on a CFRunLoop thread.
- `spacing-inject/src/macos.rs` — `CGEventPost` (keys/buttons/wheel,
  drag-aware moves, virtual cursor tracking).
- Per-OS `select_capture`/`select_inject`; Linux deps cfg-gated.
- `cargo check` green: `x86_64-pc-windows-msvc` + `aarch64-apple-darwin`
  for the platform crates (full binary needs a native toolchain —
  `aws-lc-sys`/`ring` C build; normal: Windows users build on Windows).

Deviations from the decisions table: used `windows-sys` (lighter than
`windows`) and direct CoreGraphics/CoreFoundation FFI decls (no extra
crates) — same APIs, fewer deps. Cursor warp on release was dropped:
the pointer simply resumes at the edge — identical to portal behaviour.
