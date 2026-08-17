#!/usr/bin/env bash
# Generate and sign the update manifest that installed clients poll.
#
# Only platforms passed on the command line are written. That is the whole
# safety property: the three release build jobs are independent, so this runs
# routinely with a subset green, and a manifest that named a platform whose
# artifact does not exist would point every client of that platform at a 404.
set -euo pipefail

die() { echo "generate-manifest: $*" >&2; exit 1; }

VERSION=""
OUT_DIR="dist/updates"
KEY_FILE=""
declare -a PLATFORM_ARGS=()

usage() {
  cat >&2 <<'EOF'
usage: generate-manifest.sh --version X.Y.Z --key <ed25519.pem> \
         [--out <dir>] \
         --platform <slug>=<url>=<sha256>=<size> [--platform ...]

slugs: windows-x86_64 | macos-universal | linux-x86_64
EOF
  exit 2
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version)  VERSION="${2:?--version needs a value}"; shift 2 ;;
    --key)      KEY_FILE="${2:?--key needs a path}"; shift 2 ;;
    --out)      OUT_DIR="${2:?--out needs a path}"; shift 2 ;;
    --platform) PLATFORM_ARGS+=("${2:?--platform needs a value}"); shift 2 ;;
    -h|--help)  usage ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$VERSION" ] || usage
[ -n "$KEY_FILE" ] || usage
[ -f "$KEY_FILE" ] || die "signing key not found: $KEY_FILE"
[ ${#PLATFORM_ARGS[@]} -gt 0 ] || die "refusing to publish a manifest with no platforms"

printf '%s' "$VERSION" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+$' \
  || die "version must be three-part numeric, got '$VERSION'"

PUBLISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

platforms_json=""
for spec in "${PLATFORM_ARGS[@]}"; do
  IFS='=' read -r slug url sha size <<<"$spec"
  case "$slug" in
    windows-x86_64|macos-universal|linux-x86_64) ;;
    *) die "unknown platform slug: $slug" ;;
  esac
  [ -n "$url" ]  || die "$slug: empty url"
  [ -n "$size" ] || die "$slug: empty size"
  printf '%s' "$sha" | grep -Eq '^[0-9a-f]{64}$' \
    || die "$slug: sha256 must be 64 lowercase hex characters, got '$sha'"
  printf '%s' "$size" | grep -Eq '^[0-9]+$' \
    || die "$slug: size must be numeric, got '$size'"

  entry="$(printf '"%s":{"url":"%s","sha256":"%s","size":%s}' \
             "$slug" "$url" "$sha" "$size")"
  if [ -n "$platforms_json" ]; then
    platforms_json="$platforms_json,$entry"
  else
    platforms_json="$entry"
  fi
done

mkdir -p "$OUT_DIR"
MANIFEST="$OUT_DIR/latest.json"

# Pretty-printed through jq so the published file is readable, and -S so key
# order is stable across runs. The signature covers these exact bytes, so
# nothing may touch the file after this point.
printf '{"schema_version":"trace_commons.update_manifest.v1","version":"%s","published_at":"%s","platforms":{%s}}' \
  "$VERSION" "$PUBLISHED_AT" "$platforms_json" \
  | jq -S . > "$MANIFEST"

openssl pkeyutl -sign -rawin -inkey "$KEY_FILE" -in "$MANIFEST" \
  | openssl base64 -A > "$MANIFEST.sig"

echo "wrote $MANIFEST"
echo "wrote $MANIFEST.sig"
