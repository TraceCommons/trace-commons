#!/usr/bin/env bash
# Run a lane of PostgreSQL suites in the background of a CI job, then report
# each suite's own log and result. Used by `postgres-suites` in
# .github/workflows/ci.yml; see the comment on its second PostgreSQL service
# for why a lane exists at all.
#
#   . scripts/ci/pg-lane.sh
#   lane() {
#     suite <name> [VAR=value ...] -- <command> [args ...]
#     ...
#   }
#   lane_start <dir> lane     # returns at once; the suites run in order
#   ...                       # later steps
#   lane_wait <dir>           # waits, prints every suite's log, fails if any did
#
# Each suite runs under `env VAR=value ... <command>`, so it sees the job's
# environment plus only the variables named on its own line -- the same
# per-suite separation the job's YAML steps keep with step-level `env:`.
# Every suite runs even when an earlier one fails, and each writes its own
# log, so a failure is attributed to its suite by name.
#
# The lane runs detached, with stdin, stdout and stderr all on files: a
# background process holding the step's output open would keep the step
# from finishing.

suite() {
  local name=$1
  shift
  local vars=()
  while [ "$#" -gt 0 ] && [ "$1" != "--" ]; do
    vars+=("$1")
    shift
  done
  if [ "$#" -eq 0 ]; then
    echo "suite $name: no '--' before its command" >&2
    echo "$name 2 0" >>"$LANE_DIR/status"
    return 0
  fi
  shift
  local start=$SECONDS rc=0
  echo "start $name" >>"$LANE_DIR/progress"
  env ${vars[@]+"${vars[@]}"} "$@" >"$LANE_DIR/$name.log" 2>&1 </dev/null || rc=$?
  echo "$name $rc $((SECONDS - start))" >>"$LANE_DIR/status"
}

lane_start() {
  local dir=$1 fn=$2
  mkdir -p "$dir"
  rm -f "$dir/status" "$dir/progress" "$dir/done"
  (
    export LANE_DIR="$dir"
    "$fn"
    touch "$dir/done"
  ) >"$dir/driver.log" 2>&1 </dev/null &
  echo $! >"$dir/pid"
  echo "started lane $dir (pid $!)"
}

lane_wait() {
  local dir=$1 limit=${2:-1500} waited=0
  local pid
  pid=$(cat "$dir/pid" 2>/dev/null || true)
  if [ -z "$pid" ]; then
    echo "::error::lane $dir was never started"
    return 1
  fi
  while [ ! -f "$dir/done" ]; do
    if ! kill -0 "$pid" 2>/dev/null; then
      break
    fi
    if [ "$waited" -ge "$limit" ]; then
      echo "::error::lane $dir did not finish within ${limit}s; last started: $(tail -1 "$dir/progress" 2>/dev/null)"
      break
    fi
    sleep 2
    waited=$((waited + 2))
  done
  echo "waited ${waited}s for lane $dir"
  local fail=0
  if [ ! -f "$dir/done" ]; then
    echo "::error::lane $dir stopped before its last suite; driver log follows"
    cat "$dir/driver.log" 2>/dev/null || true
    fail=1
  fi
  if [ ! -s "$dir/status" ]; then
    echo "::error::lane $dir ran no suites"
    return 1
  fi
  local name rc secs verdict
  while read -r name rc secs; do
    if [ "$rc" = 0 ]; then verdict=ok; else verdict="FAILED (exit $rc)"; fi
    echo "::group::$name: $verdict in ${secs}s"
    cat "$dir/$name.log" 2>/dev/null || echo "(no log)"
    echo "::endgroup::"
    if [ "$rc" != 0 ]; then
      echo "::error title=$name::$name failed with exit $rc in lane $dir; expand its group above for the log"
      fail=1
    fi
  done <"$dir/status"
  return $fail
}
