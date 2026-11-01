#!/usr/bin/env bash
# Pilot deployment script — run on tc-pilot-host after staging artifacts
# under ~/deploy/.
#
# Order matters:
#   1. Install binaries (built on the host with the right cargo features).
#   2. Install env files + systemd units + Caddy config.
#   3. Start cloud-sql-proxy (provides 127.0.0.1:5432 for the ingest).
#   4. Start the issuer (loopback only — no cert needed yet).
#   5. Configure + start Caddy → fetches LE certs for both subdomains.
#   6. Smoke the issuer through the public URL.
#   7. Start ingest (it fetches the issuer's JWKS over HTTPS at startup),
#      and refuse a binary from the other environment (TC_EXPECT_STAGING).
#   8. Smoke /health on both ingest and issuer.
#   9. Smoke the AGPL section 13 source offer through the public ingress.
#
# Required features for the ingest binary:
#   --features gcs-client,gcp-kms,near-ai-scorer,near-attestation-collateral
#
# The pilot no longer builds on the host. cloudbuild.yaml compiles the binary,
# publishes it to GCS with a sha256 sidecar, and pull-and-install.sh fetches
# and installs it. That build is the one whose feature list decides what the
# pilot runs, so this list has to match it -- keep the two in step. They were
# out of step: this comment named the collateral feature and cloudbuild.yaml
# did not build it, so the deployed binary silently lacked it while the
# documentation said it was required.
#
# near-attestation-collateral compiles the Intel DCAP collateral client. Without
# it POST /v1/admin/near-attestation-drill runs but refuses at its collateral
# step with missing_control:near_ai_attestation_collateral_client, so the pilot
# would hold a drill that can never pass. See
# docs/operator/near-attestation-drill.md.

set -euo pipefail

# Parameters. Every default is the pilot's, so a pilot run with none of them
# set issues exactly the commands it always did
# (scripts/ci/test-deploy-render.sh holds that against a recorded trace). The
# staging server sets them as deploy/staging-gcp/README.md describes.
#
#   TC_DB_MODE        cloudsql (default): PostgreSQL through the Cloud SQL Auth
#                     Proxy. local: PostgreSQL on this VM; the proxy is never
#                     installed or started, and both service units are
#                     installed ordered after TC_PG_UNIT instead of the proxy.
#   TC_PG_UNIT        the local PostgreSQL unit (default postgresql.service);
#                     local mode also requires pg_isready on 127.0.0.1:5432.
#   TC_DEPLOY_LABEL   PILOT (default): the closing "<label> DEPLOY OK" line;
#                     in local mode also the units' "(TraceCommons <label>)".
#   TC_EXPECT_STAGING unset (default): refuse an ingest whose /health reports
#                     near_ai_staging true. 1: refuse one that does not report
#                     it true. A staging binary on the pilot, or the reverse,
#                     stops the deploy before the public smoke, and the ingest
#                     goes back to the binary step 0 kept (or is stopped, on a
#                     first install).
#   TC_ENV_INSTALL    1 (default): install ~/deploy/ingest.env and issuer.env.
#                     0: they were rendered in place under /etc/tracecommons
#                     (deploy/staging-gcp/render-env.sh); require them, never
#                     overwrite them.
#   TC_BINARY_DIR     where the two binaries are installed from (default
#                     ~/trace-commons-server/target/release).
TC_DB_MODE="${TC_DB_MODE:-cloudsql}"
TC_PG_UNIT="${TC_PG_UNIT:-postgresql.service}"
TC_DEPLOY_LABEL="${TC_DEPLOY_LABEL:-PILOT}"
TC_EXPECT_STAGING="${TC_EXPECT_STAGING:-}"
TC_ENV_INSTALL="${TC_ENV_INSTALL:-1}"
TC_BINARY_DIR="${TC_BINARY_DIR:-$HOME/trace-commons-server/target/release}"

case "$TC_DB_MODE" in
  cloudsql | local) ;;
  *) echo "REFUSING: TC_DB_MODE must be cloudsql or local" >&2; exit 2 ;;
esac
case "$TC_EXPECT_STAGING" in
  "" | 1) ;;
  *) echo "REFUSING: TC_EXPECT_STAGING must be unset or 1" >&2; exit 2 ;;
esac
case "$TC_ENV_INSTALL" in
  0 | 1) ;;
  *) echo "REFUSING: TC_ENV_INSTALL must be 0 or 1" >&2; exit 2 ;;
esac
case "$TC_PG_UNIT" in
  *[!A-Za-z0-9@._-]* | "") echo "REFUSING: TC_PG_UNIT is not a unit name" >&2; exit 2 ;;
esac
case "$TC_DEPLOY_LABEL" in
  *[!A-Za-z0-9_-]* | "") echo "REFUSING: TC_DEPLOY_LABEL must be letters, digits, - or _" >&2; exit 2 ;;
esac

