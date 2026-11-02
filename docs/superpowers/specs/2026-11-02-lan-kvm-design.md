# spacing — Software KVM chia sẻ phím/chuột qua LAN (design spec)

Ngày: 2026-11-02 · Trạng thái: Approved (chờ user review file)

## 1. Mục tiêu

Daemon viết bằng Rust cho phép các máy tính trong cùng LAN dùng chung một bộ
bàn phím + chuột. Một máy "server" (có thiết bị vật lý) forward input tới các
máy "client" (inject vào OS). KHÔNG phải remote desktop — không truyền màn
hình, chỉ truyền input và clipboard text.

## 2. Quyết định đã chốt (từ brainstorming)

| Câu hỏi | Quyết định |
|---|---|
| OS các máy đích | Linux Wayland, Linux X11, Windows, macOS (cả 4) |
| Cách chuyển máy | Cả hai: hotkey (MVP) + đẩy chuột qua mép màn hình |
| Tính năng thêm | Clipboard text only (cap 1MB). KHÔNG file drag-drop |
| Bảo mật | TLS 1.3 + cert tự ký, pairing bằng fingerprint |
| Form factor | Một binary daemon duy nhất + file config TOML |
| Kiến trúc I/O | Portal-first trên Wayland, evdev/uinput fallback |

## 3. Kiến trúc tổng quan

### Topology

Star: 1 server ↔ N client. Tại một thời điểm input đi tới đúng 1 target
(`Local` = chính máy server, hoặc `Remote(client_id)`). Chỉ server khởi
tạo switch; client thuần nhận event rồi inject.

### Cargo workspace

| Crate | Vai trò |
|---|---|
| `spacing` | Binary duy nhất: `spacing serve`, `spacing client`, `spacing pair` |
| `spacing-core` | Protocol types, router state machine, config parse — pure Rust, không I/O, testable |
| `spacing-capture` | Trait `CaptureBackend` + impls: `portal` (libei/InputCapture), `evdev`, `windows` (LL hooks), `macos` (CGEventTap) |
| `spacing-inject` | Trait `InjectionBackend` + impls: `portal`/`libei`, `uinput`, `windows` (SendInput), `macos` (CGEventTap) |
| `spacing-net` | TCP + rustls transport, framing, handshake/pairing |

Tách capture/inject vì một máy có thể vừa là server (cần capture) vừa là
client của máy khác (cần inject).

### Traits

```rust
trait CaptureBackend {
    async fn grab(&mut self) -> Result<()>;       // chiếm input vật lý
    async fn ungrab(&mut self) -> Result<()>;
    fn events(&self) -> impl Stream<Item = InputEvent>;   // chỉ emit khi grabbed
    async fn poll_hotkeys(&self) -> Option<Hotkey>;       // check trước khi forward
}

trait InjectionBackend {
    async fn inject(&mut self, ev: &InputEvent) -> Result<()>;
    async fn set_clipboard(&mut self, text: &str) -> Result<()>;
    async fn release_all(&mut self) -> Result<()>;  // key-up mọi phím đang down
}
```

Backend portal expose thêm barrier/edge events (P2).

### Wire format

- Binary, length-prefixed frames qua TCP+TLS1.3 (rustls, cert tự ký).
- Serialize: `postcard` + `serde`. Enum `Message` versioned ngay từ đầu.
- **Canonical key representation = USB HID usage IDs.** Mỗi backend map
  native scancode/keycode ↔ HID. Tránh vấn đề layout (event truyền vị trí
  phím vật lý, client OS tự map ký tự theo layout của nó — đúng hành vi
  Synergy).

### Protocol messages (v1)

```
ClientHello { protocol_version, hostname }
ServerHello { protocol_version }
PairRequest / PairAccept { fingerprint } / PairReject
InputEvent = KeyDown(HidCode) | KeyUp(HidCode)
           | MouseMoveRel { dx, dy } | MouseMoveAbs { x, y }   // abs: P2 portal
           | ButtonDown(Btn) | ButtonUp(Btn) | Wheel { dx, dy }
ClipboardSet { text }          // cap 1MB, utf-8
ReleaseControl                 // server thông báo trả quyền về local
Heartbeat                      // mỗi 250ms khi remote active
Error { code, message }
```

## 4. Data flow

### Server

1. State `Local` (mặc định): input vào OS bình thường, backend không grab.
   Chỉ có hotkey listener chạy.
2. Hotkey `Ctrl+Alt+N` (hoặc barrier crossing — P2) → router set
   `ActiveTarget::Remote(id)` → `grab()` → mọi `InputEvent` serialize →
   TLS → client tương ứng.
