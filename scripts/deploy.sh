#!/usr/bin/env bash
# Cross-compiles argonone-rs, ships the binary + systemd unit to a Pi over
# SSH, and (re)starts the service — the manual sequence from README.md's
# Troubleshooting section, scripted, with the exact failure modes hit
# during real deployment guarded against:
#   - copying the host build instead of the cross-compiled aarch64 one
#     (systemd's "status=203/EXEC")
#   - the old Python argononed.service still enabled, fighting for the
#     same I2C bus
#   - a fresh headless boot's plymouth-quit-wait.service stalling the
#     unit's start indefinitely
#
# This does NOT do the one-time hardware setup (enabling I2C in
# /boot/firmware/config.txt, which needs a reboot) — see README.md's
# Installation section for that, once, before the first deploy.
#
# Usage: scripts/deploy.sh <ssh-host> [--skip-build] [--no-restart]
#   <ssh-host>     SSH destination (see ~/.ssh/config), e.g. euclides or pi@192.168.1.50
#   --skip-build   Reuse the existing cross-compiled binary instead of rebuilding
#   --no-restart   Copy files but don't enable/restart the systemd service
#
# Requires locally: `rustup target add aarch64-unknown-linux-gnu` and a
# cross linker (see README.md's "Cross-compile for Raspberry Pi" section).
# Requires on the remote host: sudo access (you'll be prompted).

set -euo pipefail

usage() {
  echo "Usage: $0 <ssh-host> [--skip-build] [--no-restart]" >&2
  exit 1
}

[ $# -ge 1 ] || usage
HOST="$1"
shift

SKIP_BUILD=0
NO_RESTART=0
for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD=1 ;;
    --no-restart) NO_RESTART=1 ;;
    *) usage ;;
  esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="aarch64-unknown-linux-gnu"
BINARY="$REPO_ROOT/target/$TARGET/release/argonone-rs"
UNIT="$REPO_ROOT/packaging/systemd/argonone-rs.service"

if [ "$SKIP_BUILD" -eq 0 ]; then
  echo "==> Cross-compiling for $TARGET..."
  if ! rustup target list --installed | grep -qx "$TARGET"; then
    echo "ERROR: rustup target '$TARGET' not installed. Run: rustup target add $TARGET" >&2
    exit 1
  fi
  if ! command -v "${TARGET}-gcc" >/dev/null 2>&1; then
    echo "ERROR: cross linker '${TARGET}-gcc' not found on PATH." >&2
    echo "  See README.md's 'Cross-compile for Raspberry Pi' section." >&2
    exit 1
  fi
  CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER="${TARGET}-gcc" \
    cargo build --release --target "$TARGET" --manifest-path "$REPO_ROOT/Cargo.toml"
else
  echo "==> --skip-build: reusing existing binary at $BINARY"
fi

if [ ! -f "$BINARY" ]; then
  echo "ERROR: binary not found at $BINARY (build it first, or drop --skip-build)" >&2
  exit 1
fi

# Guard against the exact mistake that bit us during manual deployment:
# shipping the native host build instead of the cross-compiled one.
ARCH_CHECK="$(file -b "$BINARY")"
case "$ARCH_CHECK" in
  *aarch64*) ;;
  *)
    echo "ERROR: $BINARY doesn't look like an aarch64 binary:" >&2
    echo "  $ARCH_CHECK" >&2
    exit 1
    ;;
esac
echo "==> Binary OK: $ARCH_CHECK"

echo "==> Copying binary and systemd unit to $HOST..."
scp -q "$BINARY" "$HOST:/tmp/argonone-rs"
scp -q "$UNIT" "$HOST:/tmp/argonone-rs.service"

echo "==> Installing on $HOST (sudo required)..."
ssh -t "$HOST" bash -s -- "$NO_RESTART" <<'REMOTE'
set -euo pipefail
NO_RESTART="$1"

sudo install -m 755 /tmp/argonone-rs /usr/local/bin/argonone-rs
rm -f /tmp/argonone-rs
sudo install -m 644 /tmp/argonone-rs.service /etc/systemd/system/argonone-rs.service
rm -f /tmp/argonone-rs.service
sudo systemctl daemon-reload

# The old Python daemon fights for the same I2C bus if left enabled.
if systemctl list-unit-files 2>/dev/null | grep -q '^argononed\.service' \
   && { systemctl is-enabled --quiet argononed.service 2>/dev/null \
        || systemctl is-active --quiet argononed.service 2>/dev/null; }; then
  echo
  echo "The legacy argononed.service (Python daemon) is still enabled/running."
  echo "It will conflict with argonone-rs over the I2C bus (0x1a) if both run."
  read -r -p "Disable it now? [Y/n] " REPLY
  if [ -z "$REPLY" ] || [ "$REPLY" = "y" ] || [ "$REPLY" = "Y" ]; then
    sudo systemctl disable --now argononed.service
  else
    echo "Leaving argononed.service as-is — expect I2C contention."
  fi
fi

if [ "$NO_RESTART" = "1" ]; then
  echo "==> --no-restart: skipping enable/start."
  echo "    Run 'sudo systemctl enable --now argonone-rs' on $HOSTNAME when ready."
  exit 0
fi

echo "==> Enabling and (re)starting argonone-rs..."
sudo systemctl enable argonone-rs
# `enable --now` is a no-op on an already-running service — restart
# explicitly so a redeploy actually picks up the new binary.
sudo systemctl restart argonone-rs

# Guard against the plymouth-quit-wait boot-stall hit during manual
# deployment: if the start is still queued after a few seconds, unstick it.
for _ in 1 2 3 4 5; do
  [ "$(systemctl is-active argonone-rs 2>/dev/null || true)" = "active" ] && break
  sleep 2
done
if [ "$(systemctl is-active argonone-rs 2>/dev/null || true)" != "active" ] \
   && systemctl list-jobs 2>/dev/null | grep -q plymouth-quit-wait; then
  echo "==> argonone-rs is queued behind plymouth-quit-wait.service — unsticking it..."
  sudo systemctl stop plymouth-quit-wait.service
  sleep 2
fi

echo
sudo systemctl status argonone-rs --no-pager || true
REMOTE

echo
echo "==> Done. Tail logs with: ssh $HOST journalctl -u argonone-rs -f"
