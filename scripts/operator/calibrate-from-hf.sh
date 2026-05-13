#!/usr/bin/env bash
#
# Offline HF bootstrap calibration:
# - Downloads OASST2 and GAIA via `huggingface-cli`.
# - Adapts each row to a single-line JSONL with key "plaintext".
# - Pipes through `tracedao-gate-calibrate` (the new binary).
# - Writes per-trace metrics to a CSV with columns:
#     dataset,row_idx,perplexity_micros,tail_fraction_micros,novelty_score_micros
#
# HF data is NEVER added to the corpus — we read each row, score it,
# discard the plaintext. Only the numeric metrics are retained.
#
# Required env: the same envs the gate service reads for model paths +
# device, plus HF_TOKEN for downloading. The binary does NOT read floor
# envs (the whole point of calibration is to derive them).
#
# Flags:
#   --output=<csv path>
#   --sample-size=<N>    OASST2 sample size (default 10000)
#   --gaia-size=<N>      GAIA sample size (default 500)

set -euo pipefail

OUTPUT="/var/tmp/gate-bootstrap.csv"
SAMPLE_SIZE=10000
GAIA_SIZE=500

while [ $# -gt 0 ]; do
  case "$1" in
    --output=*)       OUTPUT="${1#*=}";       shift ;;
    --sample-size=*)  SAMPLE_SIZE="${1#*=}";  shift ;;
    --gaia-size=*)    GAIA_SIZE="${1#*=}";    shift ;;
    *) echo "CalibrateFromHfUnknownArg: $1" >&2; exit 1 ;;
  esac
done

bail() { echo "CalibrateFromHfFailure: $1" >&2; exit 1; }

command -v huggingface-cli >/dev/null 2>&1 || bail "huggingface_cli_not_installed"
command -v jq              >/dev/null 2>&1 || bail "jq_not_installed"
command -v python3         >/dev/null 2>&1 || bail "python3_not_installed"

# We use python3 for arrow/parquet row iteration. The bootstrap script
# itself stays in bash for portability; the inline python is tiny and
# only does row decode → JSONL.
python3 -c "import pyarrow" 2>/dev/null || bail "pyarrow_not_installed"

# Locate the calibrate binary. Prefer the worktree release build; fall
# back to PATH.
BIN=""
for candidate in \
  "$(pwd)/target/release/tracedao-gate-calibrate" \
  "target/release/tracedao-gate-calibrate" \
  "$(command -v tracedao-gate-calibrate || true)"; do
  if [ -n "$candidate" ] && [ -x "$candidate" ]; then
    BIN="$candidate"
    break
  fi
done
[ -n "$BIN" ] || bail "tracedao_gate_calibrate_binary_not_found"

WORKDIR="${TMPDIR:-/tmp}/calibrate-from-hf.$$"
mkdir -p "$WORKDIR"
trap 'rm -rf "$WORKDIR"' EXIT

echo "CalibrateFromHf: downloading OASST2"
huggingface-cli download OpenAssistant/oasst2 \
  --repo-type dataset \
  --local-dir "$WORKDIR/oasst2" \
  --local-dir-use-symlinks False \
  --quiet

echo "CalibrateFromHf: downloading GAIA"
# GAIA is gated; HF_TOKEN must have access.
huggingface-cli download gaia-benchmark/GAIA \
  --repo-type dataset \
  --local-dir "$WORKDIR/gaia" \
  --local-dir-use-symlinks False \
  --quiet || echo "CalibrateFromHfWarning: gaia_download_failed_continuing_with_oasst2_only" >&2

# Emit JSONL on stdout. One {"plaintext": "..."} per line.
emit_jsonl() {
python3 - "$WORKDIR" "$SAMPLE_SIZE" "$GAIA_SIZE" <<'PYEOF'
import json
import os
import sys
import pyarrow.parquet as pq

workdir, sample_size, gaia_size = sys.argv[1], int(sys.argv[2]), int(sys.argv[3])

def iter_parquets(root):
    for dirpath, _, files in os.walk(root):
        for f in files:
            if f.endswith(".parquet"):
                yield os.path.join(dirpath, f)

def emit_oasst2(path, cap):
    """OASST2 row → one assistant-style turn-concatenated string."""
    n = 0
    for pq_path in iter_parquets(path):
        tbl = pq.read_table(pq_path)
        # Field names vary across snapshots; we look for the text column.
        cols = tbl.column_names
        text_col = next((c for c in ("text", "message", "content") if c in cols), None)
        if text_col is None:
            continue
        for row in tbl.column(text_col).to_pylist():
            if not row:
                continue
            print(json.dumps({"plaintext": str(row)[:8000]}))
            n += 1
            if n >= cap:
                return n
    return n

def emit_gaia(path, cap):
    n = 0
    for pq_path in iter_parquets(path):
        tbl = pq.read_table(pq_path)
        cols = tbl.column_names
        text_col = next((c for c in ("Question", "question", "task", "text") if c in cols), None)
        if text_col is None:
            continue
        for row in tbl.column(text_col).to_pylist():
            if not row:
                continue
            print(json.dumps({"plaintext": str(row)[:8000]}))
            n += 1
            if n >= cap:
                return n
    return n

oasst2_root = os.path.join(workdir, "oasst2")
gaia_root = os.path.join(workdir, "gaia")

if os.path.isdir(oasst2_root):
    emit_oasst2(oasst2_root, sample_size)
if os.path.isdir(gaia_root):
    emit_gaia(gaia_root, gaia_size)
PYEOF
}

echo "CalibrateFromHf: scoring (output -> $OUTPUT)"
# Write CSV header.
echo "dataset,row_idx,perplexity_micros,tail_fraction_micros,novelty_score_micros" > "$OUTPUT"

# Stream JSONL through the calibrate binary, then reshape each output
# JSONL line into a CSV row. We tag each row's dataset by counting against
# SAMPLE_SIZE: the first SAMPLE_SIZE lines are OASST2, the rest are GAIA.
row_idx=0
emit_jsonl | "$BIN" 2>>/tmp/calibrate-stderr.log | while IFS= read -r line; do
  if [ "$row_idx" -lt "$SAMPLE_SIZE" ]; then
    ds="oasst2"; idx="$row_idx"
  else
    ds="gaia"; idx=$((row_idx - SAMPLE_SIZE))
  fi
  perp=$(echo "$line" | jq -r '.perplexity_micros // empty')
  tail=$(echo "$line" | jq -r '.tail_fraction_micros // empty')
  nov=$(echo  "$line" | jq -r '.novelty_score_micros // empty')
  if [ -z "$perp" ] || [ -z "$tail" ] || [ -z "$nov" ]; then
    echo "CalibrateFromHfWarning: bad_row_skipped" >&2
    row_idx=$((row_idx + 1))
    continue
  fi
  echo "${ds},${idx},${perp},${tail},${nov}" >> "$OUTPUT"
  row_idx=$((row_idx + 1))
done

count=$(($(wc -l < "$OUTPUT") - 1))
echo "CalibrateFromHfOK rows=$count output=$OUTPUT"
