#!/usr/bin/env bash
#
# Build a bake-off corpus tarball (.tar.zst) for tracedao-gate-calibrate.
#
# The corpus layout matches the format expected by
# `crates/tracedao-server/src/bin/gate_calibrate/bakeoff_corpus.rs`:
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

# Real path (Task 8b will fill this in) ---------------------------------------

emit_real() {
  bail "BakeoffCorpusRealPathNotImplemented: see Task 8b"
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
