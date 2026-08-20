#!/usr/bin/env bash
# Build, sign, notarize and staple TraceCommons.app, then package it as a DMG.
#
# This is the release path. `make-app-bundle.sh` remains the development path
# and stays ad-hoc-signed; nothing here changes it.
#
# # Why an unsigned build is not shippable
#
# Gatekeeper blocks a downloaded app that is not signed with a Developer ID and
# notarized. Exactly what the contributor sees varies with the macOS version
# and with how the signature is broken -- an ad-hoc signature typically reads
# as "damaged", an unsigned build as "the developer cannot be verified" -- and
# some of those states can still be bypassed by a user who knows the
# right-click-Open or Open Anyway path. So this is not "the app is literally
# unopenable"; the earlier version of this comment overstated that.
#
# The argument does not need the overstatement. Shipping something whose only
# install route is teaching contributors to click past Gatekeeper is
# indefensible for a background app that reads their coding transcripts:
# that warning is exactly the signal that should stop someone installing a
# tampered build, and training people through it is training them past the
# real thing. Developer ID plus notarization is the requirement.
#
# # Arguments
#
#   $1  SHORT_VERSION  the tag version (e.g. "0.1.2"), required
#   $2  BUILD_VERSION  the build number (e.g. "42"), required
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
#   MACOS_NOTARY_ASC_KEY_P8_BASE64  App Store Connect API key (.p8), base64
#   MACOS_NOTARY_ASC_KEY_ID         that key's id
#   MACOS_NOTARY_ASC_ISSUER_ID      the issuer id for the team
#   TC_SPARKLE_PUBLIC_ED_KEY        base64 EdDSA public key from Sparkle's
#                                   generate_keys. Without it the bundle ships
#                                   no feed URL, and the released app could
#                                   never receive an update -- a failure that
#                                   is invisible in the DMG itself.
#
# notarytool takes an API key rather than an Apple ID and app-specific
# password. This removes two secrets, and closes the window where the old
# approach would have held the password in this process's argv where any
# local process could read it from `ps`.
#
# # Status
#
# STILL NEVER EXECUTED as of this change. This commit alters which credentials
# the script demands; it does not run it. No Developer ID key was available
# when it landed, so nothing here has signed or notarized anything, and a
# script that has never run is not evidence.
#
# What would change that, in order: a real run producing a signed, notarized,
# stapled DMG, and then the clean-machine gate -- open that DMG on a Mac that
# did not build it, with the network off, and confirm it launches with no
# Gatekeeper prompt. Only versions that have passed BOTH may be described as
# verified.
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
           MACOS_SIGNING_IDENTITY MACOS_NOTARY_ASC_KEY_P8_BASE64 \
           MACOS_NOTARY_ASC_KEY_ID MACOS_NOTARY_ASC_ISSUER_ID \
           TC_SPARKLE_PUBLIC_ED_KEY; do
  require_env "$var"
done

SHORT_VERSION="${1:?refusing to build a release without a version -- the caller must pass the tag version explicitly as the first argument.}"
BUILD_VERSION="${2:?refusing to build a release without a build number -- the caller must pass the build number as the second argument.}"

echo "--- building the release bundle"
TC_SKIP_ADHOC_SIGN=1 ./scripts/make-app-bundle.sh \
  "$CONFIG" "$SHORT_VERSION" "$BUILD_VERSION"

# A private scratch directory. RUNNER_TEMP exists only under GitHub Actions,
# and the header advertises this as runnable for a one-off developer release
# too -- under `set -u` that combination aborted before the cleanup trap was
# even installed, leaving nothing to clean up but also doing nothing useful.
WORK="${RUNNER_TEMP:-$(mktemp -d)}"
mkdir -p "$WORK"

# The certificate goes into a throwaway keychain rather than the login
# keychain: on a CI runner there is no login keychain worth touching, and on a
# developer's machine this must not leave a Developer ID key behind in their
# default keychain after a one-off release build.
KEYCHAIN="$WORK/tc-signing.keychain-db"
KEYCHAIN_PASSWORD="$(uuidgen)"

# Capture the search list BEFORE touching it. `security list-keychains -s`
# REPLACES the list rather than adding to it, so setting it without restoring
# would leave a developer's own keychains missing from every later `security`
# call in that login session -- long after this script exited, and with no
# clue as to why.
ORIGINAL_KEYCHAINS="$(security list-keychains -d user | sed -e 's/^[[:space:]]*"//' -e 's/"$//')"

# Installed before the first mutation, so every early failure below still
# restores the search list and removes the keychain and certificate.
cleanup() {
  security delete-keychain "$KEYCHAIN" 2>/dev/null || true
  rm -f "$WORK/cert.p12"
  rm -f "$WORK/notary.p8"
  if [ -n "${ORIGINAL_KEYCHAINS:-}" ]; then
    # shellcheck disable=SC2086
    security list-keychains -d user -s $ORIGINAL_KEYCHAINS || true
  fi
}
trap cleanup EXIT

echo "--- importing the signing certificate into a throwaway keychain"
security create-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
security set-keychain-settings -lut 900 "$KEYCHAIN"
security unlock-keychain -p "$KEYCHAIN_PASSWORD" "$KEYCHAIN"
# Remove any stale file, decode with restricted creation mode, and set mode
# again as belt-and-braces. rm defeats leftover-mode and symlink cases;
# umask guards creation; chmod is what readers grep for and what actually
# enforces the mode if a file somehow existed. The cert holds the Developer ID
# private key for the whole run, so this is the larger exposure of the two.
rm -f "$WORK/cert.p12"
( umask 077; echo "$MACOS_CERTIFICATE_P12_BASE64" | base64 --decode > "$WORK/cert.p12" )
chmod 600 "$WORK/cert.p12"
security import "$WORK/cert.p12" -k "$KEYCHAIN" \
  -P "$MACOS_CERTIFICATE_PASSWORD" -T /usr/bin/codesign
