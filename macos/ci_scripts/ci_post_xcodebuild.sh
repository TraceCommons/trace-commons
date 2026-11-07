#!/bin/bash
set -euo pipefail
if [[ "${CI_XCODEBUILD_ACTION:-}" != archive ]]; then
  echo 'No archive verification for this Xcode action.'
  exit 0
fi
: "${CI_ARCHIVE_PATH:?archive action requires CI_ARCHIVE_PATH}"
REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
/usr/bin/python3 "$REPO_ROOT/macos/ci_scripts/check-pilot-archive.py" "$CI_ARCHIVE_PATH" "$REPO_ROOT"
