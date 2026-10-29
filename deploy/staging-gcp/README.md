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
  issuer key files (0640 root:tracecommons), the kept measurement pins
  (`near-ai-expected-measurements`, 0644, not secret), the site files under
  `/var/lib/tracecommons-site`, and the Caddyfile and unit files into the
  operator's `~/deploy` for `deploy.sh`. It prints no secret value, and
  refuses by name, writing nothing, when a secret is missing or malformed.
- `near-ai-rtmr3-allowlist.json` -- the RTMR3 and compose-action allowlist
  that `trace-commons-staging-pins sign --allowlist` puts inside the signed
  NEAR AI pin set (owner decision S16, spec 5.2). A staging app earns a
  model key only if that model entry's replayed RTMR3 events carry these
  values and every post-boot compose action is listed here.

  **As committed it is a placeholder that refuses everything**: its values
  are empty and its lists are empty. An empty value matches no event (not
  even one with an empty payload), no progress marker is listed and no
  compose action is listed, so every replayed log and every compose-manager
  record is refused and a pin set signed with it earns no key. `sign`
  accepts this placeholder or a fully filled-in allowlist (all four values,
  all four markers, at least one compose-manager image and at least one
  compose file) and refuses anything in between as
  `allowlist_incomplete`. The
  operator fills it from the `--review` file of an `observe` run against
  NEAR AI staging, in a reviewed pull request, after checking each value
  (app id, KMS, OS image, key provider, the four progress markers, each
  compose-manager image digest, and each compose file, fetched from NEAR
  AI's repository at its commit, its SHA-256 checked and the images it
  names read; owner decision S17). Every later change is
  a reviewed pull request too, so a new image or compose file fails
  closed until someone has looked at it. The procedure is in
  `docs/operator/staging-pin-signing.md`.

- `backup-db.sh`, `restore-check.sh` and `systemd/tc-staging-db-*` -- the
  nightly database backup to the private bucket and its weekly restore
  check (design 3.2). Installed at bring-up step 9.

`scripts/ci/test-deploy-render.sh` renders all of it with dummy values, no
network, on every PR. `scripts/ci/test-staging-db-backup.sh` runs the
backup and the restore check against stubs, on every PR.

## Secrets

All in Secret Manager in `tracecommons-pilot-2026`, labelled `env=staging`.
`render-env.sh` reads each one marked *runtime*; grant
`roles/secretmanager.secretAccessor` on those to
`tc-staging-runtime@tracecommons-pilot-2026.iam.gserviceaccount.com` and on
nothing else.

