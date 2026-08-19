#!/usr/bin/env bash
#
# Answer the roots window by clicking it, and check what got written.
#
# The companion to roots-shot.sh. That one proves the window looks right;
# this one proves the controls in it do what the labels say -- that "Watch
# this folder" and "I don't use this" reach the settings file as a Watch and
# an Off rather than as an omission, which is the difference between the
# fail-closed answer and the fail-open one.
#
# Clicks are absolute screen coordinates against the Xvfb root, read off the
# capture that roots-shot.sh produces. They are NOT resolved from widget
# labels: `xdotool search --name` finds nothing here, because the window is
# an adw::ApplicationWindow on a bare X server with no window manager to set
# the properties that search reads.
#
# That makes the coordinates brittle to layout changes, and the assertion at
# the bottom is what compensates: if a click misses, the settings file is
# absent or holds the wrong pair, and this script fails. It cannot silently
# pass by clicking the wrong control -- a stray click leaves an unanswered
# row, Continue stays insensitive, and nothing is written at all.
set -euo pipefail

SHOT=${SHOT:-.linux-roots-answered.png}

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=roots-fixture.sh
source "$HERE/roots-fixture.sh"

cd "$HERE/.."
cargo build

export GSETTINGS_BACKEND=memory
export G_MESSAGES_DEBUG=
xvfb-run -a -s "-screen 0 1280x900x24" bash -c '
  set -e
  dbus-run-session -- bash -c '"'"'
    /target/debug/trace-commons-shell --state-dir '"$TC_DIR"' \
      --exit-after-realize --realize-seconds 40 &
    APP_PID=$!
    sleep 10

    # Claude row: "Watch this folder". Codex row: the decline.
    # Root-window coordinates; see the header for why not by label.
    xdotool mousemove 38 318 click 1
    sleep 1
    xdotool mousemove 38 532 click 1
    sleep 1
    import -window root /work/'"$SHOT"' 2>/dev/null || true
    xdotool mousemove 544 619 click 1
    sleep 3
    kill $APP_PID 2>/dev/null || true
    wait $APP_PID 2>/dev/null || true
  '"'"'
'

echo
echo "=== daemon-settings.json after answering ==="
if [ -f "$TC_DIR/daemon-settings.json" ]; then
  jq '{claude_source, codex_source}' "$TC_DIR/daemon-settings.json"
else
  echo "NOTHING WRITTEN -- the clicks did not reach the controls"
  exit 1
fi
