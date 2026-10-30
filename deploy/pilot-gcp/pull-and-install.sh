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
#   deploy/pilot-gcp/pull-and-install.sh                    # both binaries (default)
#   deploy/pilot-gcp/pull-and-install.sh ingest             # just ingest
#   deploy/pilot-gcp/pull-and-install.sh issuer             # just the issuer
#   deploy/pilot-gcp/pull-and-install.sh ingest 4655cf45    # refuse unless latest.txt is that build
#   TC_EXPECT_TAG=4655cf45 deploy/pilot-gcp/pull-and-install.sh   # same, for `both`
#   TC_FETCH_ONLY_DIR=/opt/tracecommons/staged deploy/pilot-gcp/pull-and-install.sh both 4655cf45
#                                                           # fetch and verify only (first install)
#
# Another environment sets TC_ARTIFACT_BUCKET to its own bucket; the staging
# server's is in deploy/staging-gcp/README.md.
#
# Each install verifies the sha256 sidecar, backs up the running binary, installs,
# and restarts the service. Reads the per-binary `latest.txt` pointer the build
# publishes.
#
# ALWAYS pass the expected tag when deploying a specific build. `latest.txt`
# names the last build that *published*, so running this while your build is
# still in flight reinstalls the previous binary, restarts the service, and
# prints "done." — a successful-looking no-op. The tag is now echoed on every
# run whether or not you pass one.
set -euo pipefail

BUCKET="${TC_ARTIFACT_BUCKET:-tc-pilot-artifacts-20260518}"
# Optional: the build tag this run is meant to install, e.g. the short SHA
# passed to `gcloud builds submit --substitutions _TAG=...`. When set, a
# mismatch against `latest.txt` refuses the install instead of quietly
# deploying whatever was published last.
EXPECT_TAG="${TC_EXPECT_TAG:-${2:-}}"
# Optional: fetch and verify into this directory and stop there -- no backup,
# no install, no restart. A first install has no running binary and no unit
# yet; deploy.sh then installs from here (TC_BINARY_DIR). In this mode a
# missing sha256 sidecar refuses instead of warning.
FETCH_ONLY_DIR="${TC_FETCH_ONLY_DIR:-}"
# Optional, for the staging server: 1 reads the restarted ingest's /health
# and, unless it reports near_ai_staging true, puts the previous binary back,
# restarts it and refuses. Unset (the pilot): no health check, as before; a
# staging binary on the pilot is refused by its own boot guard.
EXPECT_STAGING="${TC_EXPECT_STAGING:-}"
case "$EXPECT_STAGING" in
  "" | 1) ;;
  *) echo "REFUSING: TC_EXPECT_STAGING must be unset or 1" >&2; exit 2 ;;
esac

install_one() {
  local bin="$1" svc="$2"
  local latest="gs://${BUCKET}/binaries/${bin}/latest.txt"
  local src
  src="$(gcloud storage cat "$latest")"

  # `latest.txt` points at the most recently *published* build, which is not
  # necessarily the build you just submitted: a run against an in-flight build
  # silently reinstalls the previous binary and still reports success. Name the
  # tag on every run, and refuse when it is not the one asked for.
  local tag
  tag="$(basename "$(dirname "$src")")"
  echo "[$bin] latest.txt tag: $tag"
  if [ -n "$EXPECT_TAG" ] && [ "$tag" != "$EXPECT_TAG" ]; then
    echo "[$bin] REFUSING: expected tag '$EXPECT_TAG' but latest.txt points at '$tag'." >&2
    echo "[$bin] The build for '$EXPECT_TAG' has probably not published yet." >&2
    echo "[$bin] Check: gcloud builds list --project tracecommons-pilot-2026 --limit 3" >&2
    return 1
  fi
  echo "[$bin] pulling: $src"

  local tmp
  tmp="$(mktemp -d)"
  gcloud storage cp "$src" "$tmp/$bin"
  if gcloud storage cp "${src}.sha256" "$tmp/$bin.sha256" 2>/dev/null; then
    ( cd "$tmp" && sha256sum -c "$bin.sha256" )
    echo "[$bin] sha256 verified"
  elif [ -n "$FETCH_ONLY_DIR" ]; then
    echo "[$bin] REFUSING: no sha256 sidecar for $src." >&2
    rm -rf "$tmp"
    return 1
  else
    echo "[$bin] WARNING: no sha256 sidecar; skipping checksum" >&2
  fi
  chmod 0755 "$tmp/$bin"

  if [ -n "$FETCH_ONLY_DIR" ]; then
    install -d -m 0755 "$FETCH_ONLY_DIR"
    install -m 0755 "$tmp/$bin" "$FETCH_ONLY_DIR/$bin"
    rm -rf "$tmp"
    echo "[$bin] fetched to $FETCH_ONLY_DIR/$bin (not installed, not restarted)"
    return 0
  fi

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
  if [ "$bin" = trace-commons-ingest ] && [ "$EXPECT_STAGING" = 1 ]; then
    local health="" _
    # Asked until it answers, for up to a minute: a first boot that applies
    # migrations is not listening yet (2026-10-27, V139 and V140 on staging,
    # rolled a good build back on one refused connection). Each ask is
    # bounded too, so an ingest that accepts the connection and never
    # answers cannot hang the install.
    for _ in $(seq 1 30); do
      health="$(curl -sfS --connect-timeout 2 --max-time 5 http://127.0.0.1:3907/health || true)"
      [ -n "$health" ] && break
      sleep 2
    done
    if ! printf '%s' "$health" | grep -Eq '"near_ai_staging"[[:space:]]*:[[:space:]]*true'; then
      echo "[$bin] REFUSING: TC_EXPECT_STAGING=1 but /health does not report near_ai_staging=true." >&2
      echo "[$bin] Putting back ${dest}.bak-${stamp}." >&2
      sudo install -o root -g root -m 0755 "${dest}.bak-${stamp}" "$dest"
      sudo systemctl restart "$svc"
      return 1
    fi
    echo "[$bin] /health reports near_ai_staging=true"
  fi
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
