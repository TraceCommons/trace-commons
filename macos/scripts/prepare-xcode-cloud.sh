#!/bin/bash
# Private pilot preparation only. No upload, install, signing, or publishing.
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
MODE="${1:---release-universal}"
case "$MODE" in --metadata-only|--debug-native|--release-universal) ;; *) echo 'refusing: unknown preparation mode' >&2; exit 1;; esac
export TC_CHANNEL="${TC_CHANNEL:-production}"
[[ "$TC_CHANNEL" == production ]] || { echo 'refusing: pilot requires production channel' >&2; exit 1; }
if [[ "$MODE" == --release-universal ]] && ! git -C "$REPO_ROOT" diff --quiet HEAD --; then
  echo 'refusing: release pilot requires a clean tracked source checkout' >&2
  exit 1
fi
export RUSTFLAGS='-D warnings' CARGO_INCREMENTAL=0
# Rust 1.96's default release debug stripping can leave LC_SYMTAB.stroff
# four-byte aligned; Xcode 27 rejects these dylibs. Preserve the linker's
# valid layout without changing Rust flags or the pinned toolchain.
export CARGO_PROFILE_RELEASE_STRIP=none
export TC_PILOT_MODE="$MODE" TC_PILOT_ROOT="$REPO_ROOT"
/usr/bin/python3 <<'PY'
import base64, os, re
try:
    key = base64.b64decode(os.environ.get('TC_SPARKLE_PUBLIC_ED_KEY', ''), validate=True)
except ValueError:
    raise SystemExit('refusing: invalid Sparkle public key')
if len(key) != 32:
    raise SystemExit('refusing: Sparkle public key must decode to 32 bytes')
if not re.fullmatch(r'[0-9a-fA-F]{64}', os.environ.get('TRACE_COMMONS_UPDATE_PUBLIC_KEY_HEX', '')):
    raise SystemExit('refusing: CLI update public key must be 32 bytes of hex')
if not re.fullmatch(r'[1-9][0-9]{0,17}', os.environ.get('CI_BUILD_NUMBER', '')):
    raise SystemExit('refusing: CI_BUILD_NUMBER must be a positive integer of at most 18 digits')
PY
GENERATED="$REPO_ROOT/macos/.xcode-cloud/generated"
mkdir -p "$GENERATED/en.lproj"
VERSION="$(awk '/^\[package\]/{package=1;next} /^\[/{package=0} package && /^version = "[0-9]+\.[0-9]+\.[0-9]+"$/{gsub(/"/, "", $3); print $3; exit}' "$REPO_ROOT/crates/trace-commons-contributor/Cargo.toml")"
[[ -n "$VERSION" ]] || { echo 'refusing: no contributor release version' >&2; exit 1; }
"$REPO_ROOT/macos/scripts/info-plist.sh" "$VERSION" "$CI_BUILD_NUMBER" > "$GENERATED/Info.plist"
"$REPO_ROOT/macos/scripts/info-plist-strings.sh" > "$GENERATED/en.lproj/InfoPlist.strings"
export TC_PILOT_VERSION="$VERSION"
TRACE_COMMONS_BUILD_COMMIT="$(git -C "$REPO_ROOT" rev-parse HEAD)"
export TRACE_COMMONS_BUILD_COMMIT
/usr/bin/python3 <<'PY'
import hashlib, json, os, pathlib, subprocess
root = pathlib.Path(os.environ['TC_PILOT_ROOT'])
commit = subprocess.check_output(['git', '-C', str(root), 'rev-parse', 'HEAD'], text=True).strip()
expected = os.environ.get('CI_COMMIT')
if expected and expected != commit:
    raise SystemExit('refusing: checkout does not match CI_COMMIT')
mode = os.environ['TC_PILOT_MODE'][2:]
metadata = dict(schemaVersion=1, purpose='private-xcode-cloud-pilot', mode=mode,
               sourceCommit=commit, channel='production', shortVersion=os.environ['TC_PILOT_VERSION'],
               buildVersion=os.environ['CI_BUILD_NUMBER'], rustToolchain='1.96.0',
               architectures=['arm64', 'x86_64'] if mode != 'debug-native' else [os.uname().machine],
               sparklePublicKeySHA256=hashlib.sha256(os.environ['TC_SPARKLE_PUBLIC_ED_KEY'].encode()).hexdigest(),
               cliUpdatePublicKeySHA256=hashlib.sha256(bytes.fromhex(os.environ['TRACE_COMMONS_UPDATE_PUBLIC_KEY_HEX'])).hexdigest())
(root / 'macos/.xcode-cloud/generated/pilot-build.json').write_text(json.dumps(metadata, sort_keys=True, indent=2) + '\n')
PY
if [[ "$MODE" == --metadata-only ]]; then
  echo 'Prepared pilot metadata only; no Rust binaries built.'
  exit 0
fi
export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
FFI_DIR="$REPO_ROOT/macos/.xcode-cloud/ffi-universal"
mkdir -p "$FFI_DIR"
DYLIB=libtrace_commons_contributor_ffi.dylib
cd "$REPO_ROOT"
if [[ "$MODE" == --debug-native ]]; then
  # Explicit local-only diagnostic mode; Cloud hooks never select this.
  cargo +1.96.0 build --locked -p trace-commons-contributor-ffi
  cargo +1.96.0 build --locked -p trace-commons-contributor --bin near-ai
  cp "${CARGO_TARGET_DIR:-target}/debug/$DYLIB" "$FFI_DIR/$DYLIB"
  cp "${CARGO_TARGET_DIR:-target}/debug/near-ai" "$FFI_DIR/near-ai"
else
  for target in aarch64-apple-darwin x86_64-apple-darwin; do
    cargo +1.96.0 build --locked --release --target "$target" -p trace-commons-contributor-ffi
    cargo +1.96.0 build --locked --release --target "$target" -p trace-commons-contributor --bin near-ai
  done
  for binary in "$DYLIB" near-ai; do
    lipo -create "${CARGO_TARGET_DIR:-target}/aarch64-apple-darwin/release/$binary" "${CARGO_TARGET_DIR:-target}/x86_64-apple-darwin/release/$binary" -output "$FFI_DIR/$binary"
    arches="$(lipo -archs "$FFI_DIR/$binary")"
    [[ "$arches" == 'x86_64 arm64' || "$arches" == 'arm64 x86_64' ]] || { echo 'refusing: expected exactly two release architectures' >&2; exit 1; }
  done
fi
install_name_tool -id "@rpath/$DYLIB" "$FFI_DIR/$DYLIB"
# install_name_tool invalidates the Rust ad-hoc signature. Seed hardened
# runtime on both binaries before native CodeSignOnCopy; Xcode's managed
# re-signing must preserve these flags for the notarized distribution.
codesign --force --sign - --options runtime --timestamp=none "$FFI_DIR/$DYLIB"
codesign --force --sign - --options runtime --timestamp=none "$FFI_DIR/near-ai"
/usr/bin/python3 "$REPO_ROOT/macos/ci_scripts/check-pilot-archive.py" --check-linkedit "$FFI_DIR/$DYLIB" "$FFI_DIR/near-ai"
/usr/bin/python3 "$REPO_ROOT/macos/ci_scripts/check-pilot-archive.py" --check-runtime "$FFI_DIR/$DYLIB" "$FFI_DIR/near-ai"
echo 'Prepared private pilot native build inputs.'
