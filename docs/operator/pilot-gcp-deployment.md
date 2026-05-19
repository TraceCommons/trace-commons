# Pilot GCP deployment runbook

This runbook captures the first hosted pilot deployment of TraceCommons on
Google Cloud (project `tracecommons-pilot-2026`, host `tc-pilot-host`,
public IP `34.41.15.28`). Templated artifacts live under
[`deploy/pilot-gcp/`](../../deploy/pilot-gcp/). Use this runbook when bringing
up a new pilot project; it focuses on the steps and gotchas that are not
obvious from the source code or the broader operator docs.

## Topology

```
client (ironclaw)
   |
   v (HTTPS, public)
ingest.<host>.nip.io  <--Caddy-->  127.0.0.1:3907  trace-commons-ingest
issuer.<host>.nip.io  <--Caddy-->  127.0.0.1:3917  trace-commons-upload-claim-issuer

                                   127.0.0.1:5432  cloud-sql-proxy  --mTLS-->  Cloud SQL
                                                                   --IAM-->   tc-pilot-runtime@...

                                   GCS  tc-pilot-artifacts-<DATE>          (CMEK)
                                   KMS  projects/.../keyRings/tc-pilot
```

- Both daemons bind loopback only; Caddy is the public TLS edge.
- The Cloud SQL Auth Proxy is the TLS terminator into Cloud SQL — the ingest
  binary connects via `127.0.0.1:5432` with no SSL, the proxy upgrades to
  mTLS using the runtime service account.
- Public IP is exposed only on ports 80/443 (Caddy). The Cloud SQL
  instance has no `authorizedNetworks` — the proxy uses the admin API.

## Prereqs to provision per pilot

- GCP project under the right org / billing account.
- Service account `tc-pilot-runtime@<project>.iam.gserviceaccount.com` with
  `roles/cloudsql.client`, `roles/cloudkms.cryptoKeyEncrypterDecrypter` on
  the KEK, and read/write on the artifact bucket.
