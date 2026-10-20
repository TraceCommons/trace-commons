#!/usr/bin/env bash
# Manual acceptance check (docs/PROTOCOL.md §7.5).
#
# The automated suite proves wire fidelity against a mock. This proves the only
# thing a mock cannot: that a real coding agent, talking to real providers
# through IronWire, behaves the same as it does talking to them directly.
#
# Both native lanes are checked, because they fail independently and each has
# already hidden a bug from the other: Claude Code → Anthropic, and Codex →
# ChatGPT. A lane whose CLI is not installed is skipped and said so; a lane
# whose CLI is present and fails is a failure.
#
# It costs real subscription quota. Run it before a release, not in CI.
#
#   scripts/acceptance.sh [task-directory]
#
# Requires: a built `ironwire`, plus `claude` and/or `codex` with a login.

set -euo pipefail

TASK_ROOT="${1:-$(mktemp -d)}"
PORT="${IRONWIRE_PORT:-8464}"
IRONWIRE="${IRONWIRE_BIN:-./target/release/ironwire}"
TASK='Read src/lib.rs, work out why the test fails, fix it, and run `cargo test` to confirm.'

[ -x "$IRONWIRE" ] || { echo "build first: cargo build --release"; exit 1; }
have_claude=0; command -v claude >/dev/null && have_claude=1
have_codex=0;  command -v codex  >/dev/null && have_codex=1
if [ "$have_claude" = 0 ] && [ "$have_codex" = 0 ]; then
  echo "neither claude nor codex is on PATH — nothing to accept"; exit 1
fi

# One fresh copy of the broken crate per lane: a lane must not be handed a
# repository some other agent has already fixed.
scaffold() {
  local dir="$1"
  mkdir -p "$dir/src"
  cat > "$dir/Cargo.toml" <<'TOML'
[package]
name = "acceptance"
version = "0.1.0"
edition = "2021"
TOML
  cat > "$dir/src/lib.rs" <<'RS'
/// Sums 1..=n. Currently wrong: the upper bound is exclusive.
pub fn sum_to(n: u32) -> u32 {
    (1..n).sum()
}

#[cfg(test)]
mod tests {
    #[test]
    fn sums_inclusively() {
        assert_eq!(super::sum_to(5), 15);
    }
}
RS
}

echo "==> Task root: $TASK_ROOT"
echo "==> Starting IronWire on port $PORT"
"$IRONWIRE" serve --port "$PORT" > /tmp/ironwire-acceptance.log 2>&1 &
DAEMON=$!
trap 'kill $DAEMON 2>/dev/null || true' EXIT

# Wait for it, and fail here if it never comes up. Without this the lanes below
# still run, every request fails to connect, and the report blames the agent
# for not completing a task it was never able to start — which is exactly what
# a daemon that refused to share its home looks like.
for _ in $(seq 1 20); do
  "$IRONWIRE" status --port "$PORT" >/dev/null 2>&1 && break
  sleep 0.5
done
if ! "$IRONWIRE" status --port "$PORT" >/dev/null 2>&1; then
  echo "FAIL — IronWire did not come up on port $PORT:"
  sed 's/^/    /' /tmp/ironwire-acceptance.log
  exit 1
fi

"$IRONWIRE" doctor --port "$PORT" || true

FAILED=0

# Run one lane and report it. $1 is the lane name, the rest is the command that
# runs the task in $dir.
run_lane() {
  local lane="$1"; shift
  local dir="$TASK_ROOT/$lane"
  scaffold "$dir"

  echo
  echo "==> [$lane] running the task THROUGH IronWire"
  local before after
  before=$(date +%s)
  ( cd "$dir" && "$@" ) || true
  after=$(date +%s)

  echo
  echo "==> [$lane] verifying the agent actually fixed it"
  if (cd "$dir" && cargo test --quiet); then
    echo "PASS — [$lane] completed through IronWire in $((after - before))s"
  else
    echo "FAIL — [$lane] the test still fails; the agent did not complete the task"
    FAILED=1
  fi
}

if [ "$have_claude" = 1 ]; then
  run_lane claude env ANTHROPIC_BASE_URL="http://127.0.0.1:$PORT/anthropic" \
    claude -p "$TASK"
else
  echo "==> [claude] SKIPPED — claude is not on PATH"
fi

if [ "$have_codex" = 1 ]; then
  # Configured on the command line rather than by editing ~/.codex/config.toml:
  # an acceptance run must not leave the developer's own Codex pointed at a
  # daemon it just killed. `wire_api = "responses"` is the lane Codex actually
  # uses — the one that carries reasoning state.
  run_lane codex codex exec --skip-git-repo-check --sandbox workspace-write \
    -c "model_providers.ironwire_acceptance={name=\"IronWire\",base_url=\"http://127.0.0.1:$PORT/openai/v1\",wire_api=\"responses\"}" \
    -c model_provider=ironwire_acceptance \
    "$TASK"
else
  echo "==> [codex] SKIPPED — codex is not on PATH"
fi

echo
echo "==> What IronWire saw"
"$IRONWIRE" log --port "$PORT" --limit 40
echo
"$IRONWIRE" status --port "$PORT"

cat <<'NOTE'

Compare each lane against a direct run (same task, fresh directory, no
ANTHROPIC_BASE_URL / stock `model_provider`). What matters is not just that
both pass, but that the turn counts are close: a materially longer loop through
IronWire means a field is being mis-mapped in a way the mock-based tests do not
catch.
NOTE

exit "$FAILED"
