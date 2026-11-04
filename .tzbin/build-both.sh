#!/bin/bash
set -e
cd /Users/zakimanian/code/tc-wt-claude/native-rotation-clock
B="cargo test -p trace-commons-ingest --lib --no-run"
export RUSTFLAGS="-D warnings" CARGO_INCREMENTAL=0
bin() { $B --message-format=json 2>/dev/null | python3 -c 'import sys,json
for l in sys.stdin:
  try: m=json.loads(l)
  except: continue
  if m.get("reason")=="compiler-artifact" and m.get("executable") and m["target"]["name"]=="trace_commons_ingest" and m["profile"]["test"]: print(m["executable"])'; }
$B 2>&1 | tail -1
cp "$(bin)" .tzbin/after
git show HEAD~1:crates/trace-commons-server/src/db/postgres.rs > crates/trace-commons-server/src/db/postgres.rs
$B 2>&1 | tail -1
cp "$(bin)" .tzbin/before
git checkout HEAD -- crates/trace-commons-server/src/db/postgres.rs
$B 2>&1 | tail -1
git status --short
ls -la .tzbin
