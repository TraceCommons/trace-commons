#!/usr/bin/env bash
# Profile generation, channel restrictions and plist contract without a Rust build.
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
python3 -m unittest discover -s "$root/scripts/feature-flags" -p 'test_*.py'
for channel in production staging; do
  TC_CHANNEL="$channel" TC_FEATURE_PROFILE="$channel" bash "$root/macos/scripts/info-plist.sh" 0.0.0-test 1 >/dev/null
done
TC_CHANNEL=staging TC_FEATURE_PROFILE=testing bash "$root/macos/scripts/info-plist.sh" 0.0.0-test 1 >/dev/null
if TC_CHANNEL=production TC_FEATURE_PROFILE=testing bash "$root/macos/scripts/info-plist.sh" 0.0.0-test 1 >/dev/null 2>&1; then
  echo 'FAIL: production accepted testing profile' >&2
  exit 1
fi
