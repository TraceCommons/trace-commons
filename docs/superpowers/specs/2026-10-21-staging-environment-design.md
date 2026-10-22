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

- **(S5) Admission by allow-list.** *Superseded by S12 on 2026-10-22.*
  Asked who can sign in: "Allow-list of NEAR AI accounts". A staging NEAR AI
  account on a list the operator keeps was to be admitted without an invite,
  and any other staging NEAR AI account refused. Built in #1554 and withdrawn
  there before it merged.
- **(S6) DMG on a GitHub pre-release.** "GitHub pre-release". A `staging-*`
  pre-release in the public repo, never marked latest (6.1).
- **(S7) A staging label in the window.** "Yes, a small staging label". Its
  wording, "Staging", was approved on 2026-10-22 (4.8).
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
- **(S11) No state-directory markers.** "I think remove the markeres"
  (2026-10-22). A `channel` file in every state directory guarded only an
  override set by hand to the other channel's directory; one check on the
  override covers that case without writing anything to the directory (4.4).
- **(S12) Open sign-up on staging.** "Ideall for staging we don't use
  invites at all. People with near.ai accounts just sign up" (2026-10-22).
  Any staging NEAR AI account can sign up; there is no allow-list and no
  invite. Supersedes S5. Asked how staging opens sign-in without a witness,
  the owner chose to skip the ledger on staging (2026-10-22): a staging build
  creates accounts without the admission ledger or witness; production is
  unchanged.
- **(S13) Staging DMGs on public pre-releases.** "Github pre-relase for
  straging is fine" (2026-10-22), knowing the repository is public and
  sign-up is open, so anyone who finds a staging DMG can use staging (6.1).
- **(S14) No GitHub environment for staging.** "I don't think we need a
  staging environment right now" (2026-10-22), meaning the GitHub
  environment and its approval gate. The staging workflow runs on manual
  dispatch only, with staging-named repository secrets (6.1).
- **(S15) A public bucket for staging updates.** "New public updates
  bucket" (2026-10-22). The staging appcast and DMGs live in
  `tc-staging-updates-20261022`, readable by anyone, like production's
  updates bucket. The publisher can write only there. The artifacts bucket
  stays private: it is the staging server's object store and holds its
  database dumps.
- **(S16) Replay the RTMR3 event log.** Asked what a staging pin must
  cover once `rtmr3` is not pinned, the owner chose "Replay rtmr3 log
  (Recommended)" (2026-10-22), whose text was: "The re-sign tool and the
  app replay the rtmr3 event log from the compose-manager record and check
  the app ID, key provider and post-boot compose actions against an
  allowlist. Same guarantee as production, survives restarts. More work,
  and it delays a usable staging build." So a staging pin set pins `mrtd`,
  `rtmr0`, `rtmr1`, `rtmr2` and `mrconfigid`, and both the re-sign tool
  and the app replay RTMR3 from the event log, require the replay to equal
  the verified model quote's RTMR3, and check every event against a signed
  allowlist (5.2, 5.4, 5.5). The register set never ships without the
  replay. Which record's log replays the model quote's RTMR3 is confirmed
  first (5.2); the rule is the replay, not the record.

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

### Verified on 2026-10-22: the staging attestation report

Measured live against `https://cloud-stg-api.near.ai/v1`, model
`Qwen/Qwen3.8-27B`, with
`GET /v1/attestation/report?model=Qwen/Qwen3.8-27B&nonce=<fresh 32-byte hex>&signing_algo=ecdsa`.
The method was a throwaway program, not committed, built at `main` commit
`7d6c808af`: it passed each quote to
`trace_commons_server::near_attestation::client::HttpAttestationClient::fetch_collateral`
and then to `trace_commons_attestation::quote::verify_quote`. The register
values are deliberately not recorded here. They may change when staging
redeploys; a probe across a staging redeploy, recording for each register
whether it changed, is still to be done.

- **The gateway serves attestation, and is the configured base.**
  `GET /v1/attestation/report?model=...&nonce=...` on the gateway returns
  `gateway_attestation` and a `model_attestations` array, one entry per
  requested model. Each entry has its own `intel_quote`, `signing_address`,
  `signing_public_key`, `nvidia_payload` and an `info` object without
  `tcb_info`. The gateway reports `app_name` `dstack-0.5.11`, the model
  enclave `dstack-nvidia-0.5.12`. Whether staging also has per-model
  completions hosts is **unconfirmed**: production's gateway returns the
  same `gateway_attestation` + `model_attestations` shape
  (`crates/trace-commons-contributor/src/routing/attestation_report.rs:197`),
  and production does have per-model hosts that its report never lists, so
  this report cannot tell the two apart.
