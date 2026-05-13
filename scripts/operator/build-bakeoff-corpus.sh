#!/usr/bin/env bash
#
# Build a bake-off corpus tarball (.tar.zst) for trace-commons-gate-calibrate.
#
# The corpus layout matches the format expected by
# `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_corpus.rs`:
#
#   manifest.json                          {"version":1,"novel_sha256":...,
#                                           "duplicate_sha256":...,
#                                           "paraphrase_sha256":...}
#   novel/novel-NNNN.txt                   one entry per file
#   duplicate/dup-NNNN.txt                 one entry per file
#   paraphrase/paraphrase.jsonl            one JSON line per pair
#
# Hash convention:
#   - novel + duplicate slice sha256 is over the concatenated raw bytes of
#     the slice's files in sorted-filename order.
#   - paraphrase slice sha256 is over the raw bytes of paraphrase.jsonl.
#
# Modes:
#   BAKEOFF_CORPUS_DRY_RUN=1
#       Emit a deterministic 2-of-each synthetic corpus from inline
#       fixtures. CI-runnable, no downloads, no model.
#   (default)
#       Build a real corpus from OASST2 + GAIA + curated boilerplate +
#       Qwen3-4B back-translation. This path is operator-only and is NOT
#       exercised in CI; the dry-run path covers the manifest/tarball
#       contract end-to-end.
#
# Usage: build-bakeoff-corpus.sh <output.tar.zst>
#
# Required env (real run only):
#   HF_TOKEN                              gated dataset auth
#   BAKEOFF_PARAPHRASE_MODEL_PATH         local Qwen3-4B-Base checkpoint dir
#
# Optional env (real run):
#   BAKEOFF_NOVEL_COUNT       default 500
#   BAKEOFF_DUPLICATE_COUNT   default 500
#   BAKEOFF_PARAPHRASE_COUNT  default 500
#
# Self-test:
#   BAKEOFF_CORPUS_SELF_TEST=1 + BAKEOFF_CORPUS_DRY_RUN=1
#       After producing the dry-run tarball, verify it is zstd-compressed
#       and unpacks to a valid manifest.
#
# Error convention: <CamelCaseClass>: <hash-only-or-label-detail>.
# Never echo raw paths, tokens, or operator-secret material.

set -euo pipefail

OUTPUT="${1:-}"

bail() { echo "BakeoffCorpusFailure: $1" >&2; exit 1; }

if [ -z "$OUTPUT" ]; then
  echo "BakeoffCorpusUsage: build-bakeoff-corpus.sh <output.tar.zst>" >&2
  exit 1
fi

case "$OUTPUT" in
  *.tar.zst) ;;
  *) bail "output_must_end_with_tar_zst" ;;
esac

DRY_RUN="${BAKEOFF_CORPUS_DRY_RUN:-0}"
SELF_TEST="${BAKEOFF_CORPUS_SELF_TEST:-0}"

NOVEL_COUNT="${BAKEOFF_NOVEL_COUNT:-500}"
DUPLICATE_COUNT="${BAKEOFF_DUPLICATE_COUNT:-500}"
PARAPHRASE_COUNT="${BAKEOFF_PARAPHRASE_COUNT:-500}"

command -v tar       >/dev/null 2>&1 || bail "tar_not_installed"
command -v zstd      >/dev/null 2>&1 || bail "zstd_not_installed"
command -v sha256sum >/dev/null 2>&1 || bail "sha256sum_not_installed"

# Helpers ---------------------------------------------------------------------

# sha256_of_concat <dir>
# Print "sha256:<hex>" computed over the raw bytes of every regular file
# in <dir>, concatenated in sorted-filename order. Matches the Rust
# loader's read_text_slice / sha256_label behavior exactly.
sha256_of_concat() {
  local dir="$1"
  local hex
  # `find -maxdepth 1 -type f` + LC_ALL=C sort = stable order on every
  # POSIX system. `xargs cat` would re-order across argv batches on huge
  # slices; we explicitly stream the files to a single cat invocation.
  hex=$(cd "$dir" && \
    find . -maxdepth 1 -type f -print0 \
      | LC_ALL=C sort -z \
      | xargs -0 cat \
      | sha256sum \
      | awk '{print $1}')
  if [ -z "$hex" ]; then
    bail "sha256_of_concat_empty"
  fi
  printf 'sha256:%s' "$hex"
}

