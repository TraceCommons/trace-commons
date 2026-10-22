# Internal staging environment -- design

Date: 2026-10-21. Owner: Zaki. Status: draft, nothing built. The
implementation plan is
[`../plans/2026-10-21-staging-environment.md`](../plans/2026-10-21-staging-environment.md).
Every `file:line` below is against `origin/main` at `b8166b6b7` unless it
names another branch.

This design builds a staging copy of the sign-in, settlement and Private AI
path that internal testers use from a separate macOS app. It builds on, and
does not repeat:

- [`2026-10-21-nearai-credit-payout-design.md`](2026-10-21-nearai-credit-payout-design.md)
  (draft PR #1532, "the payout design"): owner decision D9, section 2
  (staging operator steps), section 8 (staging test plan) and section 12
  item 9. Until #1532 merges, that link resolves only on its branch.
- Task 7a of [`../plans/2026-10-21-nearai-credit-payout.md`](../plans/2026-10-21-nearai-credit-payout.md),
  which switches every NEAR AI host in the daemon under `near-ai-staging`.
- `scripts/ci/check-release-staging-feature.sh` on the
  `payout-rail-near-ai-client` branch (draft PR #1542), which refuses the
  `near-ai-staging` feature in every release, packaging and deploy file.

Nothing here contradicts D9. Where the code turned out to differ from the
decisions as briefed, section 11 says so.

## Owner decisions

All dated 2026-10-21. The owner's words are quoted.

- **(S1) A staging build of the whole path, and everyone signs in to it.**
  "What we need is a stageing build of the sign in, settlement and private
  inference system" and "what's the best way to setup servers and have
  binaries for internal testing where everyone has to sign into staging".
- **(S2) macOS only.** "Mac only I think". Testers get the native macOS app.
  No GTK, Windows or Tauri staging build.
- **(S3) A new server.** Staging server: "New VM, pilot project". A new small
  VM in the existing GCP project, not the pilot host. It has its own
  PostgreSQL, its own upload-claim issuer keys and its own subdomain, runs an
  ingest built with `near-ai-staging`, and shares nothing with the pilot: no
  database, keys, tenants or credit.
- **(S4) Signed pins.** Staging attestation: "Signed pins from our server".
  The staging app compiles in one staging pin-signing public key and accepts
  only pin sets signed by it. Our staging server publishes the current signed
  set. An operator, or a script, re-signs after each NEAR AI staging
  redeploy. There is never a switch that skips verification. Production
  builds never contain the staging pin key and never accept a signed staging
  pin set.
- **(D9, inherited)** One `near-ai-staging` build feature switches every NEAR
  AI host together: the daemon's sign-in and credential API, Private AI
  inference and its receipt endpoint, the billing dashboard links, the
  attestation pins and the server's transfer client. Attestation stays on and
  fails closed. The release check refuses the feature in shipped builds.

- **(S5) Admission by allow-list.** Asked who can sign in: "Allow-list of
  NEAR AI accounts". A staging NEAR AI account on a list the operator keeps
  is admitted without an invite; any other staging NEAR AI account is
  refused (3.12).
- **(S6) DMG on a GitHub pre-release.** "GitHub pre-release". A `staging-*`
  pre-release in the public repo, never marked latest (6.1).
- **(S7) A staging label in the window.** "Yes, a small staging label". Its
  wording still goes through the copy review in the PR that adds it (4.8).
- **(S8) Manual re-signing.** "Manual for now". An operator runs the re-sign
  tool after a NEAR AI staging redeploy (5.4).
- **(S9) Built infrastructure.** Already created in the pilot project on
  2026-10-21 at the owner's request ("Create the VM, setup the secrets
  manager setup postgres etc"): the VM `tc-staging-host` (e2-standard-4) on
  a reserved address, the `tc-staging-runtime` SA, firewall rules for the
  `tc-staging` tag, the `tc-staging/kek-v1` key, a versioned CMEK bucket
  `tc-staging-artifacts-20261021`, PostgreSQL 16 on the VM, and the DNS
  names `staging.tracecommons.ai`, `ingest.staging.tracecommons.ai` and
  `issuer.staging.tracecommons.ai`. Secret names are in 3.7.
- **(S10) Staging has its own IronWire home.** "okay new folder"
  (2026-10-22, on #1546 review finding 6). The staging app's proxy, ledger,
  admission token and tool connections live apart from production's, so
  neither app reads the other's inference rows or reconfigures the other's
  proxy (4.2, IronWire rows).

Everything else here is a proposal. Section 10 lists what is still open.

## Verified on 2026-10-21

### Our code

NEAR AI hosts compiled into the contributor (the daemon, the FFI dylib and
the bundled `near-ai` CLI all link it):

| Where | What | Task 7a covers it |
|---|---|---|
| `crates/trace-commons-contributor/src/daemon/nearai_credential/api.rs:31` | `CLOUD_API_ORIGIN`, sign-in and credential API | yes (#1542 switches it) |
| `crates/trace-commons-contributor/src/managed/route.rs:36` | `near_ai_base_url()`, Private AI inference and model list (`route.rs:62`, `daemon/managed.rs:587`) | yes |
| `crates/trace-commons-contributor/src/daemon/nearai_credential/funding.rs:24` | `CLOUD_ORIGIN`, billing dashboard | yes |
| `crates/trace-commons-contributor/src/external_url.rs:57` | dashboard link prefix | yes |
| `crates/trace-commons-contributor/src/envelope.rs:113` | client-side NEAR AI privacy-filter base default | listed as "receipt endpoint"; it is the privacy filter |
| `crates/trace-commons-contributor/src/routing/proof_attestor.rs:98` | `NEAR_AI_GATEWAY`, attestation report host | **no** |
| `crates/trace-commons-contributor/src/skill_loop/evaluation/transport.rs:19` | `CLOUD_API_BASE`, evaluation calls | **no** |
| `crates/trace-commons-protocol/src/privacy_filter_near_ai.rs:19` | `DEFAULT_BASE_URL`, privacy filter (server and client) | **no** |
| IronWire `ironwire_upstream/src/openai_chat.rs:22` (rev `a2ae5d97`, pinned at `crates/trace-commons-contributor/Cargo.toml:773`) | `NEARAI_DEFAULT_BASE_URL` | listed as "if it holds its own host"; it does |

Trace Commons hosts and app identity compiled in:

| Where | What |
|---|---|
| `crates/trace-commons-contributor/src/daemon/native_identity.rs:86` | `NATIVE_ACCOUNT_ORIGIN = https://ingest.tracecommons.ai`, the one origin trusted before any config exists |
| `crates/trace-commons-contributor/src/daemon/native_identity.rs:229` | passkey RP ID must be `tracecommons.ai` |
| `crates/trace-commons-contributor/src/daemon/native_identity.rs:377` | passkey client-data origin must be `https://tracecommons.ai` |
| `macos/Sources/TCShellCore/NativePasskey.swift:108` | Swift also requires RP ID `tracecommons.ai` |
| `macos/entitlements.plist` | application id, keychain group `KXSWJN7WY8.ai.tracecommons.shell`, `webcredentials:tracecommons.ai` |
| `crates/trace-commons-contributor-platform/src/os_secret_store.rs:21` | `ACCESS_GROUP`; parity test against the entitlements at `:517-547` |
| `crates/trace-commons-contributor-platform/src/os_secret_store.rs:14,36,111` | keychain service names |
| `crates/trace-commons-contributor/src/config.rs:900` | `STATE_DIR_NAME = "trace-commons"` (`platform_default_dir` at `:924`) |
| `crates/trace-commons-contributor/src/config.rs:105` | `DAEMON_SOCK_FILE` inside the state directory |
| `macos/Sources/TCShellCore/StateDirectory.swift:138` | Swift's own copy of the default state directory; `TRACE_COMMONS_CONTRIBUTOR_DIR` overrides it (`:131`) |
| `macos/Sources/TraceCommonsApp/WindowBreadcrumb.swift:30` | the one `Logger` subsystem, `ai.tracecommons.shell` |
| `macos/scripts/info-plist.sh:31` | Sparkle feed URL |
| `macos/scripts/info-plist.sh:56-58` | `CFBundleName`, `CFBundleDisplayName` `Near.ai`, `CFBundleIdentifier` `ai.tracecommons.shell` |
| `macos/scripts/info-plist.sh:93-97` | URL type `tracecommons` |
| `macos/scripts/make-app-bundle.sh:25` | `Near.ai.app`; `:66-69` builds the FFI dylib and `near-ai` with `CARGO_BUILD_ARGS` only (no feature pass-through today) |
| `macos/scripts/make-release-dmg.sh:74-78` | provisioning profile and entitlements checked against bundle id `ai.tracecommons.shell` |
| `crates/trace-commons-contributor/src/update/endpoint.rs:9-25` | CLI self-update manifest URL; an unset `TRACE_COMMONS_UPDATE_PUBLIC_KEY_HEX` refuses every update |
| `scripts/check-aasa.sh` | AASA smoke; the apex's file names only the production app |

Attestation pins today:

- **There are no static receipt-signer pins on the device.** A receipt's
  signer is earned per request: `proof_attestor.rs:8-20` fetches
  `/attestation/report` with a fresh nonce, verifies the TDX quote, checks
  the measurements against a pinned set, and accepts only the per-model
  `provider_tee` ed25519 keys the quote commits to. The only static input is
  the **measurement set**.
- **Production devices already accept served pins.** `proof_attestor.rs:46-55`:
  ingest publishes its measurement sets at
  `GET /v1/contributors/me/near-ai-measurements`
  (`crates/trace-commons-protocol/src/near_ai_measurements.rs:23`), unsigned;
  its digest "is a change detector, not an integrity guarantee"
  (`near_ai_measurements.rs:48-53`). TLS to ingest is the trust. The env
  override `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS`
  (`proof_attestor.rs:61-70`) wins when set.
- Sets use the `ExpectedMeasurements` syntax, `field=<96 hex>,...`
  (`crates/trace-commons-attestation/src/measurements.rs:190-205`). Only the
  named registers are checked, and an empty set cannot be built. The other
  syntax, `mrtd:<hex>+mrconfigid:<hex>`, belongs to the redaction witness
  (`crates/trace-commons-server/src/redaction_witness/verification.rs`) and
  is not used here.
- The report fetch carries the contributor's NEAR AI key
  (`proof_attestor.rs:155-217`). Both `cloud-api.near.ai` and
  `cloud-stg-api.near.ai` answered `401` to an unauthenticated
  `/v1/attestation/report` on 2026-10-21, so a staging fetch needs a staging
  key. This is a host swap, not a code-path change.

Server:

- `crates/trace-commons-server/src/near_ai_api.rs` (branch
  `payout-rail-near-ai-client`) holds the one server NEAR AI base for login
  introspection and payout, switched by `near-ai-staging`, with
  `near_ai_staging()` for readiness. Nothing reports it yet.
- The scorer base is configured, not compiled: `TRACE_COMMONS_NEAR_AI_BASE_URL`
  (`deploy/pilot-gcp/ingest.env.template:54`), a per-model completions host
  on the pilot. The attestation base defaults to production when unset
  (`ingest.env.template:70-82`). The privacy filter takes
  `TRACE_NEAR_AI_PRIVACY_BASE_URL` (`:229`) and otherwise defaults to
  production (`privacy_filter_near_ai.rs:19`).
- Passkeys: `TRACE_COMMONS_WEBAUTHN_RP_ID`, `_RP_ORIGIN`, `_RP_NAME` together
  (`crates/trace-commons-server/src/config.rs:173-230`); the origin list may
  name the RP ID or its subdomains (`account_passkey.rs:96-103`). The browser
  step-up page is served by ingest (`docs/operator/native-step-up-page.md`).
- The pilot does not read secrets from Secret Manager. Values are rendered
  into `/etc/tracecommons/ingest.env` and `issuer.env` (mode 640) with
  `envsubst` and loaded by `EnvironmentFile=`
  (`docs/operator/pilot-gcp-deployment.md`, "Render artifacts";
  `deploy/pilot-gcp/systemd/trace-commons-ingest.service`). Secret Manager in
  this project holds the release signing keys
  (`scripts/updates/README.md:9,33`).
- `deploy/pilot-gcp/deploy.sh` assumes Cloud SQL through
  `cloud-sql-proxy.service` (which the ingest unit `Requires=`), loopback
  ports 3907 and 3917, and `ingest.` / `issuer.` subdomains of
  `TC_PUBLIC_HOST`. `pull-and-install.sh:30` already takes the artifact
  bucket from `TC_ARTIFACT_BUCKET`.
- `check-release-staging-feature.sh` (branch `payout-rail-near-ai-client`)
  scans `deploy/pilot-gcp/deploy.sh`, `pull-and-install.sh`,
  `cloudbuild.yaml`, `macos/scripts/make-app-bundle.sh`,
  `macos/scripts/make-release-dmg.sh`, every `release-*.yml`, and nine
  shipped manifests. It allows the feature only as `near-ai-staging = []` in
  the server and contributor manifests and as the one forward in ingest.

### NEAR AI staging

Checked live and in NEAR AI's public repositories on 2026-10-21:

- API `https://cloud-stg-api.near.ai` (OpenAPI at `/api-docs/openapi.json`,
  136 paths), UI `https://cloud-stg.near.ai`. Staging serves `/v1/chat/completions`,
  `/v1/responses`, `/v1/signature/{chat_id}`, `/v1/attestation/report`, the
  #1271 transfer, capacity and reconciliation routes, credentials and
  `/v1/users/me`. So inference, receipts and attestation are on the same API
  host as sign-in.
- `/v1/models` lists 74 models, including `Qwen/Qwen3.8-27B` (our scorer) and
  `openai/privacy-filter`.
- `/v1/attestation/pins` is public, but it carries backup-provider pins (two
  third-party providers), not NEAR AI's own TDX measurements. It is **not** a
  pin source for us.
- Staging has its own OAuth apps and database: testers need staging
  accounts, made at `cloud-stg.near.ai`.
- Staging redeploys about 17 minutes after every merge to cloud-api `main`,
  and on a daily rolling update; a build freeze skips tagging on Tuesday and
  Wednesday. A redeploy rotates the JWT signing secret, so a session token
  can start answering `401`. (nearai/infra-docs `docs/cloud-api.md`;
  nearai/cvm-ansible-playbooks `docs/staging-on-plucky.md`.)
- Staging attestation chains to a different dstack KMS than production, and
  measurements change when the image changes. (nearai/cvm-ansible-playbooks
  `vars/cloud_api_stg.yaml`, `docs/staging-on-plucky.md`.)

## 1. Goal and non-goals

Goal: an internal tester installs "Near.ai Staging" beside the production
app, signs in with a NEAR AI staging account, contributes traces to a
staging server, makes attested Private AI calls on NEAR AI staging, and
receives a NEAR AI staging payout, with no byte, key or credential crossing
into production in either direction.

Non-goals: a staging GTK, Windows or Tauri build (S2); a public beta channel;
staging copies of pilot data; load testing; a staging redaction witness or
self-hosted privacy filter (staging uses NEAR AI's hosted filter, so the
witness path is untested in staging; see open item 9).

## 2. Shape

| | Production | Staging |
|---|---|---|
| App | `Near.ai.app`, `ai.tracecommons.shell` | `Near.ai Staging.app`, `ai.tracecommons.shell.staging` |
| Rust build | default features | `near-ai-staging` |
| NEAR AI | `cloud-api.near.ai`, `cloud.near.ai` | `cloud-stg-api.near.ai`, `cloud-stg.near.ai` |
| Our server | pilot host, `ingest.` / `issuer.tracecommons.ai` | new VM, `ingest.` / `issuer.<staging host>` |
| Device pins | ingest's unsigned document, or the env override | only a set signed by the compiled-in staging key |
| Passkey RP | `tracecommons.ai` | `<staging host>` |
| Updates | production appcast, production Sparkle key | staging appcast, staging Sparkle key |
| Distribution | `release-apps.yml` on a tag | `staging-macos.yml`, manual dispatch only |

`<staging host>` is `staging.tracecommons.ai` (S9). Below it is written
`STAGING_HOST`.

## 3. The staging server

### 3.1 The VM and what it owns

A new VM in project `tracecommons-pilot-2026`, same region as the pilot.
Default shape `e2-standard-4` (4 vCPU, 16 GB), a 50 GB balanced disk, Debian
like the pilot host. The pilot is CPU-starved by the local embedder on two
vCPUs; four keeps staging scoring from timing out at tester volume. One
tester's uploads are the whole load.

Everything the VM uses is its own:

| Resource | Staging | Never |
|---|---|---|
| Runtime service account | `tc-staging-runtime@` | the pilot's runtime SA |
| PostgreSQL | its own (3.2) | the pilot's Cloud SQL instance |
| KMS key ring | `tc-staging`, key `kek-v1` | `tc-pilot` |
| Artifact bucket | `tc-staging-artifacts-<date>`, versioned, CMEK on `tc-staging` | `tc-pilot-artifacts-*` |
| Binary bucket | a `binaries/` prefix in the staging bucket | the pilot's |
| Issuer signing keys | generated on the VM at first boot (3.4) | any pilot key |
| Secrets | Secret Manager secrets prefixed `staging-` (3.7) | any pilot secret |
| Tenants, accounts, credit | created on staging | copied from the pilot |

IAM is the separation. The staging runtime SA gets
`secretAccessor` on the `staging-*` secrets only, `cloudkms.cryptoKeyEncrypterDecrypter`
on `tc-staging` only, and object admin on the staging bucket only. It has no
role on any pilot resource, and the pilot's SA has none on staging's. Ports
80 and 443 are open; nothing else.

### 3.2 PostgreSQL

Default: PostgreSQL 16 from the distribution, on the VM, listening on
`127.0.0.1:5432`, with a nightly `pg_dump` to the staging bucket kept 14
days. Cheaper than a second Cloud SQL instance, and the same loopback
address ingest already uses, so `DATABASE_URL` keeps its shape.

What that changes: `trace-commons-ingest.service` has
`Requires=cloud-sql-proxy.service`. Staging installs a drop-in that replaces
it with `postgresql.service`, and the parameterised `deploy.sh` (3.6) skips
the proxy step when `TC_DB_MODE=local`. The login-resolver role and the
other narrow roles are created exactly as on the pilot
(`docs/operator/login-resolver-role.md`).

The alternative is the smallest Cloud SQL tier, which leaves the scripts
untouched and costs more. Open item 2.

### 3.3 Hosts, TLS and Caddy

Three DNS names point at the VM: `STAGING_HOST`, `ingest.STAGING_HOST`,
`issuer.STAGING_HOST`. Caddy renders from the pilot's
`deploy/pilot-gcp/Caddyfile.template` with `TC_PUBLIC_HOST=STAGING_HOST`, so
the ingest and issuer blocks, the body limits and all three log filters are
the same file. One extra site block, added to the template behind a
variable so the pilot render is unchanged, serves `STAGING_HOST` itself:

- `/.well-known/apple-app-site-association`, a static file:
  `{"webcredentials":{"apps":["KXSWJN7WY8.ai.tracecommons.shell.staging"]}}`,
  `application/json`, no redirect. `scripts/check-aasa.sh` already takes the
  expected app id and bundle id as arguments, so the same smoke runs against
  staging.
- `/near-ai-pins/current.json`, the current signed pin set (section 5), from
  a directory the operator writes. Public: measurements are public image
  identifiers and the signature is the trust.
- Everything else: 404.

### 3.4 Issuer keys

The staging issuer generates its own EdDSA keyset on first boot, as the
pilot's did. The staging ingest points `TRACE_COMMONS_SIGNED_TOKEN_EDDSA_KEYSET_URL`
and `_ISSUER` at `issuer.STAGING_HOST`
(`ingest.env.template:42-44` with `TC_PUBLIC_HOST=STAGING_HOST`). Because the
keysets differ, a production ingest refuses every staging token and a
staging ingest refuses every production token. That is the server-side
separation of identities, and it needs no new code.

### 3.5 Building the staging ingest

The pilot's `cloudbuild.yaml` cannot build the staging binary: the release
check refuses the feature name in it. Staging gets its own file,
`deploy/staging-gcp/cloudbuild.yaml`, which builds

```
cargo build --release -p trace-commons-ingest --bin trace-commons-ingest \
  --features gcs-client,gcp-kms,near-ai-scorer,near-attestation-collateral,near-ai-staging
cargo build --release --bin trace-commons-upload-claim-issuer
cargo build --release -p trace-commons-server --bin trace-commons-staging-pins \
  --features near-ai-staging
```

and publishes them, with sha256 sidecars, to the staging bucket only. It is
submitted by hand (`gcloud builds submit --config deploy/staging-gcp/cloudbuild.yaml`),
never by a trigger on `main`. The release check lists this file as a
staging file (6.2): it must name the feature, and it must not name any
production bucket or host.

### 3.6 Installing: parameterise, do not fork

`pull-and-install.sh` already reads `TC_ARTIFACT_BUCKET`. `deploy.sh` gains
three variables with pilot defaults, so a pilot run is unchanged:

- `TC_DB_MODE` (`cloudsql` default, `local` for staging): skip the proxy
  steps and install the PostgreSQL drop-in.
- `TC_DEPLOY_LABEL` (`PILOT` default): the closing `PILOT DEPLOY OK` line
  and unit descriptions say which environment was deployed.
- `TC_EXPECT_STAGING` (unset default): when `1`, the script refuses to
  continue unless `GET /health` on the new ingest reports
  `near_ai_staging: true` (3.11), and when unset it refuses if it does. A
  staging binary installed on the pilot, or the reverse, stops the deploy.

Neither file may name the feature (the release check scans both), and
neither needs to: the feature is in the staging Cloud Build file only.

`deploy/staging-gcp/` holds what is staging-only: `cloudbuild.yaml`,
`staging.env.template` (the overrides in 3.8, layered after the pilot
template), the PostgreSQL drop-in, the AASA file, the Caddy site block and
a `README.md` pointing at the runbook. The runbook is
`docs/operator/staging-environment.md`, listed in `docs/operator/README.md`.

### 3.7 Secrets and how they load

Secrets live only in Secret Manager in the pilot project, each named
`tc-staging-*` and labelled `env=staging`, readable only by the staging
runtime SA and the owner. The first four exist (S9):

| Secret | Used by |
|---|---|
| `tc-staging-nearai-inference-key` | scorer and privacy filter (`TRACE_COMMONS_NEAR_AI_API_KEY`, `TRACE_NEAR_AI_PRIVACY_API_KEY`) |
| `tc-staging-nearai-reseller-key` | payout rail (payout design section 2, step 6) |
| `tc-staging-db-app-password`, `tc-staging-db-login-resolver-password` | PostgreSQL roles |
| `tc-staging-nearai-pin-signing-key` | the pin re-sign tool only; **not** readable by the runtime SA (5.5) |
| `tc-staging-sparkle-signing-key` | the staging workflow only (6.1) |

They load the way the pilot's do: rendered into
`/etc/tracecommons/ingest.env` and `issuer.env`, root-owned, group
`tracecommons`, mode 640, read by `EnvironmentFile=`. The one difference is
where the renderer gets the values: `deploy/staging-gcp/render-env.sh` runs
on the VM, reads each value with
`gcloud secrets versions access latest --secret=<name>` under the runtime
SA, and pipes it to `envsubst` without echoing it, writing to a temp file in
`/etc/tracecommons` and renaming. No value reaches a shell history, a log,
the repository or a PR. Rotation is: add a secret version, rerun
`render-env.sh`, restart. systemd `LoadCredential=` is a stricter
alternative that would also need ingest to read files; not proposed now.

### 3.8 Configuration that differs from the pilot

`staging.env.template` sets only these; everything else comes from the
pilot template unchanged, so a pilot setting added later reaches staging
too.

| Setting | Staging value |
|---|---|
| `TC_PUBLIC_HOST`, `TC_WEBAUTHN_RP_ID` | `STAGING_HOST` |
| `TRACE_COMMONS_WEBAUTHN_RP_ORIGIN` | `https://STAGING_HOST,https://ingest.STAGING_HOST` |
| `TRACE_COMMONS_WEBAUTHN_RP_NAME` | `Near.ai Staging` |
| `TRACE_COMMONS_NEAR_AI_BASE_URL` | `https://cloud-stg-api.near.ai/v1` |
| `TRACE_COMMONS_NEAR_AI_MODEL` | `Qwen/Qwen3.8-27B` |
| `TRACE_COMMONS_NEAR_AI_ATTESTATION_BASE_URL` | `https://cloud-stg-api.near.ai/v1` (set, not left to the production default) |
| `TRACE_NEAR_AI_PRIVACY_BASE_URL` | `https://cloud-stg-api.near.ai/v1` |
| `TRACE_NEAR_AI_PRIVACY_MODEL` | `openai/privacy-filter` |
| `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS` | the staging set, written by the re-sign tool (5.9) |
| `TRACE_COMMONS_NEAR_AI_PAYOUT_*` | payout design section 2, step 6; mode starts `dry_run` |
| `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS` | `0`, as on the pilot's template |
| `TRACE_COMMONS_ADMISSION_ENABLED` | `false` (3.12) |
| `TRACE_COMMONS_COMMUNITY_*` | off; staging has no public community page |
| `TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME`, `_REMOTE_OBJECT_STORE_*` | the `tc-staging` key and staging bucket |

Whether staging NEAR AI serves per-model completions hosts like the pilot's
is unknown; the gateway base works for both the scorer and the filter, so
staging uses it. Open item 6.

### 3.9 Every server NEAR AI host is staging: a boot guard

The feature switches only the compiled-in base in `near_ai_api.rs`. The
scorer, attestation and privacy-filter hosts are configuration, and two of
them default to production when unset. A checklist would miss one, so a
`near-ai-staging` ingest checks at boot that every NEAR AI host it will call
-- configured or defaulted -- has host `cloud-stg-api.near.ai`, and refuses
to start with `missing_control:near_ai_staging_host_coherence`, naming the
setting (never the value), if one does not. A default build checks the
reverse: it refuses a NEAR AI setting whose host is the staging host. That
is how "shares nothing" becomes a check rather than a list to remember.

### 3.10 The payout rail in staging

Exactly the payout design's section 2 operator steps and section 8 test
plan, on this VM. The rail's host is the compiled-in staging base. Mode
starts `dry_run`; `allowed_recipient_ids` on the staging credential lists
only testers' staging orgs. The reseller org, its grant and its credential
are made by the owner on NEAR AI staging and held only in
`tc-staging-nearai-reseller-key`.

### 3.11 Readiness

`GET /health` gains `near_ai_staging: bool` from `near_ai_staging()`
(`near_ai_api.rs` on #1542). It is a build fact, not a secret, and the
deploy script (3.6) and the rollout smoke read it. The payout readiness the
payout design adds reports the same flag per rail.

### 3.12 Who can sign in

"Everyone has to sign into staging" (S1), admitted by allow-list (S5). The
staging server keeps a list of staging NEAR AI subjects, operator-edited,
hash-only in logs. A sign-in whose verified NEAR AI subject is on the list
is admitted without an invite, into its own tenant; any other subject is
refused with a safe label, and no tenant or account is created. A NEAR AI
staging account alone admits nobody, since anyone can make one. An empty or
missing list admits nobody (fail closed). Invites still work for the
operator. The list lives in staging config only; a production build never
reads it.

## 4. The staging macOS app

### 4.1 One channel

There is one switch per language, set from one input:

- **Rust:** the `near-ai-staging` feature. Every compiled-in host and
  identity constant moves into one module,
  `crates/trace-commons-contributor/src/channel.rs`, with a
  `#[cfg(feature = "near-ai-staging")]` and a
  `#[cfg(not(...))]` value for each. Every file in the tables above reads
  from it. A ratchet test fails if any other non-test source file in the
  contributor, FFI or platform crates contains a NEAR AI or Trace Commons
  host literal. This is the "one shared host module" Task 7a asks for,
  extended to identity.
- **Bundle:** `TC_CHANNEL=staging` given to `make-app-bundle.sh`,
  `info-plist.sh` and `make-release-dmg.sh`. Each reads its values from one
  file, `macos/channels/staging.env` (and `production.env`, holding today's
  values), and passes the cargo feature through an opaque variable the file
  supplies, so no build script contains the feature name (the release check
  scans them). Swift has no compile-time flag: it reads the channel from the
  dylib (4.3).

Swift never holds its own copy of a channel value. `StateDirectory.swift:138`
and `NativePasskey.swift:108` read theirs from a new FFI call,
`tc_build_channel_json()`, which returns the channel module's values the
app needs.

### 4.2 Identity, item by item

| Item | Today | Staging | Mechanism |
|---|---|---|---|
| Bundle id | `ai.tracecommons.shell` (`info-plist.sh:58`) | `ai.tracecommons.shell.staging` | `staging.env` |
| App and DMG name | `Near.ai` (`info-plist.sh:56-57`, `make-app-bundle.sh:25`, `make-release-dmg.sh:80-81,239`) | `Near.ai Staging` | `staging.env` |
| Icon | `AppIcon` | a staging variant of the same mark (open item 5) | `staging.env` names the `.icon` |
| Keychain access group | `KXSWJN7WY8.ai.tracecommons.shell` (`os_secret_store.rs:21`, `entitlements.plist`) | `KXSWJN7WY8.ai.tracecommons.shell.staging` | the platform crate takes the group as a parameter from `channel.rs`; `macos/entitlements-staging.plist`; the parity test gains a staging twin |
| Keychain service names | `trace-commons.*` (`os_secret_store.rs:14,36,111`) | `trace-commons-staging.*` | `channel.rs`; belt and braces on top of the group |
| State directory | `~/Library/Application Support/trace-commons` (`config.rs:900`, `StateDirectory.swift:138`) | `.../trace-commons-staging` | `channel.rs`; Swift asks the FFI |
| State dir override | `TRACE_COMMONS_CONTRIBUTOR_DIR` | `TRACE_COMMONS_STAGING_CONTRIBUTOR_DIR` | `channel.rs`; a staging build ignores the production variable |
| Daemon socket | `<state dir>/daemon.sock` (`config.rs:105`) | follows the state directory | none needed |
| Managed-route homes | under the state directory | follows it | none needed |
| IronWire home (S10) | `$IRONWIRE_HOME`, else `~/.ironwire` (`private_inference.rs:161`) | `~/.ironwire-staging`; a staging build ignores `IRONWIRE_HOME` and reads `IRONWIRE_STAGING_HOME` instead | `channel.rs`; the daemon passes the home to every IronWire start, ledger read and `connect` |
| IronWire discovery pointer | `~/.ironwire/endpoint.json`, written there whatever `$IRONWIRE_HOME` says (`ironwire_pointer.rs:14-20`) | not used: a staging build never reads the pointer, and accepts only a control URL and token under its own home | `channel.rs`; a test that a staging build ignores a production pointer |
| IronWire control port | IronWire's default (8463 in the pointer example) | a fixed staging port, so both proxies can run at once | `channel.rs` |
| UserDefaults, login item | per bundle id | follow the bundle id | none needed |
| Log subsystem | `ai.tracecommons.shell` (`WindowBreadcrumb.swift:30`) | the bundle id | read `Bundle.main.bundleIdentifier` |
| Native account origin | `https://ingest.tracecommons.ai` (`native_identity.rs:86`) | `https://ingest.STAGING_HOST` | `channel.rs` |
| Passkey RP ID and origin | `tracecommons.ai` (`native_identity.rs:229,377`, `NativePasskey.swift:108`) | `STAGING_HOST` | `channel.rs` |
| Associated domain | `webcredentials:tracecommons.ai` | `webcredentials:STAGING_HOST` | `entitlements-staging.plist` |
| Provisioning profile | `macos/TraceCommons-DeveloperID.provisionprofile` | a new Developer ID profile for the staging App ID (open item 4) | `staging.env` names it; `make-release-dmg.sh:76-78` checks it against the staging bundle id |
| URL scheme | `tracecommons` (`info-plist.sh:93-97`) | `tracecommons-staging` (4.5) | `staging.env`; `deep_link.rs` parses the channel's scheme |
| Sparkle feed and key | `info-plist.sh:31`, `SPARKLE_PUBLIC_ED_KEY` | staging feed URL, staging public key (4.7) | `staging.env` |
| Bundled `near-ai` CLI self-update | signed manifest (`update/endpoint.rs:9-25`) | off: the key is left unset, which refuses every update | the staging workflow never sets it |
| NEAR AI hosts | production | staging | `channel.rs` (D9, Task 7a) |
| Measurement pins | ingest's document or env | signed staging set only (section 5) | `channel.rs` |

### 4.3 The dylib and the bundle must agree

The app is a Swift bundle around a Rust dylib, built in two steps. A staging
bundle carrying a production dylib would show "Near.ai Staging" while
talking to production. At launch, before starting the daemon, the app
compares its Info.plist `TCChannel` key with `tc_build_channel_json()`'s
`channel`, and on a mismatch refuses to start with a fixed error and no
daemon. The bundled `near-ai` CLI prints its channel in `--version`, and
`make-release-dmg.sh` checks all three agree before signing.

### 4.4 A state directory belongs to one channel

`ConfigStore::open` writes a `channel` file into a new state directory and
refuses a directory whose file names the other channel. A directory with a
`contributor.json` and no `channel` file is production (every directory
before this change), so a staging build refuses it. This catches a
hand-set override pointing staging at production's enrollment, which the
separate default directories alone would not.

### 4.5 URL scheme

`tracecommons://` is shared by design between the native app and Tauri;
users install one of them. With both production and staging installed,
LaunchServices would hand a production invite link to whichever app
registered last. So the staging app does not register `tracecommons`. It
registers `tracecommons-staging`, staging invite mail carries that scheme,
and `deep_link.rs` accepts only its own channel's scheme. Testers can also
paste an invite URL, as today.

### 4.6 Passkeys

Staging uses its own RP ID, `STAGING_HOST`. A passkey made for
`STAGING_HOST` is a separate credential from a `tracecommons.ai` passkey on
the tester's authenticator, and a production assertion is never valid on
staging or the reverse. Keeping the apex as staging's RP ID would put
staging and production passkeys in one RP scope, and the apex's AASA would
have to list the staging app, so staging could assert production
credentials. That is the credential-layer version of staging data reaching
production, and it is refused.

The step-up page is served by ingest, so the staging ingest serves its own;
its origin is in the staging RP origin list (3.8).

### 4.7 Updates

A separate appcast, `.../staging/appcast.xml` in the staging bucket, signed
with a separate Sparkle EdDSA key, `tc-staging-sparkle-signing-key`. The staging
app's Info.plist names only the staging feed and staging public key, so a
production appcast can never update it and a staging appcast can never
update production. `scripts/updates/generate-appcast.sh` takes the output
path and key name as arguments rather than gaining a staging branch.

### 4.8 Unmistakable in the UI

Name, Dock and menu-bar icon, and window titles say "Near.ai Staging". The
daemon's status reports `channel: "staging"` (D9 asks for this; Task 7a
wires `near_ai_staging()` into `daemon/ipc.rs`). Whether the app also shows a
persistent label inside its windows is a new notice under
`docs/ui-copy-guide.md` section 9, so it needs the owner's approval for
both its existence and its wording (open item 5). Any staging string lives
in the Rust copy tables (section 8 of the guide), not in Swift.

## 5. Signed pins

### 5.1 What the staging app pins

The measurement sets for NEAR AI staging's model enclaves, nothing else.
Receipt signers stay earned per request (Verified, "Attestation pins
today"): a signed set replaces only the *source* of the sets that
`proof_attestor.rs` checks quotes against. The quote verification, the
nonce, the key binding and every failure rule are unchanged.

### 5.2 The format

A versioned payload, signed as exact bytes so no canonical-JSON rule is
needed:

```json
{
  "payload": "<base64url of the payload bytes>",
  "signature": "<base64url Ed25519 signature>",
  "key_id": "<first 16 hex of SHA-256 of the public key>"
}
```

The payload:

```json
{
  "format": "trace-commons-near-ai-pins/v1",
  "channel": "staging",
  "gateway_host": "cloud-stg-api.near.ai",
  "sequence": 42,
  "issued_at": "2026-10-21T18:00:00Z",
  "expires_at": "2026-10-28T18:00:00Z",
  "sets": ["mrtd=<96 hex>,rtmr0=<96 hex>,rtmr1=<96 hex>,rtmr2=<96 hex>,mrconfigid=<96 hex>"]
}
```

The signature is over `b"trace-commons-near-ai-pins/v1\n" || payload bytes`,
so the key signs nothing else usefully. Each set is `ExpectedMeasurements`
syntax and must parse; any one set matching is a pass, as today. The
re-sign tool records which registers each set names. Its default mirrors
what production pins today, `mrtd` and `rtmr0-3`
(`deploy/pilot-gcp/ingest.env.template:66`,
`docs/operator/near-attestation-drill.md`). Whether staging should drop
`rtmr3` (which may change per boot under dstack and would then force a
re-sign on every restart) or add `mrconfigid` (which commits to the
compose hash) is open item 6. Several sets cover a rolling deploy where
two images serve at once.

### 5.3 Serving

`https://STAGING_HOST/near-ai-pins/current.json` (3.3), a static file. Its
URL and the public key are constants in `channel.rs` under the feature; the
device never takes either from configuration, from ingest or from IPC.

### 5.4 The re-sign tool and the key

`trace-commons-staging-pins`, a binary in `trace-commons-server` (AGPL;
operators run it, nothing ships it) with `required-features = ["near-ai-staging"]`:

- `observe --model <id>...`: with a staging inference key from stdin, fetch
  `/attestation/report` for each model with a fresh nonce, verify each
  quote with `trace_commons_attestation::quote::verify_quote` against
  current Intel collateral, check the nonce binding, and print the distinct
  measurement sets. It refuses to print a set from a quote that does not
  verify.
- `sign --sets <file> --sequence <n> --ttl-days <d>`: read the private key
  from stdin only (never argv, never a file it writes), build the payload,
  sign, and write `current.json`. It refuses a sequence not above the one in
  the currently served file and a TTL above 14 days.
- `verify <file>`: what the app does, for the operator to check.

The private key is `tc-staging-nearai-pin-signing-key` in Secret Manager,
readable by the owner (and later a dedicated pin-signing SA, open item 7),
**never** by the staging VM's runtime SA: a compromised staging VM can then
serve a stale set but never forge a new one. The re-sign runs on an
operator machine:

```
gcloud secrets versions access latest --secret=tc-staging-nearai-pin-signing-key \
  | trace-commons-staging-pins sign --sets observed.txt --sequence <n+1> --ttl-days 7
```

then copies `current.json` to the VM's pin directory and updates the
staging ingest's own pin (5.9). `docs/operator/staging-environment.md`
holds the procedure.

What a staging pin then means, and no more: *a real TDX enclave, running the
image NEAR AI staging was serving when we observed it, under a key we hold*.
It is not a review of that image. That is the same claim production's pins
make about production.

When to re-sign: after a NEAR AI staging redeploy changes the measurements,
which `observe` shows. Staging redeploys after every cloud-api merge, so
this may be frequent. The default is a manual re-sign plus a weekly expiry.
Automating it (a scheduled job holding the key) is open item 7.

### 5.5 The app's verify path

Under `near-ai-staging` only, `PinProvider` in `proof_attestor.rs` gets its
sets from the signed document instead of ingest's unsigned one:

1. Fetch `current.json` (same refresh timing as today: `PIN_REFRESH_AFTER`,
   `PIN_TTL`).
2. Check `key_id` equals the compiled-in key's, then the Ed25519 signature
   over the domain-separated payload. Verification uses `ring`, already a
   dependency of the attestation and contributor crates.
3. Parse the payload: `format`, `channel == "staging"`, `gateway_host`
   equal to the compiled-in staging gateway host, `issued_at <= now + 5 min`,
   `now < expires_at`, `expires_at - issued_at <= 14 days`, every set parses.
4. Check `sequence` against the highest sequence this state directory has
   accepted (a `near-ai-pins-sequence` file): lower is refused; equal with
   different payload bytes is refused; higher is accepted and recorded.
5. Hand the sets to the existing checks.

The production unsigned path and the env override are not compiled into a
staging build; the signed document is the only source.

### 5.6 Failure

Every failure pins nothing, which means no key is earned and no answer is
marked as proof. That is the existing `PinState::Invalid` meaning, and it
never falls back to an older document, the unsigned endpoint or "verify
nothing".

| Condition | Result |
|---|---|
| Fetch fails | keep the last good set until `PIN_TTL` after it was fetched and not past its `expires_at`; then pin nothing |
| Bad signature, unknown `key_id` | pin nothing |
| Wrong `format`, `channel` or `gateway_host` | pin nothing |
| Expired, or issued in the future | pin nothing |
| Sequence rollback, or same sequence with other bytes | pin nothing |
| A set that does not parse | pin nothing (the whole document) |

The daemon's status reports which of these applies, as a label (for
example `pins_expired`), so a tester can say "proof marks stopped, status
says pins expired" instead of guessing.

### 5.7 Compiled out of production

The format, the verifier, the public key constant and the pin URL are all
behind `near-ai-staging`. A default build has no code that reads a signed
pin set and no staging key bytes. Two checks hold that: a unit test in the
default build that the channel module exposes no staging key, and a
release-workflow step, `scripts/ci/assert-no-staging-markers.sh`, that
greps the built dylib and CLI for the staging key's bytes, the pin URL
path and the `trace-commons-near-ai-pins/v1` format string, and fails if
it finds any. Staging host names are not markers: a production build
holds SHA-256 digests of them for its deny list (section 8), never the
names themselves.

### 5.8 Licensing

The verifier ships in the app, so it lives in a permissive crate:
`crates/trace-commons-attestation/src/staging_pins.rs` (MIT OR Apache-2.0),
behind that crate's own `near-ai-staging` feature, which the contributor's
feature forwards to. The signing side lives in the AGPL server crate's tool
and depends on the attestation crate, which is the permitted direction. No
permissive crate gains an AGPL dependency, and
`crates/trace-commons-server/tests/license_boundary.rs` is unchanged.

### 5.9 The staging server's own pins

The staging ingest checks receipts too (the signer resolver and the
attestation drills), from `TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS`,
which takes one set. `sign` also prints that variable's line for the first
set; the operator puts it in the staging drop-in and restarts ingest. The
ingest's unsigned `/v1/contributors/me/near-ai-measurements` keeps serving,
and a staging device ignores it.

## 6. Build and distribution

### 6.1 The staging workflow

`.github/workflows/staging-macos.yml`. Its name does not start with
`release-`, so it is not swept into the release glob, and the release check
lists it explicitly instead (6.2).

- Trigger: `workflow_dispatch` only, with an optional ref input. Never
  `push`, never a tag, never `workflow_call`. Not in
  `release-contributor.yml` or `release-apps.yml`.
- `environment: staging`, a GitHub environment with the owner as required
  reviewer, so a run waits for approval. It carries the Developer ID and
  notary secrets (the same team identity as production; Apple signing has no
  separate staging identity) and its own WIF provider and publisher SA,
  whose only bucket role is on the staging bucket.
- Steps: build the FFI dylib and `near-ai` with the staging channel
  (`TC_CHANNEL=staging`); `swift test`; build, sign with
  `entitlements-staging.plist` and the staging profile, notarize and staple
  `Near.ai Staging.dmg`; check the channel agreement (4.3); generate the
  staging appcast with `tc-staging-sparkle-signing-key`; upload DMG and appcast
  to the staging bucket; create a GitHub pre-release tagged
  `staging-<date>-<short sha>` marked pre-release, with the DMG attached.
  It never touches the Homebrew tap, the production bucket or a release that
  is not a pre-release.
- Creating that pre-release pushes a tag. The release workflows trigger on
  tags `app-v*` (`release-apps.yml:14-15`) and `contributor-v*`
  (`release-contributor.yml:11-12`), so a `staging-` tag starts neither.
  The release check (6.2) refuses a staging tag prefix that any release
  workflow's tag filter would match.
- Version: production's next version with a `-staging.<run number>` build
  suffix in `CFBundleShortVersionString`, and a `CFBundleVersion` that only
  rises, so Sparkle orders staging builds.

The pre-release is internal in intent but public in fact (the repository is
public). The DMG is notarized and points only at staging hosts, so a
stranger who installs it gets an app that needs a staging invite. Open item
8 asks whether to keep the DMG off GitHub and only in the bucket.

### 6.2 What the release check must allow

`check-release-staging-feature.sh` (#1542) refuses the feature in a fixed
list of files and permits exactly three manifest lines. Staging needs it
widened, in one PR, by exact lines rather than patterns:

1. **A staging file list**, `STAGING_FILES`: `.github/workflows/staging-macos.yml`,
   `deploy/staging-gcp/cloudbuild.yaml`, `macos/channels/staging.env`. Each
   must exist and must name the feature (a rename cannot silently drop one
   out), and must not name any production destination: the production
   appcast URL, the production update bucket, the pilot artifact bucket,
   `release-apps`, or the Homebrew tap.
2. **The staging workflow's triggers**: its `on:` must be exactly
   `workflow_dispatch`, and no other workflow may reference it or the
   staging channel file.
3. **New permitted manifest lines**, each exact: the contributor's
   definition becomes
   `near-ai-staging = ["trace-commons-attestation/near-ai-staging"]`; the
   server's becomes the same forward; the attestation crate's own
   `near-ai-staging = []`; and the server's `[[bin]]`
   `required-features = ["near-ai-staging"]` for the re-sign tool.
   `crates/trace-commons-attestation/Cargo.toml` and
   `crates/trace-commons-protocol/Cargo.toml` join `SHIPPED_MANIFESTS`.
4. **Self-test cases** for each: a staging file that loses the feature, a
   staging file that names a production bucket, a `push:` trigger added to
   the staging workflow, a release workflow calling the staging one, a
   release workflow tag filter that matches the staging tag prefix, each
   new manifest line widened by one more forward.

The release workflows gain `assert-no-staging-markers.sh` (5.7) after their
macOS build.

## 7. Tester onboarding

What a tester does, in order. The runbook carries the full text; any
user-facing copy it adds follows `docs/ui-copy-guide.md`.

1. Make a NEAR AI staging account at `cloud-stg.near.ai`. It is separate
   from a production NEAR AI account, even with the same GitHub or Google
   login.
2. Receive an individual staging invite from the owner.
3. Download `Near.ai Staging.dmg` from the staging pre-release and drag it to
   Applications. It sits beside `Near.ai.app` and shares nothing with it:
   separate sign-in, folder, keychain items and updates.
4. Open it, sign in with the staging NEAR AI account, and add a passkey when
   asked. The passkey is for the staging site only.
5. Declare which session folders to watch. The staging app starts every
   source in "Ask me" mode (open item 10), so nothing uploads without a
   yes.
6. When NEAR AI staging redeploys, expect sign-in to stop working for a
   moment (the JWT secret rotates) and Private AI proof marks to stop until
   the owner re-signs the pins. Sign in again; if proof marks stay off,
   status says why.
7. Report problems in the staging issue template (open item 11) with the
   app's status label and the time, never a trace body, key, org id or
   screenshot of a real session.

## 8. Threat model

**Staging data reaching production, or the reverse.** The two sides share no
database, bucket, KMS key, issuer keyset or secret (3.1, 3.4), so a token,
an envelope or a key from one is refused by the other. On the device they
share no state directory, keychain group, socket, passkey RP, URL scheme or
update feed (4.2). The checks that hold it: the server boot guard (3.9),
the deploy channel check (3.6), the dylib/bundle agreement (4.3), the state
directory channel file (4.4), the release check (6.2) and the binary marker
scan (5.7).

The one route that remains is the tester: both apps can watch the same
session folders, so one real session can be uploaded to production and to
staging. That is the tester's own data going to a server we run with the
same controls, and it earns staging credit that is worth nothing in
production. The "Ask me" default (7, step 5) makes it a choice.

**A production app enrolling at staging.** A production invite parser would
take an `issuer.STAGING_HOST` invite. Default: a production build refuses
the staging hosts as an ingest or issuer origin, from a deny list in the
channel module. The list holds SHA-256 digests of the host names, not the
names, so the marker scan (5.7) can still refuse any staging string in a
production binary.

**A staging build mistaken for production.** The name, icon, bundle id,
status `channel` field, and the dylib check (4.3). A staging app can never
show production data, because it can never reach production.

**A leaked staging pin key.** Blast radius: whoever holds it can make a
staging app accept measurement sets of their choice, so they can mark
answers from an enclave image they control as proof -- but only inside
staging apps, only for traffic those apps send to hosts they already trust
(NEAR AI staging), and only if they can also serve the document from
`STAGING_HOST` or intercept TLS to it. It never affects a production build,
which has no staging verifier and no staging key (5.7). Recovery is a new
key, a new staging build, and a Sparkle update; the sequence file does not
help against a key holder, so rotation is the bound. Keeping the key off the
VM (5.4) means a VM compromise alone cannot do this.

**A compromised staging VM.** It can serve old pins (refused once expired or
rolled back), refuse service, and read staging traces and staging keys. It
cannot sign pins, reach the pilot's database or bucket, or mint production
tokens.

**A leaked staging NEAR AI key or payout credential.** Spends staging
credit only. The payout credential is capped by its constraints and
`allowed_recipient_ids` (payout design section 2).

**Logging.** Staging follows `docs/operator/hash-only-logging.md` exactly as
production does. Staging is not a place to relax it: testers' traces are
real.

## 9. Delivery

The plan breaks this into small PRs. In order:

1. Release check widened for staging (base: #1542's branch).
2. Channel module and identity constants (base: Task 7a's branch).
3. State directory channel file.
4. Signed pin format and verifier in the attestation crate (base: PR 1).
5. Staging pin source in the daemon (needs 2 and 4).
6. Re-sign tool.
7. Server boot guard and `/health` flag (base: #1542's branch).
8. FFI channel call and Swift channel use.
9. Bundle channel files and build scripts.
10. Staging workflow and marker scan.
11. Deploy parameterisation and `deploy/staging-gcp/`.
12. Runbook and tester guide.

## 10. Open items

Settled on 2026-10-21: the host name, PostgreSQL on the VM, admission, the
in-window label (wording still to approve), re-signing and where the DMG
lives (S5-S9). Still open:

1. **Apple setup (owner).** A new App ID `ai.tracecommons.shell.staging`
   with the keychain group and associated domain, and a Developer ID
   provisioning profile for it.
2. **The staging icon** and the label's wording.
3. **NEAR AI facts still needed:** whether staging has per-model completions
   hosts; which registers staging pins (5.2); staging's current
   measurements and the per-model receipt signers for `Qwen/Qwen3.8-27B`;
   whether an API key minted before a redeploy survives the JWT rotation or
   only sessions break.
4. **Witness and self-hosted filter in staging.** Not built now.
5. **The "Ask me" default** for staging sources.
6. **Where testers report**: a GitHub issue template labelled staging, or a
   private channel.

## 11. Where the code differs from the brief

- **Production already accepts served pins.** The brief says production
  builds never accept served pins. Production devices already take an
  unsigned measurement document from ingest over TLS
  (`proof_attestor.rs:46-55`). This design leaves that path as it is. What
  production never does is compile in the staging key or accept a signed
  staging set.
- **There are no static receipt-signer pins.** Signers are earned per
  request from a nonced, verified quote; only measurement sets are pinned.
  The signed set therefore carries measurements, not keys. Task 7a's
  "placeholders that fail closed" for receipt-signer keys becomes the signed
  measurement set.
- **Task 7a misses three hosts and one label.** `proof_attestor.rs:98`,
  `skill_loop/evaluation/transport.rs:19` and
  `privacy_filter_near_ai.rs:19` are not in its file list, and
  `envelope.rs:113` is the privacy-filter default, not the receipt
  endpoint. IronWire does hold its own default host.
- **The release check refuses this design as it stands.** It refuses the
  feature name in `deploy/pilot-gcp/*` and the macOS build scripts, and
  refuses any forward of the feature. Section 6.2 is the widening.
- **The pilot does not load from Secret Manager.** It renders env files.
  Staging keeps the env-file loading and adds a Secret Manager renderer
  (3.7).
- **NEAR AI staging's public `/v1/attestation/pins` is not NEAR AI's pins.**
  It holds backup-provider pins.
