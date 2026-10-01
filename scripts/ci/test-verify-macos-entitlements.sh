#!/usr/bin/env bash
# Exercise verify-macos-entitlements.sh against fixture bundles, on every PR.
#
# The real script only ever runs in the release job, against a Developer ID
# build, so a bug in the script itself -- a syntax error, a reserved variable
# name, a wrong path -- surfaces for the first time on the day of a release.
# This builds three ad-hoc-signed bundles around a copy of /usr/bin/true:
#
#   good: carries macos/entitlements.plist and the committed profile. The
#         static checks must all pass.
#   bare: signed with no entitlements. The script must refuse it.
#   desktop: the Tauri app's bundle id and entitlements, carrying the native
#         shell's profile. The script must refuse it for the App ID mismatch.
#
# It also asserts both apps' entitlements request the same access group.
#
# The launch check is skipped (TC_VERIFY_STATIC_ONLY=1): an ad-hoc signature
# cannot be granted a keychain access group, so the launch half is only
# meaningful on the release runner. Everything before it runs as released.
#
# Also asserts make-release-dmg.sh does not name the profile or entitlements
# with a macos/ prefix: it cds into macos/ first, so such a path resolves to
# macos/macos/ and the release aborts.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
VERIFY="$ROOT/scripts/ci/verify-macos-entitlements.sh"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

bash -n "$VERIFY"

if grep -nE '(^|[^A-Za-z_/])macos/(TraceCommons-DeveloperID\.provisionprofile|entitlements\.plist)' \
  "$ROOT/macos/scripts/make-release-dmg.sh"; then
  echo "FAIL: make-release-dmg.sh runs from macos/; name these via \$PACKAGE_DIR"
  exit 1
fi

make_bundle() {
  local app="$1" bundle_id="${2:-ai.tracecommons.shell}" executable="${3:-TraceCommonsApp}"
  mkdir -p "$app/Contents/MacOS"
  cp /usr/bin/true "$app/Contents/MacOS/$executable"
  cat > "$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>$executable</string>
  <key>CFBundleIdentifier</key><string>$bundle_id</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict>
</plist>
PLIST
  cp "$ROOT/macos/TraceCommons-DeveloperID.provisionprofile" \
    "$app/Contents/embedded.provisionprofile"
}

# Both apps must request the same access group: the contributor crate's
# ACCESS_GROUP is one constant, and a credential the native shell wrote has
# to be readable by the Tauri app, and the reverse.
echo "=== both apps request the same keychain access group"
NATIVE_GROUPS="$(plutil -extract keychain-access-groups json -o - "$ROOT/macos/entitlements.plist")"
TAURI_GROUPS="$(plutil -extract keychain-access-groups json -o - "$ROOT/tauri-desktop/src-tauri/entitlements.plist")"
echo "native: $NATIVE_GROUPS"
echo "tauri:  $TAURI_GROUPS"
test "$NATIVE_GROUPS" = "$TAURI_GROUPS" \
  || { echo "FAIL: macos/ and tauri-desktop/ entitlements request different access groups"; exit 1; }

GOOD="$WORK/good/TraceCommons.app"
make_bundle "$GOOD"
codesign --force --sign - --entitlements "$ROOT/macos/entitlements.plist" "$GOOD"

BARE="$WORK/bare/TraceCommons.app"
make_bundle "$BARE"
codesign --force --sign - "$BARE"

# The profile's expiry check is real; a profile inside 180 days fails here
# too, which is the point -- it would fail the release the same way.
echo "=== fixture: entitled bundle"
TC_VERIFY_STATIC_ONLY=1 bash "$VERIFY" "$GOOD"
# And under the system bash 3.2, which is where a reserved-name assignment
# (GROUPS) killed the script with exit 1 and no message at all.
echo "=== fixture: entitled bundle, /bin/bash"
TC_VERIFY_STATIC_ONLY=1 /bin/bash "$VERIFY" "$GOOD"

echo "=== fixture: unentitled bundle must be refused"
if OUT="$(TC_VERIFY_STATIC_ONLY=1 bash "$VERIFY" "$BARE" 2>&1)"; then
  echo "$OUT"
  echo "FAIL: the script passed a bundle signed without the entitlement"
  exit 1
fi
echo "$OUT"
printf '%s' "$OUT" | grep -q '^FAIL: ' \
  || { echo "FAIL: the refusal did not say why"; exit 1; }

# The Tauri app's bundle id with the native shell's profile. Every group and
# expiry check passes -- both profiles grant KXSWJN7WY8.* -- yet the kernel
# would kill it at exec, because the profile names another App ID. There is
# no positive Tauri fixture: that needs the desktop app's own profile, which
# only the Apple Developer account can issue.
DESKTOP="$WORK/desktop/Trace Commons.app"
make_bundle "$DESKTOP" ai.tracecommons.desktop trace-commons-tauri-desktop
codesign --force --sign - \
  --entitlements "$ROOT/tauri-desktop/src-tauri/entitlements.plist" "$DESKTOP"
echo "=== fixture: desktop bundle carrying the native profile must be refused"
if OUT="$(TC_VERIFY_STATIC_ONLY=1 bash "$VERIFY" "$DESKTOP" 2>&1)"; then
  echo "$OUT"
  echo "FAIL: the script passed a desktop bundle carrying the native shell's profile"
  exit 1
fi
echo "$OUT"
printf '%s' "$OUT" \
  | grep -qx 'FAIL: embedded profile is for KXSWJN7WY8.ai.tracecommons.shell, not KXSWJN7WY8.ai.tracecommons.desktop' \
  || { echo "FAIL: the refusal was not the profile/App ID mismatch"; exit 1; }

echo "PASS: verify-macos-entitlements.sh passes the entitled fixture and refuses the bare and mismatched ones"