- Cloud SQL Postgres instance (call it `tc-pilot`) with an `app` role.
- Cloud KMS key ring `tc-pilot` with a `kek-v1` key (the artifact-bucket
  CMEK and the binary's KEK provider both point here).
- GCS bucket `tc-pilot-artifacts-<DATE>` with object versioning enabled
  and CMEK pointing at `kek-v1`.
- GCE host with the runtime SA attached, a public IP, and ports 80/443
  open. The pilot used a small `e2-standard-4`; the ingest binary CPU
  budget is dominated by fastembed.

## Build the binaries on the host

```
cargo build --release --bin trace-commons-ingest \
  --features gcs-client,gcp-kms,near-ai-scorer
cargo build --release --bin trace-commons-upload-claim-issuer
```

Pitfalls:

- A plain `cargo build --release --bin trace-commons-ingest` (no features)
  produces a 28 MB binary with the GCS + KMS providers stubbed out. The
  startup validation then reports
  `object_store=trace_commons_service_owned_remote_disabled` and refuses
  to start when `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING=true`.
  Always pass `--features gcs-client,gcp-kms,near-ai-scorer`. The
  feature-complete binary is ~65 MB.

## Render artifacts

The files under `deploy/pilot-gcp/` are templates; render with `envsubst`
(or your operator key vault's renderer):

```
export TC_GCP_PROJECT=tracecommons-pilot-2026
export TC_GCP_REGION=us-central1
export TC_CLOUD_SQL_INSTANCE=tc-pilot
export TC_PUBLIC_HOST=34-41-15-28.nip.io
export TC_LE_EMAIL=ops@example.com
export TC_KMS_KEY_NAME=projects/$TC_GCP_PROJECT/locations/$TC_GCP_REGION/keyRings/tc-pilot/cryptoKeys/kek-v1
export TC_GCS_BUCKET=tc-pilot-artifacts-<DATE>
export TC_RUNTIME_SA=tc-pilot-runtime@$TC_GCP_PROJECT.iam.gserviceaccount.com
# ...plus all secret placeholders from ingest.env.template/issuer.env.template
envsubst < deploy/pilot-gcp/ingest.env.template     > /tmp/ingest.env
envsubst < deploy/pilot-gcp/issuer.env.template     > /tmp/issuer.env
envsubst < deploy/pilot-gcp/Caddyfile.template      > /tmp/Caddyfile
envsubst < deploy/pilot-gcp/systemd/cloud-sql-proxy.service > /tmp/cloud-sql-proxy.service
```

Copy rendered files plus the two `*.service` units that have no
placeholders into `~/deploy/` on the host, then run `deploy.sh`.

## Cloud SQL Auth Proxy

```
curl -fsSL \
  -o /tmp/cloud-sql-proxy \
  https://storage.googleapis.com/cloud-sql-connectors/cloud-sql-proxy/v2.21.3/cloud-sql-proxy.linux.amd64
echo "46bef6dad3db3d10f07d69a1d76891d1a6aa942cc77b6f50369d9b8160a129e1  /tmp/cloud-sql-proxy" | sha256sum -c
sudo install -o root -g root -m 755 /tmp/cloud-sql-proxy /usr/local/bin/
```

Upgrade by repeating with a new version + sha256 from
[the upstream releases page](https://github.com/GoogleCloudPlatform/cloud-sql-proxy/releases).

## Gotchas observed during first bring-up

These cost time during the first deploy; fixed in this branch's templates
but worth knowing if you hit a regression.

### Cascading rollout-gate startup validation

`TRACE_COMMONS_OBJECT_STORE=remote_service` triggers a chain of mandatory
flags. Setting one without the others will refuse to start; remove the
whole block together if reverting to a different object-store mode.

- `OBJECT_PRIMARY_SUBMIT_REVIEW` ⇒ requires `DB_DUAL_WRITE`,
  `REQUIRE_DB_MIRROR_WRITES`, `DB_REVIEWER_READS`,
  `DB_REVIEWER_REQUIRE_OBJECT_REFS`, and an enabled object-store provider.
- `OBJECT_PRIMARY_REPLAY_EXPORT` ⇒ adds `DB_REPLAY_EXPORT_READS` and
  `DB_REPLAY_EXPORT_REQUIRE_OBJECT_REFS`.
- `OBJECT_PRIMARY_DERIVED_EXPORTS` ⇒ adds `DB_REVIEWER_READS`,
  `DERIVED_EXPORT_REQUIRE_OBJECT_REFS` (note: no `DB_` prefix, no `S` on
  `EXPORT`), and `REQUIRE_EXPORT_GUARDRAILS`.

### `ProtectHome=true` blocks `~/.ironclaw`

The ingest binary's default `TRACE_COMMONS_DATA_DIR` falls back to
`~/.ironclaw/trace_commons_ingest`, which is unwritable under the
hardened systemd unit. Always set `TRACE_COMMONS_DATA_DIR` explicitly
(template uses `/var/lib/trace-commons-data`) and list that path under
`ReadWritePaths` on the unit. Failure mode is a 500 with
`failed to create trace contribution metadata dir`.

### Caddy log directory

`/var/log/caddy` must exist and be owned `caddy:caddy` before
`systemctl reload caddy` — otherwise Caddy startup fails with
`permission denied` and the LE flow never starts.

### KEK provider naming

The env value is `gcp_cloud_kms`, not `gcp_kms`. Old runbooks may use
the latter; the binary will refuse with `KekProviderUnknown`.

### Idempotency on retry

`trace_audit_events` has a unique constraint on `(tenant_id,
audit_event_id)`, and `audit_event_id` is deterministically derived
from the submission_id. Retrying the same envelope after a partial
success will hit the constraint and fail the dual-write mirror with
`failed to mirror trace audit event`. Always generate a fresh
envelope (via `ironclaw traces preview --enqueue`) for each new
submit; don't re-submit a failed envelope.

## Post-deploy verification

```
curl -sfS https://ingest.${TC_PUBLIC_HOST}/health
curl -sfS https://issuer.${TC_PUBLIC_HOST}/health
curl -sfS https://issuer.${TC_PUBLIC_HOST}/.well-known/trace-commons-ed25519-keyset.json | head -c 300
```

Submit a real envelope end-to-end using `sign-workload-token.py` to mint
the workload JWT, then call `/v1/trace-upload-claim` (issuer) followed by
`/v1/traces` (ingest). The accepted-corpus response looks like:

```
{"status":"accepted","credit_points_pending":5.2,"explanation":["Accepted into the private redacted corpus.","Attributed to tenant tenant_sha256:<hash>"]}
```

Confirm the GCS object lands:

```
gsutil ls -r "gs://${TC_GCS_BUCKET}/**/contribution_envelope/*.json" | tail
```

And the DB row:

```
PGPASSWORD=$TC_DB_PASSWORD psql -h 127.0.0.1 -U app -d trace-commons <<SQL
SELECT set_config('trace_commons.trace_tenant_id', 'tenant-<...>', false);
SELECT submission_id, status, received_at FROM trace_submissions
  ORDER BY received_at DESC LIMIT 5;
SQL
```

(Note the GUC name is `trace_commons.trace_tenant_id` with an underscore;
PR #111 corrected an earlier hyphenated form that PostgreSQL silently
rejected.)

## Known follow-ups

- **Gate-floor calibration.** Pilot launches with
  `PERPLEXITY=0`, `TAIL_FRACTION=0`, `NOVELTY=500000` micros. Recalibrate
  using `trace-commons-gate-calibrate tail-floor` once ~1000 real traces
  have landed. Until then the novelty floor is the only active gate.
- **Tenant access grants.** `REQUIRE_TENANT_ACCESS_GRANTS=false` during
  the pilot. Flip on once grants are seeded for the contributor cohort.
- **Workload-token mint API.** `sign-workload-token.py` is a local
  operator helper. Replace with a KMS-backed mint service before
  promoting beyond the pilot cohort.