- **Two of the three quotes were verified.** The gateway quote and the
  model entry quote for `Qwen/Qwen3.8-27B` both verify against Intel
  collateral, TCB status UpToDate with no advisories, and both bind the
  probe's nonce at `report_data[32..64]`. The quote inside
  `compose_manager_attestation` was **not** verified, and the probe did not
  check whether it binds the nonce.
- **Each model entry carries a `compose_manager_attestation`**: runtime
  compose actions with their own quote. Runtime compose actions extend
  RTMR3.
- **MRCONFIGID is the v1 layout.** The model entry quote's MRCONFIGID was
  `01`, then the 32-byte compose hash, then zero padding (fifteen bytes).
  That is config-id v1 as `measurements.rs` describes it and as `quote.rs`
  (`mr_config_id_commits_to_the_reports_compose_hash`) asserts for
  production's fixture. v1 commits to the compose hash only. It does not
  commit to the app id or the key provider; only the v2 layout does
  (`measurements.rs:61-62`).
- **The server's drill does not parse this shape.** It reads only the
  single-report shape that a production per-model completions host returns,
  so against staging it fails `ReportFetched: malformed_response`. A server
  change is in progress separately (branch `staging-gateway-quotes`). That is
  follow-up work, not done here.
- **Still unknown:** whether an API key minted before a redeploy survives the
  JWT rotation.

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
| `TRACE_COMMONS_ADMISSION_ENABLED` | `false`; a staging build creates NEAR AI accounts without the ledger (3.12) |
| `TRACE_COMMONS_COMMUNITY_*` | off; staging has no public community page |
| `TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME`, `_REMOTE_OBJECT_STORE_*` | the `tc-staging` key and staging bucket |

The staging gateway serves attestation and is the configured base for both
the scorer and the filter. Whether staging also has per-model completions
hosts is unconfirmed (Verified on 2026-10-22); until it is, staging uses the
gateway base. Open item 3.

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

"Everyone has to sign into staging" (S1), and anyone with a staging NEAR AI
account may (S12). A staging ingest provisions and binds NEAR AI logins
exactly as a default build does: a verified login with no account gets one,
in its own tenant, with no invite and no list; a login that has an account
signs in to it. There is no staging-only admission code and no staging-only
setting for it. `open_sign_up::a_new_near_ai_login_gets_an_account_with_no_invite`
runs in both builds and holds them to the same answer.

The one staging difference is that the NEAR wallet ceremony is refused
(`staging_wallet_provisioning_closed`) whatever the configuration says, so
NEAR AI is the only way to make a staging account.

What opens NEAR AI sign-in is the pilot's readiness check
(`near_ai_login_readiness_named`) less one control:
`TRACE_COMMONS_NEAR_PROVISIONING_ENABLED=true`, the account registry
database, the account identity (`TRACE_COMMONS_NEAR_ACCOUNT_INDEX_PEPPER`,
`TRACE_COMMONS_NEAR_ACCOUNT_NAME_KEK_KEY`) and the issuer
(`TRACE_COMMONS_NEAR_PROVISIONING_ISSUER_URL`, `_AUDIENCE`). A default build
also requires `near_provisioning_admission_ready`, which needs the admission
ledger and therefore a pinned redaction witness. A staging build compiles
that requirement out (S12), so staging runs with the ledger off and no
witness, and NEAR AI sign-in still opens. Production is unchanged: the
requirement is a compile-time difference, not a setting.
`a_staging_build_signs_up_without_the_admission_ledger` and its default-build
twin `a_default_build_refuses_sign_up_without_the_admission_ledger` hold
both sides. Tenant access grants (`TRACE_COMMONS_REQUIRE_TENANT_ACCESS_GRANTS`)
play no part in NEAR AI sign-in.

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
| Icon | `AppIcon` | a staging variant of the same mark (open item 2) | `staging.env` names the `.icon` |
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

