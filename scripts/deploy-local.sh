#!/usr/bin/env bash
# Builds (or reuses a build of) argonone-rs and installs/(re)starts it as
# a systemd service on *this* machine — run this directly on the Pi (e.g.
# after `git clone`/`git pull`), no SSH involved. This is also what
# scripts/deploy.sh ships over and runs remotely, so there's one source
# of truth for the install steps either way.
#
# Guards the real failure modes hit during actual deployment:
#   - the old Python argononed.service still enabled, fighting for the
#     same I2C bus
#   - a fresh headless boot's plymouth-quit-wait.service stalling the
#     unit's start indefinitely
#   - `enable --now` no-oping on an already-running service, so a
#     redeploy silently keeps running the old binary
#
# This does NOT do the one-time hardware setup (enabling I2C in
# /boot/firmware/config.txt, which needs a reboot) — see README.md's
# Installation section for that, once, before the first deploy.
#
# Usage: scripts/deploy-local.sh [--skip-build] [--binary PATH] [--unit PATH] [--no-restart] [--yes]
#   --skip-build   Don't run `cargo build --release`; use an existing binary
#   --binary PATH  Binary to install (default: target/release/argonone-rs)
#   --unit PATH    systemd unit to install (default: packaging/systemd/argonone-rs.service)
#   --no-restart   Install files but don't enable/restart the systemd service
#   --yes          Don't prompt before disabling a conflicting argononed.service
#
# Requires sudo access (you'll be prompted).

set -euo pipefail

usage() {
  echo "Usage: $0 [--skip-build] [--binary PATH] [--unit PATH] [--no-restart] [--yes]" >&2
  exit 1
}

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SKIP_BUILD=0
NO_RESTART=0
ASSUME_YES=0
BINARY="$REPO_ROOT/target/release/argonone-rs"
UNIT="$REPO_ROOT/packaging/systemd/argonone-rs.service"

while [ $# -gt 0 ]; do
  case "$1" in
    --skip-build) SKIP_BUILD=1; shift ;;
    --no-restart) NO_RESTART=1; shift ;;
    --yes) ASSUME_YES=1; shift ;;
    --binary) BINARY="$2"; shift 2 ;;
    --unit) UNIT="$2"; shift 2 ;;
    *) usage ;;
  esac
done

if [ "$SKIP_BUILD" -eq 0 ]; then
  echo "==> Building (cargo build --release)..."
  cargo build --release --manifest-path "$REPO_ROOT/Cargo.toml"
else
  echo "==> --skip-build: reusing existing binary at $BINARY"
fi

[ -f "$BINARY" ] || { echo "ERROR: binary not found at $BINARY" >&2; exit 1; }
[ -f "$UNIT" ] || { echo "ERROR: systemd unit not found at $UNIT" >&2; exit 1; }

if command -v file >/dev/null 2>&1; then
  echo "==> Binary: $(file -b "$BINARY")"
fi

echo "==> Installing $BINARY -> /usr/local/bin/argonone-rs"
sudo install -m 755 "$BINARY" /usr/local/bin/argonone-rs

echo "==> Installing $UNIT -> /etc/systemd/system/argonone-rs.service"
sudo install -m 644 "$UNIT" /etc/systemd/system/argonone-rs.service
sudo systemctl daemon-reload

# The old Python daemon fights for the same I2C bus if left enabled.
if systemctl list-unit-files 2>/dev/null | grep -q '^argononed\.service' \
   && { systemctl is-enabled --quiet argononed.service 2>/dev/null \
        || systemctl is-active --quiet argononed.service 2>/dev/null; }; then
  echo
  echo "The legacy argononed.service (Python daemon) is still enabled/running."
  echo "It will conflict with argonone-rs over the I2C bus (0x1a) if both run."
  if [ "$ASSUME_YES" -eq 1 ]; then
    REPLY=Y
  else
    read -r -p "Disable it now? [Y/n] " REPLY
  fi
  if [ -z "$REPLY" ] || [ "$REPLY" = "y" ] || [ "$REPLY" = "Y" ]; then
    sudo systemctl disable --now argononed.service
  else
    echo "Leaving argononed.service as-is — expect I2C contention."
  fi
fi

if [ "$NO_RESTART" -eq 1 ]; then
  echo "==> --no-restart: skipping enable/start."
  echo "    Run 'sudo systemctl enable --now argonone-rs' when ready."
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
