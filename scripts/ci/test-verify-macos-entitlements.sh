#!/usr/bin/env bash
# Exercise verify-macos-entitlements.sh against fixture bundles, on every PR.
#
# The real script only ever runs in the release job, against a Developer ID
# build, so a bug in the script itself -- a syntax error, a reserved variable
# name, a wrong path -- surfaces for the first time on the day of a release.
# This builds two ad-hoc-signed bundles around a copy of /usr/bin/true:
#
#   good: carries macos/entitlements.plist and the committed profile. The
#         static checks must all pass.
#   bare: signed with no entitlements. The script must refuse it.
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
  local app="$1"
  mkdir -p "$app/Contents/MacOS"
  cp /usr/bin/true "$app/Contents/MacOS/TraceCommonsApp"
  cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleExecutable</key><string>TraceCommonsApp</string>
  <key>CFBundleIdentifier</key><string>ai.tracecommons.shell</string>
  <key>CFBundlePackageType</key><string>APPL</string>
</dict>
</plist>
PLIST
  cp "$ROOT/macos/TraceCommons-DeveloperID.provisionprofile" \
    "$app/Contents/embedded.provisionprofile"
}

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

echo "PASS: verify-macos-entitlements.sh passes the entitled fixture and refuses the bare one"
