#!/usr/bin/env bash
# Build fully-static binaries for test VMs (glibc-independent).
# The env rustflag only applies because --target is explicit — proc-macros
# are still built for the host with normal flags.
set -euo pipefail
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS="-C target-feature=+crt-static" \
  cargo build --release --target x86_64-unknown-linux-gnu "$@"