security set-key-partition-list -S apple-tool:,apple:,codesign: \
  -s -k "$KEYCHAIN_PASSWORD" "$KEYCHAIN" >/dev/null
# Append rather than replace, and restore in the trap.
# shellcheck disable=SC2086
security list-keychains -d user -s "$KEYCHAIN" $ORIGINAL_KEYCHAINS

echo "--- signing"
# Nested code is signed before the bundle that contains it: codesign seals
# what is inside, so signing the outer bundle first would be invalidated by
# touching anything inner afterwards.
#
# `--deep` is deliberately absent and must stay absent. It would re-sign
# Sparkle's Downloader XPC service without the entitlement it ships with,
# which is the single most common way a Sparkle integration breaks. Sparkle's
# own documentation gives exactly this ordering.
SPARKLE_FRAMEWORK="$APP/Contents/Frameworks/Sparkle.framework"
if [ ! -d "$SPARKLE_FRAMEWORK" ]; then
  echo "refusing to build a release: Sparkle.framework is not in the bundle." >&2
  echo "make-app-bundle.sh embeds it; a bundle without it builds, signs and" >&2
  echo "notarizes cleanly and then crashes on launch." >&2
  exit 1
fi

codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" \
  "$SPARKLE_FRAMEWORK/Versions/B/XPCServices/Installer.xpc"
# Sparkle >= 2.6 ships Downloader.xpc with its own entitlements; re-signing
# without preserving them removes the network access it needs.
codesign --force --timestamp --options runtime --preserve-metadata=entitlements \
  --sign "$MACOS_SIGNING_IDENTITY" \
  "$SPARKLE_FRAMEWORK/Versions/B/XPCServices/Downloader.xpc"
codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" \
  "$SPARKLE_FRAMEWORK/Versions/B/Autoupdate"
codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" \
  "$SPARKLE_FRAMEWORK/Versions/B/Updater.app"
codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" \
  "$SPARKLE_FRAMEWORK"

# The embedded dylib is signed before the bundle that contains it, for the
# same reason.
find "$APP/Contents/Frameworks" -name '*.dylib' -print0 |
  while IFS= read -r -d '' dylib; do
    codesign --force --timestamp --options runtime \
      --sign "$MACOS_SIGNING_IDENTITY" "$dylib"
  done

# Hardened runtime is required for notarization. There is deliberately no
# entitlements file: this app needs no exception to the hardened runtime, and
# adding entitlements it does not use would widen what a compromised process
# could do for no benefit. Sparkle's updater runs out of process precisely so
# that the app does not need one.
codesign --force --timestamp --options runtime \
  --sign "$MACOS_SIGNING_IDENTITY" "$APP"
codesign --verify --deep --strict --verbose=2 "$APP"

echo "--- packaging the DMG"
rm -f "$DMG"
hdiutil create -volname TraceCommons -srcfolder "$APP" -ov -format UDZO "$DMG"
codesign --force --timestamp --sign "$MACOS_SIGNING_IDENTITY" "$DMG"

echo "--- notarizing (this waits for Apple's verdict)"
# The key is written to the private scratch dir and passed by path, so unlike
# an app-specific password it never appears in this call's argv.
#
# That does NOT mean argv exposure is solved for this script: `security import
# -P "$MACOS_CERTIFICATE_PASSWORD"` above still passes a secret as an argument,
# and neither tool accepts one on stdin. So the standing rules still hold, and
# one of them now matters MORE than before: never enable shell tracing
# (`set -x`) in this script -- with tracing on, the line below would trace the
# entire base64 private key. Run release builds on an isolated ephemeral
# runner.
# Remove any stale file, decode with restricted creation mode, and set mode
# again as belt-and-braces. rm defeats leftover-mode and symlink cases;
# umask guards creation; chmod is what readers grep for and what actually
# enforces the mode if a file somehow existed.
rm -f "$WORK/notary.p8"
( umask 077; echo "$MACOS_NOTARY_ASC_KEY_P8_BASE64" | base64 --decode > "$WORK/notary.p8" )
chmod 600 "$WORK/notary.p8"
# --timeout, because `--wait` alone has no upper bound and a hung connection
# is indistinguishable from a slow queue until the job is killed. On
# 2026-08-20 the 0.4.0 release burned its entire 60-minute budget here: the
# log reached "Conducting pre-submission checks" and never printed a
# submission id, so nothing was ever queued, and the runner terminated
# notarytool as an orphan process at cleanup. The build before it took four
# minutes. Failing at 20 leaves room to retry inside one job rather than
# spending an hour to learn nothing.
# On failure, ask Apple what it thinks happened before giving up. A hang at
# the first round-trip cannot distinguish "never reached the service" from
# "submitted, and the client never reported it" -- and those want opposite
# responses. `history` answers it in one call, with the credentials already
# to hand, which no amount of reading the client's own silence can.
if ! xcrun notarytool submit "$DMG" \
  --key "$WORK/notary.p8" \
  --key-id "$MACOS_NOTARY_ASC_KEY_ID" \
  --issuer "$MACOS_NOTARY_ASC_ISSUER_ID" \
  --timeout 20m \
  --wait
then
  echo "--- notarization failed; asking Apple for this key's recent submissions" >&2
  xcrun notarytool history \
    --key "$WORK/notary.p8" \
    --key-id "$MACOS_NOTARY_ASC_KEY_ID" \
    --issuer "$MACOS_NOTARY_ASC_ISSUER_ID" >&2 || \
    echo "history call also failed: the service is unreachable from this runner" >&2
  exit 1
fi

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
