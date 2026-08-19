#!/usr/bin/env bash
#
# Photograph every onboarding page that styles itself.
#
# The roots screen got a camera before it shipped; onboarding did not, and it
# carried four class names no stylesheet defined (`tc-error`, `tc-muted`,
# `tc-section-header`, `tc-brand-emphasis`). Nothing failed -- `add_css_class`
# takes a string -- so the only way to see the damage is to look.
#
# Onboarding appears when the daemon is running and the device is NOT enrolled,
# which is what fixture.sh plus an un-enrolled state directory gives. `--start-page`
# opens it directly on a named page, so no clicking at root-window coordinates
# is needed here -- unlike roots-answer.sh, which has no such flag to lean on.
#
# Two of the labels under test start hidden (`invite_error` and
# `invite_instance` are `.visible(false)` until a connect attempt fails), so a
# plain connect shot cannot show them. TC_FORCE_CONNECT_NOTICES makes the
# Connect page reveal both with placeholder text, for the camera only.
set -euo pipefail

PAGES=${PAGES:-"welcome connect consent watch"}

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
export TC_FORCE_CONNECT_NOTICES=1

for page in $PAGES; do
  shot=".linux-onboarding-$page.png"
  echo
  echo "=== $page ==="
  xvfb-run -a -s "-screen 0 1280x900x24" bash -c '
    set -e
    dbus-run-session -- bash -c "
      /target/debug/trace-commons-shell --state-dir '"$TC_DIR"' \
        --onboarding-page '"$page"' --exit-after-realize --realize-seconds 18 &
      APP_PID=\$!
      sleep 8
      import -window root /work/'"$shot"' 2>/dev/null \
        || echo \"(screenshot failed)\"
      wait \$APP_PID
    "
  ' || echo "($page run exited non-zero)"
  ls -l "/work/$shot" 2>/dev/null || echo "(no file for $page)"
done
