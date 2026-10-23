#!/usr/bin/env bash
# Refuse a release binary that carries the staging pin verifier.
#
# The signed NEAR AI pin-set format, its verifier, the staging pin-signing
# public key and the pin URL exist only under the staging build feature
# (staging design 5.7). The release check refuses that feature on every
# release build line; this is the second, independent check, run on the
# bytes a release actually ships: the FFI dylib and the CLI.
#
# A binary is refused if any file under the given paths contains:
#
#   - the pin-set format name, `trace-commons-near-ai-pins/` followed by any
#     version (each version is also its signature's domain prefix, so no
#     verifier exists without it). The verifier reads v2, which carries the
#     RTMR3 and compose-action allowlist, and spells v1 only to refuse it;
#     both are caught by the one marker;
#   - the pin URL path, `near-ai-pins/current.json`;
#   - a staging pin-signing public key listed in the keys file, as its raw
#     32 bytes, lower- or upper-case hex, or base64 (standard or URL-safe,
#     padded or not).
#
# Staging host names are not markers: a production build holds SHA-256
# digests of them for its deny list, never the names.
#
# The keys file (default scripts/ci/staging-pin-public-keys.txt) lists one
# 64-hex-character Ed25519 public key per line; `#` comments and blank lines
# are ignored and anything else is refused. Until the staging key is
# generated and compiled in, it lists none, and the format string is the
# marker that holds. A missing or unreadable keys file is refused: a scan
# that silently dropped the key check would still print a pass.
#
# Usage:
#   scripts/ci/assert-no-staging-markers.sh check [--keys-file FILE] PATH...
#       Scan every regular file under each PATH (a file or a directory).
#       Exit 1 naming each file and marker found.
#   scripts/ci/assert-no-staging-markers.sh self-test
#       Seed each marker into a scratch binary and prove each is refused,
#       and that a clean binary passes. Also require the format marker to be
#       the one the verifier's source compiles in.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
DEFAULT_KEYS_FILE="$REPO_ROOT/scripts/ci/staging-pin-public-keys.txt"
FORMAT_MARKER="trace-commons-near-ai-pins/"
URL_MARKER="near-ai-pins/current.json"
STAGING_PINS_SOURCE="$REPO_ROOT/crates/trace-commons-attestation/src/staging_pins.rs"

