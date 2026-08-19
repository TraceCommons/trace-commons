#!/usr/bin/env bash
# Fail if the committed mark SVGs are not what the generator produces.
#
# Three solid blue squares shipped as the Windows tiles for as long as they
# did because nothing could regenerate them and nothing compared them to
# anything. The committed files under assets/mark exist so a checkout is
# usable without running Rust first; this check is what stops them being
# authoritative.
#
# `git diff --exit-code` alone is not enough: it says nothing about a file the
# generator writes that was never committed, which is exactly what a newly
# added packaging surface looks like. The untracked check below covers that.
set -euo pipefail

cd "$(dirname "$0")/../.."
ASSETS="assets/mark"

cargo run --quiet -p trace-commons-mark --bin mark-export -- "$ASSETS"

if ! git diff --exit-code -- "$ASSETS"; then
  echo "FATAL: $ASSETS does not match the generator." >&2
  echo "The SVGs are generated from crates/trace-commons-mark, not edited." >&2
  echo "Run scripts/mark/check-drift.sh locally and commit the result." >&2
  exit 1
fi

UNTRACKED="$(git ls-files --others --exclude-standard -- "$ASSETS")"
if [ -n "$UNTRACKED" ]; then
  echo "FATAL: the generator wrote files that are not committed:" >&2
  echo "$UNTRACKED" >&2
  echo "A packaging surface was added to all_exports() without being" >&2
  echo "committed, so every consumer of it would build against nothing." >&2
  exit 1
fi

MISSING=0
while IFS= read -r tracked; do
  if [ ! -f "$tracked" ]; then
    echo "FATAL: $tracked is committed but the generator did not write it." >&2
    MISSING=1
  fi
done < <(git ls-files -- "$ASSETS")
if [ "$MISSING" != 0 ]; then
  echo "A committed asset no longer corresponds to any packaging surface." >&2
  exit 1
fi

echo "assets/mark matches the generator"