| Secret | Exists | Runtime | Becomes | How to make it |
|---|---|---|---|---|
| `tc-staging-nearai-inference-key` | yes | yes | `TRACE_COMMONS_NEAR_AI_API_KEY`, `TRACE_NEAR_AI_PRIVACY_API_KEY` | a NEAR AI staging API key |
| `tc-staging-db-app-password` | yes | yes | the `app` role's password in `DATABASE_URL` (ingest and issuer) | already set on the role. 16 or more letters and digits; any other character refuses (see below) |
| `tc-staging-db-login-resolver-password` | yes | yes | `TRACE_COMMONS_LOGIN_RESOLVER_DATABASE_URL` | set on `tc_login_resolver_login` at bring-up step 6. 16 or more letters and digits |
| `tc-staging-db-gate-driver-password` | yes | yes, read only when `TC_SCORING_DRIVER=1` | the password in `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` | set on `tc_gate_driver_login` (see [Scoring driver and scoring award](#scoring-driver-and-scoring-award)). 16 or more letters and digits |
| `tc-staging-scorer-production-nearai-key` | **no** | yes, read only when `TC_SCORER_PRODUCTION_HOST` is set | `TRACE_COMMONS_NEAR_AI_SCORER_API_KEY` | a NEAR AI **production** API key that can call the scorer's direct-completions host. Its account pays for staging's scoring (see [Scoring on a production completions host](#scoring-on-a-production-completions-host)). One token of letters, digits and `._~+/=:-` |
| `tc-staging-nearai-reseller-key` | yes | yes, read only when payout is on | `TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL` | the reseller org's credential on NEAR AI staging |
| `tc-staging-payout-control-token` | **no** | yes, read only when payout is `live` | `TRACE_COMMONS_PAYOUT_CONTROL_TOKEN` | `openssl rand -hex 32` (32 or more characters of letters, digits and `._~+/=-`, one line). The operator's token for payout halt, resume and holds; a `live` rail refuses to render, and the ingest to boot, without it |
| `tc-staging-gate-master-key` | **no** | yes | `TRACE_COMMONS_GATE_SERVICE_MASTER_KEY` | `openssl rand -hex 32` (exactly 64 hex characters) |
| `tc-staging-artifact-key` | **no** | yes | `TRACE_COMMONS_ARTIFACT_KEY_HEX` | `openssl rand -hex 32` (64 hex characters). Losing it loses every stored artifact |
| `tc-staging-near-account-index-pepper` | **no** | yes | `TRACE_COMMONS_NEAR_ACCOUNT_INDEX_PEPPER` | `openssl rand -base64 32` (base64 of at least 32 bytes) |
| `tc-staging-near-account-name-kek-key` | **no** | yes | `TRACE_COMMONS_NEAR_ACCOUNT_NAME_KEK_KEY` | `openssl rand -hex 32` (64 hex characters). Its presence is required when sign-up is on; under `TRACE_COMMONS_KEK_PROVIDER=gcp_cloud_kms` the KMS key `tc-staging/kek-v1` is what seals account names, and this value is not used as key material |
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

`render-env.sh` checks the shape of every secret that goes into a
`KEY=value` line or a URL, and refuses by the secret's name: no secret but
the two keypairs may hold a newline (it would add a line of its own to
`ingest.env`), the hex keys must be 64 hex characters, the pepper base64, the
NEAR AI keys one token, and the two database passwords letters and digits
only, because they go into `postgres://` URLs unencoded. If the existing
`tc-staging-db-app-password` refuses, give the role a new one and add it as
a new version, the value reaching `psql` on stdin:

```bash
pw="$(openssl rand -hex 24)"
printf '%s' "$pw" | gcloud secrets versions add tc-staging-db-app-password \
  --project "$P" --data-file=-
printf 'ALTER ROLE app PASSWORD %s;\n' "'$pw'" | sudo -u postgres psql -v ON_ERROR_STOP=1
unset pw
```

Rotation: add a secret version, rerun `render-env.sh`, restart the service.
The rerun keeps the measurement pins from step 8 (it reads them back from
`/etc/tracecommons/near-ai-expected-measurements`).
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
   Because Ubuntu's `postgresql.service` stays active when the cluster under
   it has failed, it also requires `pg_isready -h 127.0.0.1 -p 5432` to pass
   before starting anything.
   `TC_ENV_INSTALL=0` keeps the files `render-env.sh` wrote.
   `TC_EXPECT_STAGING=1` stops the deploy unless the new ingest's `/health`
   reports `near_ai_staging: true`, and then stops the ingest (on a later
   run of `deploy.sh`, it puts back the binary it kept and restarts it). It
   ends with `STAGING DEPLOY OK`.

6. **Create the login-resolver role** (`docs/operator/login-resolver-role.md`).
   Until it exists every login redeem fails closed. The resolver pool
   connects lazily, so the ingest needs no restart. The passwords reach
   `psql` on stdin and in its environment, never on a command line:

   ```bash
   pw="$(gcloud secrets versions access latest --secret tc-staging-db-login-resolver-password \
     --project tracecommons-pilot-2026)"
   { printf '\\set pw %s\n' "'$pw'"; cat <<'SQL'
   CREATE ROLE tc_login_resolver_login LOGIN NOBYPASSRLS PASSWORD :'pw';
   GRANT trace_login_resolver TO tc_login_resolver_login;
   SQL
   } | PGPASSWORD="$(gcloud secrets versions access latest --secret tc-staging-db-app-password \
         --project tracecommons-pilot-2026)" \
       psql -h 127.0.0.1 -U app -d tracecommons -v ON_ERROR_STOP=1
   unset pw
   ```

   `app` holds the admin option on `trace_login_resolver` because its
   migration created the role (PostgreSQL 16 gives the creator of a role that
   option).

   **Payout rail grant (before the rail leaves `disabled`).** Migration V132
   makes two NOLOGIN roles the payout pass assumes, `trace_payout_enumerator`
   (lists tenants with a link) and `trace_payout_config_writer` (appends rate
   epochs and control rows), and deliberately grants neither to
   `trace_ingest_runtime`. The operator grants both to the login the ingest
   connects as, here `app`, with no inheritance, so only the pass can use them:

   ```bash
   PGPASSWORD="$(gcloud secrets versions access latest --secret tc-staging-db-app-password \
       --project tracecommons-pilot-2026)" \
     psql -h 127.0.0.1 -U app -d tracecommons -v ON_ERROR_STOP=1 <<'SQL'
   GRANT trace_payout_enumerator, trace_payout_config_writer
       TO app WITH INHERIT FALSE, SET TRUE;
   SQL
   ```

   `app` created both roles by migration, so it holds the admin option that
   `GRANT` needs. Without the grant an enabled rail reports
   `payout_store_failed` on every tick. The pass also reads `trace_accounts`,
   `trace_credit_holds` and the settlement tables; `app` owns them on staging,
   so no further grant is made. Check that before `dry_run` rather than
   assuming it (an enabled rail that reports `payout_store_failed` after the
   grant above is a missing table privilege).

   **Referral resolver grant (before referrals are on).** Migration V142
   makes `trace_referral_resolver`, the NOLOGIN role that resolves a referral
   code across tenants, and grants it to no one. Grant it to `app` the same
   way, before `TRACE_COMMONS_REFERRAL_PROGRAM_START` is set or
   `TRACE_COMMONS_NEAR_AI_REFERRAL_MODE` leaves `disabled`
   (`docs/operator/referral-resolver-role.md`):

   ```bash
   PGPASSWORD="$(gcloud secrets versions access latest --secret tc-staging-db-app-password \
       --project tracecommons-pilot-2026)" \
     psql -h 127.0.0.1 -U app -d tracecommons -v ON_ERROR_STOP=1 <<'SQL'
   GRANT trace_referral_resolver TO app WITH INHERIT FALSE, SET TRUE;
   SQL
   ```

   Without it, ingest refuses to start with referrals on
   (`referral_resolver_unprovisioned`).

7. **Verify.**

   ```bash
   curl -fsS https://ingest.staging.tracecommons.ai/health | jq .near_ai_staging   # true
   # Expected today: {"ready":false,"near_ai_login_ready":false,...}. See below.
   curl -fsS https://ingest.staging.tracecommons.ai/v1/account/near/provision/capabilities
   # The v2 contract reports the staging login path ready:
   # "near_ai_login_ready": true.
   curl -fsS https://ingest.staging.tracecommons.ai/v1/account/near/provision/capabilities/v2
   scripts/check-aasa.sh https://staging.tracecommons.ai KXSWJN7WY8.ai.tracecommons.shell.staging
   # 200, not 503 attestation_signing_key_unconfigured: proves systemd read
   # the multi-line attestation keys out of ingest.env. This is the only
   # check of that; no CI job parses ingest.env the way systemd does. Do not
   # skip it.
   curl -fsS https://ingest.staging.tracecommons.ai/.well-known/trace-commons-attestation-keyset.json
   ```

   **Sign-up from the app does not work yet.** The contributor daemon the
   macOS app drives signs up through the old routes
   (`/v1/account/near/provision/capabilities`, then
   `/v1/account/near-ai/provision/start` and `/finish`). It requires the
   capabilities response to carry a redaction witness, and the old
   readiness check requires `TRACE_COMMONS_NEAR_PROVISIONING_WITNESS_JSON`.
   A staging build compiles out only the admission ledger, not that check.
   Staging runs no witness (design 3.12), so the old capabilities response
   reads `near_ai_login_ready:false` and every tester's sign-up fails with
   `near_ai_enroll_commons_unsupported`. Do not set a placeholder witness:
   the app would then call that host. Sign-up opens once the staging client
   or server change lands (design section 11). Only the `/v2` routes, which
   no client calls yet, are ready.

   A staging ingest that names a production NEAR AI host does not reach
   this step: it refuses to boot with
   `missing_control:near_ai_staging_host_coherence`, naming the setting.

8. **Pins.** Sign the pin set on an operator machine
   (`docs/operator/staging-pin-signing.md`), copy `current.json` to
   `/var/lib/tracecommons-site/near-ai-pins/current.json` (0644), then rerun
   step 4 with `TC_NEAR_AI_EXPECTED_MEASUREMENTS=<the value sign printed>`
   and `sudo systemctl restart trace-commons-ingest`. `render-env.sh` keeps
   that value in `/etc/tracecommons/near-ai-expected-measurements` and every
   later render reads it back, so a rerun without the variable keeps the
   pins; give the variable again to replace them, or delete the file to
   unpin.

9. **Database backups.** From a checkout of the commit being deployed:

   ```bash
   sudo install -m 0755 -o root -g root deploy/staging-gcp/backup-db.sh \
     /usr/local/sbin/tc-staging-db-backup
   sudo install -m 0755 -o root -g root deploy/staging-gcp/restore-check.sh \
     /usr/local/sbin/tc-staging-db-restore-check
   sudo install -m 0644 -o root -g root \
     deploy/staging-gcp/systemd/tc-staging-db-backup.service \
     deploy/staging-gcp/systemd/tc-staging-db-backup.timer \
     deploy/staging-gcp/systemd/tc-staging-db-restore-check.service \
     deploy/staging-gcp/systemd/tc-staging-db-restore-check.timer \
     /etc/systemd/system/
   sudo systemctl daemon-reload
   # The first backup and restore check, now. Each blocks until it is done
   # and fails if the script does.
   sudo systemctl start tc-staging-db-backup.service
   sudo journalctl -u tc-staging-db-backup.service -n 5 --no-pager
   #   backup-db: uploaded gs://.../backups/postgres/<ts>.dump bytes=... tables=... sha256=...
   sudo systemctl start tc-staging-db-restore-check.service
   sudo journalctl -u tc-staging-db-restore-check.service -n 5 --no-pager
   #   restore-check: OK <ts> tables=... rows=... sha256=...
   sudo systemctl enable --now tc-staging-db-backup.timer tc-staging-db-restore-check.timer
   systemctl list-timers 'tc-staging-db-*'
   ```

   The backup runs nightly at 03:17 UTC and writes `<ts>.dump` (`pg_dump
   -Fc`), `<ts>.counts` (every table's row count, taken in the dump's own
   snapshot) and, last, `<ts>.dump.sha256` to
   `gs://tc-staging-artifacts-20261021/backups/postgres/`. A backup without
   its `.dump.sha256` is incomplete and the restore check skips it. The
   restore check runs Sundays at 05:17 UTC: it verifies the newest complete
   backup's sha256, restores it into a throwaway `tc_restore_check_<time>`
   database, requires every table's row count to equal the dump-time count,
   reports how far the live database has drifted since (not a failure), and
   drops the throwaway database. A failure leaves the unit failed
   (`systemctl --failed`); there is no alert yet.

   Both run as root and reach PostgreSQL only as `postgres`, through
   `runuser -u postgres --` over the local socket. They must: every Trace
   Commons table has FORCE ROW LEVEL SECURITY, which binds the table owner
   too, so `pg_dump` as `app` refuses ("query would be affected by
   row-level security policy"), and with `--enable-row-security` it would
   write only the rows `app` can see. Both scripts refuse a session that is
   not a superuser.

   No client-side encryption: the bucket is private, only the runtime
   service account holds a role on it, and it encrypts at rest under
   `tc-staging/kek-v1`. Nothing is ever written to a public bucket.

   **Retention** (design 3.2: 14 days) is a bucket lifecycle rule, set by an
   operator, never by these scripts. The bucket also holds the binaries and
   the artifact store, so every rule names the prefix, and because the
   bucket is versioned a second rule removes the noncurrent versions. Look
   at what is there first: `--lifecycle-file` replaces the whole
   configuration.

   ```bash
   B=gs://tc-staging-artifacts-20261021
   gcloud storage buckets describe "$B" --format='json(lifecycle_config)'
   cat >lifecycle.json <<'JSON'
   {"rule": [
     {"action": {"type": "Delete"},
      "condition": {"age": 14, "matchesPrefix": ["backups/postgres/"]}},
     {"action": {"type": "Delete"},
      "condition": {"isLive": false, "daysSinceNoncurrentTime": 7,
                    "matchesPrefix": ["backups/postgres/"]}}
   ]}
   JSON
   # Add any rules the describe printed to lifecycle.json first.
   gcloud storage buckets update "$B" --lifecycle-file=lifecycle.json
   ```

   **Restoring for real.** `pg_dump` carries no roles. On this cluster the
   roles exist and ownership and grants restore as they were. On a new
   cluster, create `app` and the narrow roles first
   (`docs/operator/login-resolver-role.md`), then, with the ingest and
   issuer stopped:

   ```bash
   TS=<timestamp of a backup that has its .dump.sha256>
   P=gs://tc-staging-artifacts-20261021/backups/postgres
   gcloud storage cp "$P/$TS.dump.sha256" "$P/$TS.dump" "$P/$TS.counts" .
   sha256sum -c "$TS.dump.sha256"
   sudo -u postgres createdb -O app -T template0 tracecommons
   sudo -u postgres pg_restore --exit-on-error -d tracecommons <"$TS.dump"
   ```

   Then compare the row counts with `$TS.counts` the way `restore-check.sh`
   does before starting the services.

Later deploys: step 1, then

```bash
sudo TC_ARTIFACT_BUCKET=tc-staging-artifacts-20261021 TC_EXPECT_STAGING=1 \
  deploy/pilot-gcp/pull-and-install.sh both <tag>
```

which installs and restarts in place, reads the new ingest's `/health`, and,
unless it reports `near_ai_staging: true`, puts the previous ingest binary
back, restarts it and fails. The issuer, installed first, is not rolled back
(it has no channel).

## Scoring driver and scoring award

Two opt-in render settings let scored traces earn a settling award, for the
settlement test. A render without them is unchanged.

| Render variable | Writes | Notes |
|---|---|---|
| `TC_SCORING_DRIVER=1` | `TRACE_COMMONS_PERPLEXITY_DRIVER_ENABLED=true` and `TRACE_COMMONS_GATE_DRIVER_DATABASE_URL` | The URL uses the LOGIN role `tc_gate_driver_login` on the local `tracecommons` database. Reads `tc-staging-db-gate-driver-password`, which must exist, or the render refuses. Any value but `1` refuses. |
| `TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` | `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA` | A non-negative decimal. Replaces the pilot template's `0`; the rendered file holds one value. |
| `TC_NOVELTY_UTILITY_SETTLE_FROM` | `TRACE_COMMONS_NOVELTY_UTILITY_SETTLE_FROM` | An RFC 3339 UTC time such as `2026-11-01T00:00:00Z`. Required when the delta is above 0; only awards written at or after it settle. Written only when the delta is above 0. |

With a delta above 0 the render also writes
`TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE=true`, and refuses
without the cut-over, mirroring the ingest's boot refusals
(`novelty_utility_credit_requires_production_gate`,
`novelty_utility_settle_from_missing`, `novelty_utility_settle_from_invalid`).
The award is settlement-eligible: a delta above 0 is paid at the next live
settlement run (`docs/operator/env-reference.md`).

Create the driver's role once, after the first boot and the migrations
(`docs/operator/perplexity-scoring-driver.md`), the password reaching `psql`
on stdin as in step 6:

```bash
pw="$(gcloud secrets versions access latest --secret tc-staging-db-gate-driver-password \
  --project tracecommons-pilot-2026)"
{ printf '\\set pw %s\n' "'$pw'"; cat <<'SQL'
CREATE ROLE tc_gate_driver_login LOGIN NOBYPASSRLS PASSWORD :'pw';
GRANT trace_gate_driver TO tc_gate_driver_login;
SQL
} | PGPASSWORD="$(gcloud secrets versions access latest --secret tc-staging-db-app-password \
      --project tracecommons-pilot-2026)" \
    psql -h 127.0.0.1 -U app -d tracecommons -v ON_ERROR_STOP=1
unset pw
```

Then rerun step 4 with, for example, `TC_SCORING_DRIVER=1`,
`TC_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=<points>` and
`TC_NOVELTY_UTILITY_SETTLE_FROM=<time>`, and restart the ingest. The driver
issues its award as `TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF`; with
`TRACE_COMMONS_CREDIT_SETTLEMENT_CENTRAL_ISSUER_PRINCIPAL_REFS` set and that
issuer unset or unlisted, every award is withheld as `central_issuer_denied`.

## Scoring on a production completions host

NEAR AI staging serves no endpoint the scorer can use: its gateway refuses
`echo` on `/v1/completions`, and it has no per-model direct-completions
host. By the owner's exception of 2026-10-24 (design 3.9), staging may score
against a NEAR AI **production** direct-completions host with a production
key. A render without these settings is unchanged.

| Render variable | Writes | Notes |
|---|---|---|
| `TC_SCORER_PRODUCTION_HOST` | `TRACE_COMMONS_NEAR_AI_BASE_URL`, `TRACE_COMMONS_NEAR_AI_SCORER_PRODUCTION_HOST=true`, and `TRACE_COMMONS_NEAR_AI_SCORER_API_KEY` from `tc-staging-scorer-production-nearai-key` | Exactly `https://<model>.completions.near.ai/v1`: https, one DNS label, no port, no userinfo, no trailing slash. Anything else refuses. The secret must exist, or the render refuses. |
| `TC_SCORER_MODEL` | `TRACE_COMMONS_NEAR_AI_MODEL` | The host's model id, such as `Qwen/Qwen3.8-27B`. Required with the host, and the host with it. |

For example, with the owner's secret in place, rerun step 4 with
`TC_SCORER_PRODUCTION_HOST=https://qwen3-8-27b.completions.near.ai/v1` and
`TC_SCORER_MODEL=Qwen/Qwen3.8-27B`, and restart the ingest. It logs one line
naming the control `near_ai_scorer_production_host` at boot, and no host or
key.

What this changes, and what it does not:

- **Cost.** Every scoring request is billed to the production NEAR AI
  account that owns the key in `tc-staging-scorer-production-nearai-key`.
  So is every operator-run attestation drill, which sends one paid chat
  completion to the scorer's host.
- **Data.** Staging traces are sent to the production TEE-hosted model on
  that host for scoring. They leave the staging channel for that one call.
- **Keys.** The production key is sent only to the scorer's host: scoring,
  and the attestation drill's report, completion and receipt there. The
  staging key (`tc-staging-nearai-inference-key`) stays the key for every
  staging host: the attestation registry, the privacy filter, login and
  payout. The ingest refuses to boot with the exception and no scorer key.
- **Hosts.** Only the scorer's base is exempt. The ingest still refuses to
  boot when any other NEAR AI host it calls is not the staging host
  (`near_ai_staging_host_coherence`), and refuses the exception on a host
  that is not a production direct-completions host. A production build
  refuses `TRACE_COMMONS_NEAR_AI_SCORER_PRODUCTION_HOST` at any value.
- **Attestation.** Scoring itself checks no attestation, on staging or in
  production. The scorer's host is attested by the operator-run drill
  (`POST /v1/admin/near-attestation-drill`), which, as in production, asks
  that host for its own `/attestation/report`, verifies the TDX quote with
  Intel collateral, checks the nonce, and compares the measurements with
  `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS`. That one pin set is also
  what the key-drift probe checks the staging gateway's registry against,
  and what the ingest publishes to the staging app. If the production host's
  measurements differ from staging's, one of the two drills reports a
  mismatch; neither check is relaxed to hide it.

## Not here yet

- The payout rail (design 3.10) stays `disabled` until the owner turns it
  on. To turn it on, in order: install the settlement table (the file that
  carries the payout rate, from the operator, not committed) at an absolute
  path the `tracecommons` user can read; run the payout rail grant in step 6;
  then rerun step 4 with `TC_NEAR_AI_PAYOUT_MODE=dry_run`,
  `TC_NEAR_AI_PAYOUT_CREDENTIAL_ID`, `_SENDER_ORG`, `_SOURCE` (`payment` for
  an org funded with paid credit; no default), `_ACCOUNT_CEILING_NANO` and
  `TC_CREDIT_SETTLEMENT_TABLE_PATH`, which also reads
  `tc-staging-nearai-reseller-key` (an `oc_` credential, `oc_` and 64 hex).
  The ingest then refuses to boot unless sign-up provisioning is on with the
  account-index pepper and account-name KEK set (the link routes seal each
  recipient under them); the staging template already sets all three. Values
  come only from the render's environment and Secret Manager.
- Before `TC_NEAR_AI_PAYOUT_MODE=live`, create the payout control token once
  (the operator does this; nothing here creates it):

  ```bash
  openssl rand -hex 32 | tr -d '\n' | new_secret tc-staging-payout-control-token
  ```

  with `new_secret` from [Secrets](#secrets). It is the only credential
  `POST /v1/admin/payout-rails/near_ai/halt` and `.../resume` accept, so
  keep a way to read it when you need to halt. A `live` render refuses
  without it (32 or more characters on one line), and the ingest refuses to
  boot a `live` rail without it (`payout_control_token_unconfigured`).
  `dry_run` sends nothing and neither reads nor needs it. Rotate it like any
  other secret: add a version, rerun step 4, restart.
- Alerting on a failed backup or restore check: today a failure shows only
  as a failed unit (`systemctl --failed`) and in the journal.
- Open sign-up from the app (step 7).
- The staging environment runbook, `docs/operator/staging-environment.md`
  (plan Task 12), which will take this order over.