# sha256_of_file <path>
# Print "sha256:<hex>" of the file's raw bytes.
sha256_of_file() {
  local path="$1"
  local hex
  hex=$(sha256sum < "$path" | awk '{print $1}')
  if [ -z "$hex" ]; then
    bail "sha256_of_file_empty"
  fi
  printf 'sha256:%s' "$hex"
}

# emit_manifest <staging_dir> <novel_sha> <dup_sha> <para_sha>
emit_manifest() {
  local staging="$1"
  local novel_sha="$2"
  local dup_sha="$3"
  local para_sha="$4"
  # JSON written by hand to avoid a python/jq dep here; values are
  # well-formed sha256 strings produced by our own helpers, so there's
  # nothing to escape.
  cat > "$staging/manifest.json" <<JSON
{"version":1,"novel_sha256":"$novel_sha","duplicate_sha256":"$dup_sha","paraphrase_sha256":"$para_sha"}
JSON
  [ -s "$staging/manifest.json" ] || bail "manifest_write_failed"
}

# pack_tarball <staging_dir> <output_path>
# Produce a tar.zst of staging_dir's contents at output_path.
# Note: older tar (BSD/macOS) doesn't infer .zst from -caf, so we pipe
# explicitly through zstd.
pack_tarball() {
  local staging="$1"
  local out="$2"
  # COPYFILE_DISABLE=1 suppresses macOS BSD tar's AppleDouble `._*`
  # entries; on Linux GNU tar it's a harmless no-op env. We deliberately
  # do not rely on `--no-xattrs` since BSD tar and GNU tar spell that
  # flag differently.
  ( cd "$staging" && COPYFILE_DISABLE=1 tar -cf - . ) | zstd -q -o "$out" \
    || bail "tarball_pack_failed"
  [ -s "$out" ] || bail "tarball_empty"
}

# Staging ---------------------------------------------------------------------

STAGING="$(mktemp -d -t bakeoff-corpus.XXXXXX)"
trap 'rm -rf "$STAGING"' EXIT

mkdir -p "$STAGING/novel" "$STAGING/duplicate" "$STAGING/paraphrase"

# Dry-run path ----------------------------------------------------------------

emit_dry_run() {
  echo "BakeoffCorpusStep: phase=dry_run_inline_fixtures"

  printf '%s' "synthetic novel reasoning trace zero" > "$STAGING/novel/novel-0000.txt"
  printf '%s' "synthetic novel reasoning trace one"  > "$STAGING/novel/novel-0001.txt"

  printf '%s' "common boilerplate zero" > "$STAGING/duplicate/dup-0000.txt"
  printf '%s' "common boilerplate one"  > "$STAGING/duplicate/dup-0001.txt"

  # paraphrase JSONL — the loader test pins orig-0/para-0 etc.
  {
    printf '{"original":"orig-0","paraphrase":"para-0"}\n'
    printf '{"original":"orig-1","paraphrase":"para-1"}\n'
  } > "$STAGING/paraphrase/paraphrase.jsonl"
}

# Real path -------------------------------------------------------------------
#
# NOTE: the real path is NOT exercised in CI. It requires HF auth + a
# multi-GB Qwen3-4B-Base checkpoint and GPU-class hardware. Use the
# dry-run path for CI coverage; the real path is operator work, and the
# resulting tarball's sha256 should be appended to
# scripts/operator/.bakeoff-corpus-checksums after each successful run.