check() {
  local keys_file="$DEFAULT_KEYS_FILE"
  if [[ "${1:-}" == --keys-file ]]; then
    keys_file="${2:?--keys-file needs a path}"
    shift 2
  fi
  if (($# == 0)); then
    echo "REFUSING: no binary or directory to scan" >&2
    return 1
  fi
  # Byte search in python: a key's raw bytes can hold NUL, which no grep
  # pattern can carry. Isolated mode, so nothing beside the scanned files
  # or this script can shadow a standard module.
  python3 -I - "$keys_file" "$FORMAT_MARKER" "$URL_MARKER" "$@" <<'PY'
import base64, os, re, sys

keys_file, format_marker, url_marker, *paths = sys.argv[1:]
failures = 0

def refuse(msg):
    global failures
    failures += 1
    print(f"REFUSING: {msg}", file=sys.stderr)

try:
    with open(keys_file, encoding="ascii") as f:
        lines = f.read().splitlines()
except (OSError, UnicodeDecodeError) as e:
    print(f"REFUSING: cannot read the keys file {keys_file}: {e.__class__.__name__}", file=sys.stderr)
    sys.exit(1)

keys = []
for n, line in enumerate(lines, 1):
    text = line.strip()
    if not text or text.startswith("#"):
        continue
    if not re.fullmatch(r"[0-9a-fA-F]{64}", text):
        print(f"REFUSING: {keys_file}:{n} is not a 64-hex-character public key", file=sys.stderr)
        sys.exit(1)
    keys.append(bytes.fromhex(text))

markers = [
    (format_marker.encode(), "the pin-set format name"),
    (url_marker.encode(), "the pin URL path"),
]
for i, key in enumerate(keys, 1):
    label = f"staging pin key {i}"
    markers.append((key, f"{label} (raw bytes)"))
    markers.append((key.hex().encode(), f"{label} (hex)"))
    markers.append((key.hex().upper().encode(), f"{label} (hex)"))
    std = base64.b64encode(key)
    url = base64.urlsafe_b64encode(key)
    # The unpadded forms are prefixes of the padded ones, so they cover both.
    markers.append((std.rstrip(b"="), f"{label} (base64)"))
    markers.append((url.rstrip(b"="), f"{label} (base64url)"))

files = []
for path in paths:
    if os.path.isfile(path):
        files.append(path)
    elif os.path.isdir(path):
        found = []
        for root, _dirs, names in os.walk(path):
            for name in names:
                full = os.path.join(root, name)
                if os.path.isfile(full) and not os.path.islink(full):
                    found.append(full)
        if not found:
            refuse(f"{path} holds no file to scan")
        files.extend(sorted(found))
    else:
        refuse(f"{path} does not exist")

for file in files:
    with open(file, "rb") as f:
        data = f.read()
    for marker, label in markers:
        if marker in data:
            refuse(f"{file} contains {label}")

if failures:
    print(f"{failures} staging marker problem(s); this binary must not ship.", file=sys.stderr)
    sys.exit(1)
print(f"no staging pin verifier in {len(files)} file(s); {len(keys)} staging key(s) listed")
PY
}

# The verifier's source must spell the format and the signing prefix --
# the v2 ones it reads and the v1 ones it recognises to refuse -- with
# FORMAT_MARKER, so the scan looks for the bytes a staging build carries.
# The pin URL and the compiled-in key do not exist yet; the change that adds
# them extends this check (staging plan, Task 5).
sources_agree() {
  local source="${1:-$STAGING_PINS_SOURCE}" failures=0 name
  if [[ ! -f "$source" ]]; then
    echo "REFUSING: the verifier source $source does not exist" >&2
    return 1
  fi
  for name in 'FORMAT: &str = "' 'SIGNING_PREFIX: &[u8] = b"' \
    'FORMAT_V1: &str = "' 'SIGNING_PREFIX_V1: &[u8] = b"'; do
    if ! grep -q -F -- "pub const $name$FORMAT_MARKER" "$source"; then
      echo "REFUSING: $source has no 'pub const $name$FORMAT_MARKER...'; the scan's format marker has drifted from the verifier" >&2
      failures=$((failures + 1))
    fi
  done
  ((failures == 0))
}

self_test() {
  local scratch status=0
  scratch="$(mktemp -d)"
  trap 'rm -rf "$scratch"' RETURN

  local key_hex
  key_hex="$(python3 -I -c 'import os; print(os.urandom(32).hex())')"
  printf '# a test key\n\n%s\n' "$key_hex" >"$scratch/keys.txt"
  printf '# no keys yet\n' >"$scratch/no-keys.txt"

  # A stand-in binary: random bytes around ordinary strings a production
  # build does carry, the staging host digests' neighbours included.
  make_clean() {
    python3 -I -c '
import os, sys
with open(sys.argv[1], "wb") as f:
    f.write(os.urandom(4096))
    f.write(b"\x00https://cloud-api.near.ai/v1\x00near-ai-measurements\x00")
    f.write(b"trace-commons-near-ai\x00near-ai-pins/\x00current.json\x00")
    f.write(os.urandom(4096))
' "$1"
  }
  # Append bytes to a copy of the clean binary, mid-file, so a marker is
  # found wherever it sits.
  seed() {
    python3 -I -c '
import sys
data = open(sys.argv[1], "rb").read()
mid = len(data) // 2
open(sys.argv[1], "wb").write(data[:mid] + sys.argv[2].encode("latin-1").decode("unicode_escape").encode("latin-1") + data[mid:])
' "$1" "$2"
  }
  seed_raw_key() {
    python3 -I -c '
import sys
data = open(sys.argv[1], "rb").read()
mid = len(data) // 2
open(sys.argv[1], "wb").write(data[:mid] + bytes.fromhex(sys.argv[2]) + data[mid:])
' "$1" "$2"
  }
  seed_b64_key() {
    python3 -I -c '
import base64, sys
key = bytes.fromhex(sys.argv[2])
forms = {
    "std": base64.b64encode(key),
    "std-unpadded": base64.b64encode(key).rstrip(b"="),
    "url": base64.urlsafe_b64encode(key),
    "url-unpadded": base64.urlsafe_b64encode(key).rstrip(b"="),
}
data = open(sys.argv[1], "rb").read()
mid = len(data) // 2
open(sys.argv[1], "wb").write(data[:mid] + forms[sys.argv[3]] + data[mid:])
' "$1" "$2" "$3"
  }

  expect_pass() {
    local desc="$1"
    shift
    if (check "$@" >/dev/null 2>&1); then
      echo "self-test: passed as expected: $desc"
    else
      echo "self-test: '$desc' should pass and was refused" >&2
      (check "$@" 2>&1 | sed 's/^/  /') || true
      status=1
    fi
  }
  # A refusal must be the scan's own (a REFUSING line), not a crash that
  # happens to exit non-zero.
  expect_refused() {
    local desc="$1" out
    shift
    if out="$(check "$@" 2>&1)"; then
      echo "self-test: '$desc' was NOT refused" >&2
      status=1
    elif ! grep -q '^REFUSING: ' <<<"$out"; then
      echo "self-test: '$desc' failed without a refusal:" >&2
      printf '  %s\n' "$out" >&2
      status=1
    else
      echo "self-test: refused as expected: $desc"
    fi
  }

  mkdir -p "$scratch/clean"
  make_clean "$scratch/clean/lib.dylib"
  make_clean "$scratch/clean/near-ai"
  expect_pass "a clean dylib and CLI" --keys-file "$scratch/keys.txt" \
    "$scratch/clean/lib.dylib" "$scratch/clean/near-ai"
  expect_pass "a clean bundle directory" --keys-file "$scratch/keys.txt" "$scratch/clean"
  expect_pass "a keys file that lists no key yet" --keys-file "$scratch/no-keys.txt" \
    "$scratch/clean/lib.dylib"
  expect_pass "the repository's keys file" "$scratch/clean/lib.dylib"

  local case_dir n=0
  new_case() {
    n=$((n + 1))
    case_dir="$scratch/case$n"
    mkdir -p "$case_dir/Contents/Frameworks"
    cp "$scratch/clean/lib.dylib" "$case_dir/Contents/Frameworks/lib.dylib"
    cp "$scratch/clean/near-ai" "$case_dir/near-ai"
  }

  new_case
  seed "$case_dir/Contents/Frameworks/lib.dylib" "\\x00trace-commons-near-ai-pins/v1\\x00"
  expect_refused "the pin-set format string" --keys-file "$scratch/keys.txt" \
    "$case_dir/Contents/Frameworks/lib.dylib"
  expect_refused "the pin-set format string deep in a bundle directory" \
    --keys-file "$scratch/keys.txt" "$case_dir"

  new_case
  seed "$case_dir/near-ai" "trace-commons-near-ai-pins/v1\\n"
  expect_refused "the signing domain prefix" --keys-file "$scratch/keys.txt" "$case_dir/near-ai"

  new_case
  seed "$case_dir/near-ai" "trace-commons-near-ai-pins/v2"
  expect_refused "a later pin-set format version" --keys-file "$scratch/keys.txt" "$case_dir/near-ai"

  new_case
  seed "$case_dir/near-ai" "https://staging.example/$URL_MARKER"
  expect_refused "the pin URL path" --keys-file "$scratch/keys.txt" "$case_dir/near-ai"

  new_case
  seed_raw_key "$case_dir/Contents/Frameworks/lib.dylib" "$key_hex"
  expect_refused "the staging key's raw bytes" --keys-file "$scratch/keys.txt" "$case_dir"

  new_case
  seed "$case_dir/near-ai" "$key_hex"
  expect_refused "the staging key in lower-case hex" --keys-file "$scratch/keys.txt" "$case_dir"

  new_case
  seed "$case_dir/near-ai" "$(tr '[:lower:]' '[:upper:]' <<<"$key_hex")"
  expect_refused "the staging key in upper-case hex" --keys-file "$scratch/keys.txt" "$case_dir"

  local form
  for form in std std-unpadded url url-unpadded; do
    new_case
    seed_b64_key "$case_dir/near-ai" "$key_hex" "$form"
    expect_refused "the staging key in base64 ($form)" --keys-file "$scratch/keys.txt" "$case_dir"
  done

  # Fail closed on the inputs themselves.
  expect_refused "a missing keys file" --keys-file "$scratch/absent.txt" "$scratch/clean/lib.dylib"
  printf 'not-a-key\n' >"$scratch/bad-keys.txt"
  expect_refused "a keys file with a line that is not a key" \
    --keys-file "$scratch/bad-keys.txt" "$scratch/clean/lib.dylib"
  printf '%s\n' "${key_hex:0:62}" >"$scratch/short-keys.txt"
  expect_refused "a keys file with a short key" \
    --keys-file "$scratch/short-keys.txt" "$scratch/clean/lib.dylib"
  expect_refused "no path to scan" --keys-file "$scratch/keys.txt"
  expect_refused "a path that does not exist" --keys-file "$scratch/keys.txt" "$scratch/absent"
  mkdir -p "$scratch/empty"
  expect_refused "a directory with no file in it" --keys-file "$scratch/keys.txt" "$scratch/empty"

  # The markers must be the strings the verifier compiles in. A rename of the
  # format there, with these markers left behind, would make every release
  # scan pass vacuously while every case above stays green.
  if (sources_agree >/dev/null 2>&1); then
    echo "self-test: passed as expected: the markers match the verifier's source"
  else
    echo "self-test: the markers no longer match the verifier's source" >&2
    (sources_agree 2>&1 | sed 's/^/  /') || true
    status=1
  fi
  local renamed out
  for renamed in \
    's#^pub const FORMAT: &str = "trace-commons-near-ai-pins/#pub const FORMAT: &str = "trace-commons-near-ai-pinset/#m' \
    's#^pub const SIGNING_PREFIX: &\[u8\] = b"trace-commons-near-ai-pins/#pub const SIGNING_PREFIX: &[u8] = b"tc-pins/#m' \
    's#^pub const FORMAT: .*\n##m' \
    's#^pub const FORMAT_V1: &str = "trace-commons-near-ai-pins/#pub const FORMAT_V1: &str = "tc-pins/#m' \
    's#^pub const SIGNING_PREFIX_V1: &\[u8\] = b"trace-commons-near-ai-pins/#pub const SIGNING_PREFIX_V1: &[u8] = b"tc-pins/#m'; do
    cp "$STAGING_PINS_SOURCE" "$scratch/staging_pins.rs"
    perl -0pi -e "$renamed" "$scratch/staging_pins.rs"
    if cmp -s "$STAGING_PINS_SOURCE" "$scratch/staging_pins.rs"; then
      echo "self-test: seeding '$renamed' changed nothing; the fixture drifted" >&2
      status=1
    elif out="$(sources_agree "$scratch/staging_pins.rs" 2>&1)"; then
      echo "self-test: a verifier source with '$renamed' was NOT refused" >&2
      status=1
    elif ! grep -q '^REFUSING: ' <<<"$out"; then
      echo "self-test: '$renamed' failed without a refusal" >&2
      status=1
    else
      echo "self-test: refused as expected: a verifier source whose format drifted from the marker"
    fi
  done

  return "$status"
}

case "${1:-}" in
  check)
    shift
    check "$@"
    ;;
  self-test) self_test ;;
  *)
    echo "usage: $0 {check [--keys-file FILE] PATH...|self-test}" >&2
    exit 2
    ;;
esac
