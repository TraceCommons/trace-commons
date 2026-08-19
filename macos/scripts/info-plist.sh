#!/usr/bin/env bash
# Print the Info.plist for TraceCommons.app.
#
# Extracted from make-app-bundle.sh so the version can be injected from a
# release tag and asserted in a test without a Swift toolchain. The old
# heredoc hardcoded CFBundleShortVersionString to 0.1.0, which meant any
# tagged release would have shipped a DMG claiming 0.1.0 -- and Homebrew
# compares a cask's declared version against what is installed, so that also
# broke `brew upgrade`.
#
# # Sparkle
#
# The updater's configuration lives here rather than in Swift because Sparkle
# reads it from the bundle before any of our code runs.
#
# TC_SPARKLE_PUBLIC_ED_KEY is the base64 EdDSA public key from Sparkle's
# generate_keys. When it is unset -- the development case -- this script
# emits NO feed URL, NO key, and automatic checks off. That is deliberate and
# it is the fail-closed direction: a bundle that carried a feed without a key
# would be asking Sparkle to fetch an appcast it cannot authenticate. A
# release cannot reach that state because make-release-dmg.sh refuses to run
# without the key.
set -euo pipefail

SHORT_VERSION="${1:?usage: info-plist.sh <short_version> <build_version>}"
BUILD_VERSION="${2:?usage: info-plist.sh <short_version> <build_version>}"

# The published appcast, written by scripts/updates/generate-appcast.sh into
# the same public bucket as the flatpak repo. HTTPS is not decoration here:
# the appcast is what authorizes an install.
SPARKLE_FEED_URL="https://storage.googleapis.com/tracecommons-flatpak/updates/appcast.xml"

SPARKLE_KEYS=""
if [ -n "${TC_SPARKLE_PUBLIC_ED_KEY:-}" ]; then
  SPARKLE_KEYS="$(cat <<KEYS
    <key>SUFeedURL</key><string>${SPARKLE_FEED_URL}</string>
    <key>SUPublicEDKey</key><string>${TC_SPARKLE_PUBLIC_ED_KEY}</string>
    <!-- Check automatically, never install automatically. Sparkle looks for
         an update on launch and once a day thereafter without asking
         permission to look; the bytes on disk do not change until a person
         says yes. -->
    <key>SUEnableAutomaticChecks</key><true/>
    <key>SUAutomaticallyUpdate</key><false/>
    <key>SUScheduledCheckInterval</key><integer>86400</integer>
KEYS
)"
else
  SPARKLE_KEYS="    <key>SUEnableAutomaticChecks</key><false/>"
fi

cat <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key><string>Trace Commons</string>
    <key>CFBundleDisplayName</key><string>Trace Commons</string>
    <key>CFBundleIdentifier</key><string>ai.tracecommons.shell</string>
    <key>CFBundleExecutable</key><string>TraceCommonsApp</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>${SHORT_VERSION}</string>
    <key>CFBundleVersion</key><string>${BUILD_VERSION}</string>
    <key>LSMinimumSystemVersion</key><string>14.0</string>
    <key>NSHumanReadableCopyright</key><string>Trace Commons</string>
    <!-- There is no LSUIElement here, and its absence is the decision.

         This app was a menu-bar-only utility until a contributor could not
         find it: on a display with a notch, once the menu bar fills up, the
         status item is still assigned a frame -- the accessibility API
         answers with a plausible 18x24 rectangle -- but it is placed past
         the notch in a band that never draws. Nothing renders, nothing
         reports an error, and the app has no other way in. A menu-bar-only
         app is one crowded menu bar away from being unreachable.

         So the app is a regular one: Dock icon, App menu, Cmd-Tab. The
         MenuBarExtra stays, because it is still the right place for
         at-a-glance status; it is simply no longer the only door. -->
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <!-- Invite mail carries a tracecommons:// link to contributors on all
         three platforms. onOpenURL has been wired in the Connect screen
         since before the first release, but nothing was ever declared
         here, so every link macOS received went nowhere. -->
    <key>CFBundleURLTypes</key>
    <array>
      <dict>
        <key>CFBundleURLName</key><string>ai.tracecommons.shell.invite</string>
        <key>CFBundleURLSchemes</key><array><string>tracecommons</string></array>
        <!-- Viewer, not Editor: the app shows what the URL names and waits
             for a person to press the button. A deep link never enrols. -->
        <key>CFBundleTypeRole</key><string>Viewer</string>
      </dict>
    </array>
${SPARKLE_KEYS}
</dict>
</plist>
PLIST
