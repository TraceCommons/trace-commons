# deploy/staging-gcp

The staging server: an ingest built with `near-ai-staging`, its own
upload-claim issuer, PostgreSQL on the VM, and the staging site, on the VM
`tc-staging-host` in project `tracecommons-pilot-2026`. It shares nothing
with the pilot: no database, key, secret, bucket or tenant. Design:
[`docs/superpowers/specs/2026-10-21-staging-environment-design.md`](../../docs/superpowers/specs/2026-10-21-staging-environment-design.md),
section 3.

Staging reuses the pilot's deploy files through parameters rather than a
fork (design 3.6). This directory holds only what is staging's own.

## Files

- `cloudbuild.yaml` -- builds the staging ingest
  (`gcs-client,gcp-kms,near-ai-scorer,near-attestation-collateral,near-ai-staging`),
  the issuer and `trace-commons-staging-pins`, and publishes them to
  `tc-staging-artifacts-20261021` only. Submitted by hand.
- `ingest.env.template` -- the staging settings, applied over the pilot's
  `deploy/pilot-gcp/ingest.env.template` (design 3.8): an entry replaces the
  pilot's line for the same key, and an empty entry removes it.
- `issuer.env.template` -- the staging issuer config.
- `Caddyfile.site.template` -- the `staging.tracecommons.ai` site block,
  appended to the pilot's `Caddyfile.template` (which supplies
  `ingest.` and `issuer.staging.tracecommons.ai`). Serves
  `/.well-known/apple-app-site-association` and
  `/near-ai-pins/current.json`; everything else is 404.
- `apple-app-site-association` -- the passkey association for
  `KXSWJN7WY8.ai.tracecommons.shell.staging`.
- `render-env.sh` -- reads the `tc-staging-*` secrets from Secret Manager on
  the VM and writes `/etc/tracecommons/ingest.env`, `issuer.env` and the
  issuer key files (0640 root:tracecommons), the site files under
  `/var/lib/tracecommons-site`, and the Caddyfile and unit files into the
  operator's `~/deploy` for `deploy.sh`. It prints no secret value, and
  refuses by name, writing nothing, when a secret is missing or malformed.

`scripts/ci/test-deploy-render.sh` renders all of it with dummy values, no
network, on every PR.

## Secrets

All in Secret Manager in `tracecommons-pilot-2026`, labelled `env=staging`.
`render-env.sh` reads each one marked *runtime*; grant
`roles/secretmanager.secretAccessor` on those to
`tc-staging-runtime@tracecommons-pilot-2026.iam.gserviceaccount.com` and on
nothing else.

| Secret | Exists | Runtime | Becomes | How to make it |
|---|---|---|---|---|
| `tc-staging-nearai-inference-key` | yes | yes | `TRACE_COMMONS_NEAR_AI_API_KEY`, `TRACE_NEAR_AI_PRIVACY_API_KEY` | a NEAR AI staging API key |
| `tc-staging-db-app-password` | yes | yes | the `app` role's password in `DATABASE_URL` (ingest and issuer) | already set on the role |
| `tc-staging-db-login-resolver-password` | yes | yes | `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL` | set on `tc_login_resolver_login` at bring-up step 6; letters and digits only |
| `tc-staging-nearai-reseller-key` | yes | yes, read only when payout is on | `TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL` | the reseller org's credential on NEAR AI staging |
| `tc-staging-gate-master-key` | **no** | yes | `TRACE_COMMONS_GATE_SERVICE_MASTER_KEY` | `openssl rand -hex 32` (at least 32 characters) |
| `tc-staging-artifact-key` | **no** | yes | `TRACE_COMMONS_ARTIFACT_KEY_HEX` | `openssl rand -hex 32`. Losing it loses every stored artifact |
| `tc-staging-near-account-index-pepper` | **no** | yes | `TRACE_COMMONS_NEAR_ACCOUNT_INDEX_PEPPER` | `openssl rand -base64 32` (base64 of at least 32 bytes) |
| `tc-staging-near-account-name-kek-key` | **no** | yes | `TRACE_COMMONS_NEAR_ACCOUNT_NAME_KEK_KEY` | `openssl rand -hex 32`. Its presence is required when sign-up is on; under `TRACE_COMMONS_KEK_PROVIDER=gcp_cloud_kms` the KMS key `tc-staging/kek-v1` is what seals account names, and this value is not used as key material |
| `tc-staging-issuer-signing-keypair` | **no** | yes | `/etc/tracecommons/issuer-signing-v1.pem`, `issuer-signing.pub`, `TRACE_COMMONS_UPLOAD_CLAIM_ISSUER_SIGNING_KID` | the whole output of `trace-commons-upload-claim-issuer --generate-keypair` (private PEM, public PEM, `kid:` line) |
| `tc-staging-issuer-workload-public-key` | **no** | yes | `/etc/tracecommons/workload-signing.pub` | the public PEM from a second `--generate-keypair`. The issuer will not start without one. Staging issues no workload tokens, so the private half and its kid can be discarded |
| `tc-staging-ingest-attestation-keypair` | **no** | yes | `TRACE_COMMONS_INGEST_ATTESTATION_SIGNING_KID`, `_SIGNING_KEY_PEM`, `_PUBLIC_KEY_PEM` | the whole output of `trace-commons-ingest --generate-attestation-keypair` |
| `tc-staging-nearai-pin-signing-key` | being made | **no** | nothing on the VM | `docs/operator/staging-pin-signing.md`; the runtime SA must never read it |
| `tc-staging-sparkle-signing-key` | yes | **no** | nothing on the VM | the staging macOS workflow's appcast key |

