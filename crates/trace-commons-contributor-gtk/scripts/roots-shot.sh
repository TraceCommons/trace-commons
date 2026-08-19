#!/usr/bin/env bash
#
# Photograph the roots-declaration window.
#
# What this proves that a unit test cannot: what the window LOOKS like. The
# roots screen was written, tested to 126 passing assertions, and merged
# without anyone ever seeing it -- GTK's accessibility bridge on macOS
# reports the window but none of its labels, so on the development host there
# is no way to read the layout, the evidence lines, or which controls start
# selected.
#
# Unlike headless-run.sh, no daemon is started. The state directory has no
# daemon-settings.json, so the shell takes the roots path and the window under
# the camera is the one a contributor sees on first launch.
set -euo pipefail

SHOT=${SHOT:-.linux-roots-screenshot.png}

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=roots-fixture.sh
source "$HERE/roots-fixture.sh"

cd "$HERE/.."
cargo build

echo
echo "=== what discovery sees, before the window opens ==="
# The same probe the window runs, so a wrong screenshot can be told from a
# wrong discovery result.
cargo run --quiet --bin trace-commons-shell -- --print-discovery 2>/dev/null \
  || echo "(no --print-discovery flag; relying on the window itself)"

echo
echo "=== starting the roots window under Xvfb ==="
export GSETTINGS_BACKEND=memory
export G_MESSAGES_DEBUG=
xvfb-run -a -s "-screen 0 1280x900x24" bash -c '
  set -e
  dbus-run-session -- bash -c "
    /target/debug/trace-commons-shell --state-dir '"$TC_DIR"' \
      --exit-after-realize --realize-seconds 25 &
    APP_PID=\$!
    sleep 10
    import -window root /work/'"$SHOT"' 2>/dev/null \
      || echo \"(screenshot failed)\"
    wait \$APP_PID
  "
'

echo
echo "=== state directory after the window closed unanswered ==="
ls -la "$TC_DIR"
if [ -f "$TC_DIR/daemon-settings.json" ]; then
  echo "UNEXPECTED: settings were written despite nothing being answered"
  cat "$TC_DIR/daemon-settings.json"
else
  echo "no daemon-settings.json -- correct, nothing was answered"
fi

ls -l "/work/$SHOT" 2>/dev/null || true
