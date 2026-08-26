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
#   7. Start ingest (it fetches the issuer's JWKS over HTTPS at startup).
#   8. Smoke /health on both ingest and issuer.
#
# Required features when building the ingest binary on the host:
#   cargo build --release --bin trace-commons-ingest \
#     --features gcs-client,gcp-kms,near-ai-scorer

set -euo pipefail

# 1. Binaries.
sudo install -o tracecommons -g tracecommons -m 755 \
  ~/trace-commons-server/target/release/trace-commons-ingest \
  /opt/tracecommons/bin/
sudo install -o tracecommons -g tracecommons -m 755 \
  ~/trace-commons-server/target/release/trace-commons-upload-claim-issuer \
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
sudo install -o root -g tracecommons -m 640 ~/deploy/ingest.env /etc/tracecommons/ingest.env
sudo install -o root -g tracecommons -m 640 ~/deploy/issuer.env /etc/tracecommons/issuer.env
sudo install -m 644 ~/deploy/trace-commons-ingest.service /etc/systemd/system/
sudo install -m 644 ~/deploy/trace-commons-upload-claim-issuer.service /etc/systemd/system/
sudo install -m 644 ~/deploy/cloud-sql-proxy.service /etc/systemd/system/
sudo install -m 644 ~/deploy/Caddyfile /etc/caddy/Caddyfile

sudo systemctl daemon-reload

# 3. Cloud SQL Auth Proxy (TLS to Cloud SQL, loopback to ingest).
sudo systemctl enable --now cloud-sql-proxy.service
sleep 3
sudo systemctl is-active cloud-sql-proxy.service

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
for i in $(seq 1 60); do
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
curl -sfS http://127.0.0.1:3907/health | head -c 400; echo

# 8. Smoke through Caddy.
curl -sf "https://ingest.${TC_PUBLIC_HOST}/health" | head -c 300; echo
echo "PILOT DEPLOY OK"
