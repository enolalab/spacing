#!/usr/bin/env bash
# Portal backend smoke test on a live Wayland session (GNOME 45+/KDE 6.1+).
# Sandboxes both sides into $XDG_ROOT/{srv,cli} (does not touch
# ~/.config/spacing).
#
# INTERACTIVE — you must be at the machine:
#   1. Approve the "Remote Desktop" dialog when the client starts.
#   2. Push the mouse through the RIGHT screen edge → input gets captured.
#   3. Type a few keys — they travel serve → client → back into this same
#      session (loopback). Press ESC or Ctrl+Alt+0 to return local.
#   4. Ctrl+C each process when done.
#
# Usage: contrib/portal-smoke.sh setup|serve|client
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
BIN="${BIN:-$ROOT/target/debug/spacing}"
XDG_ROOT="${XDG_ROOT:-/tmp/spacing-portal-smoke}"
SRV="$XDG_ROOT/srv"
CLI="$XDG_ROOT/cli"
H="$(hostname)"

case "${1:-}" in
setup)
    mkdir -p "$SRV/spacing" "$CLI/spacing"
    cat > "$SRV/spacing/config.toml" <<EOF
mode = "server"
bind = "127.0.0.1:24800"
backend = "portal"

[server.hotkeys]
local = "Ctrl+Alt+0"
"Ctrl+Alt+1" = "$H"

[server.layout.edges]
right = "$H"
EOF
    cat > "$CLI/spacing/config.toml" <<EOF
mode = "client"
connect_to = "127.0.0.1:24800"
backend = "portal"
EOF
    echo "server fp: $(XDG_CONFIG_HOME=$SRV "$BIN" fingerprint)"
    echo "client fp: $(XDG_CONFIG_HOME=$CLI "$BIN" fingerprint)"
    echo "next: '$0 serve' in terminal 1, then '$0 client' in terminal 2"
    ;;
serve)
    export XDG_CONFIG_HOME="$SRV"
    exec "$BIN" serve --pairing --config "$SRV/spacing/config.toml"
    ;;
client)
    export XDG_CONFIG_HOME="$CLI"
    echo "1) pairing (one-shot)…"
    "$BIN" pair "127.0.0.1:24800" || true
    echo "2) starting client — APPROVE the Remote Desktop dialog"
    exec "$BIN" client --to "127.0.0.1:24800" --config "$CLI/spacing/config.toml"
    ;;
*)
    echo "usage: $0 setup|serve|client" >&2
    exit 2
    ;;
esac
