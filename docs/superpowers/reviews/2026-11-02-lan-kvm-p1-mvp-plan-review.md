# Plan Review: spacing P1 — Linux↔Linux Hotkey-Only MVP

**Mode:** Focused
**Scope:** toàn bộ plan (9 tasks) + traceability về spec
**Plan:** `docs/superpowers/plans/2026-11-02-lan-kvm-p1-mvp.md` @ 86802d2
**Spec:** `docs/superpowers/specs/2026-11-02-lan-kvm-design.md`
**Reviewers:** inline — technical + security pass (không dispatch subagent)

## Verdict: Needs Changes — đã sửa inline, sẵn sàng execute

Plan có cấu trúc task TDD tốt, interface được ghi rõ, nhưng code mẫu chưa qua
compile check nên chứa 4 compile blocker và một số lỗi correctness vi phạm
"golden rule" (không bao giờ kẹt input). Tất cả đã được sửa trực tiếp trong
plan ở lượt review này.

## Blocking Findings (đã sửa)

1. **F1 — `Identity` derive(Clone) không compile** — Critical — Blocking: yes
   - Evidence: Task 4 `cert.rs` — `PrivateKeyDer` không impl `Clone` (chỉ `clone_key()`).
   - Impact: `cargo build` fail ngay tại Task 4.
   - Fix: manual `impl Clone for Identity` dùng `key.clone_key()`.

2. **F2 — `UnixTimestamp` không tồn tại trong rustls 0.23** — Critical — Blocking: yes
   - Evidence: `verify.rs` import `pki_types::UnixTimestamp`; type đúng là `UnixTime`.
   - Fix: đổi import + 2 signature `verify_*_signature`/`verify_server_cert`.

3. **F3 — `cap.into_stream()` consume `cap`, nhưng `cap.grab()` gọi sau đó** — Critical — Blocking: yes
   - Evidence: Task 5 `into_stream(self)` consume struct; `server.rs` dùng `cap.grab()/ungrab()` trong loop.
   - Impact: borrow/move error — thiết kế không tồn tại được.
   - Fix: `EvdevCapture` giữ `Vec<evdev::EventStream>`; `grab()/ungrab()` qua `s.get_mut()`;
     API mới `next_event(&mut self) -> Option<InputEvent>` (borrow `&mut self` chỉ trong
     lúc await → `select!` trong server.rs hoạt động).

4. **F4 — Workspace không parse được ở Task 1** — Critical — Blocking: yes
   - Evidence: root `[dependencies]` trỏ `crates/{net,capture,inject}` chưa tồn tại →
     `cargo test -p spacing-core` fail từ bước đầu.
   - Fix: Task 1 tạo skeleton `Cargo.toml` + `lib.rs` rỗng cho 3 crate; deps điền dần ở các task sau.

5. **F5 — Wheel gửi `REL_WHEEL = dy*120` → scroll nhanh 120×** — Important — Blocking: yes
   - Evidence: `map.rs::to_evdev` push `(0x08, dy*120)`; `0x08` là lo-res detent axis.
   - Fix: emit `REL_WHEEL=dy` + `REL_WHEEL_HI_RES(0x0B)=dy*120` (giống chuột thật); test cập nhật.

6. **F6 — Remote→remote switch không release peer cũ + switch tới peer offline** — Important — Blocking: yes
   - Evidence: `server.rs` `SwitchToRemote` chỉ `cap.grab()` — modifier/key đang down trên
     client cũ kẹt tới watchdog; peer chưa connect vẫn bị grab → input biến mất.
   - Fix: check `conns.contains_key(&name)` trước grab; gửi `ReleaseControl` tới remote cũ khi đổi remote.

7. **F7 — PeerStore clone per-connection → client vừa pair phải restart server** — Important — Blocking: yes
   - Evidence: `accept_server` nhận `PeerStore` by value, wrap `Arc` mới → insert chỉ vào clone.
   - Fix: signature nhận `Arc<RwLock<PeerStore>>` chia sẻ từ `server.rs`; paired client connect ngay; README bỏ bước restart.

