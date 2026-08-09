#!/usr/bin/env bash
#
# Start the real application against a real daemon, headlessly.
#
# What this proves: the process starts, the GTK/libadwaita widget tree is
# realized on a display, the backend attaches to a running daemon over the
# socket, and the queue is populated from it -- with a screenshot of the
# result taken from the X server.
#
# What it does not prove: that the layout looks right. Nobody is looking at
# it, and a screenshot under Xvfb with no theme, no icon theme beyond
# Adwaita's built-in, and a fixed 1280x900 frame is evidence that widgets
# exist and are laid out, not that the design reads well.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=fixture.sh
source "$HERE/fixture.sh"

cd "$HERE/.."
cargo build
cargo build --manifest-path ../trace-commons-contributor/Cargo.toml \
  --bin trace-commons-contributor

/target/debug/trace-commons-contributor daemon run &
DAEMON_PID=$!
trap 'kill $DAEMON_PID 2>/dev/null || true' EXIT

for _ in $(seq 1 40); do
  [ -S "$TC_DIR/daemon.sock" ] && break
  sleep 0.25
done
sleep 8

echo "=== the daemon's own view, before the window opens ==="
/target/debug/trace-commons-contributor daemon pending

echo
echo "=== starting the window under Xvfb ==="
export GSETTINGS_BACKEND=memory
export G_MESSAGES_DEBUG=
# dbus-run-session gives libadwaita and notify-rust a session bus; without
# one the portal and notification lookups log and carry on, which is itself
# the "no desktop services" case worth exercising at least once.
xvfb-run -a -s "-screen 0 1280x900x24" bash -c '
  set -e
  dbus-run-session -- bash -c "
    /target/debug/trace-commons-shell --state-dir '"$TC_DIR"' --exit-after-realize --realize-seconds 14 &
    APP_PID=\$!
    sleep 8
    import -window root /work/.linux-shell-screenshot.png 2>/dev/null \
      || xwd -root -silent > /tmp/shell-window.xwd 2>/dev/null \
      || echo \"(no screenshot tool available)\"
    wait \$APP_PID
  "
'
ls -l /work/.linux-shell-screenshot.png 2>/dev/null || true
