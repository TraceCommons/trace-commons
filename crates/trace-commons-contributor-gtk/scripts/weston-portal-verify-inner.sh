#!/usr/bin/env bash
#
# Runs inside `dbus-run-session` (see weston-portal-verify.sh, which is the
# only caller). Not meant to be run standalone: it assumes
# $DBUS_SESSION_BUS_ADDRESS is already a private session bus.
#
# Args: <scratch-dir> <shell-binary> <state-dir>
set -uo pipefail

WORKDIR="$1"
SHELL_BIN="$2"
TC_DIR="$3"

PORTAL_BIN=/usr/libexec/xdg-desktop-portal
PORTAL_GTK_BIN=/usr/libexec/xdg-desktop-portal-gtk

FAIL=0
fail() {
  echo "FAIL: $1" >&2
  FAIL=1
}

# --- XDG_RUNTIME_DIR: weston's socket and the portal both want one --------

if [ -z "${XDG_RUNTIME_DIR:-}" ] || [ ! -d "$XDG_RUNTIME_DIR" ]; then
  export XDG_RUNTIME_DIR="$WORKDIR/xdg-runtime"
  mkdir -p "$XDG_RUNTIME_DIR"
  chmod 700 "$XDG_RUNTIME_DIR"
fi

PIDS=()
cleanup() {
  for pid in "${PIDS[@]:-}"; do
    kill "$pid" 2>/dev/null || true
  done
}
trap cleanup EXIT

# --- axis 2: a real portal daemon ------------------------------------------

"$PORTAL_BIN" &
PIDS+=("$!")
"$PORTAL_GTK_BIN" &
PIDS+=("$!")

PORTAL_UP=0
for _ in $(seq 1 20); do
  if gdbus call --session \
      --dest org.freedesktop.DBus \
      --object-path /org/freedesktop/DBus \
      --method org.freedesktop.DBus.NameHasOwner \
      org.freedesktop.portal.Desktop 2>/dev/null | grep -q 'true'; then
    PORTAL_UP=1
    break
  fi
  sleep 0.5
done

if [ "$PORTAL_UP" -ne 1 ]; then
  fail "xdg-desktop-portal never claimed org.freedesktop.portal.Desktop on the session bus"
else
  echo "portal daemon is alive and owns org.freedesktop.portal.Desktop"

  # Independent of the app: call RequestBackground directly and check the
  # failure mode. A live portal with no Background backend must answer
  # (with an error) inside a bounded time, and that error must not be the
  # ServiceUnknown/NameHasNoOwner class -- that is exactly what "nothing is
  # listening at all" looks like, and getting it here would mean this
  # setup is not actually testing anything different from headless-run.sh.
  PROBE_OUT=$(timeout 10 gdbus call --session \
    --dest org.freedesktop.portal.Desktop \
    --object-path /org/freedesktop/portal/desktop \
    --method org.freedesktop.portal.Background.RequestBackground "" "{}" 2>&1)
  PROBE_RC=$?
  echo "RequestBackground probe (rc=$PROBE_RC): $PROBE_OUT"

  if [ "$PROBE_RC" -eq 124 ]; then
    fail "RequestBackground did not return within 10s against a live portal -- should fail fast with no Background backend registered, not hang"
  elif echo "$PROBE_OUT" | grep -Eqi 'ServiceUnknown|NameHasNoOwner|was not provided by any \.service files'; then
    fail "RequestBackground got the same 'nothing is listening' error as no-portal-at-all -- the live daemon above did not actually field the call"
  else
    echo "RequestBackground reached the live portal daemon and got a real (non-absence) reply"
  fi
fi

# --- axis 1: real rendering under a real compositor ------------------------

WAYLAND_SOCKET=wayland-ci
weston --backend=headless-backend.so --width=1280 --height=900 \
  --socket="$WAYLAND_SOCKET" --idle-time=0 &
WESTON_PID=$!
PIDS+=("$WESTON_PID")

WESTON_UP=0
for _ in $(seq 1 40); do
  [ -S "$XDG_RUNTIME_DIR/$WAYLAND_SOCKET" ] && { WESTON_UP=1; break; }
  sleep 0.25
done

if [ "$WESTON_UP" -ne 1 ]; then
  fail "weston headless backend never created its Wayland socket"
else
  echo "weston headless compositor is up on $WAYLAND_SOCKET"

  (
    cd "$WORKDIR" || exit 1
    WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      "$SHELL_BIN" --state-dir "$TC_DIR" --exit-after-realize --realize-seconds 10 \
      >"$WORKDIR/app.log" 2>&1 &
    APP_PID=$!

    # Give the window time to realize and composite at least one frame
    # before asking the compositor for a screenshot.
    sleep 6
    WAYLAND_DISPLAY="$WAYLAND_SOCKET" weston-screenshooter || true

    wait "$APP_PID"
  )

  echo "--- application log ---"
  cat "$WORKDIR/app.log" 2>/dev/null || true
  echo "-----------------------"

  SHOT=$(ls -t "$WORKDIR"/*.png 2>/dev/null | head -1 || true)
  if [ -z "$SHOT" ]; then
    fail "weston-screenshooter produced no PNG -- cannot check rendering"
  else
    echo "screenshot: $SHOT ($(stat -c%s "$SHOT" 2>/dev/null || echo '?') bytes)"

    # Not blank/uniform: a flat-colour frame (nothing composited, or a
    # black/white rectangle) has a standard deviation at or near zero. This
    # is deliberately a low bar -- it is a floor under "the compositor
    # actually received pixels from the app", not a claim about layout
    # quality.
    STDDEV=$(convert "$SHOT" -colorspace Gray -format "%[fx:standard_deviation]" info: 2>/dev/null || echo "0")
    echo "grayscale standard deviation: $STDDEV"
    PASSES_STDDEV=$(awk -v v="$STDDEV" 'BEGIN { print (v+0 > 0.01) ? "1" : "0" }')
    if [ "$PASSES_STDDEV" != "1" ]; then
      fail "screenshot is blank/uniform (stddev=$STDDEV) -- nothing was actually composited"
    else
      echo "screenshot is not blank/uniform"
    fi

    if command -v tesseract >/dev/null 2>&1; then
      OCR_TEXT=$(tesseract "$SHOT" stdout 2>/dev/null || echo "")
      echo "--- OCR text ---"
      echo "$OCR_TEXT"
      echo "----------------"
      if echo "$OCR_TEXT" | grep -qi 'Queue'; then
        echo "OCR found the expected header-bar text (Queue)"
      else
        fail "OCR did not find the expected header-bar text (Queue) in the screenshot"
      fi
    else
      fail "tesseract not available -- OCR text-presence check could not run"
    fi
  fi
fi

exit "$FAIL"
