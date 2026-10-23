#!/usr/bin/env bash
# Run an identical Dagger check selection twice; keep evidence outside inputs.
set -euo pipefail
cd "$(dirname "$0")/../.."
output=".local/dagger-runs/$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$output"
for run in 1 2; do
  started=$(date +%s)
  printf 'Dagger run %s; log: %s/run-%s.log\n' "$run" "$output" "$run"
  dagger --progress=plain -v check "$@" 2>&1 | tee "$output/run-$run.log"
  elapsed=$(( $(date +%s) - started ))
  hits=$(grep -c 'CACHED' "$output/run-$run.log" || true)
  printf 'run=%s elapsed_seconds=%s cached_vertices=%s\n' "$run" "$elapsed" "${hits:-0}" | tee -a "$output/summary.txt"
done
printf 'Both runs passed. Compare command/function CACHED lines in %s.\n' "$output"