Make each one by piping the generator straight into Secret Manager, so the
value never reaches a file, a terminal or a command line. The two
`--generate-*` flags need no configuration; run them with the binaries
fetched in step 2:

```bash
P=tracecommons-pilot-2026
SA=tc-staging-runtime@$P.iam.gserviceaccount.com
new_secret() {  # name; the value on stdin
  gcloud secrets create "$1" --project "$P" --labels env=staging --data-file=-
  gcloud secrets add-iam-policy-binding "$1" --project "$P" \
    --member "serviceAccount:$SA" --role roles/secretmanager.secretAccessor
}
openssl rand -hex 32 | tr -d '\n' | new_secret tc-staging-gate-master-key
openssl rand -hex 32 | tr -d '\n' | new_secret tc-staging-artifact-key
openssl rand -base64 32 | tr -d '\n' | new_secret tc-staging-near-account-index-pepper
openssl rand -hex 32 | tr -d '\n' | new_secret tc-staging-near-account-name-kek-key
B=/opt/tracecommons/staged
$B/trace-commons-upload-claim-issuer --generate-keypair | new_secret tc-staging-issuer-signing-keypair
$B/trace-commons-upload-claim-issuer --generate-keypair \
  | awk '/BEGIN PUBLIC KEY/,/END PUBLIC KEY/' | new_secret tc-staging-issuer-workload-public-key
$B/trace-commons-ingest --generate-attestation-keypair | new_secret tc-staging-ingest-attestation-keypair
```

Rotation: add a secret version, rerun `render-env.sh`, restart the service.
Rotating the artifact key, the gate master key or the pepper strands what was
written under the old one; do not rotate them on a staging server holding
data you want to keep.

## Bring-up order

Already done (design S9): the VM, PostgreSQL 16 on `127.0.0.1:5432` with the
database `tracecommons` owned by the role `app` (LOGIN, CREATEROLE, not a
superuser), the KMS key `tc-staging/kek-v1`, the private bucket
`tc-staging-artifacts-20261021`, the runtime service account, and DNS for
`staging.tracecommons.ai`, `ingest.` and `issuer.` under it. The host also
needs what the pilot host has (`docs/operator/pilot-gcp-deployment.md`):
Caddy, `gettext-base`, the `tracecommons` user and group, and the
directories `/etc/tracecommons`, `/var/log/tracecommons` and
`/opt/tracecommons/bin`.

1. **Build.** From an operator checkout of the commit to deploy:

   ```bash
   gcloud builds submit --config deploy/staging-gcp/cloudbuild.yaml \
     --project tracecommons-pilot-2026 \
     --substitutions _TAG=$(git rev-parse --short HEAD),_CODE_REVISION_HASH=none
   ```

2. **Fetch** on the VM, from a checkout of the same commit, without
   installing (there is no unit yet):

   ```bash
   sudo TC_ARTIFACT_BUCKET=tc-staging-artifacts-20261021 \
     TC_FETCH_ONLY_DIR=/opt/tracecommons/staged \
     deploy/pilot-gcp/pull-and-install.sh both <tag>
   ```

   In this mode a missing sha256 sidecar refuses.