The channels keep apart by default: different state directory names
(`channel::STATE_DIR_NAME`), different override variables (a staging build
ignores production's), separate keychain groups and services, and the
refusal of the other channel's hosts before any request. What that leaves is
an override set by hand to the other channel's directory.

So when the state directory comes from the channel's override variable,
`ConfigStore::resolve` refuses it if it resolves to the other channel's
default state directory (`channel::OTHER_STATE_DIR_NAME` under the platform
base; on Windows both the LocalAppData and the roaming one) or anything
inside it. Both sides are resolved before comparing: symlinks are followed,
and for a path that does not exist yet the nearest existing ancestor is
canonicalized and the rest re-appended. The comparison is by path
component, so `trace-commons` and `trace-commons-staging` never match each
other. On macOS and Windows a component matches ignoring ASCII case, since
APFS and NTFS are case-insensitive by default; on Linux it is exact. The
refusal is the label `state-dir-other-channel`, with no path, and
nothing is created. It applies in both builds. An explicit `--config-dir`
and the platform default are not checked. No file is written to the state
directory (S11).

The rule is `config::resolve_state_dir`, which `ConfigStore::resolve`
calls. The macOS app chooses its directory in Swift and opens it with
`ConfigStore::open`, so it would never reach `resolve`; it is covered
through the C ABI's `tc_resolve_state_dir`, which runs the same function.
`StateDirectory` passes every directory that came from the override variable
to it, and on a refusal the app starts neither the daemon nor compute. The
startup refusal shows the core's sentence
(`health_copy::STATE_DIR_OVERRIDE_REFUSED`, read through
`tc_state_dir_override_refused_line`); compute shows its existing
`compute-unavailable` failure. The macOS app is the only client with a staging build.

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
with a separate Sparkle EdDSA key, held as the repository secret
`TC_STAGING_SPARKLE_PRIVATE_ED_KEY` (S14; its public half is
`TC_STAGING_SPARKLE_PUBLIC_ED_KEY`), not in Secret Manager. The staging
app's Info.plist names only the staging feed and staging public key, so a
production appcast can never update it and a staging appcast can never
update production. `scripts/updates/generate-appcast.sh` takes the output
path and key name as arguments rather than gaining a staging branch.

### 4.8 Unmistakable in the UI

Name, Dock and menu-bar icon, and window titles say "Near.ai Staging". The
daemon's status reports `channel: "staging"` (D9 asks for this; Task 7a
wires `near_ai_staging()` into `daemon/ipc.rs`). The staging app also shows
a label, the one word "Staging", at the left of the Monitor's top row and at
the top of the first-run window, so it is there from sign-in on. It is a
notice under `docs/ui-copy-guide.md` section 9: the owner approved it
existing (S7) and its wording (2026-10-22). It is `health_copy::STAGING_WINDOW_LABEL`, read through
`tc_channel_window_label`, which answers with an empty string on production,
so a production build shows nothing. Any staging string lives in the Rust
copy tables (section 8 of the guide), not in Swift.

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
  "sets": ["mrtd=<96 hex>,rtmr0=<96 hex>,rtmr1=<96 hex>,rtmr2=<96 hex>,rtmr3=<96 hex>"]
}
```

A v1 document carries production's register form. The S16 set (`mrtd`,
`rtmr0-2`, `mrconfigid`) appears only in a v2 document, beside its
allowlist (below).

The signature is over `b"trace-commons-near-ai-pins/v1\n" || payload bytes`,
so the key signs nothing else usefully. Each set is `ExpectedMeasurements`
syntax and must parse; any one set matching is a pass, as today. The
re-sign tool records which registers each set names. Several sets cover a
rolling deploy where two images serve at once. The tool re-verifies the
quote on every run and never copies `info.tcb_info` into a set.

**Which registers.** Production and staging differ, and are stated
separately:

- Production pins `mrtd` and `rtmr0-3`
  (`deploy/pilot-gcp/ingest.env.template:66`,
  `docs/operator/near-attestation-drill.md`). Its `rtmr3` pin is a static
  value, so it pins one instance of the image; production does not replay
  any event log, and a restart of that instance is a mismatch.
- The staging re-sign tool's default, by owner decision S16, is `mrtd`,
  `rtmr0`, `rtmr1`, `rtmr2` and `mrconfigid`, with RTMR3 replayed and
  checked against an allowlist (below) instead of pinned. That default
  ships only together with the replay. Until the replay lands, the tool's
  default stays production's `mrtd` and `rtmr0-3` (plan Task 6, Task 6b).

**Why RTMR3 cannot be pinned statically.** dstack extends RTMR3 at boot
with an `instance-id` seeded from `getrandom`, so two instances of
identical code differ and any restart or redeploy is a mismatch
(`crates/trace-commons-attestation/src/measurements.rs:68-70`). Runtime
compose actions also extend it after boot. Neither has anything to do with
whether the code changed.

**Why dropping RTMR3 alone is not enough.** Staging's MRCONFIGID is the
v1 layout: `01`, the 32-byte compose hash, fifteen zero bytes (Verified on
2026-10-22). v1 commits to the compose hash and nothing else; app id and
key provider are only in v2 (`measurements.rs:61-62`). So `mrtd`,
`rtmr0-2` and `mrconfigid` say "booted from the pinned image and compose".
They do not say which KMS released the keys, and they say nothing about
what ran after boot: a model CVM could boot from the pinned compose, then
add or swap a container through a compose action (one that logs prompts,
say), and still match the set, and its provider_tee key would still be
accepted. RTMR3 is the only register that records those things, which is
why S16 replays it.

**The RTMR3 replay.** For a model entry, start from 48 zero bytes and, for
each RTMR3 event in the log in order, set `rtmr3 = SHA-384(rtmr3 ||
digest)`. The result must equal `rtmr[3]` of the **verified** model entry
quote, byte for byte; otherwise the log is not the one that quote measured
and the entry is refused. The digest of each event is recomputed from its
name and payload the way dstack derives it, never taken from the log's
digest field; an event whose recorded digest differs is refused, because
otherwise a log could carry the right digests beside invented payloads.
Only after the replay matches are the events read.

Which log: the one that, replayed, reproduces the verified model entry
quote's RTMR3. There are two candidates, and the probe did not settle
which it is:

- the model entry's own event log. Production model-host reports carry it
  at `info.tcb_info.event_log`
  (`crates/trace-commons-server/src/near_attestation/measurements.rs:376`;
  the checked-in fixtures trim it, `receipt.rs:1808`), but the staging
  model entry's `info` has no `tcb_info`, so the probe saw no log there;
- the event log in the entry's `compose_manager_attestation`, the record
  the owner's option named. It sits beside its own quote, which the probe
  did not verify.

Plan Task 6b settles this first, from an untrimmed staging capture kept
out of the repo: verify the compose-manager quote, compare its `mrtd` and
`rtmr0-2` with the model entry quote's (equal registers indicate the same
TD, and then its log is the model TD's log), and replay each candidate.
Whatever the answer, the rule is the replay: a log that does not replay to
the verified model quote's RTMR3 is refused, and if no candidate does,
`observe` emits nothing and the app pins nothing.

**The allowlist.** A replayed log is accepted only if every event in it is
allowed:

- the `instance-id` event, with any value: it is the per-boot random value
  the replay exists to absorb, so it is the one event whose value is not
  checked;
- the compose-hash event, whose value must equal the compose hash in the
  quote's MRCONFIGID;
- the app-id event, whose value must equal the staging app id in the
  allowlist;
- the key-provider event, whose value must equal the staging KMS identity
  in the allowlist (from `vars/cloud_api_stg.yaml`'s KMS, confirmed against
  a capture);
- every post-boot compose action, by event name and digest, each listed in
  the allowlist;
- the other boot events dstack writes, by name, with the values the
  allowlist lists.

An event whose name is not listed, or whose value is not the listed one, is
refused. The allowlist is a list of what may appear, not of what must, so a
log missing an event is not by itself a refusal, but the replay still has
to match.

Where the allowlist comes from and how it is reviewed: `observe` prints the
decoded RTMR3 events of each model entry (name and digest, and the payload
for app id, key provider and compose actions; never the instance-id value).
The operator reviews them and commits the allowlist as a file under
`deploy/staging-gcp/`, so every change to it is a reviewed pull request. A
new compose action that NEAR AI staging starts using therefore fails closed
until someone has looked at it and merged it. `sign` reads that file and
carries the allowlist **inside the signed payload**, next to `sets`, so the
same key, sequence and expiry cover both and there is no second signature
to check. Adding it bumps the payload format to
`trace-commons-near-ai-pins/v2` (and the domain-separation prefix with it);
a replay-aware build refuses a v1 document, so a document without an
allowlist cannot pass as one with an empty allowlist. The v2 payload adds
one field:

```json
{
  "format": "trace-commons-near-ai-pins/v2",
  "...": "the v1 fields, unchanged",
  "rtmr3_allowlist": {
    "app_id": "<40 hex>",
    "key_provider": "<the staging KMS identity, as the event carries it>",
    "boot_events": [{"name": "<event name>", "payload": "<hex>"}],
    "compose_actions": [{"name": "<event name>", "digest": "<96 hex>"}]
  }
}
```

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
  verify. It emits sets **only** from `model_attestations[].intel_quote`
  for the requested model. The gateway quote and the compose-manager quote
  are verified but never emitted as a pin set: since any one set matching
  passes, a gateway-image set in `current.json` would let the app accept a
  model entry whose quote comes from the gateway image, and take a
  gateway-held key as the model's provider_tee signer. #1563 already reads
  only these quotes (`pin_observe::observe_model` calls
  `model_entry_quotes(&report, model)`); plan Task 6 pins it with a test.
  Once the replay lands (S16), `observe` also replays each model entry's
  RTMR3 log, refuses the run on a mismatch, and prints the decoded events
  for the allowlist review (5.2).
- `sign --sets <file> --sequence <n> --ttl-days <d>`: read the private key
  from stdin only (never argv, never a file it writes), build the payload,
  sign, and write `current.json`. It refuses a sequence not above the one in
  the currently served file and a TTL above 14 days. With the replay, it
  also reads the reviewed allowlist file from `deploy/staging-gcp/` and
  puts it in the payload (5.2).
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

What a staging pin then means, and no more (S16): *a real TDX enclave that
booted the image and compose NEAR AI staging was serving when we observed
it, whose RTMR3 log replays to its quote, names the staging app id and the
staging KMS, and shows no compose action after boot that we have not
reviewed; the set is signed by a key we hold*. It is not a review of that
image. Production's pins cover the same ground by a different mechanism: a
static `rtmr3` fixes one instance's whole log, so a restart is a mismatch
and a re-pin. Until the replay lands, a staging
build must not pin `mrtd`, `rtmr0-2` and `mrconfigid` alone, because that
set says only "booted from the pinned compose" (5.2).

When to re-sign: after a NEAR AI staging redeploy that changes the
measurements or the allowlisted events, which `observe` shows. Whether a
redeploy changes each register is not yet measured (Verified on
2026-10-22). Staging redeploys after every cloud-api merge, so
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
6. With the replay (S16, plan Task 6b): for the model entry whose quote
   matched a set, replay the RTMR3 log taken from the same report as that
   quote (5.2), require it to equal the verified quote's `rtmr[3]`, and
   check every event against the payload's `rtmr3_allowlist`. A key is
   earned only if the set, the replay and the allowlist all pass. Until
   this step lands, a staging build pins with the tool's current default
   (production's `mrtd` and `rtmr0-3`), never with the S16 register set
   alone.

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
| A v1 document in a replay-aware build | pin nothing |
| RTMR3 log missing, or its replay differs from the verified quote's RTMR3 | earn no key for that entry |
| An RTMR3 event not in the allowlist, or with a value other than the listed one | earn no key for that entry |

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
ingest does not replay RTMR3, so that line keeps production's form (`mrtd`
and `rtmr0-3`, from the same observation) and never the S16 register set
without the replay; it goes stale when the model instance restarts. The
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
- No GitHub environment and no approval gate (S14). The Developer ID and
  notary secrets are the repository's existing ones (the same team identity
  as production; Apple signing has no separate staging identity). Its own
  WIF provider and publisher SA, whose only bucket role is on the staging
  bucket, and the staging Sparkle key are repository secrets with
  staging-specific names; the release check refuses production names in
  staging files.
- Steps: build the FFI dylib and `near-ai` with the staging channel
  (`TC_CHANNEL=staging`); `swift test`; build, sign with
  `entitlements-staging.plist` and the staging profile, notarize and staple
  `Near.ai Staging.dmg`; check the channel agreement (4.3); generate the
  staging appcast with the staging Sparkle key; upload DMG and appcast
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
public). The DMG is notarized and points only at staging hosts. Since
sign-up is open (S12), a stranger who installs it and makes a staging NEAR
AI account can use staging (section 8). The owner accepted that: staging
DMGs go on GitHub pre-releases (S13).

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
2. Get the staging DMG link from the owner. No invite is needed: any
   staging NEAR AI account can sign up (S12).
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
directory override check (4.4), the release check (6.2) and the binary marker
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

**Anyone can use staging.** Anyone can make a staging NEAR AI account
(S12), so anyone who has the app can sign up, upload traces to our staging
server, and spend our staging NEAR AI credit through it: Private AI
inference, scoring and the privacy filter. Rate limits bound how fast;
nothing bounds who. Staging credit is
worth nothing in production and staging shares nothing with it (above).
Staging also has no per-account cost limits: it runs without the admission
ledger (S12), so only rate limits bound one account.
The operator stops all of it by shutting the staging service down; there is
no per-person switch.

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
3. State directory override check (S11; it replaced a channel file).
4. Signed pin format and verifier in the attestation crate (base: PR 1).
5. Staging pin source in the daemon (needs 2 and 4).
6. Re-sign tool. Then 6b: RTMR3 replay and allowlist in the re-sign tool
   and the app (S16; needs 5 and 6). The S16 register set becomes the
   default there, not before.
7. Server boot guard and `/health` flag (base: #1542's branch).
8. FFI channel call and Swift channel use.
9. Bundle channel files and build scripts.
10. Staging workflow and marker scan.
11. Deploy parameterisation and `deploy/staging-gcp/`.
12. Runbook and tester guide.

## 10. Open items

Settled on 2026-10-21: the host name, PostgreSQL on the VM, the in-window
label (wording approved 2026-10-22), re-signing and where the DMG lives
(S6-S9). Admission was settled on 2026-10-22 as open sign-up (S12), and the
public pre-release as the DMG's home (S13). The Apple setup is done: the App
ID `ai.tracecommons.shell.staging` exists and its Developer ID provisioning
profile is in `macos/` (#1545). Still open:

1. **Apple setup (owner).** Done (see above).
2. **The staging icon.**
3. **NEAR AI facts.** Partly answered on 2026-10-22 (see Verified on
   2026-10-22). The register set is decided: owner decision S16, `mrtd`,
   `rtmr0-2` and `mrconfigid` with RTMR3 replayed from the event log and
   checked against a signed allowlist (5.2); it ships only with the replay.
   The gateway quote and the `Qwen/Qwen3.8-27B` model entry quote verify,
   TCB UpToDate; the compose-manager quote was not verified. Staging's
   MRCONFIGID is the v1 layout. The gateway serves attestation and is the
   configured base. The measurements are not recorded here and are
   collected by the re-sign tool. The per-model receipt signers for
   `Qwen/Qwen3.8-27B` are in the report's `signing_address` and are
   checked per request, not pinned. Still open:
   - whether staging has per-model completions hosts (the report cannot
     tell; ask NEAR AI or read the `cvm-ansible-playbooks` staging vars);
   - which record's event log replays the model quote's RTMR3 (5.2; plan
     Task 6b, step 1);
   - a probe across a staging redeploy recording, per register, whether it
     changed;
   - whether an API key minted before a redeploy survives the JWT rotation
     or only sessions break.
4. **Witness and self-hosted filter in staging.** Not built now. That no
   longer blocks sign-in: resolved 2026-10-22, a staging build creates NEAR
   AI accounts without the admission ledger, which is what needed the
   witness (S12, 3.12).
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
- **The macOS app held two channel values in Swift.** The funding row's
  credits link (`macos/Sources/TraceCommonsApp/Views/FundingRow.swift`) named
  the production NEAR AI dashboard, and the invite deep link
  (`macos/Sources/TraceCommonsApp/DeepLink.swift`) accepted only the
  production URL scheme. Neither was in the 4.2 table or Task 7a. Both now
  come from `tc_build_channel_json` (`near_ai_dashboard_origin`,
  `url_scheme`).
- **The release check refuses this design as it stands.** It refuses the
  feature name in `deploy/pilot-gcp/*` and the macOS build scripts, and
  refuses any forward of the feature. Section 6.2 is the widening.
- **The pilot does not load from Secret Manager.** It renders env files.
  Staging keeps the env-file loading and adds a Secret Manager renderer
  (3.7).
- **NEAR AI staging's public `/v1/attestation/pins` is not NEAR AI's pins.**
  It holds backup-provider pins.
