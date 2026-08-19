#!/usr/bin/env bash
# Build a throwaway contributor state directory containing two projects -- an
# ordinary one and the unresolvable bucket -- so the Settings screen can be
# photographed with the bucket row present.
#
# Both are made CONFIGURED on purpose. `list_projects` reports configured
# projects from the persisted policy and discovered ones from the in-memory
# queue, and the queue is rebuilt on each poll; a freshly launched app runs its
# one `refreshAll()` before the daemon's first poll, so a discovered-only
# fixture renders as an empty list no matter how long the app then runs.
#
# Takes the state directory as its only argument. Keep it SHORT: the daemon's
# control socket lives inside it and the path cannot exceed 104 bytes.
set -euo pipefail

STATE_DIR="${1:?usage: settings-bucket-fixture.sh <state-dir>}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN="$REPO_ROOT/target/debug/trace-commons-contributor"
CLAUDE_ROOT="$STATE_DIR/claude/projects"
CODEX_ROOT="$STATE_DIR/codex"

rm -rf "$STATE_DIR"
mkdir -p "$CLAUDE_ROOT/ordinary" "$CLAUDE_ROOT/-" "$CODEX_ROOT"
# The ordinary project's working directory has to exist: the daemon refuses a
# project path it cannot resolve on disk. Kept inside the fixture so nothing
# outside the throwaway directory is created.
ORDINARY_CWD="$STATE_DIR/frob"
mkdir -p "$ORDINARY_CWD"

# One session per project. `cwd: "/"` has no usable final path segment, which
# is exactly what the daemon buckets as unresolvable.
write_session() {
  local dir="$1" uuid="$2" cwd="$3" text="$4"
  cat > "$CLAUDE_ROOT/$dir/$uuid.jsonl" <<SESSION
{"type":"user","uuid":"$uuid-1","timestamp":"2026-08-19T10:00:00.000Z","cwd":"$cwd","sessionId":"$uuid","message":{"role":"user","content":"$text"}}
{"type":"assistant","uuid":"$uuid-2","parentUuid":"$uuid-1","timestamp":"2026-08-19T10:00:05.000Z","cwd":"$cwd","sessionId":"$uuid","message":{"role":"assistant","content":[{"type":"text","text":"Renamed the helper and updated its call sites."}]}}
SESSION
  # Backdated so the session counts as quiescent on the first poll.
  touch -t 202608190800 "$CLAUDE_ROOT/$dir/$uuid.jsonl"
}

write_session "ordinary" "11111111-1111-4111-8111-111111111111" "$ORDINARY_CWD" "Rename the helper in parser.rs"
write_session "-" "22222222-2222-4222-8222-222222222222" "/" "What does this stack trace mean?"

"$BIN" --config-dir "$STATE_DIR" daemon settings \
  --set claude_root="$CLAUDE_ROOT" --set codex_root="$CODEX_ROOT" > /dev/null

# NOTHING is ruled on. Both rows arrive as discovered projects once the daemon
# polls, which is the state a contributor is actually in before they have
# decided anything. Ruling on the ordinary project would also duplicate it:
# the CLI canonicalizes /tmp to /private/tmp while the session's own cwd says
# /tmp, so the two spellings become two keys and two rows.
#
# The bucket in particular is deliberately NOT ruled on. Ruling on it would store a policy
# entry keyed by the PATH -- `/` -- and `key == UNKNOWN_PROJECT_KEY` would then
# be false, so the row would not carry `is_unresolved_bucket` even though
# `project_label_for` still renders it as the slug. The real bucket is the
# queue key the daemon mints for a session whose cwd has no usable final
# segment, and it reaches `list_projects` as a DISCOVERED row. Configuring it
# would produce a lookalike that tests nothing.
#
# Discovered rows require a poll, and a freshly launched app runs its one
# refreshAll before the first poll, so shorten the interval: the settings file
# carries it and the CLI has no setter for it.
python3 - "$STATE_DIR/daemon-settings.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["poll_interval_secs"] = 2
json.dump(d, open(p, "w"), indent=2)
PY

"$BIN" --config-dir "$STATE_DIR" daemon projects
