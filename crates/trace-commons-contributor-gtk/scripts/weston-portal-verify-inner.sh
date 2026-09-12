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

# The default destination is Insights. Require its heading and an interactive
# file-selection control, rather than OCR of a neighboring navigation tab.
insights_frame() {
  grep -qiE '^[[:space:]]*Insights[[:space:]]*$' <<< "$1" &&
    grep -qi 'Choose file' <<< "$1"
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

# --- axis 1: real rendering under a real compositor ------------------------

WAYLAND_SOCKET=wayland-ci
# A SOFTWARE renderer, explicitly. Without this weston finds no GL on a CI
# runner (no GPU; MESA reports "ZINK: vkCreateInstance failed" and "failed to
# get driver name for fd -1") and silently falls back to its NO-OP renderer.
# The no-op renderer composites nothing, so there is no framebuffer to capture
# and weston-screenshooter aborts on `assertion 'width > 0' failed` -- which
# reads like a screenshot tool bug and is actually "nothing was ever drawn".
#
# The flag was renamed across weston versions, so probe rather than guess:
# --renderer=pixman on weston 12+, --use-pixman before that.
if weston --help 2>&1 | grep -q -- '--renderer='; then
  RENDERER_FLAG=(--renderer=pixman)
elif weston --help 2>&1 | grep -q -- '--use-pixman'; then
  RENDERER_FLAG=(--use-pixman)
else
  echo "note: this weston exposes neither --renderer= nor --use-pixman;" >&2
  echo "      it will pick its own renderer and may fall back to no-op." >&2
  RENDERER_FLAG=()
fi

# --debug is required, not optional, on this weston: verified against
# weston 13.0.0 (the version apt installs on ubuntu-latest/24.04) that
# weston-screenshooter's capture request is refused with "unauthorized"
# without it. weston's man page states plainly why: the output-capture
# interface weston-screenshooter binds to is gated behind --debug, because
# an unrestricted client could otherwise silently read every output's
# pixels. That is a real production concern and exactly why this stays off
# by default -- but this compositor is a throwaway, single-purpose CI
# instance with nothing sensitive on it, torn down at the end of this
# script, so the tradeoff --debug makes (screenshot access for any client)
# costs nothing here.
weston --backend=headless-backend.so --width=1280 --height=900 \
  "${RENDERER_FLAG[@]}" \
  --socket="$WAYLAND_SOCKET" --idle-time=0 --debug &
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

  # Drive the real roots controls while settings publication is blocked,
  # then refused, then retried. The normal test step has no display.
  GTK_MANIFEST="$(dirname "$SHELL_BIN")/../../Cargo.toml"
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
      ui::roots::submission_tests::pending_submission_is_single_and_failure_allows_retry \
      -- --exact --ignored --test-threads=1; then
    fail "roots submission did not preserve pending, failure and retry behavior"
  fi
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --bin trace-commons-shell \
      startup_tests::quit_during_pending_start_suppresses_completion_and_releases_daemon \
      -- --exact --ignored --test-threads=1; then
    fail "application shutdown did not retire pending startup"
  fi

  # Insights must work before contributor state exists, and its bounded
  # worker must not publish after a window is hidden or closed.
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
      ui::insights::tests::account_free_view_analyzes_saves_explains_deletes_and_ignores_closed_results \
      -- --exact --ignored --test-threads=1; then
    fail "local Insights lifecycle or close cancellation failed"
  fi
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --bin trace-commons-shell \
      insights_startup_tests::first_run_local_window_does_not_create_contributor_state \
      -- --exact --ignored --test-threads=1; then
    fail "first-run Insights created contributor state"
  fi

  # Mission drafts must be reachable without starting Contributions, and
  # inspection/quit failures must preserve the live local view.
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
      ui::insights::tests::local_first_window_exposes_insights_and_mission_drafts_without_a_worker \
      -- --exact --ignored --test-threads=1; then
    fail "local-first mission draft navigation required contributor startup"
  fi
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
      ui::mission_drafts::tests::account_free_view_imports_shows_and_deletes_plain_text_draft \
      -- --exact --ignored --test-threads=1; then
    fail "mission draft lifecycle, failed inspection or declined close failed"
  fi

  # --- axis 2: a real portal daemon ------------------------------------------
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
      ui::credential::provider_tests::wallet_provider_signals_preserve_pending_and_recover \
      -- --exact --ignored --test-threads=1; then
    fail "wallet provider controls did not preserve pending, failure and retry behavior"
  fi
  if ! WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      cargo test --locked --manifest-path "$GTK_MANIFEST" --lib \
      ui::funding::widget_tests::billing_widgets_bind_and_invalidate_browser_handoffs \
      -- --exact --ignored --test-threads=1; then
    fail "billing controls did not preserve organization binding and invalidation"
  fi

  #
  # ORDER MATTERS, and getting it wrong is why the first run of this job proved
  # nothing about the portal. xdg-desktop-portal-gtk is itself a GTK application
  # and needs a display to start. Launched before the compositor exists it dies
  # immediately, and the log fills with
  #   Activated service 'org.freedesktop.impl.portal.desktop.gtk' failed:
  #   Process ... exited with status 1
  # repeated once per portal interface -- while the FRONTEND still claims
  # org.freedesktop.portal.Desktop perfectly happily. So a bus-name check alone
  # reports a healthy portal with no backend behind it at all.
  #
  # Weston is therefore started above, and the backend is given WAYLAND_DISPLAY.

  WAYLAND_DISPLAY="$WAYLAND_SOCKET" "$PORTAL_BIN" &
  PIDS+=("$!")
  WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland "$PORTAL_GTK_BIN" &
  PIDS+=("$!")

  # The frontend owning the bus name is NOT evidence the backend is alive -- see
  # above. Assert the backend separately.
  BACKEND_UP=0
  for _ in $(seq 1 20); do
    if gdbus call --session --dest org.freedesktop.DBus \
        --object-path /org/freedesktop/DBus \
        --method org.freedesktop.DBus.NameHasOwner \
        org.freedesktop.impl.portal.desktop.gtk 2>/dev/null | grep -q 'true'; then
      BACKEND_UP=1
      break
    fi
    sleep 0.5
  done
  if [ "$BACKEND_UP" -ne 1 ]; then
    fail "the gtk portal BACKEND never came up (org.freedesktop.impl.portal.desktop.gtk has no owner) -- a frontend-only portal is not the thing this job claims to verify"
  else
    echo "gtk portal backend is alive and owns org.freedesktop.impl.portal.desktop.gtk"
  fi

  (
    cd "$WORKDIR" || exit 1
    WAYLAND_DISPLAY="$WAYLAND_SOCKET" GDK_BACKEND=wayland GSETTINGS_BACKEND=memory \
      GSK_RENDERER=cairo "$SHELL_BIN" --state-dir "$TC_DIR" --exit-after-realize --realize-seconds 60 \
      >"$WORKDIR/app.log" 2>&1 &
    APP_PID=$!

    # Use GTK's software renderer on a compositor with no GPU, and keep
    # the app alive beyond all ten capture attempts. A ten-second lifetime
    # could end before text was captured and leave only desktop screenshots.
    # Give the window time to realize and composite at least one frame
    # before asking the compositor for a screenshot.
    # Capture until the frame actually contains text, not once after a fixed
    # wait.
    #
    # `sleep 6` then one shot was a race, and it failed in the way races do:
    # the job went red on documentation PRs that never touch this crate, with
    # "OCR did not find the expected header-bar text". Comparing artifacts
    # showed a 76 KB frame on the failures against 158 KB on the pass -- the
    # window had been composited enough to be non-uniform (so the blank check
    # passed) but had not finished painting its text. Under a loaded runner
    # six seconds is simply not a guarantee.
    #
    # This does NOT weaken the assertion: the loop still requires readable
    # text, and still fails if it never appears. It removes a timing
    # dependency, which is the thing that was making a required gate flaky and
    # therefore ignorable.
    CAPTURED=0
    for attempt in $(seq 1 10); do
      sleep 3
      rm -f "$WORKDIR"/*.png 2>/dev/null || true
      WAYLAND_DISPLAY="$WAYLAND_SOCKET" weston-screenshooter || true
      CANDIDATE=$(ls -t "$WORKDIR"/*.png 2>/dev/null | head -1 || true)
      [ -z "$CANDIDATE" ] && continue
      CANDIDATE_TEXT=$(tesseract "$CANDIDATE" - 2>/dev/null || true)
      if insights_frame "$CANDIDATE_TEXT"; then
        echo "frame with readable text captured on attempt $attempt"
        CAPTURED=1
        break
      fi
      echo "attempt $attempt: frame captured but no readable text yet"
    done
    if [ "$CAPTURED" -ne 1 ]; then
      echo "no frame containing readable text after 10 attempts" >&2
    fi

    wait "$APP_PID"
  )

  echo "--- application log ---"
  cat "$WORKDIR/app.log" 2>/dev/null || true
  if grep -q "panicked at" "$WORKDIR/app.log"; then
    fail "application background thread panicked during desktop smoke"
  fi
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
      if insights_frame "$OCR_TEXT"; then
        echo "OCR found the Insights heading and Choose file control"
      else
        fail "OCR did not find the Insights heading and Choose file control in the screenshot"
      fi
    else
      fail "tesseract not available -- OCR text-presence check could not run"
    fi
  fi
fi

exit "$FAIL"

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
