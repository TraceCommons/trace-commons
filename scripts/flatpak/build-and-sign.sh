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

# Refuse to hand back an unsigned repo. A plain `ostree show "$ref" | grep -qi
# signature` is not a real test: it is a case-insensitive substring match,
# and `ostree show`'s own no-signature and untrusted-key error paths BOTH
# contain the word "signature" ("no signatures found",
# "Can't check signature: public key not found") -- confirmed against a real
# unsigned commit, which matches that grep despite carrying no signature at
# all. Instead check for the presence of the detached ostree.gpgsigs
# metadata key that `build-sign` actually writes: it exits non-zero with
# "No detached metadata for commit ..." when unsigned, and prints the
# signature bytes with exit 0 when signed -- confirmed against both a
# signed and an unsigned commit with this libostree version.
ostree --repo="$REPO" refs | grep '^app/' | while read -r ref; do
  if ! ostree --repo="$REPO" show --print-detached-metadata-key=ostree.gpgsigs "$ref" >/dev/null 2>&1; then
    echo "refusing to publish: $ref carries no signature" >&2
    exit 1
  fi
done

echo "PASS: signed $REPO with $KEYID"
