#!/usr/bin/env bash
#
# Photograph the "What gets removed?" dialog.
#
# The list inside it is generated from the protocol's detector table, so the
# thing worth looking at is whether eight generated rows plus the concession
# fit and read, which no test can answer. onboarding-shots.sh cannot reach it:
# the dialog needs a click, and that script only realizes a page and shoots.
#
# The click is at root-window coordinates for the same reason roots-answer.sh
# uses them -- there is no window manager here, so `xdotool search --name`
# finds nothing. THEY MOVE WHEN THE WELCOME LAYOUT DOES. Re-derive them from a
# fresh --onboarding-shots welcome rather than nudging until something opens:
# a miss looks identical to a dialog that failed to build, which is what the
# size assertion at the bottom is guarding against.
set -euo pipefail

SHOT=${SHOT:-.linux-removed-dialog.png}

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
sleep 5

export GSETTINGS_BACKEND=memory
export G_MESSAGES_DEBUG=

xvfb-run -a -s "-screen 0 1280x900x24" bash -c '
  set -e
  dbus-run-session -- bash -c '"'"'
    /target/debug/trace-commons-shell --state-dir '"$TC_DIR"' \
      --onboarding-page welcome --exit-after-realize --realize-seconds 40 &
    APP_PID=$!
    sleep 10

    # The "What gets removed?" link, under the scrubbing paragraph.
    xdotool mousemove 90 384 click 1
    sleep 2
    import -window root /work/'"$SHOT"' 2>/dev/null || true
    kill $APP_PID 2>/dev/null || true
    wait $APP_PID 2>/dev/null || true
  '"'"'
'

ls -l "/work/$SHOT"