3. **Secrets.** Create the ones marked **no** above.

4. **Render.**

   ```bash
   sudo TC_GCP_REGION=<region of tc-staging/kek-v1> TC_LE_EMAIL=<acme email> \
     deploy/staging-gcp/render-env.sh
   ```

5. **Install and first boot.** The ingest applies the migrations on its
   first start, which creates the `trace_login_resolver` role.

   ```bash
   TC_PUBLIC_HOST=staging.tracecommons.ai TC_DB_MODE=local \
   TC_DEPLOY_LABEL=STAGING TC_EXPECT_STAGING=1 TC_ENV_INSTALL=0 \
   TC_BINARY_DIR=/opt/tracecommons/staged \
     deploy/pilot-gcp/deploy.sh
   ```

   `TC_DB_MODE=local` never installs the Cloud SQL proxy and installs both
   units ordered after `postgresql.service` (`TC_PG_UNIT` to change it).
   `TC_ENV_INSTALL=0` keeps the files `render-env.sh` wrote.
   `TC_EXPECT_STAGING=1` stops the deploy unless the new ingest's `/health`
   reports `near_ai_staging: true`. It ends with `STAGING DEPLOY OK`.

6. **Create the login-resolver role** (`docs/operator/login-resolver-role.md`).
   Until it exists every login redeem fails closed. The resolver pool
   connects lazily, so the ingest needs no restart. The passwords reach
   `psql` on stdin and in its environment, never on a command line:

   ```bash
   pw="$(gcloud secrets versions access latest --secret tc-staging-db-login-resolver-password)"
   { printf '\\set pw %s\n' "'$pw'"; cat <<'SQL'
   CREATE ROLE tc_login_resolver_login LOGIN NOBYPASSRLS PASSWORD :'pw';
   GRANT trace_login_resolver TO tc_login_resolver_login;
   SQL
   } | PGPASSWORD="$(gcloud secrets versions access latest --secret tc-staging-db-app-password)" \
       psql -h 127.0.0.1 -U app -d tracecommons -v ON_ERROR_STOP=1
   unset pw
   ```

   `app` holds the admin option on `trace_login_resolver` because its
   migration created the role (PostgreSQL 16 gives the creator of a role that
   option).

7. **Verify.**

   ```bash
   curl -fsS https://ingest.staging.tracecommons.ai/health | jq .near_ai_staging   # true
   curl -fsS https://ingest.staging.tracecommons.ai/v1/account/near/provision/capabilities
   scripts/check-aasa.sh https://staging.tracecommons.ai KXSWJN7WY8.ai.tracecommons.shell.staging
   ```

   A staging ingest that names a production NEAR AI host does not reach
   this step: it refuses to boot with
   `missing_control:near_ai_staging_host_coherence`, naming the setting.

8. **Pins.** Sign the pin set on an operator machine
   (`docs/operator/staging-pin-signing.md`), copy `current.json` to
   `/var/lib/tracecommons-site/near-ai-pins/current.json` (0644), then rerun
   step 4 with `TC_NEAR_AI_EXPECTED_MEASUREMENTS=<the value sign printed>`
   and `sudo systemctl restart trace-commons-ingest`.

Later deploys: steps 1, then `sudo TC_ARTIFACT_BUCKET=tc-staging-artifacts-20261021
deploy/pilot-gcp/pull-and-install.sh both <tag>`, which installs and
restarts in place, then `curl -fsS http://127.0.0.1:3907/health` to confirm
`near_ai_staging` is still `true`.

## Not here yet

- The payout rail (design 3.10) stays `disabled`. Its settings are not read
  by the ingest yet; when they are, rerun step 4 with
  `TC_NEAR_AI_PAYOUT_MODE=dry_run` and `TC_NEAR_AI_PAYOUT_CREDENTIAL_ID`,
  `_SENDER_ORG`, `_SOURCE` and `_ACCOUNT_CEILING_NANO`, which also reads
  `tc-staging-nearai-reseller-key`.
- The nightly `pg_dump` to the staging bucket (design 3.2).
- The staging environment runbook, `docs/operator/staging-environment.md`
  (plan Task 12), which will take this order over.
