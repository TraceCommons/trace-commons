#!/usr/bin/env bash
# Sign a built flatpak OSTree repo.
#
# Both the commit and the repo summary are signed. Signing only the commit
# would leave the summary -- the index a client reads to discover what
# versions exist -- unsigned, so whoever serves the repo could still roll a
# contributor back to an older build or hide an update.
set -euo pipefail

REPO="${1:?usage: build-and-sign.sh <repo-dir> <gpg-key-id>}"
KEYID="${2:?usage: build-and-sign.sh <repo-dir> <gpg-key-id>}"

flatpak build-sign "$REPO" --gpg-sign="$KEYID"
flatpak build-update-repo "$REPO" \
  --gpg-sign="$KEYID" \
  --generate-static-deltas \
  --prune

# Refuse to hand back an unsigned repo: `ostree show` must report a signature.
ostree --repo="$REPO" refs | grep '^app/' | while read -r ref; do
  if ! ostree --repo="$REPO" show "$ref" | grep -qi 'signature'; then
    echo "refusing to publish: $ref carries no signature" >&2
    exit 1
  fi
done

echo "PASS: signed $REPO with $KEYID"
