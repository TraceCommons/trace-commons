#!/usr/bin/env bash
# Build, sign, notarize and staple TraceCommons.app, then package it as a DMG.
#
# This is the release path. `make-app-bundle.sh` remains the development path
# and stays ad-hoc-signed; nothing here changes it.
#
# # Why an unsigned build is not shippable
#
# Gatekeeper refuses a downloaded app that is not signed with a Developer ID
# and notarized. The contributor would see "TraceCommons.app is damaged and
# can't be opened" -- which is not a warning they can click through, and it is
# indistinguishable from an actual tampered download. A background app that
# reads coding transcripts is precisely the kind of thing a user should refuse
# to run when macOS says that, so shipping unsigned is not an option.
#
# # Credentials
#
# Every value below comes from the environment. There are no defaults, and
# the script refuses rather than falling back to an ad-hoc signature -- an
# unsigned artifact named like a release is worse than no artifact, because
# somebody will eventually try to ship it.
#
#   MACOS_CERTIFICATE_P12_BASE64  Developer ID Application cert + key, as a
#                                 base64 .p12
#   MACOS_CERTIFICATE_PASSWORD    password for that .p12
#   MACOS_SIGNING_IDENTITY        e.g. "Developer ID Application: Example
#                                 Inc (TEAMID)"
#   MACOS_NOTARY_APPLE_ID         Apple ID for notarytool
#   MACOS_NOTARY_PASSWORD         app-specific password for that Apple ID
#   MACOS_NOTARY_TEAM_ID          the team ID notarization is filed under
#
# # Status
#
# NOT YET EXECUTED. No Developer ID certificate exists for this project, so
# this script has never signed or notarized anything. It is written so the
# release path is reviewable and ready the moment a certificate is
# provisioned -- but until it has produced a stapled DMG that opens cleanly
# on a machine that did not build it, treat notarization as unverified. The
# same rule the Windows pipe ACL was held to applies here: a script that has
# never run is not evidence.
set -euo pipefail

cd "$(dirname "$0")/.."
PACKAGE_DIR="$PWD"
CONFIG=release
APP="$PACKAGE_DIR/.build/TraceCommons.app"
DMG="$PACKAGE_DIR/.build/TraceCommons.dmg"

require_env() {
  local name="$1"
  if [ -z "${!name:-}" ]; then
    echo "refusing to build a release: $name is not set." >&2
    echo "An unsigned or ad-hoc-signed build cannot be shipped -- Gatekeeper" >&2
    echo "rejects it on the contributor's machine. See the header of this" >&2
    echo "script for the full credential list." >&2
    exit 1
  fi
}

for var in MACOS_CERTIFICATE_P12_BASE64 MACOS_CERTIFICATE_PASSWORD \
           MACOS_SIGNING_IDENTITY MACOS_NOTARY_APPLE_ID \
           MACOS_NOTARY_PASSWORD MACOS_NOTARY_TEAM_ID; do
  require_env "$var"
done

echo "--- building the release bundle"
./scripts/make-app-bundle.sh "$CONFIG"

# The certificate goes into a throwaway keychain rather than the login
# keychain: on a CI runner there is no login keychain worth touching, and on a
# developer's machine this must not leave a Developer ID key behind in their
# default keychain after a one-off release build.
KEYCHAIN="$RUNNER_TEMP/tc-signing.keychain-db"
KEYCHAIN_PASSWORD="$(uuidgen)"
cleanup() {
  security delete-keychain "$KEYCHAIN" 2>/dev/null || true
  rm -f "$RUNNER_TEMP/cert.p12"
}
trap cleanup EXIT

echo "--- importing the signing certificate into a throwaway keychain"
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 900 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
echo "$MACOS_CERTIFICATE_P12_BASE64" | base64 --decode > "$RUNNER_TEMP/cert.p12"
security import "$RUNNER_TEMP/cert.p12" -k "$KEYCHAIN" \
  -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple:,codesign: \
  -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
security list-keychains -d user -s "$KEYCHAIN" login.keychain-db

echo "--- signing"
# The embedded dylib is signed before the bundle that contains it: codesign
# seals nested code, so signing the outer bundle first would be invalidated
# by touching the inner one afterwards.
find "$APP/Contents/Frameworks" -name '*.dylib' -print0 |
  while IFS= read -r -d '' dylib; do
    codesign --force --timestamp --options runtime \
      --sign "$MACOS_SIGNING_IDENTITY" "$dylib"
  done

# Hardened runtime is required for notarization. There is deliberately no
# entitlements file: this app needs no exception to the hardened runtime, and
# adding entitlements it does not use would widen what a compromised process
# could do for no benefit.
codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

echo "--- packaging the DMG"
rm -f "$DMG"
hdiutil create -volname TraceCommons -srcfolder "$APP" -ov -format UDZO "$DMG"
codesign --force --timestamp --sign "$MACOS_SIGNING_IDENTITY" "$DMG"

echo "--- notarizing (this waits for Apple's verdict)"
xcrun notarytool submit "$DMG" \
  --apple-id "$MACOS_NOTARY_APPLE_ID" \
  --password "$MACOS_NOTARY_PASSWORD" \
  --team-id "$MACOS_NOTARY_TEAM_ID" \
  --wait

echo "--- stapling"
# Stapling is what makes the DMG open without a network round trip to Apple.
# Skipping it produces an artifact that works on the build machine and fails
# for a user who is offline or behind a filtering proxy.
xcrun stapler staple "$DMG"
xcrun stapler validate "$DMG"

echo "--- verifying Gatekeeper accepts it"
# The real check: what Gatekeeper says, not what codesign says. `spctl`
# assessment is the same evaluation the user's machine performs.
spctl --assess --type open --context context:primary-signature --verbose=2 "$DMG"

echo
echo "PASS: signed, notarized and stapled at $DMG"
echo "This still needs one manual confirmation before any release is called"
echo "good: open the DMG on a Mac that did not build it, with the network"
echo "off, and confirm it launches without a Gatekeeper prompt."
