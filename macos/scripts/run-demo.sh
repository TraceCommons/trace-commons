#!/usr/bin/env bash
# Launch the shell against a throwaway state directory holding FIXTURE
# sessions.
#
# It never points at the developer's real ~/.claude or ~/.codex: it writes
# daemon-settings.json with both roots inside the temp directory before the
# daemon starts, which is the same pre-seed the FFI demo uses (the C ABI has
# no call to set the roots after start). The app itself refuses to run
# without those roots declared, so this is the only way in.
set -euo pipefail

cd "$(dirname "$0")/.."
PACKAGE_DIR="$PWD"
APP="$PACKAGE_DIR/.build/TraceCommons.app"

# Short path on purpose: the control socket is <dir>/daemon.sock and the
# daemon refuses a socket path over 104 bytes.
STATE_DIR="$(mktemp -d /tmp/tcapp-XXXXXX)"
CLAUDE_ROOT="$STATE_DIR/claude-root"
CODEX_ROOT="$STATE_DIR/codex-root"
mkdir -p "$CLAUDE_ROOT" "$CODEX_ROOT"

# Two fixture projects. Claude Code encodes a session's cwd as a directory
# name with every '/' replaced by '-'.
mkfixture() {
  local project_dir="$CLAUDE_ROOT/$1"
  local session="$project_dir/$2.jsonl"
  mkdir -p "$project_dir"
  cat > "$session"
}

mkfixture "-tmp-tcdemo-payments-api" "11111111-1111-4111-8111-111111111111" <<'JSONL'
{"type":"user","cwd":"/tmp/tcdemo/payments-api","message":{"role":"user","content":"The Northwind settlement job is failing on retry. Can you look at the backoff logic in worker.rs?"}}
{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4","content":"Reading worker.rs. The retry loop multiplies the delay before checking the attempt cap, so the fourth attempt sleeps past the lease timeout."}}
{"type":"user","message":{"role":"user","content":"I exported AWS_ACCESS_KEY_ID=AKIAIOSFODNN7EXAMPLE and GITHUB_TOKEN=ghp_abcdefghijklmnop0123456789 into the shell, does the job read those?"}}
{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4","content":"It reads them at startup only. Move the cap check above the multiply and the retry converges in three attempts."}}
{"type":"user","message":{"role":"user","content":"That fixed it. Northwind settlements are draining again."}}
JSONL

mkfixture "-tmp-tcdemo-dotfiles" "22222222-2222-4222-8222-222222222222" <<'JSONL'
{"type":"user","cwd":"/tmp/tcdemo/dotfiles","message":{"role":"user","content":"Make my zsh prompt show the current git branch without slowing down every cd."}}
{"type":"assistant","message":{"role":"assistant","model":"claude-opus-4","content":"Cache the branch in a precmd hook keyed on $PWD rather than shelling out to git in PROMPT."}}
{"type":"user","message":{"role":"user","content":"Works. Prompt is instant again."}}
JSONL

# Mirrors DaemonSettings::default() with the session roots overridden and the
# timings shortened so a demo does not wait half an hour for quiescence.
# local_notifications is false because the app renders its own notifications,
# which is what keeps the action list to Review / Not now.
cat > "$STATE_DIR/daemon-settings.json" <<JSON
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
  "local_notifications": false,
  "near_ai": null,
  "claude_root": "$CLAUDE_ROOT",
  "codex_root": "$CODEX_ROOT"
}
JSON

# A local, offline contributor config. `preview` needs one to know which
# consent scopes an envelope would carry; it performs no network I/O. There
# is no device key here, so the daemon reports not-logged-in and nothing can
# be uploaded -- which is exactly the state a demo should be in.
cat > "$STATE_DIR/contributor.json" <<'JSON'
{
  "schema_version": "trace_commons.contributor.v1",
  "issuer_url": "https://issuer.invalid",
  "ingest_url": "https://ingest.invalid",
  "audience": "trace-commons-ingest",
  "tenant_id": "00000000-0000-4000-8000-000000000000",
  "instance_id": "demo-instance",
  "user_subject": "demo-user",
  "device_key_id": "demo-device",
  "consent_scopes": ["debugging_evaluation"],
  "pii_filter": null,
  "allowed_hosts": null
}
JSON

chmod 600 "$STATE_DIR/daemon-settings.json" "$STATE_DIR/contributor.json"
chmod 700 "$STATE_DIR"

echo "state dir: $STATE_DIR"
# ALWAYS rebuild, never "only if missing".
#
# This used to be `if [ ! -d "$APP" ]`, which meant the second and every
# subsequent run launched whatever binary happened to be sitting in .build --
# so any source change after the first run was silently ignored and the demo
# reported on stale code. Anything "verified" that way was verified against a
# build that no longer existed in the tree, which is worse than not checking:
# it produces confident output about code you are not running.
#
# make-app-bundle.sh runs `swift build`, which is incremental, so an unchanged
# tree costs almost nothing here.
"$PACKAGE_DIR/scripts/make-app-bundle.sh"

TRACE_COMMONS_CONTRIBUTOR_DIR="$STATE_DIR" \
TRACE_COMMONS_SHOW_WINDOW="${TRACE_COMMONS_SHOW_WINDOW:-1}" \
  open -n "$APP" \
  --env TRACE_COMMONS_CONTRIBUTOR_DIR="$STATE_DIR" \
  --env TRACE_COMMONS_SHOW_WINDOW="${TRACE_COMMONS_SHOW_WINDOW:-1}" \
  --env TRACE_COMMONS_SCREENSHOT_DIR="${TRACE_COMMONS_SCREENSHOT_DIR:-}" \
  --env TRACE_COMMONS_DEMO_PREVIEW="${TRACE_COMMONS_DEMO_PREVIEW:-}" \
  --env TRACE_COMMONS_QUIT_AFTER_SHOT="${TRACE_COMMONS_QUIT_AFTER_SHOT:-}" \
  --env TRACE_COMMONS_SELFTEST_OUT="${TRACE_COMMONS_SELFTEST_OUT:-}" \
  --env TRACE_COMMONS_APPEARANCE="${TRACE_COMMONS_APPEARANCE:-}"

echo "launched. quit from the menu bar when done; then: rm -rf $STATE_DIR"
