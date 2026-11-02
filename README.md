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

Client config (optional — có thể dùng `--to` thay):

    mode = "client"
    connect_to = "192.168.1.5:24800"

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
