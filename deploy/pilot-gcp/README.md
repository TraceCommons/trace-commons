# deploy/pilot-gcp

Templated deployment artifacts for the TraceCommons pilot on Google Cloud.

See [`docs/operator/pilot-gcp-deployment.md`](../../docs/operator/pilot-gcp-deployment.md)
for the bring-up runbook, gotchas, and post-deploy verification.
The public `tracecommons.ai` Pages surface is deployed from
[`community/`](../../community/) and operated with
[`docs/operator/tracecommons-ai-community-site.md`](../../docs/operator/tracecommons-ai-community-site.md).

## Files

- `ingest.env.template` — `trace-commons-ingest` config; render to
  `/etc/tracecommons/ingest.env`.
- `issuer.env.template` — `trace-commons-upload-claim-issuer` config;
  render to `/etc/tracecommons/issuer.env`.
- `Caddyfile.template` — reverse proxy + ACME config.
- `systemd/trace-commons-ingest.service` — ingest unit (loopback, hardened).
- `systemd/trace-commons-upload-claim-issuer.service` — issuer unit.
- `systemd/cloud-sql-proxy.service` — Cloud SQL Auth Proxy sidecar; the
  ingest and onboarding-enabled issuer connect via 127.0.0.1:5432 with
  no SSL, the proxy upgrades to mTLS to Cloud SQL.
- `deploy.sh` — host-side install/start sequence. Its parameters
  (`TC_DB_MODE`, `TC_DEPLOY_LABEL`, `TC_EXPECT_STAGING`, `TC_ENV_INSTALL`,
  `TC_BINARY_DIR`) default to the pilot; the staging server sets them, see
  [`deploy/staging-gcp/README.md`](../staging-gcp/README.md).
  `scripts/ci/test-deploy-render.sh` holds the pilot renders and the pilot
  `deploy.sh` command sequence unchanged.
- `sign-workload-token.py` — operator helper for minting pilot workload
  JWTs. Not for production.
