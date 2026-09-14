#!/bin/sh
set -eu

if [ "$#" -ne 3 ]; then
  echo "usage: measure.sh DEBUG_TEST_BINARY RELEASE_TEST_BINARY OUTPUT_DIRECTORY" >&2
  exit 2
fi

debug_binary=$1
release_binary=$2
output_directory=$3
test_name=insights::comparison_estimator::calibration::measure_exact_candidate_evaluation

if [ -e "$output_directory" ]; then
  echo "output directory already exists" >&2
  exit 2
fi
mkdir -p "$output_directory/debug" "$output_directory/release"

shasum -a 256 "$debug_binary" "$release_binary" > "$output_directory/BINARY_SHA256SUMS"

for profile in debug release; do
  if [ "$profile" = debug ]; then
    binary=$debug_binary
  else
    binary=$release_binary
  fi
  for case_name in balanced_boundary balanced_interior imbalanced_supported imbalanced_suppressed; do
    for run_index in 0 1 2 3; do
      prefix="$output_directory/$profile/$case_name-$run_index"
      TRACE_COMMONS_EXACT_EVALUATION_CASE=$case_name \
        TRACE_COMMONS_EXACT_EVALUATION_OUTPUT="$prefix.json" \
        /usr/bin/time -l -p -o "$prefix.time" \
        "$binary" --exact "$test_name" --ignored --nocapture \
        > "$prefix.stdout" 2> "$prefix.stderr"
    done
  done
done

