#!/usr/bin/env bash
# Cross-compiles argonone-rs on this (dev) machine, ships the binary +
# systemd unit + scripts/deploy-local.sh to a Pi over SSH, and runs
# deploy-local.sh there to install/(re)start it — see deploy-local.sh
# for the actual install steps and the failure modes it guards (I2C
# conflict with the old Python daemon, the Plymouth boot-stall, etc.).
# Run scripts/deploy-local.sh directly instead if you're already on the
# Pi (e.g. after `git clone`/`git pull`) — no SSH needed for that case.
#
# This does NOT do the one-time hardware setup (enabling I2C in
# /boot/firmware/config.txt, which needs a reboot) — see README.md's
# Installation section for that, once, before the first deploy.
#
# Usage: scripts/deploy.sh <ssh-host> [--skip-build] [--no-restart] [--yes]
#   <ssh-host>     SSH destination (see ~/.ssh/config), e.g. euclides or pi@192.168.1.50
#   --skip-build   Reuse the existing cross-compiled binary instead of rebuilding
#   --no-restart   Copy files but don't enable/restart the systemd service
#   --yes          Don't prompt before disabling a conflicting argononed.service
#
# Requires locally: `rustup target add aarch64-unknown-linux-gnu` and a
# cross linker (see README.md's "Cross-compile for Raspberry Pi" section).
# Requires on the remote host: sudo access (you'll be prompted).

set -euo pipefail

usage() {
  echo "Usage: $0 <ssh-host> [--skip-build] [--no-restart] [--yes]" >&2
  exit 1
}

[ $# -ge 1 ] || usage
HOST="$1"
shift

SKIP_BUILD=0
NO_RESTART=0
ASSUME_YES=0
for arg in "$@"; do
  case "$arg" in
    --skip-build) SKIP_BUILD=1 ;;
    --no-restart) NO_RESTART=1 ;;
    --yes) ASSUME_YES=1 ;;
    *) usage ;;
  esac
done

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="aarch64-unknown-linux-gnu"
BINARY="$REPO_ROOT/target/$TARGET/release/argonone-rs"
UNIT="$REPO_ROOT/packaging/systemd/argonone-rs.service"
DEPLOY_LOCAL="$REPO_ROOT/scripts/deploy-local.sh"

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

echo "==> Copying binary, systemd unit, and deploy-local.sh to $HOST..."
scp -q "$BINARY" "$HOST:/tmp/argonone-rs"
scp -q "$UNIT" "$HOST:/tmp/argonone-rs.service"
scp -q "$DEPLOY_LOCAL" "$HOST:/tmp/deploy-local.sh"

REMOTE_ARGS=(--skip-build --binary /tmp/argonone-rs --unit /tmp/argonone-rs.service)
[ "$NO_RESTART" -eq 1 ] && REMOTE_ARGS+=(--no-restart)
[ "$ASSUME_YES" -eq 1 ] && REMOTE_ARGS+=(--yes)

echo "==> Running deploy-local.sh on $HOST (sudo required)..."
ssh -t "$HOST" "chmod +x /tmp/deploy-local.sh && /tmp/deploy-local.sh ${REMOTE_ARGS[*]}; rc=\$?; rm -f /tmp/argonone-rs /tmp/argonone-rs.service /tmp/deploy-local.sh; exit \$rc"

echo
echo "==> Done. Tail logs with: ssh $HOST journalctl -u argonone-rs -f"