3. Quay về (hotkey local / barrier ngược): gửi `ReleaseControl` + flush
   mọi phím đang down (key-up) → `ungrab()` → state `Local`. Cursor trả
   về mép do compositor lo (portal) hoặc vị trí ảo clamped (evdev, P5).
4. Hotkey check TRƯỚC khi forward event → `Ctrl+Alt+0` (về local) luôn
   thoát được kể cả khi đang remote.

### Client

Frame → decode → `InjectionBackend::inject()`. Keyboard state tracker ghi
nhận phím đang down (phục vụ `release_all`).

### Clipboard

Cả hai phía watch clipboard. Khi switch target, hoặc clipboard local đổi
trong lúc remote active → gửi `ClipboardSet`. Latest-wins. Text only.

### Pairing (lần đầu)

1. `spacing pair <ip>:<port>` ở client → TLS connect tới server.
2. Server hiện SHA256 fingerprint của peer cert + chờ user confirm.
3. Confirm → lưu vào `peers.toml` ở cả hai phía. Mọi kết nối sau bắt buộc
   fingerprint khớp (TOFU — trust on first use).

## 5. Error handling

Nguyên tắc vàng: **không bao giờ để mất quyền điều khiển phím chuột vật lý.**

| Tình huống | Hành vi |
|---|---|
| Mất kết nối khi remote | Server auto-ungrab → `Local`. Client watchdog: >500ms không event/heartbeat → `release_all()` |
| Inject fail phía client | Client gửi `Error` → server về `Local` + log |
| Auth fail / fingerprint lạ | Reject + log fingerprint |
| Grab fail (permission) | Daemon từ chối vào remote, in hướng dẫn fix (input group / portal dialog) |
| Frame hỏng, version mismatch | Đóng kết nối, về `Local` — fail-safe |
| Client crash | TCP close → server detect → ungrab về `Local` |

## 6. Config (`~/.config/spacing/config.toml`)

```toml
mode = "server"              # hoặc "client"
bind = "0.0.0.0:24800"       # server; client: connect_to = "ip:24800"

[server.hotkeys]
local = "Ctrl+Alt+0"
"Ctrl+Alt+1" = "laptop"
"Ctrl+Alt+2" = "desktop"

[server.layout.edges]        # P2 trở đi
right = "laptop"
```

`peers.toml` (tự sinh, không sửa tay): `name`, `fingerprint`, `last_seen`.

## 7. Testing

- `spacing-core`: unit test router state machine (mọi transition), config
  parse, **proptest** encode/decode roundtrip cho mọi `Message`.
- `spacing-net`: integration test loopback — cặp cert self-signed, pair,
  echo event end-to-end.
- Backends: fake `CaptureBackend`/`InjectionBackend` in-memory để test
  router độc lập OS. evdev/uinput test trong container có `/dev/uinput`.
- Ma trận manual test per OS (giới hạn cố hữu của loại app này).

## 8. Phasing — mỗi phase chạy end-to-end được

- **P1 — MVP Linux↔Linux, hotkey only**: evdev grab + uinput inject,
  TCP+TLS, fingerprint pairing, router. Chứng minh full đường dây.
- **P2 — Wayland native**: clipboard + InputCapture portal (capture) +
  RemoteDesktop/libei (inject) + edge-switch qua portal barriers.
- **P3 — Windows** (LL hooks + SendInput + clipboard).
- **P4 — macOS** (CGEventTap + clipboard).
- **P5 — Polish**: edge-switch cho evdev path (vị trí ảo), auto-reconnect,
  mDNS discovery, systemd unit.

## 9. Non-goals (YAGNI)

- Remote display/video/audio. File drag-drop. Multi-directional switching
  (client chủ động điều khiển server). GUI/tray app. Internet/WAN relay.
- Mobile OS.

## 10. Rủi ro đã biết

- InputCapture portal chỉ có trên GNOME 47+/KDE 6.1+ → evdev fallback phải
  hoạt động tốt cho desktop cũ.
- Edge-switch trên evdev path không warp được cursor trên Wayland → chấp
  nhận trải nghiệm kém hơn ở P5, hoặc chỉ hỗ trợ edge-switch qua portal.
- macOS cần user cấp quyền Accessibility/Input Monitoring thủ công.
- Windows: LL hooks bị một số app elevated chặn (UIPI) — tài liệu hoá giới
  hạn, không chạy-elevated-by-default.
