#!/usr/bin/env bash
#
# CI-only. Runs the Linux contributor shell under a real Wayland compositor
# (weston, headless backend) and a real xdg-desktop-portal daemon, to
# upgrade the desktop-integration evidence beyond scripts/headless-run.sh.
#
# scripts/headless-run.sh already proves the process starts under Xvfb and
# that the portal/tray calls fire and fail correctly when nothing is
# listening on the bus at all. That is the easy half. This script targets
# the two things it explicitly does not establish:
#
#   1. Real rendering, not "did not crash". weston's headless backend is an
#      actual Wayland compositor (protocol dispatch, surface commits, output
#      composition) -- not a display-less pixel sink. This captures the
#      composited output with weston-screenshooter and asserts the frame is
#      not blank/uniform (a standard-deviation check on pixel values) and
#      that the header-bar switcher text ("Queue") is present via OCR
#      (tesseract). A test that only checked "the process stayed alive"
#      would be exactly the weak evidence this script exists to replace.
#
#   2. A real portal daemon, not an absent one. This starts the actual
#      xdg-desktop-portal core process (plus xdg-desktop-portal-gtk) under
#      dbus-run-session, so RequestBackground reaches a live D-Bus service
#      rather than getting ServiceUnknown before the call even leaves the
#      process.
#
#      One correction to make explicit: xdg-desktop-portal-gtk does NOT
#      implement org.freedesktop.impl.portal.Background. Checked directly
#      against its shipped manifest
#      (/usr/share/xdg-desktop-portal/portals/gtk.portal): the Interfaces
#      list has FileChooser, AppChooser, Print, Notification, Inhibit,
#      Access, Account, Email, DynamicLauncher, Lockdown, Settings -- no
#      Background. The only backend that claims Background is
#      xdg-desktop-portal-gnome, and its background.c calls
#      org.gnome.Shell.Introspect to do it -- a real GNOME Shell dependency
#      this job does not have and will not fake by installing the package
#      without the shell behind it. So this script does not attempt to get
#      an actual grant/deny answer out of a permission dialog; nothing in
#      CI can pop that dialog. What it asserts instead, and can assert
#      honestly: RequestBackground reaches a genuinely running portal
#      daemon and gets back a real, bounded D-Bus reply -- not the
#      ServiceUnknown/NameHasNoOwner class of error that "nothing is
#      listening" produces. That is a real, if partial, upgrade: the
#      request now fails because no backend implements the interface,
#      which is a different and more honest failure than no portal
#      existing at all.
#
# Neither of these proves GNOME/KDE/Mutter-specific behaviour. See the
# job's own comments in ci.yml and
# docs/superpowers/plans/linux-desktop-verification-report.md for exactly
# what is and is not established.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$HERE/../../.." && pwd)"
CRATE_DIR="$REPO_ROOT/crates/trace-commons-contributor-gtk"

# shellcheck source=fixture.sh
source "$HERE/fixture.sh"

WORKDIR=$(mktemp -d)
trap 'rm -rf "$WORKDIR"' EXIT

SHELL_BIN="$CRATE_DIR/target/debug/trace-commons-shell"
DAEMON_BIN="$REPO_ROOT/target/debug/trace-commons-contributor"

for bin in "$SHELL_BIN" "$DAEMON_BIN"; do
  if [ ! -x "$bin" ]; then
    echo "FAIL: expected binary missing: $bin (build steps must run before this script)" >&2
    exit 1
  fi
done

# --- start the real daemon the shell attaches to --------------------------

"$DAEMON_BIN" daemon run &
DAEMON_PID=$!

cleanup() {
  kill "$DAEMON_PID" 2>/dev/null || true
}
trap 'cleanup; rm -rf "$WORKDIR"' EXIT

for _ in $(seq 1 40); do
  [ -S "$TC_DIR/daemon.sock" ] && break
  sleep 0.25
done
sleep 8

# --- everything below shares one private session bus ----------------------
#
# weston's headless backend needs no X server at all, so unlike
# headless-run.sh this does not nest inside xvfb-run -- dbus-run-session is
# the only wrapper needed.

set +e
dbus-run-session -- bash "$HERE/weston-portal-verify-inner.sh" \
  "$WORKDIR" "$SHELL_BIN" "$TC_DIR"
INNER_RC=$?
set -e

if [ "$INNER_RC" -ne 0 ]; then
  echo "FAIL: weston/portal verification failed (rc=$INNER_RC)" >&2
  exit "$INNER_RC"
fi

echo "OK: real-compositor rendering and real-portal-daemon checks both passed"