# Install one service unit. cloudsql: the file as staged, byte for byte.
# local: the same file with the proxy dependency replaced by the local
# PostgreSQL unit and the label in its description. A drop-in cannot do
# this: systemd lets a drop-in add dependencies but not remove one, and the
# ingest unit Requires= the proxy.
install_unit() {
  local src="$1"
  if [ "$TC_DB_MODE" = cloudsql ]; then
    sudo install -m 644 "$src" /etc/systemd/system/
    return
  fi
  local label rendered
  label="$(printf '%s' "$TC_DEPLOY_LABEL" | tr '[:upper:]' '[:lower:]')"
  rendered="$(mktemp -d)/$(basename "$src")"
  sed -e "s/cloud-sql-proxy\.service/$TC_PG_UNIT/g" \
    -e "s/(TraceCommons pilot)/(TraceCommons $label)/" "$src" >"$rendered"
  sudo install -m 644 "$rendered" /etc/systemd/system/
  rm -rf "$(dirname "$rendered")"
}

# Read the ingest's /health and refuse a build from the other environment.
# An ingest that predates the field reports nothing, which the pilot
# accepts and a staging deploy refuses.
check_health_channel() {
  local health="$1" staging=absent
  if printf '%s' "$health" | grep -Eq '"near_ai_staging"[[:space:]]*:[[:space:]]*true'; then
    staging=true
  elif printf '%s' "$health" | grep -Eq '"near_ai_staging"[[:space:]]*:[[:space:]]*false'; then
    staging=false
  fi
  if [ "$TC_EXPECT_STAGING" = 1 ] && [ "$staging" != true ]; then
    echo "REFUSING: TC_EXPECT_STAGING=1 but the ingest /health reports near_ai_staging=$staging." >&2
    echo "  This binary is not the staging build." >&2
    withdraw_ingest
  fi
  if [ -z "$TC_EXPECT_STAGING" ] && [ "$staging" = true ]; then
    echo "REFUSING: the ingest /health reports near_ai_staging=true on a deploy that does not expect it." >&2
    echo "  A staging binary was installed here." >&2
    withdraw_ingest
  fi
}

# The ingest from the other environment is already running when /health
# shows it, so refusing is not enough: put the binary step 0 kept back and
# restart it, or, on a first install with nothing kept, stop the ingest.
withdraw_ingest() {
  if [ -n "$INGEST_BACKUP" ]; then
    echo "  Putting back $INGEST_BACKUP and restarting the ingest." >&2
    sudo install -o tracecommons -g tracecommons -m 755 "$INGEST_BACKUP" \
      /opt/tracecommons/bin/trace-commons-ingest
    sudo systemctl restart trace-commons-ingest.service
  else
    echo "  No earlier binary to put back; stopping the ingest." >&2
    sudo systemctl stop trace-commons-ingest.service
  fi
  exit 1
}

# 0. Keep the outgoing binaries.
#
# `install` overwrites in place, so without this there is nothing to roll back
# to. That is not hypothetical: on 2026-09-01 a binary built with
# near-attestation-collateral panicked at startup on a rustls provider
# ambiguity, crash-looped 23 times, and could not be rolled back -- the newest
# backup in this directory was almost a month old, and recovery meant waiting
# for a fresh build with the service down. The hand-made `.bak-*` copies
# already scattered through /opt/tracecommons/bin are evidence that people have
# hit this before and solved it by hand each time.
#
# Cheap insurance: a timestamped copy of whatever is currently installed,
# before anything replaces it. `|| true` because a first-ever install has
# nothing to preserve, and that must not abort the deploy under `set -e`.
DEPLOY_STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
INGEST_BACKUP=""
for b in trace-commons-ingest trace-commons-upload-claim-issuer; do
  if [ -f "/opt/tracecommons/bin/$b" ]; then
    if sudo cp -p "/opt/tracecommons/bin/$b" \
      "/opt/tracecommons/bin/$b.bak-$DEPLOY_STAMP" && [ "$b" = trace-commons-ingest ]; then
      INGEST_BACKUP="/opt/tracecommons/bin/$b.bak-$DEPLOY_STAMP"
    fi
    echo "preserved /opt/tracecommons/bin/$b.bak-$DEPLOY_STAMP"
  fi
done

# To roll back, stop the unit, copy the chosen .bak-* back over the live name,
# and start it again. Check `ls -t /opt/tracecommons/bin/*.bak-*` for what is
# available -- and note a rollback across a migration boundary is NOT safe on
# its own: a binary older than the applied schema may silently misbehave rather
# than fail. V56, for instance, forces RLS on a table whose drain worker only
# sets the required GUC in binaries that postdate it.

# 1. Binaries.
sudo install -o tracecommons -g tracecommons -m 755 \
  "$TC_BINARY_DIR/trace-commons-ingest" \
  /opt/tracecommons/bin/
sudo install -o tracecommons -g tracecommons -m 755 \
  "$TC_BINARY_DIR/trace-commons-upload-claim-issuer" \
  /opt/tracecommons/bin/

