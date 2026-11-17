#!/usr/bin/env bash
#
# Decide whether a pull request's changed files can affect any build or
# test, for the `changes` job in .github/workflows/ci.yml.
#
#   scripts/ci/ci-changes.sh decide < changed-paths   # prints build=true|false
#   scripts/ci/ci-changes.sh self-test
#
# Reads one repository path per line (a renamed file's old path too), and
# prints a verdict per path and then `build=true` or `build=false`. Run from
# the repository root of a checkout of the pull request's head.
#
# The rule is an allow-list of what may be skipped, never a list of what
# must run: a path anywhere else -- source, manifests, migrations, scripts,
# workflows, fixtures, deploy/, assets/, community/, design-tokens/, tools/,
# tests/, a crate's own README -- runs everything. A path is skippable only
# when both hold:
#
#   1. it is under docs/, or a markdown file at the repository root; and
#   2. nothing outside docs/ and the root markdown files names it: neither
#      its full path nor its file name appears in any other tracked file.
#
# (2) is what keeps a document a test reads, or a script greps, out of the
# skippable set without a hand-kept list: `include_str!("../../docs/x.md")`
# and a test opening "docs/x.md" both contain the path, and a joined path
# still contains the file name. Matching the bare file name is deliberately
# coarse -- every README.md and status.md is named somewhere, so a change to
# one runs everything -- because a false "run" costs minutes and a false
# "skip" costs a missed failure. An empty list also runs everything.

set -euo pipefail

skippable() {
  local path=$1 base
  case "$path" in
    docs/*) ;;
    */*) return 1 ;;
    *.md) ;;
    *) return 1 ;;
  esac
  base=${path##*/}
  # Every tracked file except docs/ and the root markdown files. `glob`
  # magic makes `*.md` match the root only.
  if git grep -qlF -e "$path" -e "$base" -- . ':(exclude)docs/' ':(exclude,glob)*.md'; then
    return 1
  fi
  return 0
}

decide() {
  local n=0 run=0 path
  while IFS= read -r path; do
    [ -n "$path" ] || continue
    n=$((n + 1))
    if skippable "$path"; then
      echo "  skippable  $path"
    else
      echo "  RUNS       $path"
      run=1
    fi
  done
  if [ "$n" -eq 0 ]; then
    echo "  no changed paths listed; running everything"
    run=1
  fi
  if [ "$run" -eq 1 ]; then echo "build=true"; else echo "build=false"; fi
}

self_test() {
  local fail=0 got
  # Built at run time, so the names appear in no tracked file, this one
  # included.
  local u="ci-changes-self-test-$$-$RANDOM"
  local U
  U=$(printf '%s' "$u" | tr 'a-z-' 'A-Z_')
  expect() {
    local want=$1 name=$2
    shift 2
    got=$(printf '%s\n' "$@" | decide | tail -1)
    if [ "$got" = "build=$want" ]; then
      echo "ok    $name"
    else
      echo "FAIL  $name: got $got, want build=$want"
      fail=1
    fi
  }
  expect true  "a source file"            crates/trace-commons-server/src/lib.rs
  expect true  "a workflow"               .github/workflows/ci.yml
  expect true  "a migration"              migrations/V1__x.sql
  expect true  "a crate README"           crates/trace-commons-protocol/README.md
  expect true  "nothing listed"
  expect false "an unreferenced doc"      "docs/$u.md"
  expect false "an unreferenced root md"  "$U.md"
  expect true  "a doc named in a workflow" docs/operator/login-resolver-role.md
  expect true  "a root md named in code"  README.md
  expect true  "one doc and one source"   "docs/$u.md" Cargo.toml
  return $fail
}

case "${1:-}" in
  decide) decide ;;
  self-test) self_test ;;
  *) echo "usage: $0 decide|self-test" >&2; exit 2 ;;
esac
