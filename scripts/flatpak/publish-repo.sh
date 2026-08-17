#!/usr/bin/env bash
# Publish a signed OSTree repo plus its .flatpakref to GCS.
#
# The .flatpakref embeds the public key, so `flatpak install --from <url>`
# verifies against a key the contributor received with the ref rather than
# one fetched separately from the same host. That is a weaker property than
# out-of-band key distribution and worth being plain about: it protects
# against a compromised mirror, not against a compromised origin.
set -euo pipefail

# --verify-only runs the signature gate and stops before uploading anything.
# The gate is a CHECK, not a publication: it should run on every build so a
# broken signature is caught by a dispatch rather than discovered on a tag.
# Only the upload is release-only.
VERIFY_ONLY=0
if [ "${1:-}" = "--verify-only" ]; then VERIFY_ONLY=1; shift; fi

REPO="${1:?usage: publish-repo.sh [--verify-only] <repo-dir> <pubkey-file> <bucket>}"
PUBKEY="${2:?usage: publish-repo.sh [--verify-only] <repo-dir> <pubkey-file> <bucket>}"
BUCKET="${3:-unused-when-verify-only}"
if [ "$VERIFY_ONLY" = 0 ] && [ "$BUCKET" = "unused-when-verify-only" ]; then
  echo "usage: publish-repo.sh [--verify-only] <repo-dir> <pubkey-file> <bucket>" >&2
  exit 2
fi

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
# --mode=archive, not bare-user-only: a bare-user-only repo cannot store xattrs
# or setuid bits, so legitimate app content would abort the pull and get blamed
# on the signature.
ostree --repo="$VERIFY_REPO" init --mode=archive
# The remote is added ONCE, with verification on from the start, and the key is
# imported afterwards. An earlier version added it twice -- once with
# --no-gpg-verify, then again with --force to turn verification on -- and
# imported the key in between, which depended on --force preserving the
# keyring. Do not reintroduce that ordering; it is undocumented behaviour to
# rely on and it is not needed.
# file:// needs an ABSOLUTE path. `file://flatpak-repo` parses "flatpak-repo"
# as a HOSTNAME with an empty path, and ostree then fails with
# "opening repo: opendir((null)): Bad address" -- which the old swallowed-stderr
# gate reported as "the summary signature did not verify", sending two release
# cycles chasing a signature problem that did not exist.
REPO_ABS="$(cd "$REPO" && pwd)"
ostree --repo="$VERIFY_REPO" remote add --set=gpg-verify-summary=true verify-source "file://$REPO_ABS"
ostree --repo="$VERIFY_REPO" remote gpg-import verify-source -k "$PUBKEY" >/dev/null

REFS="$(ostree --repo="$REPO" refs | grep '^app/' || true)"
if [ -z "$REFS" ]; then
  echo "refusing to publish: $REPO has no app/ refs to verify" >&2
  exit 1
fi
while IFS= read -r ref; do
  # Capture and PRINT the real error. An earlier version sent both streams to
  # /dev/null and reported every failure as "the summary signature did not
  # verify", which is a lie whenever the cause was anything else -- a missing
  # key, an unsupported repo mode, a transport problem -- and it cost a full
  # debug cycle on the first real release to discover that.
  if ! pull_err="$(ostree --repo="$VERIFY_REPO" pull verify-source "$ref" 2>&1)"; then
    echo "refusing to publish: could not verify $REPO against $PUBKEY (ref $ref)" >&2
    echo "ostree pull said:" >&2
    printf '%s\n' "$pull_err" | sed 's/^/  /' >&2
    exit 1
  fi
done <<<"$REFS"
rm -rf "$VERIFY_REPO"
trap - EXIT
echo "PASS: every app/ ref in $REPO verifies against $PUBKEY"

if [ "$VERIFY_ONLY" = 1 ]; then
  echo "--verify-only: stopping before upload, nothing was published"
  exit 0
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
