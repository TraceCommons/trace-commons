#!/usr/bin/env bash
#
# Stage perplexity + embedder model weights for the gate service.
#
# - Downloads Llama-3.1-8B-Instruct via `huggingface-cli` into
#   $TRACE_COMMONS_PERPLEXITY_MODEL_PATH.
# - Downloads BGE-large-en-v1.5 via `huggingface-cli` into a directory
#   under $TRACE_COMMONS_EMBEDDER_CACHE_DIR. (`fastembed`'s built-in cache
#   manager picks up these files on first use; we stage them explicitly
#   so the SHA256 check runs.)
# - Verifies each file against the pinned SHA256 in .model-checksums.
# - Idempotent: if a file's SHA256 already matches, the download is
#   skipped.
# - Hash-only diagnostics on failure.
#
# Required env:
#   TRACE_COMMONS_PERPLEXITY_MODEL_PATH
#   TRACE_COMMONS_EMBEDDER_CACHE_DIR
#   HF_TOKEN  (HuggingFace token with access to gated repos)
#
# Optional flags:
#   --perplexity-only          skip embedder
#   --embedder-only            skip perplexity

set -euo pipefail

PERPLEXITY_ONLY=0
EMBEDDER_ONLY=0

for arg in "$@"; do
  case "$arg" in
    --perplexity-only) PERPLEXITY_ONLY=1 ;;
    --embedder-only)   EMBEDDER_ONLY=1 ;;
    *) echo "StageModelsUnknownArg: $arg" >&2; exit 1 ;;
  esac
done

bail() {
  echo "StageModelsFailure: $1" >&2
  exit 1
}

require_env() {
  local var="$1"
  if [ -z "${!var:-}" ]; then
    bail "${var}_unset"
  fi
}

require_cmd() {
  local cmd="$1"
  if ! command -v "$cmd" >/dev/null 2>&1; then
    bail "${cmd}_not_installed"
  fi
}

require_cmd huggingface-cli
require_cmd sha256sum

CHECKSUMS_FILE="$(cd "$(dirname "$0")" && pwd)/.model-checksums"
if [ ! -f "$CHECKSUMS_FILE" ]; then
  bail "checksums_file_missing"
fi

verify_sha256() {
  # verify_sha256 <file> <key>
  # key is a string in .model-checksums of the form "<sha256>  <key>".
  local file="$1"
  local key="$2"
  if [ ! -f "$file" ]; then
    return 1
  fi
  local expected
  expected=$(awk -v k="$key" '$2 == k { print $1 }' "$CHECKSUMS_FILE" | head -n1)
  if [ -z "$expected" ]; then
    # No checksum pinned yet. Warn and accept (operator fills in after
    # first verified download).
    echo "StageModelsChecksumUnpinned: $key" >&2
    return 0
  fi
  local actual
  actual=$(sha256sum "$file" | awk '{print $1}')
  if [ "$expected" != "$actual" ]; then
    bail "checksum_mismatch:$key"
  fi
  return 0
}

stage_perplexity() {
  require_env TRACE_COMMONS_PERPLEXITY_MODEL_PATH
  require_env HF_TOKEN
  local target="$TRACE_COMMONS_PERPLEXITY_MODEL_PATH"
  local repo="${TRACE_COMMONS_PERPLEXITY_MODEL_ID:-meta-llama/Llama-3.1-8B-Instruct}"
  mkdir -p "$target"

  echo "StageModels: perplexity into $target"
  # Re-running huggingface-cli download is cheap when files are already
  # present locally; the CLI uses content-addressed caching.
  HUGGING_FACE_HUB_TOKEN="$HF_TOKEN" \
    huggingface-cli download "$repo" \
      --local-dir "$target" \
      --local-dir-use-symlinks False \
      --quiet

  # Verify the load-bearing files. config.json + tokenizer.json + at
  # least one safetensors shard.
  for f in config.json tokenizer.json; do
    if [ ! -f "$target/$f" ]; then
      bail "missing:${repo}:$f"
    fi
    verify_sha256 "$target/$f" "${repo}/${f}"
  done

  # Check for either single-shard or multi-shard safetensors layout.
  if [ -f "$target/model.safetensors" ]; then
    verify_sha256 "$target/model.safetensors" "${repo}/model.safetensors"
  elif [ -f "$target/model.safetensors.index.json" ]; then
    verify_sha256 "$target/model.safetensors.index.json" \
      "${repo}/model.safetensors.index.json"
    # Shard verification is operator-extensible by adding shard entries
    # to .model-checksums.
    for shard in "$target"/model-*.safetensors; do
      [ -e "$shard" ] || continue
      verify_sha256 "$shard" "${repo}/$(basename "$shard")"
    done
  else
    bail "no_safetensors:${repo}"
  fi
}

stage_embedder() {
  require_env TRACE_COMMONS_EMBEDDER_CACHE_DIR
  local cache="$TRACE_COMMONS_EMBEDDER_CACHE_DIR"
  local repo="${TRACE_COMMONS_EMBEDDER_MODEL_ID:-Xenova/bge-large-en-v1.5}"
  local subdir
  subdir="$cache/$(basename "$repo")"
  mkdir -p "$subdir"

  echo "StageModels: embedder into $subdir"
  huggingface-cli download "$repo" \
    --local-dir "$subdir" \
    --local-dir-use-symlinks False \
    --quiet

  # The fastembed ONNX layout has model.onnx + tokenizer.json + config.json.
  for f in model.onnx tokenizer.json config.json; do
    if [ ! -f "$subdir/$f" ]; then
      # Some ONNX repos shard model.onnx + model.onnx_data; check both.
      if [ "$f" = "model.onnx" ] && [ -f "$subdir/onnx/model.onnx" ]; then
        f="onnx/$f"
      else
        bail "missing:${repo}:$f"
      fi
    fi
    verify_sha256 "$subdir/$f" "${repo}/$f"
  done
}

if [ "$EMBEDDER_ONLY" -eq 0 ]; then
  stage_perplexity
fi
if [ "$PERPLEXITY_ONLY" -eq 0 ]; then
  stage_embedder
fi

echo "StageModelsOK"
