#!/usr/bin/env bash
#
# The other half of the process model: nothing else is running, so the
# application takes the exclusive lock and hosts the watch loop itself.
#
# This is the deployment where the preview sheet can show the redacted
# transcript and search it, because the full body is available in-process
# only. The run opens the sheet with a search term already typed -- a
# planted string that really is in the fixture session -- and photographs
# the result.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=fixture.sh
source "$HERE/fixture.sh"

SHOT=${SHOT:-.shot-hosted.png}
TERM_TO_FIND=${TERM_TO_FIND:-frobnicator}

cd "$HERE/.."
cargo build

echo "=== no daemon is running; the application should take the lock ==="
export GSETTINGS_BACKEND=memory
xvfb-run -a -s "-screen 0 1280x900x24" bash -c '
  set -e
  dbus-run-session -- bash -c "
    /target/debug/trace-commons-shell --state-dir '"$TC_DIR"' \
      --exit-after-realize --realize-seconds 40 \
      --open-preview --search '"$TERM_TO_FIND"' &
    APP_PID=\$!
    sleep 30
    import -window root /work/'"$SHOT"' 2>/dev/null || echo \"(no screenshot)\"
    wait \$APP_PID
  "
'
ls -l "/work/$SHOT" 2>/dev/null || true