# Locate the HF CLI. Newer huggingface_hub renamed `huggingface-cli` to
# `hf`; older installs still ship the long name. Either works.
resolve_hf_cli() {
  if command -v hf >/dev/null 2>&1; then
    printf '%s' "hf"
    return 0
  fi
  if command -v huggingface-cli >/dev/null 2>&1; then
    printf '%s' "huggingface-cli"
    return 0
  fi
  return 1
}

# download_hf_dataset <repo_id> <local_dir>
download_hf_dataset() {
  local repo="$1"
  local dir="$2"
  local cli
  cli=$(resolve_hf_cli) || bail "BakeoffCorpusMissingHfCli: install hf or huggingface-cli"
  # `hf` and `huggingface-cli` share the same `download` subcommand for
  # datasets. --quiet keeps stdout grep-able for our key=value lines.
  "$cli" download "$repo" \
    --repo-type dataset \
    --local-dir "$dir" \
    --local-dir-use-symlinks False \
    --quiet \
    || bail "hf_download_failed repo_label=$(printf '%s' "$repo" | sha256sum | awk '{print substr($1,1,12)}')"
}

# emit_hf_text_samples <parquet_root> <count> <out_dir> <prefix>
# Selects up to <count> rows whose text body is 200–2000 words and
# contains at least one reasoning marker, and writes each as a separate
# UTF-8 .txt file in <out_dir>, named <prefix>-NNNN.txt.
# Returns the actual number of files written on stdout.
emit_hf_text_samples() {
  local root="$1"
  local cap="$2"
  local out_dir="$3"
  local prefix="$4"
  command -v python3 >/dev/null 2>&1 || bail "python3_not_installed"
  python3 - "$root" "$cap" "$out_dir" "$prefix" <<'PYEOF'
import os
import re
import sys

root, cap_s, out_dir, prefix = sys.argv[1:5]
cap = int(cap_s)

try:
    import pyarrow.parquet as pq
except Exception:
    print("BakeoffCorpus: pyarrow_not_installed", file=sys.stderr)
    sys.exit(2)

MARKER = re.compile(r"\bstep 1\b|\bfirst,|\bbecause\b|\btherefore\b|\bso we\b", re.IGNORECASE)
TEXT_COLS = ("text", "message", "content", "Question", "question", "task")

def iter_rows(parquet_root):
    for dp, _, files in os.walk(parquet_root):
        for f in files:
            if not f.endswith(".parquet"):
                continue
            try:
                tbl = pq.read_table(os.path.join(dp, f))
            except Exception:
                continue
            col = next((c for c in TEXT_COLS if c in tbl.column_names), None)
            if col is None:
                continue
            for v in tbl.column(col).to_pylist():
                if v:
                    yield str(v)

n = 0
for body in iter_rows(root):
    words = body.split()
    if not (200 <= len(words) <= 2000):
        continue
    if not MARKER.search(body):
        continue
    path = os.path.join(out_dir, f"{prefix}-{n:04d}.txt")
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(body)
    n += 1
    if n >= cap:
        break

print(n)
PYEOF
}

