#!/usr/bin/env bash
#
# Photograph the Settings projects list, with a mode selector open.
#
# The thing worth looking at is that the unresolvable bucket's selector has no
# "Contribute automatically" in it while an ordinary project's does. Collapsed,
# both selectors look identical, so a screenshot of the resting screen proves
# nothing about the defect this guards.
#
# ROW=bucket opens the bucket's selector; ROW=ordinary opens the first
# project's. Both are worth taking: the bucket shot shows the entry is gone,
# and the ordinary shot shows it is gone from that row ONLY, which is the half
# a shorter-list change can silently break.
#
# TC_SUPPRESS_ONBOARDING is set because the fixture is deliberately not
# enrolled, so onboarding correctly opens over the main window and hides the
# screen under test. See the note at `onboarding::present_if_needed`.
#
# Root-window coordinates, for the reason roots-answer.sh states: no window
# manager here, so `xdotool search --name` finds nothing. THEY MOVE WHEN THE
# PROJECTS LIST DOES -- re-derive them from a fresh resting shot rather than
# nudging until a popover appears, because a miss photographs as a screen with
# no popover, which looks like a selector that failed to open.
set -euo pipefail

ROW=${ROW:-bucket}
case "$ROW" in
  bucket) CLICK_X=648; CLICK_Y=600; SHOT=${SHOT:-.linux-settings-bucket.png} ;;
  ordinary) CLICK_X=648; CLICK_Y=542; SHOT=${SHOT:-.linux-settings-ordinary.png} ;;
  *) echo "ROW must be bucket or ordinary" >&2; exit 2 ;;
esac

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
export TC_SUPPRESS_ONBOARDING=1

xvfb-run -a -s "-screen 0 1280x900x24" bash -c '
  set -e
  dbus-run-session -- bash -c '"'"'
    /target/debug/trace-commons-shell --state-dir '"$TC_DIR"' \
      --start-page settings --exit-after-realize --realize-seconds 40 &
    APP_PID=$!
    sleep 10
    xdotool mousemove '"$CLICK_X"' '"$CLICK_Y"' click 1
    sleep 2
    import -window root /work/'"$SHOT"' 2>/dev/null || true
    kill $APP_PID 2>/dev/null || true
    wait $APP_PID 2>/dev/null || true
  '"'"'
'

ls -l "/work/$SHOT"
