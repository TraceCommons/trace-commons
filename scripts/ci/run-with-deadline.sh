#!/usr/bin/env bash
# Runs a test command with a wall-clock deadline, streaming its output to
# the log and to a file, so a hung test fails the step instead of running
# into the job's timeout.
#
#   run-with-deadline.sh <seconds> <out-dir> -- <command> [args...]
#
# Why: a job that hits `timeout-minutes` is cancelled, and GitHub keeps no
# log for a cancelled step, so nothing says which test hung. A step that
# fails keeps its log.
#
# On the deadline it records what is stuck before killing it: a `sample`
# (macOS) of every xctest process, and the last test that started and never
# finished. Everything lands in <out-dir> for the artifact upload.
#
# XCTest writes its progress to stdout, which is block-buffered when stdout
# is a pipe, as it is on CI: the last line in the log can lag the test that
# is running by several kilobytes. NSUnbufferedIO=YES makes Foundation write
# it unbuffered.
set -uo pipefail

if [[ $# -lt 4 || $3 != "--" ]]; then
  echo "usage: $0 <seconds> <out-dir> -- <command> [args...]" >&2
  exit 2
fi
deadline=$1
out=$2
shift 3
mkdir -p "$out"
log="$out/output.log"
export NSUnbufferedIO=YES

# Job control puts the background pipeline in its own process group. swift
# test starts xctest in a group of its own, so the deadline path also walks
# the process tree: only this wrapper's descendants are sampled or killed,
# never another xctest on the same machine.
set -m
( "$@" 2>&1 | tee "$log" ) &
group=$!
set +m

# The last XCTest case or Swift Testing test that started and has not ended.
last_unfinished() {
  awk '
    /^Test Case .* started/ { s = $0 }
    /^Test Case .* (passed|failed|skipped)/ { s = "" }
    /Test .* started\.$/ && !/^Test Case/ { t = $0 }
    /Test .* (passed|failed|skipped)/ && !/^Test Case/ { t = "" }
    END { if (s != "") print s; if (t != "") print t }
  ' "$log"
}

descendants() {
  local child
  for child in $(pgrep -P "$1"); do
    echo "$child"
    descendants "$child"
  done
}

start=$SECONDS
while kill -0 "$group" 2>/dev/null; do
  if (( SECONDS - start >= deadline )); then
    echo
    echo "::error::'$*' passed its ${deadline}s deadline"
    echo "Still running when the deadline passed:"
    last_unfinished | tee "$out/unfinished.txt"
    tree=$(descendants "$group")
    if command -v sample >/dev/null; then
      for pid in $tree; do
        [[ $(ps -o comm= -p "$pid" 2>/dev/null) == *xctest* ]] || continue
        echo "Sampling xctest $pid into $out/xctest-$pid.sample.txt"
        sample "$pid" 5 -file "$out/xctest-$pid.sample.txt" >/dev/null 2>&1 || true
      done
    fi
    kill -TERM -- "-$group" $tree 2>/dev/null
    sleep 10
    kill -KILL -- "-$group" $tree 2>/dev/null
    exit 124
  fi
  sleep 2
done

wait "$group"
status=$?
echo "Finished in $(( SECONDS - start ))s with exit $status"
exit "$status"