8. **F8 — `grab()` fail giữa chừng để một số device kẹt grab** — Important — Blocking: yes
   - Evidence: grab loop `d.grab()?` — lỗi ở device N giữ N-1 device bị grab dù router về Local.
   - Fix: grab fail → `ungrab()` tất cả rồi mới trả lỗi.

9. **F9 — Client inject fail bail mà không gửi `Error` / `release_all`** — Important — Blocking: yes
   - Evidence: spec §5 yêu cầu "Client gửi Error → server về Local"; client.rs dùng `inject(&ev)?`.
   - Fix: gửi `Message::Error` + `release_all()` rồi mới bail.

10. **F10 — `cargo test` với `default-members=["."]` bỏ qua test các crate** — Important — Blocking: no
    - Fix: mọi verify command → `cargo test --workspace`; Global Constraints cập nhật.

11. **F11 — Thiếu test pairing window (spec §7 yêu cầu)** — Important — Blocking: no
    - Thêm `pairing_window_registers_client_once` vào `loopback.rs`: PairRequest → PairAccept →
      fingerprint persist vào peers.toml + window đóng.

12. **F12 — Thiếu proptest encode/decode roundtrip cho `Message` (spec §7 yêu cầu)** — Important — Blocking: no
    - Thêm strategy + roundtrip test vào `protocol.rs` (Task 1); `Message` derive thêm `PartialEq`.

## Follow-ups (đã patch luôn vì rẻ)

- `requires_raw_public_keys()` cho `AcceptAnyServerCert` (required ở rustls ≥0.23.15).
- `PeerStore::seen()` cập nhật `last_seen` khi Hello (spec khai báo field này).
- `to_local` gửi `ReleaseControl` chỉ tới remote cũ (không broadcast).
- `release_all` track cả mouse buttons (BTN_* cũng có thể kẹt).
- `inject` emit một batch `dev.emit(&out)` thay vì từng event.
- Notes mới: `Hello/HelloAck` thay `ClientHello/ServerHello` và `PairAccept` không mang
  fingerprint — intentional (fp đã biết ở TLS layer); IPv6 `host:port` chưa hỗ trợ;
  modifier kẹt trên server local sau grab giữa combo = documented limitation P1 (xử lý P5
  bằng uinput key-up cục bộ).
- README: tên trong hotkey config = hostname của client; thêm mục E2E remote→remote.

## Coverage and Resolution

| Spec criterion | Plan location | Status |
|---|---|---|
| Hotkey switch, check trước forward (§4) | Task 2 router + Task 7 | covered |
| ReleaseControl → release_all → ungrab (§4.3) | server.rs to_local + client.rs | covered (sau F6) |
| Heartbeat 250ms / watchdog 500ms (§5) | server.rs hb arm + client.rs tick | covered |
| TLS + fingerprint TOFU pairing (§2) | Task 4 + `spacing pair` | covered (sau F7, F11) |
| Auth fail → reject + log fp (§5) | accept_server/verifier | covered |
| Frame hỏng/version mismatch → đóng + local (§5) | conn::recv_msg cap + Hello check + reader_task | covered |
| Grab fail → refuse remote + hướng dẫn (§5) | SwitchToRemote arm + enumerate context | covered (sau F8) |
| Inject fail → Error → local (§5) | client.rs sau F9 + reader_task | covered |
| proptest Message roundtrip (§7) | protocol.rs sau F12 | covered |
| Loopback TLS + pair test (§7) | loopback.rs sau F11 | covered |
| peers.toml name/fingerprint/last_seen (§6) | peers.rs + seen() | covered |
| Media keys dropped — documented constraint | map.rs + Global Constraints | covered |

Strengths: phân rã crate sạch, router là pure state machine test được không cần OS,
fail-safe paths có mặt ở đúng chỗ, code mẫu đủ cụ thể để execute không cần reopen
quyết định kiến trúc.
