#!/usr/bin/env bash
# Publish a signed OSTree repo plus its .flatpakref to GCS.
#
# The .flatpakref embeds the public key, so `flatpak install --from <url>`
# verifies against a key the contributor received with the ref rather than
# one fetched separately from the same host. That is a weaker property than
# out-of-band key distribution and worth being plain about: it protects
# against a compromised mirror, not against a compromised origin.
set -euo pipefail

REPO="${1:?usage: publish-repo.sh <repo-dir> <pubkey-file> <bucket>}"
PUBKEY="${2:?usage: publish-repo.sh <repo-dir> <pubkey-file> <bucket>}"
BUCKET="${3:?usage: publish-repo.sh <repo-dir> <pubkey-file> <bucket>}"

# Refuse to publish an unsigned repo. build-and-sign.sh already checks each
# commit's signature, but that script and this one run as separate CI steps
# -- a caller that skips straight to publish-repo.sh (or a future workflow
# edit that reorders the steps) must not be able to serve an unsigned
# summary silently. flatpak build-update-repo --gpg-sign writes a detached
# GPG signature to summary.sig alongside the summary; verify it names an
# actual signature rather than just checking the file exists.
if [ ! -f "$REPO/summary" ] || [ ! -f "$REPO/summary.sig" ]; then
  echo "refusing to publish: $REPO has no signed summary; run build-and-sign.sh first" >&2
  exit 1
fi
if ! gpg --verify "$REPO/summary.sig" "$REPO/summary" >/dev/null 2>&1; then
  echo "refusing to publish: $REPO/summary.sig does not verify against $REPO/summary" >&2
  exit 1
fi

BASE="https://storage.googleapis.com/$BUCKET"

cat > ai.tracecommons.Contributor.flatpakref <<REF
[Flatpak Ref]
Title=Trace Commons
Name=ai.tracecommons.Contributor
Branch=master
Url=$BASE/repo
IsRuntime=false
GPGKey=$(base64 < "$PUBKEY" | tr -d '\n')
RuntimeRepo=https://dl.flathub.org/repo/flathub.flatpakrepo
REF

gcloud storage rsync --recursive --delete-unmatched-destination-objects \
  "$REPO" "gs://$BUCKET/repo"
gcloud storage cp ai.tracecommons.Contributor.flatpakref "gs://$BUCKET/"
gcloud storage cp "$PUBKEY" "gs://$BUCKET/tracecommons-flatpak.gpg"

echo "PASS: published to $BASE"
echo "install with: flatpak install --from $BASE/ai.tracecommons.Contributor.flatpakref"