# emit_duplicate_slice <out_dir>
# Loops over scripts/operator/bakeoff-duplicate-seeds.txt and writes
# DUPLICATE_COUNT files into out_dir, sampling with replacement.
emit_duplicate_slice() {
  local out_dir="$1"
  local script_dir
  script_dir="$(cd "$(dirname "$0")" && pwd)"
  local seeds="$script_dir/bakeoff-duplicate-seeds.txt"
  [ -f "$seeds" ] || bail "duplicate_seeds_file_missing"

  # Read non-blank lines into a numbered set we can index from bash.
  local -a SEED_LINES=()
  local line
  while IFS= read -r line || [ -n "$line" ]; do
    [ -z "$line" ] && continue
    SEED_LINES+=("$line")
  done < "$seeds"
  local n_seeds=${#SEED_LINES[@]}
  [ "$n_seeds" -gt 0 ] || bail "duplicate_seeds_empty"

  local i
  for (( i = 0; i < DUPLICATE_COUNT; i++ )); do
    local pick=$(( i % n_seeds ))
    printf '%s' "${SEED_LINES[$pick]}" > "$out_dir/dup-$(printf '%04d' "$i").txt"
  done
}

# emit_paraphrase_slice <novel_dir> <out_jsonl>
emit_paraphrase_slice() {
  local novel_dir="$1"
  local out_jsonl="$2"
  local script_dir
  script_dir="$(cd "$(dirname "$0")" && pwd)"
  local helper="$script_dir/bakeoff_paraphrase.py"
  [ -f "$helper" ] || bail "paraphrase_helper_missing"
  command -v python3 >/dev/null 2>&1 || bail "python3_not_installed"

  # Build the helper's input JSONL from the first PARAPHRASE_COUNT novel
  # entries (sorted-filename order, which matches what the loader sees).
  local tmp_in
  tmp_in="$(mktemp)"
  # shellcheck disable=SC2064
  trap "rm -f \"$tmp_in\"" RETURN

  python3 - "$novel_dir" "$PARAPHRASE_COUNT" > "$tmp_in" <<'PYEOF'
import json
import os
import sys
novel_dir, cap_s = sys.argv[1], sys.argv[2]
cap = int(cap_s)
files = sorted(f for f in os.listdir(novel_dir) if f.endswith(".txt"))
for i, fname in enumerate(files[:cap]):
    with open(os.path.join(novel_dir, fname), "r", encoding="utf-8") as fh:
        body = fh.read()
    sys.stdout.write(json.dumps({"original": body}) + "\n")
PYEOF

  BAKEOFF_PARAPHRASE_MODEL_PATH="$BAKEOFF_PARAPHRASE_MODEL_PATH" \
    python3 "$helper" "$BAKEOFF_PARAPHRASE_MODEL_PATH" < "$tmp_in" > "$out_jsonl" \
    || bail "BakeoffCorpusParaphraseHelperFailed: see paraphrase helper stderr"

  [ -s "$out_jsonl" ] || bail "paraphrase_output_empty"
}

emit_real() {
  [ -n "${HF_TOKEN:-}" ] || bail "BakeoffCorpusMissingHfToken: HF_TOKEN env required for OASST2"
  [ -n "${BAKEOFF_PARAPHRASE_MODEL_PATH:-}" ] \
    || bail "BakeoffCorpusMissingParaphraseModel: BAKEOFF_PARAPHRASE_MODEL_PATH env required"
  [ -d "$BAKEOFF_PARAPHRASE_MODEL_PATH" ] \
    || bail "BakeoffCorpusMissingParaphraseModel: model path is not a directory"

  command -v python3 >/dev/null 2>&1 || bail "python3_not_installed"

  local hf_cli
  hf_cli=$(resolve_hf_cli) || bail "BakeoffCorpusMissingHfCli: install hf or huggingface-cli"
  echo "BakeoffCorpusStep: phase=hf_cli_resolved cli=$hf_cli"

  local dl_dir
  dl_dir="$(mktemp -d -t bakeoff-hf.XXXXXX)"
  # Append to the existing EXIT trap so we still clean STAGING.
  # shellcheck disable=SC2064
  trap "rm -rf \"$STAGING\" \"$dl_dir\"" EXIT

  local half=$(( NOVEL_COUNT / 2 ))
  local oasst_target=$half
  local gaia_target=$(( NOVEL_COUNT - half ))

  echo "BakeoffCorpusStep: phase=oasst2_download"
  download_hf_dataset "OpenAssistant/oasst2" "$dl_dir/oasst2"

  echo "BakeoffCorpusStep: phase=gaia_download"
  download_hf_dataset "gaia-benchmark/GAIA" "$dl_dir/gaia"

  echo "BakeoffCorpusStep: phase=oasst2_filter target=$oasst_target"
  local oasst_n
  oasst_n=$(emit_hf_text_samples "$dl_dir/oasst2" "$oasst_target" "$STAGING/novel" "novel")
  echo "BakeoffCorpusStep: phase=oasst2_filter_done count=$oasst_n"

  echo "BakeoffCorpusStep: phase=gaia_filter target=$gaia_target"
  # GAIA entries are renamed novel-NNNN.txt continuing the index after
  # OASST2's last write. We re-emit by passing a different prefix then
  # renaming, but simplest is to call emit_hf_text_samples with the
  # offset baked into the prefix. We use a temp dir + rename to keep
  # the unified naming convention novel-NNNN.txt.
  local gaia_tmp
  gaia_tmp="$(mktemp -d -t bakeoff-gaia.XXXXXX)"
  local gaia_n
  gaia_n=$(emit_hf_text_samples "$dl_dir/gaia" "$gaia_target" "$gaia_tmp" "g")
  local i=0
  for f in $(cd "$gaia_tmp" && find . -maxdepth 1 -type f -name 'g-*.txt' | LC_ALL=C sort); do
    local idx=$(( oasst_n + i ))
    mv "$gaia_tmp/${f#./}" "$STAGING/novel/novel-$(printf '%04d' "$idx").txt"
    i=$(( i + 1 ))
  done
  rm -rf "$gaia_tmp"
  echo "BakeoffCorpusStep: phase=gaia_filter_done count=$gaia_n"

  local novel_total=$(( oasst_n + gaia_n ))
  [ "$novel_total" -gt 0 ] || bail "novel_slice_empty"

  echo "BakeoffCorpusStep: phase=duplicate_curate count=$DUPLICATE_COUNT"
  emit_duplicate_slice "$STAGING/duplicate"

  echo "BakeoffCorpusStep: phase=paraphrase_backtranslate count=$PARAPHRASE_COUNT"
  emit_paraphrase_slice "$STAGING/novel" "$STAGING/paraphrase/paraphrase.jsonl"
}

# Dispatch --------------------------------------------------------------------

if [ "$DRY_RUN" = "1" ]; then
  emit_dry_run
else
  emit_real
fi

# Hash + manifest -------------------------------------------------------------

echo "BakeoffCorpusStep: phase=hash_slices"
NOVEL_SHA=$(sha256_of_concat "$STAGING/novel")
DUP_SHA=$(sha256_of_concat "$STAGING/duplicate")
PARA_SHA=$(sha256_of_file   "$STAGING/paraphrase/paraphrase.jsonl")

echo "BakeoffCorpusStep: phase=emit_manifest"
emit_manifest "$STAGING" "$NOVEL_SHA" "$DUP_SHA" "$PARA_SHA"

echo "BakeoffCorpusStep: phase=pack_tarball"
pack_tarball "$STAGING" "$OUTPUT"

TARBALL_SHA=$(sha256_of_file "$OUTPUT")
echo "BakeoffCorpusOK output_sha256=$TARBALL_SHA"

# Self-test -------------------------------------------------------------------

if [ "$SELF_TEST" = "1" ]; then
  echo "BakeoffCorpusStep: phase=self_test"
  command -v file >/dev/null 2>&1 || bail "self_test_file_cmd_missing"
  if ! file "$OUTPUT" | grep -qi "zstandard"; then
    bail "self_test_not_zstd"
  fi
  VERIFY_DIR="$(mktemp -d -t bakeoff-verify.XXXXXX)"
  # shellcheck disable=SC2064
  trap "rm -rf \"$STAGING\" \"$VERIFY_DIR\"" EXIT
  zstd -dc "$OUTPUT" | ( cd "$VERIFY_DIR" && tar -xf - ) \
    || bail "self_test_unpack_failed"
  [ -s "$VERIFY_DIR/manifest.json" ] || bail "self_test_manifest_missing"
  grep -q '"version":1' "$VERIFY_DIR/manifest.json" \
    || bail "self_test_manifest_version_mismatch"
  grep -q '"novel_sha256":"sha256:' "$VERIFY_DIR/manifest.json" \
    || bail "self_test_manifest_novel_sha_missing"
  echo "BakeoffCorpusSelfTestOK"
fi
