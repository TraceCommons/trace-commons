#!/usr/bin/env bash
#
# Local smoke harness for trace-commons-pilot-bootstrap.
#
# Spins up a stdlib-only mock server on 127.0.0.1:3907 that pretends to be
# both a HuggingFace Hub API (so the binary's `repo.info()` call resolves)
# and a trace-commons-ingest `/v1/traces` endpoint. Pre-populates the hf-hub
# cache with tiny checked-in swival-schema JSONL fixtures so the binary
# does no real HF downloads. Then runs the binary against this loopback
# environment for a small N and asserts:
#
#   1. The sidecar JSONL grew by N lines.
#   2. The mock server saw N distinct submission_ids.
#   3. A second identical run produces the same distinct count
#      (no double-submission) but the sidecar grows.
#
# Exits 0 on success; 1 with a short diagnostic label on failure.
# Stdlib only (python3 http.server, curl). No new Cargo or Python deps;
# safe to run repeatedly. Fully offline — no huggingface.co requests.
#
# Prereqs:
#   - python3 on PATH
#   - curl on PATH
#   - target/release/trace-commons-pilot-bootstrap built (or pass $SMOKE_BINARY)

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"

# Defaults — overridable for CI / debugging.
COUNT="${SMOKE_COUNT:-10}"
PORT="${SMOKE_PORT:-3907}"
HOST="127.0.0.1"
SOURCE="${SMOKE_SOURCE:-jedisct1/agent-traces-swival}"
TENANT_TOKEN="${SMOKE_TENANT_TOKEN:-smoke-test}"
SIDECAR="${SMOKE_SIDECAR:-/tmp/pilot-bootstrap-smoke-sidecar.jsonl}"
BINARY="${SMOKE_BINARY:-$REPO_ROOT/target/release/trace-commons-pilot-bootstrap}"
MOCK_LOG="${SMOKE_MOCK_LOG:-/tmp/pilot-bootstrap-smoke-mock.log}"
BINARY_LOG="${SMOKE_BINARY_LOG:-/tmp/pilot-bootstrap-smoke-binary.log}"
HF_CACHE="${SMOKE_HF_CACHE:-/tmp/pilot-bootstrap-smoke-hf-cache}"
FIXTURE_DIR="$SCRIPT_DIR/fixtures/swival-smoke"
# Fixed pseudo-commit hash so reruns reuse the same snapshot directory.
FIXTURE_COMMIT="smokecommit0000000000000000000000000001"

bail() {
  echo "SmokePilotBootstrapFailure: $1" >&2
  exit 1
}

cleanup() {
  if [ -n "${MOCK_PID:-}" ] && kill -0 "$MOCK_PID" 2>/dev/null; then
    kill -TERM "$MOCK_PID" 2>/dev/null || true
    wait "$MOCK_PID" 2>/dev/null || true
  fi
}
trap cleanup EXIT INT TERM

command -v python3 >/dev/null 2>&1 || bail "python3_not_installed"
command -v curl    >/dev/null 2>&1 || bail "curl_not_installed"
[ -x "$BINARY" ]                   || bail "binary_not_built:$BINARY"
[ -d "$FIXTURE_DIR" ]              || bail "fixture_dir_not_found:$FIXTURE_DIR"

# Make sure the port is free before we bind.
if curl -sS --max-time 1 "http://$HOST:$PORT/__smoke/stats" >/dev/null 2>&1; then
  bail "port_${PORT}_already_in_use"
fi

# Fresh sidecar each smoke run.
rm -f "$SIDECAR"
: > "$MOCK_LOG"
: > "$BINARY_LOG"

# Pre-populate the hf-hub cache layout so repo.get(filename) hits a
# local file instead of touching the network. Layout is:
#   {HF_CACHE}/datasets--{repo}/refs/main           -> commit hash
#   {HF_CACHE}/datasets--{repo}/snapshots/{hash}/{file}
folder_name="datasets--$(printf '%s' "$SOURCE" | tr '/' '-' | tr '/' '-')"
# tr is idempotent on the second pass; we just want '/' -> '--' but tr is
# single-char so we do it manually:
folder_name="datasets--${SOURCE//\//--}"
repo_root="$HF_CACHE/$folder_name"
snapshot_dir="$repo_root/snapshots/$FIXTURE_COMMIT"
refs_dir="$repo_root/refs"

mkdir -p "$snapshot_dir" "$refs_dir"
printf '%s' "$FIXTURE_COMMIT" > "$refs_dir/main"

