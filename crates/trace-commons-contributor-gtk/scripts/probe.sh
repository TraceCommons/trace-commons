#!/usr/bin/env bash
#
# Milestone check: start a real daemon, then have the shell's client layer
# connect to it over the control socket and print what came back.
#
# This runs the actual `trace-commons-contributor daemon run` binary -- not a
# stub -- so it exercises the socket, the framing, the v1_1 method set, and
# the typed layer over it in one pass.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=fixture.sh
source "$HERE/fixture.sh"

cd "$HERE/.."
cargo build --bin trace-commons-shell-probe
cargo build --manifest-path ../trace-commons-contributor/Cargo.toml \
  --bin trace-commons-contributor

DAEMON=$(ls -d /target/debug/trace-commons-contributor)
"$DAEMON" daemon run &
DAEMON_PID=$!
trap 'kill $DAEMON_PID 2>/dev/null || true' EXIT

# Wait for the socket, then for the watcher to see the session go quiet. The
# eligibility rule needs two polls at a stable size, so this is a couple of
# poll intervals at the primed 2-second rate.
for _ in $(seq 1 40); do
  [ -S "$TC_DIR/daemon.sock" ] && break
  sleep 0.25
done
sleep 8

echo "=== probe, attached to the running daemon ==="
/target/debug/trace-commons-shell-probe "$TC_DIR"

echo
echo "=== the same daemon, over the CLI, for comparison ==="
"$DAEMON" daemon pending
