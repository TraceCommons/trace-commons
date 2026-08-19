#!/usr/bin/env bash
#
# Build a throwaway contributor state directory with one real, discoverable
# Claude Code session in it, and prime the daemon settings so the watcher
# picks it up in seconds rather than half an hour.
#
# Sourced by probe.sh and headless-run.sh. Everything it writes lives under
# /tmp inside the container; it never touches a real ~/.claude or a real
# state directory.
set -euo pipefail

TC_DIR=${TC_DIR:-/tmp/tc-state}
CLAUDE_ROOT=${CLAUDE_ROOT:-/tmp/tc-claude}

rm -rf "$TC_DIR" "$CLAUDE_ROOT"
mkdir -p "$TC_DIR"
chmod 700 "$TC_DIR"

# A session under a project directory, in the layout the Claude Code source
# reads: ~/.claude/projects/<encoded-cwd>/<uuid>.jsonl.
PROJECT_CWD=/tmp/work/frobnicator
mkdir -p "$PROJECT_CWD"
SESSION_DIR="$CLAUDE_ROOT/projects/-tmp-work-frobnicator"
mkdir -p "$SESSION_DIR"

# Deliberately contains a secret-shaped string and an email address, so the
# redaction receipt in the queue row has something real to report and the
# search tab has something to find.
cat >"$SESSION_DIR/11111111-1111-1111-1111-111111111111.jsonl" <<'EOF'
{"type":"user","cwd":"/tmp/work/frobnicator","version":"1.0.0","timestamp":"2026-08-08T09:00:00Z","message":{"role":"user","content":"Fix the retry loop in the frobnicator client so it stops hammering the API."}}
{"type":"assistant","timestamp":"2026-08-08T09:00:05Z","message":{"model":"claude-opus-4","content":[{"type":"text","text":"Looking at the client now."},{"type":"tool_use","name":"Read","input":{"path":"src/client.rs"}}]}}
{"type":"user","timestamp":"2026-08-08T09:00:20Z","message":{"role":"user","content":"The staging key is AKIAIOSFODNN7EXAMPLE and the on-call is jane.doe@example.com if it breaks."}}
{"type":"assistant","timestamp":"2026-08-08T09:00:30Z","message":{"model":"claude-opus-4","content":[{"type":"text","text":"Added exponential backoff with a cap of 30 seconds."}]}}
EOF

# A second session whose working directory has no usable final segment, so
# `policy::project_key_for` buckets it into UNKNOWN_PROJECT_KEY rather than
# minting a key that would carry a path. `Path::new("/").file_name()` is
# `None`, which is one of the three real cwds that reach that branch.
#
# Without this the watch screen only ever renders resolvable projects, and the
# permanently-notify-only row -- the one the shared spec requires a note on --
# could not be photographed at all.
UNKNOWN_SESSION_DIR="$CLAUDE_ROOT/projects/-"
mkdir -p "$UNKNOWN_SESSION_DIR"
cat >"$UNKNOWN_SESSION_DIR/22222222-2222-2222-2222-222222222222.jsonl" <<'EOF'
{"type":"user","cwd":"/","version":"1.0.0","timestamp":"2026-08-08T10:00:00Z","message":{"role":"user","content":"Check whether the disk is filling up on this host."}}
{"type":"assistant","timestamp":"2026-08-08T10:00:06Z","message":{"model":"claude-opus-4","content":[{"type":"text","text":"Checking the mounted filesystems."}]}}
EOF

# poll_interval_secs and quiescence_secs are not settable over the socket;
# writing the settings file before the daemon starts is the supported way to
# prime the next start, and is what the CLI itself does when nothing is
# running.
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
  "codex_root": "/tmp/tc-codex-empty"
}
EOF
chmod 600 "$TC_DIR/daemon-settings.json"
mkdir -p /tmp/tc-codex-empty

export TRACE_COMMONS_CONTRIBUTOR_DIR="$TC_DIR"
