#!/usr/bin/env bash
#
# Read the per-trace metrics CSV from calibrate-from-hf.sh, print
# recommended TRACE_COMMONS_GATE_*_FLOOR_MICROS values that target a
# configurable pass rate.
#
# A target pass rate of P means: pick the floor at the (1 - P) percentile
# so that P fraction of bootstrap traces clear it.
#
# Flags:
#   --input=<csv path>             (required)
#   --target-pass-rate=<0..1>      default 0.30
#
# CSV schema (header expected):
#   dataset,row_idx,perplexity_micros,tail_fraction_micros,novelty_score_micros

set -euo pipefail

INPUT=""
TARGET_PASS_RATE="0.30"

while [ $# -gt 0 ]; do
  case "$1" in
    --input=*)             INPUT="${1#*=}";             shift ;;
    --target-pass-rate=*)  TARGET_PASS_RATE="${1#*=}";  shift ;;
    *) echo "AnalyzeCalibrationUnknownArg: $1" >&2; exit 1 ;;
  esac
done

bail() { echo "AnalyzeCalibrationFailure: $1" >&2; exit 1; }

[ -n "$INPUT" ]   || bail "input_unset"
[ -f "$INPUT" ]   || bail "input_not_found"

command -v awk  >/dev/null 2>&1 || bail "awk_not_installed"
command -v sort >/dev/null 2>&1 || bail "sort_not_installed"

# Compute the percentile cut-off index. With awk's integer arithmetic
# only, we scale to avoid float surprises.
# rate is in [0,1]; percentile = 1 - rate; index = ceil(percentile * N).
percentile_index_for_n() {
  local n="$1"
  awk -v n="$n" -v r="$TARGET_PASS_RATE" '
    BEGIN {
      p = 1.0 - r;
      idx = int(p * n + 0.999999);
      if (idx < 1) idx = 1;
      if (idx > n) idx = n;
      printf "%d", idx;
    }'
}

cut_at_percentile() {
  # cut_at_percentile <column-index, 3=perp, 4=tail, 5=nov>
  local col="$1"
  # Skip header, extract column, sort ascending, count, pick at index.
  local sorted
  sorted=$(awk -F, -v c="$col" 'NR>1 && $c ~ /^[0-9]+$/ { print $c }' "$INPUT" | sort -n)
  local n
  n=$(echo "$sorted" | wc -l | awk '{print $1}')
  if [ "$n" = "0" ]; then
    echo "0"
    return
  fi
  local idx
  idx=$(percentile_index_for_n "$n")
  echo "$sorted" | awk -v i="$idx" 'NR==i { print; exit }'
}

PERP_FLOOR=$(cut_at_percentile 3)
TAIL_FLOOR=$(cut_at_percentile 4)
NOV_FLOOR=$(cut_at_percentile 5)

rows=$(($(wc -l < "$INPUT") - 1))
echo "# AnalyzeCalibrationOK rows=$rows target_pass_rate=$TARGET_PASS_RATE"
echo "RECOMMENDED TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS=$PERP_FLOOR"
echo "RECOMMENDED TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS=$TAIL_FLOOR"
echo "RECOMMENDED TRACE_COMMONS_GATE_NOVELTY_FLOOR_MICROS=$NOV_FLOOR"
