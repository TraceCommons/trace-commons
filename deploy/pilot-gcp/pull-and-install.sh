#!/usr/bin/env bash
# Pull a Cloud Build-produced trace-commons-ingest binary from GCS and install it
# on the pilot host, then restart the service. Pairs with cloudbuild.yaml.
#
# Usage (on tc-pilot-host):
#   deploy/pilot-gcp/pull-and-install.sh [<gs://.../trace-commons-ingest>]
# With no arg it reads the `latest.txt` pointer the build publishes.
#
# Verifies the sha256 sidecar, backs up the running binary, installs, and
# restarts trace-commons-ingest (which auto-applies any pending migrations).
set -euo pipefail

BUCKET="${TC_ARTIFACT_BUCKET:-tc-pilot-artifacts-20260518}"
BIN_DEST="/opt/tracecommons/bin/trace-commons-ingest"
LATEST="gs://${BUCKET}/binaries/trace-commons-ingest/latest.txt"

SRC="${1:-}"
if [ -z "$SRC" ]; then
  SRC="$(gcloud storage cat "$LATEST")"
fi
echo "Pulling: $SRC"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
gcloud storage cp "$SRC" "$TMP/trace-commons-ingest"
gcloud storage cp "${SRC}.sha256" "$TMP/trace-commons-ingest.sha256" || true

if [ -f "$TMP/trace-commons-ingest.sha256" ]; then
  ( cd "$TMP" && awk '{print $1"  trace-commons-ingest"}' trace-commons-ingest.sha256 | sha256sum -c - )
  echo "sha256 verified"
else
  echo "WARNING: no sha256 sidecar found; skipping checksum verification" >&2
fi

chmod 0755 "$TMP/trace-commons-ingest"
"$TMP/trace-commons-ingest" --version 2>/dev/null || true

STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
sudo cp -av "$BIN_DEST" "${BIN_DEST}.bak-${STAMP}"
sudo install -o root -g root -m 0755 "$TMP/trace-commons-ingest" "$BIN_DEST"
echo "installed $BIN_DEST"

sudo systemctl restart trace-commons-ingest
sleep 10
systemctl is-active trace-commons-ingest
echo "done; rollback with: sudo install -m0755 ${BIN_DEST}.bak-${STAMP} ${BIN_DEST} && sudo systemctl restart trace-commons-ingest"
