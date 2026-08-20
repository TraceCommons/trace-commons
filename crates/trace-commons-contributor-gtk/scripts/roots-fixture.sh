#!/usr/bin/env bash
#
# Build a throwaway state directory that has NOT declared its session roots,
# plus conventional session stores for discovery to find.
#
# This is the counterpart to fixture.sh. That one primes daemon-settings.json
# so the daemon starts and the queue fills; this one deliberately writes no
# settings file at all, so the shell takes the roots-declaration path -- the
# path that used to print a label and call exit(1).
#
# The stores go under the container's HOME at the conventional locations, so
# what gets exercised is discovery's default resolution rather than the
# CLAUDE_CONFIG_DIR / CODEX_HOME override. Everything lives inside the
# container; nothing here reads or mounts a real home directory.
set -euo pipefail

TC_DIR=${TC_DIR:-/tmp/tc-roots-state}

# Deliberately different, deliberately not round: a screenshot showing "7"
# and "23" cannot be a placeholder, and cannot be one count rendered twice.
CLAUDE_SESSIONS=${CLAUDE_SESSIONS:-7}
CODEX_ROLLOUTS=${CODEX_ROLLOUTS:-23}

rm -rf "$TC_DIR" "$HOME/.claude" "$HOME/.codex"
mkdir -p "$TC_DIR"
chmod 700 "$TC_DIR"

# No daemon-settings.json is written. That absence IS the fixture.

# ~/.claude/projects/<encoded-cwd>/<uuid>.jsonl -- the layout the Claude Code
# source reads. Two project directories so the count is not trivially the
# number of directories.
claude_session() {
  local dir="$1" uuid="$2" stamp="$3"
  mkdir -p "$dir"
  cat >"$dir/$uuid.jsonl" <<EOF
{"type":"user","cwd":"/tmp/work/frobnicator","version":"1.0.0","timestamp":"$stamp","message":{"role":"user","content":"Fix the retry loop so it stops hammering the API."}}
{"type":"assistant","timestamp":"$stamp","message":{"model":"claude-opus-4","content":[{"type":"text","text":"Added exponential backoff capped at 30 seconds."}]}}
EOF
}

for i in $(seq 1 "$CLAUDE_SESSIONS"); do
  if [ "$i" -le 4 ]; then
    project="$HOME/.claude/projects/-tmp-work-frobnicator"
  else
    project="$HOME/.claude/projects/-tmp-work-widgetry"
  fi
  claude_session "$project" "$(printf '%08d-1111-1111-1111-111111111111' "$i")" \
    "2026-08-19T09:0$((i % 10)):00Z"
done

# ~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl -- date-nested, which is why
# the Codex source recurses rather than reading one directory.
for i in $(seq 1 "$CODEX_ROLLOUTS"); do
  day=$(printf '%02d' $(((i % 27) + 1)))
  dir="$HOME/.codex/sessions/2026/08/$day"
  mkdir -p "$dir"
  cat >"$dir/rollout-2026-08-${day}T10-00-0$((i % 10))-000000.jsonl" <<'EOF'
{"type":"message","role":"user","content":"Rename the widget factory."}
{"type":"message","role":"assistant","content":"Renamed and updated call sites."}
EOF
done

# Most-recent times are what the evidence line reports, so make them
# distinguishable rather than all "just now": Claude touched minutes ago,
# Codex hours ago.
find "$HOME/.claude/projects" -name '*.jsonl' -exec touch -d '5 minutes ago' {} +
find "$HOME/.codex/sessions" -name '*.jsonl' -exec touch -d '6 hours ago' {} +

echo "roots fixture:"
echo "  state dir      $TC_DIR (no daemon-settings.json)"
echo "  claude store   $HOME/.claude/projects ($(find "$HOME/.claude/projects" -name '*.jsonl' | wc -l | tr -d ' ') sessions)"
echo "  codex store    $HOME/.codex/sessions ($(find "$HOME/.codex/sessions" -name '*.jsonl' | wc -l | tr -d ' ') rollouts)"

export TRACE_COMMONS_CONTRIBUTOR_DIR="$TC_DIR"