# 1b. State directories.
#
# Nothing else in this repo creates them; they were made by hand at
# provisioning, which is how the dedup index came to have no directory at
# all. `install -d` is idempotent and does not disturb an existing one.
for d in /var/lib/trace-commons-vector-index \
         /var/lib/trace-commons-dedup-index \
         /var/lib/trace-commons-data \
         /var/cache/trace-commons-embedder; do
  sudo install -d -o tracecommons -g tracecommons -m 750 "$d"
done

# 2. Env files + systemd units + Caddy.
if [ "$TC_ENV_INSTALL" = 1 ]; then
  sudo install -o root -g tracecommons -m 640 ~/deploy/ingest.env /etc/tracecommons/ingest.env
  sudo install -o root -g tracecommons -m 640 ~/deploy/issuer.env /etc/tracecommons/issuer.env
else
  for f in /etc/tracecommons/ingest.env /etc/tracecommons/issuer.env; do
    if ! sudo test -f "$f"; then
      echo "REFUSING: TC_ENV_INSTALL=0 but $f does not exist; render it first." >&2
      exit 1
    fi
  done
fi
install_unit ~/deploy/trace-commons-ingest.service
install_unit ~/deploy/trace-commons-upload-claim-issuer.service
if [ "$TC_DB_MODE" = cloudsql ]; then
  sudo install -m 644 ~/deploy/cloud-sql-proxy.service /etc/systemd/system/
fi
sudo install -m 644 ~/deploy/Caddyfile /etc/caddy/Caddyfile

sudo systemctl daemon-reload

# 3. The database: the Cloud SQL Auth Proxy (TLS to Cloud SQL, loopback to
#    ingest), or PostgreSQL on this VM.
if [ "$TC_DB_MODE" = cloudsql ]; then
  sudo systemctl enable --now cloud-sql-proxy.service
  sleep 3
  sudo systemctl is-active cloud-sql-proxy.service
else
  sudo systemctl is-active "$TC_PG_UNIT"
  # Ubuntu's postgresql.service is a placeholder that reports active even
  # when the cluster under it (postgresql@16-main) has failed, so ask the
  # server itself.
  if ! pg_isready -h 127.0.0.1 -p 5432; then
    echo "REFUSING: PostgreSQL on 127.0.0.1:5432 is not accepting connections." >&2
    exit 1
  fi
fi

# 4. Issuer.
sudo systemctl enable --now trace-commons-upload-claim-issuer.service
sleep 3
sudo systemctl is-active trace-commons-upload-claim-issuer.service
curl -sfS http://127.0.0.1:3917/health | head -c 400; echo

# 5. Caddy — fetches certs in the background; first request may 502 briefly.
sudo systemctl reload caddy || sudo systemctl restart caddy
sleep 5

# 6. Wait for cert + smoke issuer through public URL.
echo "Waiting for Let's Encrypt cert for issuer subdomain..."
for _ in $(seq 1 60); do
  if curl -sfk "https://issuer.${TC_PUBLIC_HOST}/health" -o /tmp/issuer-health.out; then
    echo "issuer reachable via HTTPS"
    break
  fi
  sleep 5
done
curl -sf "https://issuer.${TC_PUBLIC_HOST}/.well-known/trace-commons-ed25519-keyset.json" \
  | head -c 300; echo

# 7. Ingest — depends on the issuer keyset being fetchable and Cloud SQL
#    proxy listening on 127.0.0.1:5432.
sudo systemctl enable --now trace-commons-ingest.service
sleep 8
sudo systemctl is-active trace-commons-ingest.service
# Asked until it answers, for up to a minute, each ask bounded: a first
# install applies every migration, so the ingest may not listen yet, and a
# refused connection must not end the deploy before the channel check
# (pull-and-install.sh waits the same way, #1684).
INGEST_HEALTH=""
for _ in $(seq 1 30); do
  INGEST_HEALTH="$(curl -sfS --connect-timeout 2 --max-time 5 http://127.0.0.1:3907/health || true)"
  [ -n "$INGEST_HEALTH" ] && break
  sleep 2
done
if [ -z "$INGEST_HEALTH" ]; then
  echo "REFUSING: the ingest /health did not answer within a minute." >&2
  withdraw_ingest
fi
printf '%s' "$INGEST_HEALTH" | head -c 400; echo
check_health_channel "$INGEST_HEALTH"

# 8. Smoke through Caddy.
curl -sf "https://ingest.${TC_PUBLIC_HOST}/health" | head -c 300; echo

# 9. AGPL section 13: the source offer has to be reachable by a network user
#    holding no credentials, which means through the public ingress and not
#    merely on localhost. --fail so a proxy rule that swallows it breaks the
#    deploy rather than going unnoticed.
curl -sfS "https://ingest.${TC_PUBLIC_HOST}/v1/source" | head -c 300; echo
echo "$TC_DEPLOY_LABEL DEPLOY OK"
