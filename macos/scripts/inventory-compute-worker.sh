#!/usr/bin/env bash
# Read-only inventory of explicit local build artifacts. Does not execute the
# worker, select a release, sign, stage, or authorize compute. Run with bash.
set -euo pipefail
worker="${1:?absolute worker path required}"
metallib="${2:?absolute mlx.metallib path required}"
for artifact in "$worker" "$metallib"; do
  case "$artifact" in /*) ;; *) echo 'artifact-path-invalid' >&2; exit 1 ;; esac
  if [ ! -f "$artifact" ] || [ -L "$artifact" ]; then
    echo 'artifact-file-invalid' >&2; exit 1
  fi
done
if [ "$(/usr/bin/lipo -archs "$worker")" != arm64 ]; then
  echo 'worker-architecture-invalid' >&2; exit 1
fi
if [ "$(/usr/bin/od -An -tx1 -N4 "$metallib" | /usr/bin/tr -d ' \n')" != 4d544c42 ]; then
  echo 'metal-library-magic-invalid' >&2; exit 1
fi
echo 'worker-sha256'
/usr/bin/shasum -a 256 "$worker" | /usr/bin/awk '{print $1}'
echo 'worker-size-bytes'
/usr/bin/stat -f '%z' "$worker"
echo 'metal-library-sha256'
/usr/bin/shasum -a 256 "$metallib" | /usr/bin/awk '{print $1}'
echo 'metal-library-size-bytes'
/usr/bin/stat -f '%z' "$metallib"
echo 'worker-build-load-commands'
/usr/bin/xcrun vtool -show-build "$worker" | /usr/bin/sed '1d'
echo 'worker-linked-libraries'
libraries="$(/usr/bin/otool -L "$worker" | /usr/bin/sed '1d')"
echo "$libraries"
# This inventory only qualifies a system-library-only artifact. If MLX starts
# linking other native libraries, require an explicit signed library inventory.
if ! echo "$libraries" | /usr/bin/awk '
  NF && $1 !~ /^\/System\/Library\// && $1 !~ /^\/usr\/lib\// { bad=1 }
  END { exit bad }
'; then
  echo 'worker-non-system-library-requires-review' >&2; exit 1
fi
echo 'inventory-only-not-signature-or-runtime-verification'
