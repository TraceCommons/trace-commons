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
# summary silently.
#
# summary.sig is NOT a bare detached OpenPGP signature over the summary
# file, even though the name suggests it: OSTree writes it as a GVariant of
# type a{sv} whose "ostree.gpgsigs" key holds an array of detached
# signatures. `gpg --verify summary.sig summary` therefore always fails
# with "no valid OpenPGP data found", regardless of whether the repo is
# actually signed -- confirmed against a real signed repo built with this
# same libostree version. That check would have aborted every publish.
#
# The genuine verification is the one an OSTree/flatpak client performs:
# import the public key into a throwaway repo, add this repo as a remote
# with gpg-verify-summary enabled, and pull. This exercises OSTree's own
# summary-signature verification path -- confirmed to succeed against a
# validly signed summary and to fail with "BAD signature" against a
# tampered one.
if [ ! -f "$REPO/summary" ] || [ ! -f "$REPO/summary.sig" ]; then
  echo "refusing to publish: $REPO has no signed summary; run build-and-sign.sh first" >&2
  exit 1
fi

VERIFY_REPO="$(mktemp -d)"
trap 'rm -rf "$VERIFY_REPO"' EXIT
ostree --repo="$VERIFY_REPO" init --mode=bare-user-only
ostree --repo="$VERIFY_REPO" remote add --no-gpg-verify --if-not-exists verify-source "file://$REPO"
ostree --repo="$VERIFY_REPO" remote gpg-import verify-source -k "$PUBKEY" >/dev/null
ostree --repo="$VERIFY_REPO" remote add --force --set=gpg-verify-summary=true verify-source "file://$REPO"

REFS="$(ostree --repo="$REPO" refs | grep '^app/' || true)"
if [ -z "$REFS" ]; then
  echo "refusing to publish: $REPO has no app/ refs to verify" >&2
  exit 1
fi
while IFS= read -r ref; do
  if ! ostree --repo="$VERIFY_REPO" pull verify-source "$ref" >/dev/null 2>&1; then
    echo "refusing to publish: summary signature for $REPO did not verify against $PUBKEY (ref $ref)" >&2
    exit 1
  fi
done <<<"$REFS"
rm -rf "$VERIFY_REPO"
trap - EXIT

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
