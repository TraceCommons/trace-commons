#!/usr/bin/env bash
# Print Contents/Resources/en.lproj/InfoPlist.strings for the channel's
# bundle: the name Finder, Launchpad, the app switcher and the app menu show.
#
# The bundle keeps CFBundleName (TC_BUNDLE_NAME) as the name installs carry
# on disk, and Info.plist sets LSHasLocalizedDisplayName, so macOS shows
# TC_DISPLAY_NAME instead. channel.sh limits both names to letters, digits,
# dots and spaces, so nothing here needs escaping.
#
#   TC_CHANNEL=staging scripts/info-plist-strings.sh

set -euo pipefail

# shellcheck source=channel.sh
. "$(dirname "$0")/channel.sh"
load_channel

printf '"CFBundleDisplayName" = "%s";\n"CFBundleName" = "%s";\n' \
  "$TC_DISPLAY_NAME" "$TC_DISPLAY_NAME"
