#!/usr/bin/env bash
# Pull Cloud Build-produced binaries from GCS and install them on the pilot host,
# restarting the services. Pairs with cloudbuild.yaml.
#
# The pilot runs two services; both can change, so this deploys BOTH by default:
#   - trace-commons-upload-claim-issuer  (EdDSA claims, device-key registration,
#     per-user subject, /v1/enroll; serves the JWKS the ingest verifies at boot)
#   - trace-commons-ingest               (account API; applies migrations on boot)
# The issuer is installed first so its (possibly rotated) JWKS is up before ingest
# restarts and fetches it.
#
# Usage (on tc-pilot-host):
#   deploy/pilot-gcp/pull-and-install.sh            # both binaries (default)
#   deploy/pilot-gcp/pull-and-install.sh ingest     # just ingest
#   deploy/pilot-gcp/pull-and-install.sh issuer     # just the issuer
#
# Each install verifies the sha256 sidecar, backs up the running binary, installs,
# and restarts the service. Reads the per-binary `latest.txt` pointer the build
# publishes.
set -euo pipefail

BUCKET="${TC_ARTIFACT_BUCKET:-tc-pilot-artifacts-20260518}"

install_one() {
  local bin="$1" svc="$2"
  local latest="gs://${BUCKET}/binaries/${bin}/latest.txt"
  local src
  src="$(gcloud storage cat "$latest")"
  echo "[$bin] pulling: $src"

  local tmp
  tmp="$(mktemp -d)"
  gcloud storage cp "$src" "$tmp/$bin"
  if gcloud storage cp "${src}.sha256" "$tmp/$bin.sha256" 2>/dev/null; then
    ( cd "$tmp" && sha256sum -c "$bin.sha256" )
    echo "[$bin] sha256 verified"
  else
    echo "[$bin] WARNING: no sha256 sidecar; skipping checksum" >&2
  fi
  chmod 0755 "$tmp/$bin"

  local stamp dest
  stamp="$(date -u +%Y%m%dT%H%M%SZ)"
  dest="/opt/tracecommons/bin/$bin"
  sudo cp -av "$dest" "${dest}.bak-${stamp}"
  sudo install -o root -g root -m 0755 "$tmp/$bin" "$dest"
  rm -rf "$tmp"
  echo "[$bin] installed $dest"

  sudo systemctl restart "$svc"
  sleep 8
  echo "[$bin] $(systemctl is-active "$svc")"
  echo "[$bin] rollback: sudo install -m0755 ${dest}.bak-${stamp} ${dest} && sudo systemctl restart $svc"
}

case "${1:-both}" in
  ingest) install_one trace-commons-ingest trace-commons-ingest ;;
  issuer) install_one trace-commons-upload-claim-issuer trace-commons-upload-claim-issuer ;;
  both)
    install_one trace-commons-upload-claim-issuer trace-commons-upload-claim-issuer
    install_one trace-commons-ingest trace-commons-ingest
    ;;
  *) echo "usage: $0 [both|ingest|issuer]" >&2; exit 2 ;;
esac
echo "done."
