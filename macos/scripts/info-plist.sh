#!/usr/bin/env bash
# Print the Info.plist for TraceCommons.app.
#
# Extracted from make-app-bundle.sh so the version can be injected from a
# release tag and asserted in a test without a Swift toolchain. The old
# heredoc hardcoded CFBundleShortVersionString to 0.1.0, which meant any
# tagged release would have shipped a DMG claiming 0.1.0 -- and Homebrew
# compares a cask's declared version against what is installed, so that also
# broke `brew upgrade`.
set -euo pipefail

SHORT_VERSION="${1:?usage: info-plist.sh <short_version> <build_version>}"
BUILD_VERSION="${2:?usage: info-plist.sh <short_version> <build_version>}"

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
    <!-- Menu-bar item, no Dock icon: the shape macOS users expect from a
         background utility. -->
    <key>LSUIElement</key><true/>
</dict>
</plist>
PLIST