# Copy every real fixture into the snapshot dir and advertise it. The trailing
# decoy siblings are deliberately advertised but not cached or served: a lazy
# `--count` run stops after the real fixtures, while the old eager loader tries
# to resolve a decoy and fails. This keeps count-bounded fetching in the CI
# smoke contract without contacting huggingface.co.
sibling_args=()
for jsonl in "$FIXTURE_DIR"/*.jsonl; do
  [ -e "$jsonl" ] || bail "fixture_dir_empty:$FIXTURE_DIR"
  name="$(basename "$jsonl")"
  cp -f "$jsonl" "$snapshot_dir/$name"
  sibling_args+=("--hf-sibling" "$name")
done
for suffix in {00..49}; do
  sibling_args+=("--hf-sibling" "zz-decoy-$suffix.jsonl")
done

echo "SmokePilotBootstrap: hf-hub cache primed at $HF_CACHE"
echo "SmokePilotBootstrap: starting mock server on $HOST:$PORT"
python3 "$SCRIPT_DIR/pilot-bootstrap-mock-server.py" \
  --host "$HOST" --port "$PORT" \
  --hf-dataset "$SOURCE" \
  --hf-commit "$FIXTURE_COMMIT" \
  "${sibling_args[@]}" \
  >"$MOCK_LOG" 2>&1 &
MOCK_PID=$!

# Wait up to 5s for the server to accept connections.
for i in 1 2 3 4 5 6 7 8 9 10; do
  if curl -sS --max-time 1 "http://$HOST:$PORT/__smoke/stats" >/dev/null 2>&1; then
    break
  fi
  if ! kill -0 "$MOCK_PID" 2>/dev/null; then
    cat "$MOCK_LOG" >&2
    bail "mock_server_exited_early"
  fi
  sleep 0.5
  if [ "$i" = "10" ]; then
    bail "mock_server_never_ready"
  fi
done

run_binary() {
  local label="$1"
  echo "SmokePilotBootstrap: running binary ($label) count=$COUNT source=$SOURCE"
  TRACE_COMMONS_PILOT_TENANT_TOKEN="$TENANT_TOKEN" \
  HF_ENDPOINT="http://$HOST:$PORT" \
    "$BINARY" \
      --source "$SOURCE" \
      --count "$COUNT" \
      --target "http://$HOST:$PORT" \
      --rate 50 \
      --sidecar "$SIDECAR" \
      --cache-dir "$HF_CACHE" \
      >>"$BINARY_LOG" 2>&1 \
    || { tail -40 "$BINARY_LOG" >&2; bail "binary_run_failed_${label}"; }
}

count_sidecar_lines() {
  if [ -f "$SIDECAR" ]; then
    wc -l < "$SIDECAR" | tr -d ' '
  else
    echo "0"
  fi
}

fetch_distinct() {
  curl -sS --max-time 5 "http://$HOST:$PORT/__smoke/stats" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["distinct"])'
}

fetch_total() {
  curl -sS --max-time 5 "http://$HOST:$PORT/__smoke/stats" \
    | python3 -c 'import json,sys; print(json.load(sys.stdin)["total"])'
}

# --- Run 1: happy path ---
run_binary "run1"

sidecar_lines_1="$(count_sidecar_lines)"
distinct_1="$(fetch_distinct)"
total_1="$(fetch_total)"

echo "SmokePilotBootstrap: run1 sidecar_lines=$sidecar_lines_1 distinct=$distinct_1 total=$total_1"

[ "$sidecar_lines_1" = "$COUNT" ] \
  || bail "run1_sidecar_lines_${sidecar_lines_1}_expected_${COUNT}"
[ "$distinct_1" = "$COUNT" ] \
  || bail "run1_distinct_${distinct_1}_expected_${COUNT}"
[ "$total_1" = "$COUNT" ] \
  || bail "run1_total_${total_1}_expected_${COUNT}"

# --- Run 2: idempotency ---
run_binary "run2"

sidecar_lines_2="$(count_sidecar_lines)"
distinct_2="$(fetch_distinct)"
total_2="$(fetch_total)"

echo "SmokePilotBootstrap: run2 sidecar_lines=$sidecar_lines_2 distinct=$distinct_2 total=$total_2"

expected_sidecar_2=$((COUNT * 2))
expected_total_2=$((COUNT * 2))

[ "$sidecar_lines_2" = "$expected_sidecar_2" ] \
  || bail "run2_sidecar_lines_${sidecar_lines_2}_expected_${expected_sidecar_2}"
[ "$distinct_2" = "$COUNT" ] \
  || bail "run2_distinct_${distinct_2}_expected_${COUNT}_idempotency_broken"
[ "$total_2" = "$expected_total_2" ] \
  || bail "run2_total_${total_2}_expected_${expected_total_2}"

echo "SmokePilotBootstrapOK: $COUNT submissions, idempotency confirmed (distinct stayed at $COUNT across 2 runs)"
