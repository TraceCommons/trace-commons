#!/usr/bin/env bash
#
# Photograph the queue with one-click submit: the row's three controls, a
# project header's `Submit all`, and the submit toast at its longest form.
#
# `fixture.sh` only ever discovers one session per project, so it cannot show
# what a project header even looks like -- `render` in `ui/queue.rs` only
# draws one when a project has more than one session waiting. This fixture
# puts three sessions under one project (frobnicator) and one under another
# (widgetco), so both the header's `Submit all` and a bare row's own `Submit`
# are on screen at once, next to `Look inside` and `Not this one`.
#
# The toast is not reached by clicking `Submit`: getting all four of its
# clauses non-empty in one response means engineering entries the daemon
# actually refuses for different reasons, which a screenshot fixture has no
# business doing. `--show-toast` (see `main.rs`) raises the same toast this
# screen would show, built from `crate::toast::toast`'s own output rather
# than typed out here -- so what is photographed is what that function
# really renders, at the shape most likely to overflow or clip: all four
# clauses, in the 47-session example the design's worked examples use.
set -euo pipefail

SHOT=${SHOT:-.linux-queue-screenshot.png}

TC_DIR=${TC_DIR:-/tmp/tc-queue-state}
CLAUDE_ROOT=${CLAUDE_ROOT:-/tmp/tc-queue-claude}

rm -rf "$TC_DIR" "$CLAUDE_ROOT"
mkdir -p "$TC_DIR"
chmod 700 "$TC_DIR"

# Three sessions under one project, so its header draws with `Submit all (3
# waiting)` next to three ordinary rows each carrying their own `Submit`.
FROB_CWD=/tmp/work/frobnicator
FROB_DIR="$CLAUDE_ROOT/projects/-tmp-work-frobnicator"
mkdir -p "$FROB_DIR"
cat >"$FROB_DIR/11111111-1111-1111-1111-111111111111.jsonl" <<'EOF'
{"type":"user","cwd":"/tmp/work/frobnicator","version":"1.0.0","timestamp":"2026-08-08T09:00:00Z","message":{"role":"user","content":"Fix the retry loop in the frobnicator client so it stops hammering the API."}}
{"type":"assistant","timestamp":"2026-08-08T09:00:05Z","message":{"model":"claude-opus-4","content":[{"type":"text","text":"Looking at the client now."}]}}
{"type":"user","timestamp":"2026-08-08T09:00:20Z","message":{"role":"user","content":"The staging key is AKIAIOSFODNN7EXAMPLE and the on-call is jane.doe@example.com if it breaks."}}
{"type":"assistant","timestamp":"2026-08-08T09:00:30Z","message":{"model":"claude-opus-4","content":[{"type":"text","text":"Added exponential backoff with a cap of 30 seconds."}]}}
EOF
cat >"$FROB_DIR/33333333-3333-3333-3333-333333333333.jsonl" <<'EOF'
{"type":"user","cwd":"/tmp/work/frobnicator","version":"1.0.0","timestamp":"2026-08-08T11:00:00Z","message":{"role":"user","content":"Add a changelog entry for the frobnicator retry fix."}}
{"type":"assistant","timestamp":"2026-08-08T11:00:05Z","message":{"model":"claude-opus-4","content":[{"type":"text","text":"Added the entry under Unreleased."}]}}
EOF
cat >"$FROB_DIR/44444444-4444-4444-4444-444444444444.jsonl" <<'EOF'
{"type":"user","cwd":"/tmp/work/frobnicator","version":"1.0.0","timestamp":"2026-08-08T12:00:00Z","message":{"role":"user","content":"Write a test for the frobnicator backoff cap."}}
{"type":"assistant","timestamp":"2026-08-08T12:00:05Z","message":{"model":"claude-opus-4","content":[{"type":"text","text":"Test added and green."}]}}
EOF

# One session in a second project, so a bare row (no header above it) is on
# screen at the same time as the group.
WIDGET_CWD=/tmp/work/widgetco
WIDGET_DIR="$CLAUDE_ROOT/projects/-tmp-work-widgetco"
mkdir -p "$WIDGET_DIR"
cat >"$WIDGET_DIR/55555555-5555-5555-5555-555555555555.jsonl" <<'EOF'
{"type":"user","cwd":"/tmp/work/widgetco","version":"1.0.0","timestamp":"2026-08-08T13:00:00Z","message":{"role":"user","content":"Rename the widgetco config key from `endpoint` to `api_endpoint`."}}
{"type":"assistant","timestamp":"2026-08-08T13:00:05Z","message":{"model":"claude-opus-4","content":[{"type":"text","text":"Renamed across the config loader and its tests."}]}}
EOF

mkdir -p "$FROB_CWD" "$WIDGET_CWD"

cat >"$TC_DIR/daemon-settings.json" <<EOF
{
  "schema_version": "trace_commons.daemon_settings.v1",
  "poll_interval_secs": 2,
  "quiescence_secs": 1,
  "digest_interval_secs": 14400,
  "queue_ttl_days": 14,
  "growth_factor": 2.0,
  "growth_min_new_bytes": 65536,
  "max_reuploads": 3,
  "max_uploads_per_day": 50,
  "max_bytes_per_day": 209715200,
  "max_queue_entries": 500,
  "history_poll_secs": 1800,
  "canary_interval_secs": 3600,
  "approval_hold_secs": 10,
  "local_notifications": false,
  "near_ai": null,
  "claude_root": "$CLAUDE_ROOT/projects",
  "codex_root": "/tmp/tc-queue-codex-empty"
}
EOF
chmod 600 "$TC_DIR/daemon-settings.json"
mkdir -p /tmp/tc-queue-codex-empty

export TRACE_COMMONS_CONTRIBUTOR_DIR="$TC_DIR"

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
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

echo
echo "=== the queue, before the toast ==="
xvfb-run -a -s "-screen 0 1280x1100x24" bash -c '
  set -e
  dbus-run-session -- bash -c "
    /target/debug/trace-commons-shell --state-dir '"$TC_DIR"' \
      --start-page queue --exit-after-realize --realize-seconds 25 &
    APP_PID=\$!
    sleep 10
    import -window root /work/'"$SHOT"' 2>/dev/null || echo \"(screenshot failed)\"
    wait \$APP_PID
  "
'
ls -l "/work/$SHOT" 2>/dev/null || true

TOAST_SHOT="${SHOT%.png}-toast.png"
echo
echo "=== the submit toast, at its longest form ==="
xvfb-run -a -s "-screen 0 1280x1100x24" bash -c '
  set -e
  dbus-run-session -- bash -c "
    /target/debug/trace-commons-shell --state-dir '"$TC_DIR"' \
      --start-page queue --show-toast --exit-after-realize --realize-seconds 25 &
    APP_PID=\$!
    sleep 12
    import -window root /work/'"$TOAST_SHOT"' 2>/dev/null || echo \"(screenshot failed)\"
    wait \$APP_PID
  "
'
ls -l "/work/$TOAST_SHOT" 2>/dev/null || true
