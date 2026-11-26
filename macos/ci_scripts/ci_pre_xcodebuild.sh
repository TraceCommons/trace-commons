#!/bin/bash
set -euo pipefail
"$(cd "$(dirname "$0")/.." && pwd)/scripts/prepare-xcode-cloud.sh" --release-universal
