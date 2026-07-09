# Contributor Uploader CLI Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. Execution model for this plan: Sonnet 5 subagents implement tasks; the planning session reviews between tasks.

**Goal:** A `trace-commons-contributor` CLI that discovers local Claude Code and Codex session transcripts, redacts them through the existing deterministic pipeline, and submits them as `TraceContributionEnvelope`s via instance-vouched enrollment and per-user upload claims.

**Architecture:** New workspace crate `crates/trace-commons-contributor` (lib + bin). Source adapters normalize transcripts into `SessionTranscript`; `envelope.rs` maps that to `RawTraceContribution` and runs `DeterministicTraceRedactor::redact_trace` (optionally with the NEAR AI privacy-filter adapter) to produce the envelope; `identity.rs` handles device keypairs, enrollment, and Ed25519-signed upload-claim minting; `submit.rs` uploads via `trace-commons-operator-client`'s `Client` with a new explicit-bearer-token builder method.

**Tech Stack:** Rust (edition 2024, rust-version 1.92), clap 4, ring 0.17 (Ed25519), reqwest 0.12 (rustls), serde/serde_json, `trace-commons-protocol`, `trace-commons-operator-client`. Dev: axum 0.8, tower 0.5, tempfile 3, `trace-commons-server` (issuer router for e2e).

## Global Constraints

- No new external dependencies. Every dependency below already appears in the workspace tree at these versions: clap 4, tokio 1, ring 0.17, base64 0.22, reqwest 0.12 (default-features = false, features = ["json", "rustls-tls-native-roots"]), serde 1, serde_json 1, chrono 0.4, uuid 1, anyhow 1, thiserror 2, dirs 6, sha2 0.10, hex 0.4, tracing 0.1, async-trait 0.1; dev-only: axum 0.8, tower 0.5 (features = ["util"]), tempfile 3.
- Verify with `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor` and `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor` — plain `cargo check` does not catch what CI catches.
- Clippy is CI-enforced workspace-wide: `cargo clippy --workspace --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`. Do not widen the allow-list.
- Hash-only logging: no bearer tokens, raw URLs with credentials, trace bodies, contributor identity, or key material in log strings or error output. Errors show hashes or short labels.
- Fail closed: if `--pii-filter near-ai` is requested and the adapter cannot be built or a call fails, refuse that session; never silently downgrade.
- No emojis anywhere. Commit style: short imperative subjects without `feat:`/`fix:` prefixes (match `git log`).
- The envelope schema is `ironclaw.trace_contribution.v1`, unchanged. **No server-crate behavior changes** in this plan (the only non-contributor edits are one additive builder method on operator-client and two CI lines).
- Device-key upload claims are server-capped to consent scopes `[debugging_evaluation, public_attribution]` and allowed uses `[debugging, evaluation, aggregate_analytics]` (trace_upload_claim_issuer.rs:2189-2202). v1 therefore requests and stamps `debugging_evaluation` only. Broader scopes are server-side follow-up work, out of scope here.

## Key wire facts (single source of truth for all tasks)

- **Enroll**: `POST {issuer_url}/v1/enroll`, body = `trace_commons_protocol::onboarding::TraceInstanceEnrollRequest` (`schema_version = "trace_commons.instance_enroll_request.v1"`, `instance_public_key`/`device_public_key` = base64-STANDARD 32-byte Ed25519 keys, `attestation` = `TraceInstanceEnrollAttestation { device_key_id, aud, instance_id, user_subject, nonce, exp }`, `attestation_sig` = base64-STANDARD Ed25519 signature over `instance_enroll_attestation_signing_bytes(&attestation)`, `client_info: TraceOnboardClientInfo { agent, version }`). `exp` must satisfy `now < exp <= now + 300`. Response = `TraceOnboardResponse { schema_version, tenant_id, ingest_url, issuer_url, audience, device_key_id, .. }`. Errors: `{"error":"EnrollMalformed|EnrollNotAuthorized|EnrollRateLimited|EnrollCapExceeded|..."}`.
- **Claim**: `POST {issuer_url}/v1/trace-upload-claim`, body JSON `{ "schema_version": "ironclaw.trace_upload_claim_request.v1", "tenant_id": <string>, "audience": <string>, "consent_scopes": ["debugging_evaluation"], "allowed_uses": ["debugging","evaluation"], "subject": <string>, "requested_at": <RFC3339> }`. Headers: `x-trace-device-key-id: sha256:<64 hex>`, `x-trace-device-signature: <base64-STANDARD of 64-byte Ed25519 sig over the EXACT raw body bytes transmitted>`. Serialize once, sign those bytes, send those bytes. Response: `{ "access_token": <EdDSA JWT>, "token_type": "Bearer", "expires_at": <RFC3339>, "expires_in": <i64 seconds> }`. Issuer body limit 64 KiB.
- **Subject**: always sent as `user_subject_hash(config.user_subject)` (`"sha256:<hex>"` — passes the issuer's `normalize_subject` charset of ASCII alphanumerics plus `:_-`, max 128 bytes, regardless of what the instance used as `user_subject`).
- **Submit**: `POST {ingest_url}/v1/traces`, `Authorization: Bearer <access_token>`, body = bare `TraceContributionEnvelope` JSON. Server body limit 2 MiB; CLI guard `MAX_ENVELOPE_BYTES = 1_500_000`. Idempotent on `submission_id`. Response = `TraceSubmissionReceipt { status, credit_points_pending, credit_points_final, explanation }` (no submission_id echo).
- **Status**: `POST {ingest_url}/v1/contributors/me/submission-status`, same bearer, body `{ "submission_ids": [<uuid>, ...] }`, max 500 ids per call. Response = array of `TraceSubmissionStatusUpdate`.
- **Redaction**: use `trace_commons_protocol::trace_contribution::{DeterministicTraceRedactor, TraceRedactor, RawTraceContribution, RawTraceContributionEvent, PrivacyFilterBackendTag}`. `DeterministicTraceRedactor::new(known_path_prefixes: Vec<String>)`, `.with_privacy_filter(adapter, PrivacyFilterBackendTag::NearAi)`, `redactor.redact_trace(raw).await -> Result<TraceContributionEnvelope, TraceContributionError>`. NEAR AI adapter: `trace_commons_protocol::privacy_filter_near_ai::NearAiPrivacyFilterAdapter::build_from_env()` (env `TRACE_NEAR_AI_PRIVACY_API_KEY`, optional `TRACE_NEAR_AI_PRIVACY_BASE_URL`/`_MODEL`/`_TIMEOUT_MS`/`_MAX_INPUT_BYTES`; requires protocol feature `near-ai-privacy-filter`). Canary helpers: `synthetic_privacy_filter_canary_text()`, `synthetic_privacy_filter_canary_values()`.
- **Allowlist file** (e2e): JSON `{ "version": 1, "generated_at": <RFC3339>, "policy_label": <string>, "entries": [ { "kind": "instance", "instance_id": <string>, "instance_public_key": <base64 32-byte Ed25519>, "max_enrollments": <u32>, "rate_per_min": <u32> } ] }`, referenced as `file:<path>`.

---

### Task 1: Crate scaffold, workspace membership, CLI skeleton, CI

**Files:**
- Create: `crates/trace-commons-contributor/Cargo.toml`
- Create: `crates/trace-commons-contributor/src/lib.rs`
- Create: `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs`
- Modify: `Cargo.toml` (workspace root, `members` list)
- Modify: `.github/workflows/ci.yml` (check job ~line 91, test job ~line 155)

**Interfaces:**
- Produces: workspace member `trace-commons-contributor`; bin with clap `Cli`/`Command` enums (`Login`, `List`, `Submit`, `Status`, `Whoami`, `Logout`, `MintGrant`); every subcommand initially returns `anyhow::bail!("not implemented")`.

- [ ] **Step 1: Create `crates/trace-commons-contributor/Cargo.toml`**

```toml
[package]
name = "trace-commons-contributor"
version = "0.1.0"
edition.workspace = true
rust-version.workspace = true
license.workspace = true
repository.workspace = true

[dependencies]
trace-commons-protocol = { path = "../trace-commons-protocol", features = ["near-ai-privacy-filter"] }
trace-commons-operator-client = { path = "../trace-commons-operator-client" }
anyhow = "1"
base64 = "0.22"
chrono = { version = "0.4", features = ["serde"] }
clap = { version = "4", features = ["derive", "env"] }
dirs = "6"
hex = "0.4"
reqwest = { version = "0.12", default-features = false, features = ["json", "rustls-tls-native-roots"] }
ring = "0.17"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
sha2 = "0.10"
thiserror = "2"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "time", "fs", "io-util"] }
tracing = "0.1"
uuid = { version = "1", features = ["v4", "v5", "serde"] }

[dev-dependencies]
trace-commons-server = { path = "../trace-commons-server" }
axum = "0.8"
tower = { version = "0.5", features = ["util"] }
tempfile = "3"
```

- [ ] **Step 2: Add to workspace members** in root `Cargo.toml` (alphabetical):

```toml
members = [
    "crates/trace-commons-contributor",
    "crates/trace-commons-gate-enclave",
    "crates/trace-commons-operator-client",
    "crates/trace-commons-protocol",
    "crates/trace-commons-server",
]
```

- [ ] **Step 3: Create `src/lib.rs`** (modules land in later tasks; keep only what exists):

```rust
//! Contributor-side client for trace-commons-server: discovers local coding
//! agent transcripts, redacts them through the deterministic pipeline, and
//! submits TraceContributionEnvelopes under instance-vouched per-user
//! identities.
```

- [ ] **Step 4: Create `src/bin/trace-commons-contributor.rs`**:

```rust
use clap::{Parser, Subcommand};
use std::path::PathBuf;

#[derive(Parser)]
#[command(name = "trace-commons-contributor", version, about = "Submit local coding-agent traces to Trace Commons")]
struct Cli {
    /// Override the config directory (default: $TRACE_COMMONS_CONTRIBUTOR_DIR, then OS config dir)
    #[arg(long, global = true)]
    config_dir: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Enroll this device with an instance-signed enrollment grant
    Login {
        /// Base64 enrollment grant minted by your instance; omit to print this device's key id
        #[arg(long)]
        grant: Option<String>,
    },
    /// List discoverable local sessions
    List,
    /// Redact and submit selected sessions
    Submit {
        #[arg(long)]
        all: bool,
        /// Only sessions started within this duration (e.g. 2d, 12h)
        #[arg(long)]
        since: Option<String>,
        /// Only sessions whose project directory matches this path
        #[arg(long)]
        project: Option<PathBuf>,
        /// Restrict to one source: claude-code | codex
        #[arg(long)]
        source: Option<String>,
        /// Skip the interactive picker confirmation
        #[arg(long)]
        yes: bool,
        /// Run the full pipeline but upload nothing
        #[arg(long)]
        dry_run: bool,
        /// PII filter backend: near-ai (requires TRACE_NEAR_AI_PRIVACY_API_KEY)
        #[arg(long)]
        pii_filter: Option<String>,
    },
    /// Show server-side status of previously submitted sessions
    Status,
    /// Print local identity (no network)
    Whoami,
    /// Delete local keystore, config, and receipts
    Logout,
    /// Operator/dogfood tool: mint an enrollment grant with an instance private key
    MintGrant {
        #[arg(long)]
        instance_key_pem: PathBuf,
        #[arg(long)]
        instance_id: String,
        #[arg(long)]
        user_subject: String,
        #[arg(long)]
        audience: String,
        #[arg(long)]
        issuer_url: String,
        /// Device key id to bind; defaults to this machine's local device key
        #[arg(long)]
        device_key_id: Option<String>,
        #[arg(long, default_value_t = 300)]
        ttl_seconds: i64,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::Login { .. } => anyhow::bail!("not implemented"),
        Command::List => anyhow::bail!("not implemented"),
        Command::Submit { .. } => anyhow::bail!("not implemented"),
        Command::Status => anyhow::bail!("not implemented"),
        Command::Whoami => anyhow::bail!("not implemented"),
        Command::Logout => anyhow::bail!("not implemented"),
        Command::MintGrant { .. } => anyhow::bail!("not implemented"),
    }
}
```

- [ ] **Step 5: Extend CI.** In `.github/workflows/ci.yml`: in the `cargo check (default features)` job after `cargo check -p trace-commons-server --bins` add `- run: cargo check -p trace-commons-contributor`; in the `cargo test (default features)` job after `cargo test -p trace-commons-server` add `- run: cargo test -p trace-commons-contributor`. Hold `actions/checkout@v6` / `actions/cache@v5` as-is.

- [ ] **Step 6: Verify**

Run: `RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor && cargo run -p trace-commons-contributor -- --help`
Expected: check passes; help lists login, list, submit, status, whoami, logout, mint-grant.

- [ ] **Step 7: Commit**

```bash
git add Cargo.toml Cargo.lock crates/trace-commons-contributor .github/workflows/ci.yml
git commit -m "Scaffold trace-commons-contributor crate with CLI skeleton"
```

---

### Task 2: operator-client explicit bearer token

**Files:**
- Modify: `crates/trace-commons-operator-client/src/client.rs`

**Interfaces:**
- Produces: `ClientBuilder::bearer_token(self, token: impl Into<String>) -> Self`. When set, `build()` uses this token verbatim and does NOT read the env var. Existing env-var path unchanged (all four operator binaries keep working).

- [ ] **Step 1: Write the failing test** in the existing `#[cfg(test)]` module of `client.rs`:

```rust
#[test]
fn explicit_bearer_token_bypasses_env() {
    // Env var deliberately not set; explicit token must win.
    let client = Client::builder("https://ingest.example", "DEFINITELY_UNSET_ENV_VAR_XYZ")
        .bearer_token("claim-token-abc")
        .build()
        .expect("explicit token should not require env var");
    let _ = client.endpoint();
}

#[test]
fn blank_explicit_bearer_token_is_rejected() {
    let err = Client::builder("https://ingest.example", "DEFINITELY_UNSET_ENV_VAR_XYZ")
        .bearer_token("   ")
        .build()
        .expect_err("blank explicit token must fail");
    assert_eq!(err.kind(), "bearer-missing");
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p trace-commons-operator-client explicit_bearer -v`
Expected: FAIL — no method `bearer_token` on `ClientBuilder`.

- [ ] **Step 3: Implement.** Add field `explicit_bearer: Option<String>` to `ClientBuilder` (initialize `None` in `Client::builder`). Add method after `timeout`:

```rust
/// Provide the bearer token directly instead of naming an env var.
/// Used by clients that mint short-lived tokens in memory (e.g. the
/// contributor CLI's upload claims). Blank tokens are rejected at build.
pub fn bearer_token(mut self, token: impl Into<String>) -> Self {
    self.explicit_bearer = Some(token.into());
    self
}
```

In `build()`, replace the env resolution with:

```rust
let bearer_token = match self.explicit_bearer {
    Some(token) => {
        let trimmed = token.trim();
        if trimmed.is_empty() {
            return Err(Error::BearerMissing { env_var: "<explicit>".to_string() });
        }
        trimmed.to_string()
    }
    None => /* existing env-var resolution unchanged */,
};
```

- [ ] **Step 4: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-operator-client`
Expected: PASS, including all pre-existing tests.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-operator-client/src/client.rs
git commit -m "Allow explicit bearer token on operator-client builder"
```

---

### Task 3: Config store, keystore paths, receipts

**Files:**
- Create: `crates/trace-commons-contributor/src/config.rs`
- Modify: `crates/trace-commons-contributor/src/lib.rs` (add `pub mod config;`)

**Interfaces:**
- Produces:
  - `pub struct ContributorConfig { pub schema_version: String, pub issuer_url: String, pub ingest_url: String, pub audience: String, pub tenant_id: String, pub instance_id: String, pub user_subject: String, pub device_key_id: String, pub consent_scopes: Vec<String>, pub pii_filter: Option<String>, pub allowed_hosts: Option<String> }` (serde Serialize/Deserialize; consent scopes stored as wire strings like `"debugging_evaluation"`).
  - `pub const CONTRIBUTOR_CONFIG_SCHEMA_VERSION: &str = "trace_commons.contributor_config.v1";`
  - `pub struct ConfigStore { dir: PathBuf }` with:
    - `pub fn resolve(explicit: Option<PathBuf>) -> anyhow::Result<Self>` — precedence: explicit flag, `TRACE_COMMONS_CONTRIBUTOR_DIR` env, `dirs::config_dir().join("trace-commons")`; creates the dir with mode 0700 if missing.
    - `pub fn open(dir: PathBuf) -> anyhow::Result<Self>` — same but with an explicit dir (tests use this; no env reads).
    - `pub fn load_config(&self) -> anyhow::Result<Option<ContributorConfig>>` / `pub fn save_config(&self, cfg: &ContributorConfig) -> anyhow::Result<()>` (file `contributor.json`, mode 0600).
    - `pub fn device_key_path(&self) -> PathBuf` (file `device.pk8`).
    - `pub fn save_device_key(&self, pkcs8_der: &[u8]) -> anyhow::Result<()>` (0600) / `pub fn load_device_key(&self) -> anyhow::Result<Option<Vec<u8>>>`.
    - `pub fn append_receipt(&self, r: &Receipt) -> anyhow::Result<()>` / `pub fn load_receipts(&self) -> anyhow::Result<Vec<Receipt>>` (file `receipts.jsonl`, one JSON object per line; unparseable lines are skipped with a `tracing::warn!` counting them).
    - `pub fn wipe(&self) -> anyhow::Result<()>` — removes `contributor.json`, `device.pk8`, `receipts.jsonl` (logout).
  - `pub struct Receipt { pub submission_id: Uuid, pub session_hash: String, pub source: String, pub submitted_at: DateTime<Utc>, pub status: String }` — hash-only; never contains paths or content.

- [ ] **Step 1: Write failing tests** at the bottom of `config.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    fn sample_config() -> ContributorConfig {
        ContributorConfig {
            schema_version: CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "user-1".into(),
            device_key_id: "sha256:00".into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        }
    }

    #[test]
    fn config_round_trip_and_permissions() {
        let (_d, store) = store();
        assert!(store.load_config().unwrap().is_none());
        store.save_config(&sample_config()).unwrap();
        let loaded = store.load_config().unwrap().unwrap();
        assert_eq!(loaded.tenant_id, "tenant-abc");
        let mode = std::fs::metadata(store_path(&store, "contributor.json")).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn device_key_round_trip_and_permissions() {
        let (_d, store) = store();
        assert!(store.load_device_key().unwrap().is_none());
        store.save_device_key(b"fake-der-bytes").unwrap();
        assert_eq!(store.load_device_key().unwrap().unwrap(), b"fake-der-bytes");
        let mode = std::fs::metadata(store.device_key_path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn receipts_append_load_and_skip_garbage() {
        let (_d, store) = store();
        let r = Receipt {
            submission_id: uuid::Uuid::new_v4(),
            session_hash: "sha256:aa".into(),
            source: "claude-code".into(),
            submitted_at: chrono::Utc::now(),
            status: "accepted".into(),
        };
        store.append_receipt(&r).unwrap();
        // Simulate a corrupt line.
        std::fs::write(
            store_path(&store, "receipts.jsonl"),
            format!("{}\nnot-json\n", serde_json::to_string(&r).unwrap()),
        )
        .unwrap();
        let loaded = store.load_receipts().unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].session_hash, "sha256:aa");
    }

    #[test]
    fn wipe_removes_state() {
        let (_d, store) = store();
        store.save_config(&sample_config()).unwrap();
        store.save_device_key(b"k").unwrap();
        store.wipe().unwrap();
        assert!(store.load_config().unwrap().is_none());
        assert!(store.load_device_key().unwrap().is_none());
    }

    // Test helper: expose the file path for assertions.
    fn store_path(store: &ConfigStore, name: &str) -> std::path::PathBuf {
        store.dir().join(name)
    }
}
```

Add a public accessor `pub fn dir(&self) -> &Path` to `ConfigStore` — `whoami` wants it anyway.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p trace-commons-contributor config -v`
Expected: FAIL — module `config` does not exist.

- [ ] **Step 3: Implement `config.rs`.** Notes beyond the interface block: writes go to a temp file in the same dir then `std::fs::rename` (atomic); set permissions with `std::fs::set_permissions(path, Permissions::from_mode(0o600))` after write (guard with `#[cfg(unix)]`); `resolve` errors if no config dir can be determined. `append_receipt` opens with `OpenOptions::new().create(true).append(true)` and sets 0600 on create.

- [ ] **Step 4: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor config`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/config.rs crates/trace-commons-contributor/src/lib.rs
git commit -m "Add contributor config store, keystore paths, and receipts log"
```

---

### Task 4: Device identity, enrollment grants, claim-request signing

**Files:**
- Create: `crates/trace-commons-contributor/src/identity.rs`
- Modify: `crates/trace-commons-contributor/src/lib.rs` (add `pub mod identity;`)

**Interfaces:**
- Consumes: `config::ConfigStore` (Task 3); from `trace_commons_protocol::onboarding`: `TraceInstanceEnrollAttestation`, `TraceInstanceEnrollRequest`, `TraceOnboardClientInfo`, `TraceOnboardResponse`, `instance_enroll_attestation_signing_bytes`, `device_key_id_from_public_key_bytes`, `user_subject_hash`, `TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION`.
- Produces:
  - `pub struct DeviceIdentity { pub device_key_id: String, pub public_key_b64: String, keypair: ring::signature::Ed25519KeyPair }` with `pub fn load_or_generate(store: &ConfigStore) -> anyhow::Result<Self>` (loads `device.pk8` if present, else `Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())` and saves), `pub fn sign_b64(&self, bytes: &[u8]) -> String` (base64-STANDARD of signature).
  - `pub const ENROLLMENT_GRANT_SCHEMA_VERSION: &str = "trace_commons.enrollment_grant.v1";`
  - `pub struct EnrollmentGrant { pub schema_version: String, pub issuer_url: String, pub instance_public_key: String, pub attestation: TraceInstanceEnrollAttestation, pub attestation_sig: String }` (serde) with `pub fn decode(b64: &str) -> anyhow::Result<Self>` (base64-STANDARD of the JSON; rejects wrong schema_version) and `pub fn encode(&self) -> String`.
  - `pub fn mint_grant(instance_key_pkcs8_der: &[u8], issuer_url: &str, instance_id: &str, user_subject: &str, audience: &str, device_key_id: &str, ttl_seconds: i64, now: DateTime<Utc>) -> anyhow::Result<EnrollmentGrant>` — builds the attestation (`nonce = Uuid::new_v4().to_string()`, `exp = now.timestamp() + ttl_seconds`), signs `instance_enroll_attestation_signing_bytes` with the instance key, fills `instance_public_key` from the keypair's public bytes.
  - `pub fn pem_to_pkcs8_der(pem: &str) -> anyhow::Result<Vec<u8>>` — strips `-----BEGIN PRIVATE KEY-----`/`-----END PRIVATE KEY-----` lines, joins, base64-STANDARD decodes (the issuer's `generate_upload_claim_keypair` emits exactly this shape).
  - `pub fn build_enroll_request(grant: &EnrollmentGrant, device: &DeviceIdentity) -> anyhow::Result<TraceInstanceEnrollRequest>` — errors if `grant.attestation.device_key_id != device.device_key_id`; `client_info = TraceOnboardClientInfo { agent: "trace-commons-contributor".into(), version: env!("CARGO_PKG_VERSION").into() }`.
  - `pub struct SignedClaimRequest { pub body: String, pub device_key_id: String, pub signature_b64: String }`
  - `pub fn build_signed_claim_request(cfg: &ContributorConfig, device: &DeviceIdentity, now: DateTime<Utc>) -> anyhow::Result<SignedClaimRequest>` — body per Key wire facts (subject = `user_subject_hash(&cfg.user_subject)`, consent_scopes `["debugging_evaluation"]`, allowed_uses `["debugging","evaluation"]`), serialized ONCE with `serde_json::to_string`; `signature_b64 = device.sign_b64(body.as_bytes())`.
  - `pub const TRACE_DEVICE_KEY_ID_HEADER: &str = "x-trace-device-key-id";` and `pub const TRACE_DEVICE_SIGNATURE_HEADER: &str = "x-trace-device-signature";`

- [ ] **Step 1: Write failing tests** in `identity.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ring::signature::{Ed25519KeyPair, KeyPair, UnparsedPublicKey, ED25519};
    use trace_commons_protocol::onboarding::{
        device_key_id_from_public_key_bytes, instance_enroll_attestation_signing_bytes,
    };

    #[test]
    fn device_identity_is_stable_across_loads() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let a = DeviceIdentity::load_or_generate(&store).unwrap();
        let b = DeviceIdentity::load_or_generate(&store).unwrap();
        assert_eq!(a.device_key_id, b.device_key_id);
        assert!(a.device_key_id.starts_with("sha256:"));
    }

    #[test]
    fn minted_grant_signature_verifies_against_signing_bytes() {
        let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let grant = mint_grant(
            doc.as_ref(), "https://issuer.example", "instance-1", "alice@example.com",
            "trace-commons-upload", "sha256:ab", 300, chrono::Utc::now(),
        )
        .unwrap();
        let kp = Ed25519KeyPair::from_pkcs8(doc.as_ref()).unwrap();
        let pk = UnparsedPublicKey::new(&ED25519, kp.public_key().as_ref());
        use base64::Engine as _;
        let sig = base64::engine::general_purpose::STANDARD.decode(&grant.attestation_sig).unwrap();
        pk.verify(&instance_enroll_attestation_signing_bytes(&grant.attestation), &sig).unwrap();
        assert_eq!(grant.attestation.user_subject, "alice@example.com");
        // Grant round-trips through base64.
        let decoded = EnrollmentGrant::decode(&grant.encode()).unwrap();
        assert_eq!(decoded.attestation.nonce, grant.attestation.nonce);
    }

    #[test]
    fn enroll_request_rejects_foreign_device_key_id() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let doc = Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let grant = mint_grant(
            doc.as_ref(), "https://issuer.example", "instance-1", "alice",
            "trace-commons-upload", "sha256:not-this-device", 300, chrono::Utc::now(),
        )
        .unwrap();
        assert!(build_enroll_request(&grant, &device).is_err());
    }

    #[test]
    fn claim_request_signature_covers_exact_body_bytes() {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device.device_key_id.clone(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        };
        let signed = build_signed_claim_request(&cfg, &device, chrono::Utc::now()).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&signed.body).unwrap();
        assert_eq!(parsed["schema_version"], "ironclaw.trace_upload_claim_request.v1");
        assert_eq!(parsed["tenant_id"], "tenant-abc");
        // Subject is the sha256 form, never the raw user_subject.
        let subject = parsed["subject"].as_str().unwrap();
        assert!(subject.starts_with("sha256:"));
        assert_ne!(subject, "alice");
        // Signature verifies over the exact body string.
        let pk_bytes = base64::engine::general_purpose::STANDARD.decode(&device.public_key_b64).unwrap();
        let pk = ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, pk_bytes);
        let sig = base64::engine::general_purpose::STANDARD.decode(&signed.signature_b64).unwrap();
        pk.verify(signed.body.as_bytes(), &sig).unwrap();
    }
}
```

(Add `use base64::Engine as _;` where needed.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p trace-commons-contributor identity -v`
Expected: FAIL — module `identity` does not exist.

- [ ] **Step 3: Implement `identity.rs`** per the Interfaces block. Implementation notes: `device_key_id` = `device_key_id_from_public_key_bytes(keypair.public_key().as_ref())`; `public_key_b64` = base64-STANDARD of the same bytes; claim body built as a `serde_json::json!` object then `to_string()` once — `SignedClaimRequest.body` is the only serialization, reused verbatim by the HTTP layer. Do not log key material; errors mention file names only.

- [ ] **Step 4: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor identity`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/identity.rs crates/trace-commons-contributor/src/lib.rs
git commit -m "Add device identity, enrollment grants, and claim-request signing"
```

---

### Task 5: Issuer HTTP client + login/whoami/logout/mint-grant commands

**Files:**
- Create: `crates/trace-commons-contributor/src/issuer_client.rs`
- Create: `crates/trace-commons-contributor/src/commands.rs`
- Modify: `crates/trace-commons-contributor/src/lib.rs` (add `pub mod issuer_client; pub mod commands;`)
- Modify: `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs` (wire Login/Whoami/Logout/MintGrant)

**Interfaces:**
- Consumes: Task 4 types; `trace_commons_operator_client::host_allowlist::HostAllowlist`.
- Produces:
  - `pub struct IssuerClient { http: reqwest::Client, allowlist: HostAllowlist }` with `pub fn new(allowlist: HostAllowlist) -> anyhow::Result<Self>` (30s timeout);
    - `pub async fn enroll(&self, issuer_url: &str, req: &TraceInstanceEnrollRequest) -> anyhow::Result<TraceOnboardResponse>` — POST `{issuer_url}/v1/enroll`; checks allowlist on the parsed URL first; non-2xx: parse `{"error": <label>}` and return `anyhow::anyhow!("enroll refused: {label}")` (label only, no body echo).
    - `pub async fn mint_claim(&self, issuer_url: &str, signed: &SignedClaimRequest) -> anyhow::Result<ClaimToken>` — POST `{issuer_url}/v1/trace-upload-claim` with headers `x-trace-device-key-id`/`x-trace-device-signature` and `signed.body` as the raw body (`.body(signed.body.clone())` with `content-type: application/json` — NOT `.json()`, which would re-serialize).
  - `pub struct ClaimToken { pub access_token: String, pub expires_at: DateTime<Utc> }` — parsed from the response; carries an `is_fresh(&self, now) -> bool` method using a 60-second skew (`now + 60s < expires_at`).
  - In `commands.rs`: `pub async fn login(store: &ConfigStore, grant_b64: Option<&str>) -> anyhow::Result<()>`, `pub fn whoami(store: &ConfigStore) -> anyhow::Result<()>`, `pub fn logout(store: &ConfigStore) -> anyhow::Result<()>`, `pub fn mint_grant_cmd(args...) -> anyhow::Result<()>` (prints `grant.encode()` to stdout).
- Login semantics: no grant → load-or-generate device identity, print `device_key_id` and instructions ("give this to your instance to mint an enrollment grant"), exit 0. With grant → decode, `build_enroll_request`, `enroll`, then save `ContributorConfig` populated from the grant (`instance_id`, `user_subject`, `issuer_url`) and the `TraceOnboardResponse` (`tenant_id`, `ingest_url`, `audience`, `device_key_id`), `consent_scopes = ["debugging_evaluation"]`, and print tenant id + one-line consent statement: "Traces you submit carry the debugging_evaluation consent scope; secrets are removed locally, PII is scrubbed server-side."

- [ ] **Step 1: Write failing test** — an async test with a stub issuer in `issuer_client.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn mint_claim_sends_signed_body_verbatim_and_parses_token() {
        let router = Router::new().route(
            "/v1/trace-upload-claim",
            post(|headers: axum::http::HeaderMap, body: String| async move {
                assert_eq!(body, r#"{"k":"v"}"#);
                assert_eq!(headers.get("x-trace-device-key-id").unwrap(), "sha256:ab");
                assert_eq!(headers.get("x-trace-device-signature").unwrap(), "c2ln");
                Json(serde_json::json!({
                    "access_token": "jwt-token",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                }))
            }),
        );
        let base = spawn(router).await;
        let client = IssuerClient::new(
            trace_commons_operator_client::host_allowlist::HostAllowlist::permissive(),
        )
        .unwrap();
        let signed = crate::identity::SignedClaimRequest {
            body: r#"{"k":"v"}"#.into(),
            device_key_id: "sha256:ab".into(),
            signature_b64: "c2ln".into(),
        };
        let token = client.mint_claim(&base, &signed).await.unwrap();
        assert_eq!(token.access_token, "jwt-token");
        assert!(token.is_fresh(chrono::Utc::now()));
    }

    #[tokio::test]
    async fn enroll_error_label_is_surfaced_without_body() {
        let router = Router::new().route(
            "/v1/enroll",
            post(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "EnrollNotAuthorized"})),
                )
            }),
        );
        let base = spawn(router).await;
        let client = IssuerClient::new(
            trace_commons_operator_client::host_allowlist::HostAllowlist::permissive(),
        )
        .unwrap();
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
        let grant = crate::identity::mint_grant(
            doc.as_ref(), &base, "instance-1", "alice", "aud", "sha256:ab", 300, chrono::Utc::now(),
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        // Force a matching device_key_id so we reach the HTTP call.
        let grant2 = crate::identity::mint_grant(
            doc.as_ref(), &base, "instance-1", "alice", "aud", &device.device_key_id, 300, chrono::Utc::now(),
        )
        .unwrap();
        let req = crate::identity::build_enroll_request(&grant2, &device).unwrap();
        let err = client.enroll(&base, &req).await.unwrap_err();
        assert!(err.to_string().contains("EnrollNotAuthorized"));
        let _ = grant;
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p trace-commons-contributor issuer_client -v`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement `issuer_client.rs` and `commands.rs`**, wire the four subcommands in the bin (`ConfigStore::resolve(cli.config_dir)` once in main, pass to commands). `whoami` prints instance_id, tenant_id, device_key_id, config dir path — never the user_subject (it may be an email; print `user_subject_hash` instead). `mint_grant_cmd` reads the PEM file, uses `pem_to_pkcs8_der`, defaults `device_key_id` to the local device identity when the flag is omitted.

- [ ] **Step 4: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor && RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src
git commit -m "Add issuer client and login, whoami, logout, mint-grant commands"
```

---

### Task 6: Source model — trait, session types, deterministic ids

**Files:**
- Create: `crates/trace-commons-contributor/src/source/mod.rs`
- Modify: `crates/trace-commons-contributor/src/lib.rs` (add `pub mod source;`)

**Interfaces:**
- Produces:

```rust
pub const SOURCE_CLAUDE_CODE: &str = "claude-code";
pub const SOURCE_CODEX: &str = "codex";

#[derive(Debug, Clone)]
pub struct SessionRef {
    pub source: &'static str,
    pub path: PathBuf,
    pub project: Option<String>,      // basename only, never a full path
    pub started_at: Option<DateTime<Utc>>,
    pub size_bytes: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub enum SessionEventKind { User, Assistant, ToolCall, ToolResult, Opaque }

#[derive(Debug, Clone)]
pub struct SessionEvent {
    pub kind: SessionEventKind,
    pub timestamp: Option<DateTime<Utc>>,
    pub content: Option<String>,
    pub structured: serde_json::Value,   // Value::Null when absent
    pub tool_name: Option<String>,
    pub token_counts: Option<(u32, u32)>, // (input, output)
}

#[derive(Debug, Clone)]
pub struct SessionTranscript {
    pub source: &'static str,
    pub agent_version: Option<String>,
    pub model: Option<String>,
    pub project: Option<String>,      // basename
    pub cwd: Option<String>,          // full path; used for redactor prefixes + hashing, NEVER serialized
    pub started_at: Option<DateTime<Utc>>,
    pub session_hash: String,         // "sha256:<hex>" of raw file bytes
    pub events: Vec<SessionEvent>,
}

pub trait TraceSource {
    fn name(&self) -> &'static str;
    fn discover(&self) -> anyhow::Result<Vec<SessionRef>>;
    fn load(&self, r: &SessionRef) -> anyhow::Result<SessionTranscript>;
}

pub fn session_hash(bytes: &[u8]) -> String;                       // "sha256:<hex>"
pub fn submission_id_for(session_hash: &str) -> uuid::Uuid;        // Uuid::new_v5(&Uuid::NAMESPACE_OID, session_hash.as_bytes())
pub fn all_sources(claude_root: Option<PathBuf>, codex_root: Option<PathBuf>) -> Vec<Box<dyn TraceSource>>;
```

`all_sources` constructs `ClaudeCodeSource::new(root)` / `CodexSource::new(root)` (Tasks 7-8) with defaults `dirs::home_dir().join(".claude/projects")` and `dirs::home_dir().join(".codex/sessions")`. Until those tasks land, `all_sources` returns an empty vec with a `// populated by claude_code/codex tasks` comment replaced in Tasks 7-8.

- [ ] **Step 1: Write failing tests** in `source/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_hash_is_prefixed_and_deterministic() {
        let h = session_hash(b"abc");
        assert!(h.starts_with("sha256:"));
        assert_eq!(h, session_hash(b"abc"));
        assert_ne!(h, session_hash(b"abd"));
    }

    #[test]
    fn submission_id_is_deterministic_per_session() {
        let a = submission_id_for("sha256:aa");
        assert_eq!(a, submission_id_for("sha256:aa"));
        assert_ne!(a, submission_id_for("sha256:bb"));
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p trace-commons-contributor source -v`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement** per the interface block (sha2 for hashing, hex for encoding).

- [ ] **Step 4: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor source`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/source crates/trace-commons-contributor/src/lib.rs
git commit -m "Add source trait, session model, and deterministic submission ids"
```

---

### Task 7: Claude Code adapter

**Files:**
- Create: `crates/trace-commons-contributor/src/source/claude_code.rs`
- Create: `crates/trace-commons-contributor/fixtures/claude-code/-Users-testuser-code-myproj/11111111-1111-1111-1111-111111111111.jsonl`
- Modify: `crates/trace-commons-contributor/src/source/mod.rs` (add `pub mod claude_code;`, wire into `all_sources`)

**Interfaces:**
- Consumes: Task 6 types.
- Produces: `pub struct ClaudeCodeSource { root: PathBuf }` with `pub fn new(root: PathBuf) -> Self`, implementing `TraceSource` with `name() == SOURCE_CLAUDE_CODE`.

**Format facts (verified against real files on disk 2026-07-07):** one session per `<root>/<encoded-cwd>/<uuid>.jsonl`. The directory name is the cwd with `/` replaced by `-` (e.g. `-Users-testuser-code-myproj`). Records are JSON objects with a `type` field. Relevant types: `user` and `assistant` carry `message`, `cwd`, `timestamp` (RFC3339), `version` (agent version), plus other fields. `user` message: `{"role":"user","content":<string or array of blocks>}`; content blocks include `{"type":"text","text":...}` and `{"type":"tool_result","tool_use_id":...,"content":<string or array>}`. `assistant` message: `{"role":"assistant","model":<string>,"content":[blocks],"usage":{"input_tokens":N,"output_tokens":N,...}}`; blocks include `{"type":"text","text":...}`, `{"type":"tool_use","id":...,"name":...,"input":{...}}`, `{"type":"thinking",...}`. Other record types seen: `system`, `attachment`, `file-history-snapshot`, `mode`, `permission-mode`, `last-prompt`, `ai-title`, `queue-operation`, `summary`.

**Mapping rules:**
- `user` record, string content → one `SessionEvent { kind: User, content: Some(s), .. }`. Array content: `text` blocks concatenated (joined with `\n`) into one User event; each `tool_result` block becomes `SessionEvent { kind: ToolResult, content: <flattened text of block content>, structured: Value::Null, .. }`.
- `assistant` record: `text` blocks joined into one `SessionEvent { kind: Assistant, content, token_counts: Some((usage.input_tokens, usage.output_tokens)) }` (token counts only on the event carrying text; default `(0,0)` missing usage → `None`); each `tool_use` block → `SessionEvent { kind: ToolCall, tool_name: Some(name), structured: input.clone(), content: None }`. `thinking` blocks are dropped (deliberate v1 privacy posture).
- Every other record type (and unknown future types) → `SessionEvent { kind: Opaque, structured: json!({"record_type": <type>}), content: None }`. **Never** copy the record payload — attachments and snapshots contain file contents.
- `model` = first assistant `message.model` seen; `agent_version` = first `version` seen; `cwd` = first `cwd` seen; `project` = `cwd` basename (fallback: decode the directory name after the last `-` segment is NOT reliable — use cwd only, else `None`); `started_at` = first record `timestamp`.
- Unparseable lines: count them, skip, continue; a file with zero parseable user/assistant records still loads (empty events) — the picker filters out sessions with no User events.
- `discover` walks `root/*/*.jsonl` (both levels flat, non-recursive beyond that), builds `SessionRef` from file metadata; `started_at` in discover may be None (avoid reading whole files during discovery; use file mtime as an approximation: `std::fs::metadata(...).modified()`).

- [ ] **Step 1: Create the fixture** at the path above, exactly these 8 lines (sanitized, no real data; the seeded fake secret is intentional — Task 9 asserts it gets redacted):

```
{"type":"user","message":{"role":"user","content":"Fix the login bug in auth.rs"},"cwd":"/Users/testuser/code/myproj","timestamp":"2026-07-01T10:00:00Z","version":"2.0.1","sessionId":"11111111-1111-1111-1111-111111111111","uuid":"a1"}
{"type":"assistant","message":{"role":"assistant","model":"claude-fable-5","content":[{"type":"thinking","thinking":"secret reasoning"},{"type":"text","text":"Looking at the file now."},{"type":"tool_use","id":"tu_1","name":"Read","input":{"file_path":"/Users/testuser/code/myproj/src/auth.rs","api_key":"sk-fake-fixture-secret-1234"}}],"usage":{"input_tokens":100,"output_tokens":25}},"cwd":"/Users/testuser/code/myproj","timestamp":"2026-07-01T10:00:05Z","version":"2.0.1","uuid":"a2"}
{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"tu_1","content":"fn login() { /* bug here */ }"}]},"cwd":"/Users/testuser/code/myproj","timestamp":"2026-07-01T10:00:06Z","version":"2.0.1","uuid":"a3"}
{"type":"assistant","message":{"role":"assistant","model":"claude-fable-5","content":[{"type":"text","text":"Found it. The comparison is inverted."}],"usage":{"input_tokens":150,"output_tokens":12}},"cwd":"/Users/testuser/code/myproj","timestamp":"2026-07-01T10:00:10Z","version":"2.0.1","uuid":"a4"}
{"type":"system","subtype":"hook","cwd":"/Users/testuser/code/myproj","timestamp":"2026-07-01T10:00:11Z","uuid":"a5"}
{"type":"attachment","attachment":{"path":"/Users/testuser/code/myproj/notes.txt","contents":"do not leak me"},"timestamp":"2026-07-01T10:00:12Z","uuid":"a6"}
{"type":"future-unknown-record","payload":{"x":1}}
not valid json at all
```

- [ ] **Step 2: Write failing tests** in `claude_code.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SessionEventKind, TraceSource};
    use std::path::PathBuf;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code")
    }

    #[test]
    fn discovers_fixture_session() {
        let src = ClaudeCodeSource::new(fixture_root());
        let found = src.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "claude-code");
    }

    #[test]
    fn loads_and_maps_events_leniently() {
        let src = ClaudeCodeSource::new(fixture_root());
        let r = &src.discover().unwrap()[0];
        let t = src.load(r).unwrap();
        assert_eq!(t.model.as_deref(), Some("claude-fable-5"));
        assert_eq!(t.agent_version.as_deref(), Some("2.0.1"));
        assert_eq!(t.cwd.as_deref(), Some("/Users/testuser/code/myproj"));
        assert_eq!(t.project.as_deref(), Some("myproj"));
        let kinds: Vec<_> = t.events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                SessionEventKind::User,
                SessionEventKind::Assistant,
                SessionEventKind::ToolCall,
                SessionEventKind::ToolResult,
                SessionEventKind::Assistant,
                SessionEventKind::Opaque,   // system
                SessionEventKind::Opaque,   // attachment
                SessionEventKind::Opaque,   // future-unknown-record
            ]
        );
        // Thinking dropped; token counts captured on the assistant text event.
        assert_eq!(t.events[1].token_counts, Some((100, 25)));
        assert_eq!(t.events[2].tool_name.as_deref(), Some("Read"));
        // Opaque events carry only the record type, never payloads.
        let serialized = serde_json::to_string(&t.events[6].structured).unwrap();
        assert!(!serialized.contains("do not leak me"));
        assert!(serialized.contains("attachment"));
        // Thinking text is gone entirely.
        let all = format!("{:?}", t.events);
        assert!(!all.contains("secret reasoning"));
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p trace-commons-contributor claude_code -v`
Expected: FAIL — module does not exist.

- [ ] **Step 4: Implement**, then wire `ClaudeCodeSource` into `all_sources` in `source/mod.rs`.

- [ ] **Step 5: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor claude_code`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/source crates/trace-commons-contributor/fixtures
git commit -m "Add Claude Code transcript adapter with sanitized fixture"
```

---

### Task 8: Codex adapter

**Files:**
- Create: `crates/trace-commons-contributor/src/source/codex.rs`
- Create: `crates/trace-commons-contributor/fixtures/codex/2026/07/01/rollout-2026-07-01T10-00-00-22222222-2222-2222-2222-222222222222.jsonl`
- Modify: `crates/trace-commons-contributor/src/source/mod.rs` (add `pub mod codex;`, wire into `all_sources`)

**Interfaces:**
- Consumes: Task 6 types.
- Produces: `pub struct CodexSource { root: PathBuf }` with `pub fn new(root: PathBuf) -> Self`, implementing `TraceSource` with `name() == SOURCE_CODEX`.

**Format facts (verified against real files on disk 2026-07-07):** sessions at `<root>/YYYY/MM/DD/rollout-<timestamp>-<uuid>.jsonl`. Every record: `{"type": <t>, "timestamp": <RFC3339>, "payload": {...}}`. Types: `session_meta` (payload has `id`, `cwd`, `cli_version`, `model_provider`, `git`, ...), `turn_context` (payload has `model`, `cwd`, ...), `response_item`, `event_msg`. `response_item` payload has its own `type`: `message` (`{"role":"user"|"assistant","content":[{"type":"input_text"|"output_text","text":...}]}`), `reasoning` (skip entirely), `function_call` (`{"name":...,"arguments":<JSON string>,"call_id":...}`), `function_call_output` (`{"call_id":...,"output":<string>}`), `custom_tool_call` (`{"name":...,"input":...,"call_id":...}`), `custom_tool_call_output`, `web_search_call`.

**Mapping rules:**
- `session_meta` → `cwd` + `agent_version` (from `cli_version`); not an event.
- `turn_context` → `model` (first seen); not an event.
- `message` role=user → `SessionEvent { kind: User, content: Some(joined input_text/output_text texts) }`; role=assistant → same with `kind: Assistant`.
- `function_call` → `SessionEvent { kind: ToolCall, tool_name: Some(name), structured: serde_json::from_str(&arguments).unwrap_or(json!({"arguments_raw_len": arguments.len()})) }` — if the arguments string parses as JSON, keep it; otherwise record only its length (lenient, no raw dump of unparseable content).
- `function_call_output` → `SessionEvent { kind: ToolResult, content: Some(output-as-text) }` (`output` may be a string or an object; if object, use its `output` string field if present else `serde_json::to_string` of it).
- `custom_tool_call` / `custom_tool_call_output` → same as function_call/output (input under `input`).
- `reasoning`, `event_msg`, `web_search_call`, unknown payload types, unknown record types → `SessionEvent { kind: Opaque, structured: json!({"record_type": <outer or inner type>}) }`.
- Unparseable lines: skip and count. `started_at` = first record timestamp. `project` = cwd basename.

- [ ] **Step 1: Create the fixture** (7 lines):

```
{"type":"session_meta","timestamp":"2026-07-01T10:00:00Z","payload":{"id":"22222222-2222-2222-2222-222222222222","cwd":"/Users/testuser/code/otherproj","cli_version":"0.48.0","model_provider":"openai","source":"cli"}}
{"type":"turn_context","timestamp":"2026-07-01T10:00:01Z","payload":{"model":"gpt-5.2-codex","cwd":"/Users/testuser/code/otherproj","approval_policy":"on-request"}}
{"type":"response_item","timestamp":"2026-07-01T10:00:02Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Add a healthcheck endpoint"}]}}
{"type":"response_item","timestamp":"2026-07-01T10:00:03Z","payload":{"type":"reasoning","content":[],"summary":["thinking about it"]}}
{"type":"response_item","timestamp":"2026-07-01T10:00:04Z","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":\"ls src/\"}","call_id":"c1"}}
{"type":"response_item","timestamp":"2026-07-01T10:00:05Z","payload":{"type":"function_call_output","call_id":"c1","output":"main.rs\nroutes.rs"}}
{"type":"response_item","timestamp":"2026-07-01T10:00:06Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Added GET /health returning 200."}]}}
```

- [ ] **Step 2: Write failing tests** in `codex.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{SessionEventKind, TraceSource};
    use std::path::PathBuf;

    fn fixture_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/codex")
    }

    #[test]
    fn discovers_nested_rollout_files() {
        let src = CodexSource::new(fixture_root());
        let found = src.discover().unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].source, "codex");
    }

    #[test]
    fn maps_response_items() {
        let src = CodexSource::new(fixture_root());
        let r = &src.discover().unwrap()[0];
        let t = src.load(r).unwrap();
        assert_eq!(t.model.as_deref(), Some("gpt-5.2-codex"));
        assert_eq!(t.agent_version.as_deref(), Some("0.48.0"));
        assert_eq!(t.project.as_deref(), Some("otherproj"));
        let kinds: Vec<_> = t.events.iter().map(|e| e.kind.clone()).collect();
        assert_eq!(
            kinds,
            vec![
                SessionEventKind::User,
                SessionEventKind::Opaque,   // reasoning
                SessionEventKind::ToolCall,
                SessionEventKind::ToolResult,
                SessionEventKind::Assistant,
            ]
        );
        assert_eq!(t.events[2].tool_name.as_deref(), Some("shell"));
        assert_eq!(t.events[2].structured["command"], "ls src/");
        // Reasoning summary text must not survive.
        assert!(!format!("{:?}", t.events).contains("thinking about it"));
    }
}
```

- [ ] **Step 3: Run to verify failure**

Run: `cargo test -p trace-commons-contributor codex -v`
Expected: FAIL — module does not exist.

- [ ] **Step 4: Implement** (recursive walk of `root` collecting `rollout-*.jsonl`; use `std::fs::read_dir` recursion — no walkdir dependency), then wire `CodexSource` into `all_sources`.

- [ ] **Step 5: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor codex`
Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/trace-commons-contributor/src/source crates/trace-commons-contributor/fixtures
git commit -m "Add Codex rollout adapter with sanitized fixture"
```

---

### Task 9: Envelope assembly and redaction pipeline

**Files:**
- Create: `crates/trace-commons-contributor/src/envelope.rs`
- Modify: `crates/trace-commons-contributor/src/lib.rs` (add `pub mod envelope;`)

**Interfaces:**
- Consumes: `SessionTranscript`/`SessionEvent` (Task 6), `ContributorConfig` (Task 3), `submission_id_for` (Task 6); from `trace_commons_protocol::trace_contribution`: `RawTraceContribution`, `RawTraceContributionEvent`, `TraceContributionEventType`, `TraceChannel`, `IronclawTraceMetadata`, `ConsentMetadata`, `ConsentScope`, `ContributorMetadata`, `OutcomeMetadata`, `ReplayMetadata`, `ValueMetadata`, `TokenCounts`, `DeterministicTraceRedactor`, `TraceRedactor`, `PrivacyFilterBackendTag`, `TRACE_CONTRIBUTION_SCHEMA_VERSION`, `TRACE_CONTRIBUTION_POLICY_VERSION`, `synthetic_privacy_filter_canary_text`, `synthetic_privacy_filter_canary_values`; `trace_commons_protocol::onboarding::user_subject_hash`; `trace_commons_protocol::privacy_filter_near_ai::NearAiPrivacyFilterAdapter`.
- Produces:
  - `pub const MAX_ENVELOPE_BYTES: usize = 1_500_000;`
  - `pub fn build_raw_contribution(t: &SessionTranscript, cfg: &ContributorConfig, now: DateTime<Utc>) -> RawTraceContribution`
  - `pub struct NearAiSettings { pub api_key: String, pub base_url: Option<String>, pub model: Option<String> }`
  - `pub fn near_ai_settings_from_env() -> Option<NearAiSettings>` — reads `TRACE_NEAR_AI_PRIVACY_API_KEY` (None if unset/blank), `TRACE_NEAR_AI_PRIVACY_BASE_URL`, `TRACE_NEAR_AI_PRIVACY_MODEL`. Read-only env access (no `set_var`/`remove_var` anywhere — they are `unsafe` in edition 2024 and racy under parallel tests).
  - `pub fn build_redactor_with(cfg: &ContributorConfig, transcript_cwd: Option<&str>, near_ai: Option<NearAiSettings>) -> anyhow::Result<DeterministicTraceRedactor>` — prefixes: home dir (if resolvable) + transcript cwd. When `cfg.pii_filter == Some("near-ai")`: **error (fail closed) if `near_ai` is None**; else construct `NearAiPrivacyFilterAdapter::new(base_url.unwrap_or("https://cloud-api.near.ai/v1"), model.unwrap_or("openai/privacy-filter"), api_key, Duration::from_millis(10_000), 1024*1024)` and attach via `.with_privacy_filter(Arc::new(adapter), PrivacyFilterBackendTag::NearAi)`. Any other `pii_filter` value: error with label `"unknown-pii-filter"`.
  - `pub fn build_redactor(cfg: &ContributorConfig, transcript_cwd: Option<&str>) -> anyhow::Result<DeterministicTraceRedactor>` — thin wrapper: `build_redactor_with(cfg, transcript_cwd, near_ai_settings_from_env())`. Production callers use this; tests use `build_redactor_with` so they never touch process env.
  - `pub fn canary_self_test(redactor: &DeterministicTraceRedactor) -> anyhow::Result<()>` — runs `redactor.redact_text(&synthetic_privacy_filter_canary_text())` and errors if any value from `synthetic_privacy_filter_canary_values()` appears in the output.
  - `pub async fn redact_to_envelope(redactor: &DeterministicTraceRedactor, raw: RawTraceContribution) -> anyhow::Result<TraceContributionEnvelope>` — thin wrapper over `redactor.redact_trace(raw)` mapping the error to a label-only message.
  - `pub fn envelope_size_ok(envelope: &TraceContributionEnvelope) -> anyhow::Result<usize>` — serializes, errors with a "session too large" message when over `MAX_ENVELOPE_BYTES`, else returns the byte size.

**`build_raw_contribution` field mapping (exact):**
- `trace_id: Uuid::new_v4()`, `submission_id: submission_id_for(&t.session_hash)`, `created_at: now`.
- `ironclaw: IronclawTraceMetadata { version: t.agent_version.clone().unwrap_or_else(|| "unknown".into()), engine_version: None, feature_flags: BTreeMap from [("agent", t.source), ("agent_version", t.agent_version or "unknown"), ("project", t.project or "unknown"), ("cwd_hash", sha256:<hex of t.cwd bytes> or "unknown")], channel: TraceChannel::Cli, model_name: t.model.clone() }`.
- `consent: ConsentMetadata { policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.into(), scopes: vec![ConsentScope::DebuggingEvaluation], message_text_included: true, tool_payloads_included: true, revocable: true }`.
- `contributor: ContributorMetadata { pseudonymous_contributor_id: Some(user_subject_hash(&cfg.user_subject)), tenant_scope_ref: Some(cfg.tenant_id.clone()), credit_account_ref: None, revocation_handle: Uuid::new_v4() }`.
- `events`: each `SessionEvent` → `RawTraceContributionEvent { event_id: Uuid::new_v4(), event_type: match kind { User => UserMessage, Assistant => AssistantMessage, ToolCall => ToolCall, ToolResult => ToolResult, Opaque => ToolResult }, timestamp: e.timestamp.unwrap_or(now), content: e.content.clone(), structured_payload: e.structured.clone(), tool_name: e.tool_name.clone(), latency_ms: None, token_counts: e.token_counts.map(|(i, o)| TokenCounts { input_tokens: i, output_tokens: o }), cost_usd: None }` — EXCEPT Opaque events map to `event_type: ToolResult` with `content: None` and their `{"record_type": ...}` structured payload (there is no generic event type in the v1 schema; the record-type marker in `structured_payload` preserves provenance).
- `outcome: OutcomeMetadata::default()`, `replay: ReplayMetadata { replayable: false, required_tools: vec![], tool_manifest_hashes: BTreeMap::new(), expected_assertions: vec![], replay_notes: vec!["imported transcript; not replayable".into()] }`, `embedding_analysis: None`, `value: ValueMetadata::default()`.

- [ ] **Step 1: Write failing tests** in `envelope.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::{claude_code::ClaudeCodeSource, TraceSource};

    fn fixture_transcript() -> crate::source::SessionTranscript {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let src = ClaudeCodeSource::new(root);
        let refs = src.discover().unwrap();
        src.load(&refs[0]).unwrap()
    }

    fn test_config() -> crate::config::ContributorConfig {
        crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: "sha256:00".into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        }
    }

    #[tokio::test]
    async fn envelope_has_schema_version_and_no_local_paths_or_secrets() {
        let t = fixture_transcript();
        let cfg = test_config();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        assert_eq!(raw.submission_id, crate::source::submission_id_for(&t.session_hash));
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::new(
                vec!["/Users/testuser".into()],
            )
            .unwrap();
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        assert_eq!(
            envelope.schema_version,
            trace_commons_protocol::trace_contribution::TRACE_CONTRIBUTION_SCHEMA_VERSION
        );
        let json = serde_json::to_string(&envelope).unwrap();
        // The fixture's fake secret value must not survive redaction.
        assert!(!json.contains("sk-fake-fixture-secret-1234"));
        // The full local path prefix must not survive.
        assert!(!json.contains("/Users/testuser"));
        // Project basename and agent tag do survive.
        assert!(json.contains("myproj"));
        assert!(json.contains("claude-code"));
    }

    #[test]
    fn canary_self_test_passes_for_deterministic_redactor() {
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        canary_self_test(&redactor).unwrap();
    }

    #[test]
    fn near_ai_filter_fails_closed_without_key() {
        let mut cfg = test_config();
        cfg.pii_filter = Some("near-ai".into());
        // No settings injected: must refuse, never downgrade to deterministic-only.
        assert!(build_redactor_with(&cfg, None, None).is_err());
    }

    #[tokio::test]
    async fn near_ai_filter_redacts_via_mock_endpoint() {
        // Stub NEAR AI classify endpoint: flags "bob@example.com" as private_email.
        use axum::{routing::post, Json, Router};
        let router = Router::new().route(
            "/privacy/classify",
            post(|Json(req): Json<serde_json::Value>| async move {
                let input = req["input"].as_str().unwrap_or_default().to_string();
                let spans = match input.find("bob@example.com") {
                    Some(start) => serde_json::json!([{
                        "category": "private_email",
                        "start": start,
                        "end": start + "bob@example.com".len(),
                        "score": 0.99
                    }]),
                    None => serde_json::json!([]),
                };
                Json(serde_json::json!({"data": [{"spans": spans}]}))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            kind: crate::source::SessionEventKind::User,
            timestamp: None,
            content: Some("please email bob@example.com about this".into()),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
        });
        let mut cfg = test_config();
        cfg.pii_filter = Some("near-ai".into());
        let redactor = build_redactor_with(
            &cfg,
            Some("/Users/testuser/code/myproj"),
            Some(NearAiSettings {
                api_key: "test-key".into(),
                base_url: Some(base),
                model: None,
            }),
        )
        .unwrap();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        let json = serde_json::to_string(&envelope).unwrap();
        assert!(!json.contains("bob@example.com"));
    }

    #[tokio::test]
    async fn oversized_envelope_is_refused() {
        let mut t = fixture_transcript();
        t.events.push(crate::source::SessionEvent {
            kind: crate::source::SessionEventKind::Assistant,
            timestamp: None,
            content: Some("x".repeat(MAX_ENVELOPE_BYTES + 1)),
            structured: serde_json::Value::Null,
            tool_name: None,
            token_counts: None,
        });
        let cfg = test_config();
        let raw = build_raw_contribution(&t, &cfg, chrono::Utc::now());
        let redactor =
            trace_commons_protocol::trace_contribution::DeterministicTraceRedactor::try_default()
                .unwrap();
        let envelope = redact_to_envelope(&redactor, raw).await.unwrap();
        assert!(envelope_size_ok(&envelope).is_err());
    }
}
```

Note: if `canary_self_test_passes_for_deterministic_redactor` fails because the deterministic pass alone does not strip all canary values (some canaries target the PII filter), scope the check: `canary_self_test` must only assert on canary values the deterministic pipeline is responsible for — inspect `synthetic_privacy_filter_canary_values()` and, if needed, restrict the hard-fail to secret-shaped values (those caught by `redact_text` on the canary text). Whatever subset is chosen, the test asserts the self-test passes on a correctly-constructed redactor and the plan's contract stands: a canary hit at submit time aborts the batch.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p trace-commons-contributor envelope -v`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement `envelope.rs`** per the mapping table.

- [ ] **Step 4: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor envelope`
Expected: PASS (4 tests).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/envelope.rs crates/trace-commons-contributor/src/lib.rs
git commit -m "Map transcripts to envelopes through the deterministic redaction pipeline"
```

---

### Task 10: Submit pipeline, receipts, status

**Files:**
- Create: `crates/trace-commons-contributor/src/submit.rs`
- Modify: `crates/trace-commons-contributor/src/lib.rs` (add `pub mod submit;`)

**Interfaces:**
- Consumes: everything above; `trace_commons_operator_client::Client` with `bearer_token` (Task 2); `trace_commons_protocol::trace_contribution::{TraceSubmissionReceipt, TraceSubmissionStatusRequest, TraceSubmissionStatusUpdate}`.
- Produces:

```rust
#[derive(Debug)]
pub enum SubmitOutcome {
    Submitted { submission_id: Uuid, status: String },
    AlreadySubmitted { submission_id: Uuid },
    SkippedParseFailure { reason_label: String },
    Refused { reason_label: String },      // canary hit, fail-closed PII filter, too large
    Failed { reason_label: String },       // network/auth after retries
}

pub struct SubmitOptions {
    pub dry_run: bool,
    pub pii_filter: Option<String>,
}

pub async fn submit_sessions(
    store: &ConfigStore,
    cfg: &ContributorConfig,
    sessions: Vec<(Box<dyn TraceSource>, SessionRef)>,   // pre-selected by the caller
    opts: &SubmitOptions,
) -> anyhow::Result<Vec<SubmitOutcome>>;

pub async fn status(store: &ConfigStore, cfg: &ContributorConfig) -> anyhow::Result<Vec<TraceSubmissionStatusUpdate>>;
```

**`submit_sessions` per-session flow (sessions are independent; one failure never aborts the batch):**
1. `source.load(&r)` — error → `SkippedParseFailure`.
2. Skip if a receipt with the same `session_hash` and status in `{"submitted","accepted","quarantined"}` exists → `AlreadySubmitted`.
3. Build redactor via `build_redactor(cfg_with_opts_pii_filter, transcript.cwd)` (opts.pii_filter overrides cfg.pii_filter) — error → `Refused` with label `"pii-filter-unavailable"`. Run `canary_self_test` once per batch (before session 1); failure aborts the whole batch with an error (fail closed).
4. `build_raw_contribution` → `redact_to_envelope` (error → `Refused { "redaction-failed" }`) → `envelope_size_ok` (error → `Refused { "session-too-large" }`).
5. If `opts.dry_run`: print submission_id + byte size, outcome `Submitted { status: "dry-run" }`, do not upload, do not write a receipt.
6. Mint/reuse claim: keep one `Option<ClaimToken>` across the batch; re-mint via `IssuerClient::mint_claim` when `!is_fresh(now)` (60s skew inside `ClaimToken::is_fresh`). Mint error → `Failed { "claim-mint-failed" }` for this session (and subsequent sessions will retry the mint).
7. Upload: build `Client::builder(&cfg.ingest_url, "UNUSED").bearer_token(&token.access_token)` with `HostAllowlist::from_csv(...)` when `cfg.allowed_hosts` is set, then `client.call_json::<TraceContributionEnvelope, TraceSubmissionReceipt>(Method::POST, "/v1/traces", &[], Some(&envelope))`. On `Error::Transport`: retry up to 3 attempts total with 1s then 4s sleeps. On `Error::ServerLabel`/`HttpFailure` with status 401/403: re-mint the claim once and retry once; if still failing → `Failed { "auth-failed" }`. Other errors → `Failed { <error.kind()> }`.
8. Append `Receipt { submission_id, session_hash, source, submitted_at: now, status: receipt.status.clone() }`; outcome `Submitted`.

**`status` flow:** load receipts; if empty print nothing and return empty; chunk submission_ids by 500; mint a claim; `client.call_json::<TraceSubmissionStatusRequest, Vec<TraceSubmissionStatusUpdate>>(Method::POST, "/v1/contributors/me/submission-status", &[], Some(&req))` per chunk; concatenate.

- [ ] **Step 1: Write failing tests** in `submit.rs` — a stub ingest server plus a stub issuer:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use axum::{routing::post, Json, Router};
    use std::sync::{Arc, Mutex};

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    fn stub_issuer() -> Router {
        Router::new().route(
            "/v1/trace-upload-claim",
            post(|| async {
                Json(serde_json::json!({
                    "access_token": "stub-claim-jwt",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                }))
            }),
        )
    }

    fn stub_ingest(received: Arc<Mutex<Vec<serde_json::Value>>>) -> Router {
        Router::new().route(
            "/v1/traces",
            post(move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
                let received = received.clone();
                async move {
                    assert_eq!(
                        headers.get("authorization").unwrap(),
                        "Bearer stub-claim-jwt"
                    );
                    received.lock().unwrap().push(body);
                    Json(serde_json::json!({
                        "status": "accepted",
                        "credit_points_pending": 0.0,
                        "explanation": []
                    }))
                }
            }),
        )
    }

    fn fixture_selection() -> Vec<(Box<dyn crate::source::TraceSource>, crate::source::SessionRef)> {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
        let src = crate::source::claude_code::ClaudeCodeSource::new(root.clone());
        let r = src.discover().unwrap().remove(0);
        vec![(
            Box::new(crate::source::claude_code::ClaudeCodeSource::new(root)) as Box<dyn crate::source::TraceSource>,
            r,
        )]
    }

    fn cfg_for(issuer: &str, ingest: &str, device_key_id: &str) -> crate::config::ContributorConfig {
        crate::config::ContributorConfig {
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.into(),
            issuer_url: issuer.into(),
            ingest_url: ingest.into(),
            audience: "trace-commons-upload".into(),
            tenant_id: "tenant-abc".into(),
            instance_id: "instance-1".into(),
            user_subject: "alice".into(),
            device_key_id: device_key_id.into(),
            consent_scopes: vec!["debugging_evaluation".into()],
            pii_filter: None,
            allowed_hosts: None,
        }
    }

    #[tokio::test]
    async fn submits_fixture_session_and_is_idempotent_on_rerun() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions { dry_run: false, pii_filter: None };

        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts).await.unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        assert_eq!(received.lock().unwrap().len(), 1);
        let sent = &received.lock().unwrap()[0];
        assert_eq!(sent["schema_version"], "ironclaw.trace_contribution.v1");
        assert!(!serde_json::to_string(sent).unwrap().contains("sk-fake-fixture-secret-1234"));

        // Second run: receipt short-circuits, no second upload.
        let outcomes2 = submit_sessions(&store, &cfg, fixture_selection(), &opts).await.unwrap();
        assert!(matches!(outcomes2[0], SubmitOutcome::AlreadySubmitted { .. }));
        assert_eq!(received.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn dry_run_uploads_nothing_and_writes_no_receipt() {
        let received = Arc::new(Mutex::new(Vec::new()));
        let issuer = spawn(stub_issuer()).await;
        let ingest = spawn(stub_ingest(received.clone())).await;
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        let cfg = cfg_for(&issuer, &ingest, &device.device_key_id);
        let opts = SubmitOptions { dry_run: true, pii_filter: None };
        let outcomes = submit_sessions(&store, &cfg, fixture_selection(), &opts).await.unwrap();
        assert!(matches!(outcomes[0], SubmitOutcome::Submitted { .. }));
        assert_eq!(received.lock().unwrap().len(), 0);
        assert!(store.load_receipts().unwrap().is_empty());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p trace-commons-contributor submit -v`
Expected: FAIL — module does not exist.

- [ ] **Step 3: Implement `submit.rs`** per the flow above. `dry_run` short-circuits BEFORE claim minting (no network at all).

- [ ] **Step 4: Run tests**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor submit`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src/submit.rs crates/trace-commons-contributor/src/lib.rs
git commit -m "Add submit pipeline with claim reuse, receipts, and status readback"
```

---

### Task 11: List and submit UX (picker, filters)

**Files:**
- Create: `crates/trace-commons-contributor/src/picker.rs`
- Modify: `crates/trace-commons-contributor/src/commands.rs` (add `list` and `submit` command fns, `status` printing)
- Modify: `crates/trace-commons-contributor/src/bin/trace-commons-contributor.rs` (wire List/Submit/Status)

**Interfaces:**
- Produces:
  - `pub fn parse_selection(input: &str, max: usize) -> anyhow::Result<Vec<usize>>` — accepts `"3"`, `"1,3-5"`, `"a"`/`"all"`; 1-based in input, returns 0-based sorted deduped indices; rejects out-of-range with a clear message.
  - `pub fn parse_since(s: &str) -> anyhow::Result<chrono::Duration>` — suffixes `h`/`d` (e.g. `12h`, `2d`); bare integers are days.
  - `commands::list(...)`: discovers across `all_sources(None, None)`, prints a table (via `trace_commons_operator_client::format::print_table`) with columns `#`, `SOURCE`, `PROJECT`, `AGE`, `SIZE`, and a trailing `submitted` marker sourced from receipts (match on session_hash requires loading; for list, mark from receipts by path-independent lazy load only when `--verbose`; default table skips the marker if not cheaply available — keep v1 simple: no marker in `list`, the picker in `submit` shows it because submit loads sessions anyway).
  - `commands::submit(...)`: applies `--source`, `--project` (matches `SessionRef.project` basename OR the ref path prefix), `--since` (against `started_at`/mtime) filters; with `--all` or `--yes` skips the picker; otherwise prints the numbered table and reads a selection line from stdin; maps selections to `(Box<dyn TraceSource>, SessionRef)` pairs; calls `submit_sessions`; prints one outcome line per session (`submitted <uuid> accepted`, `skipped (parse-failure)`, `refused (session-too-large)`, ...); exit code: `anyhow::bail!` at the end if any outcome was `Refused` or `Failed` (nonzero exit), otherwise Ok.
  - `commands::status(...)`: calls `submit::status`, prints table `SUBMISSION`, `STATUS`, `PENDING`, `FINAL`.

- [ ] **Step 1: Write failing tests** for the pure helpers in `picker.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_single_range_and_all() {
        assert_eq!(parse_selection("3", 5).unwrap(), vec![2]);
        assert_eq!(parse_selection("1,3-5", 5).unwrap(), vec![0, 2, 3, 4]);
        assert_eq!(parse_selection("a", 3).unwrap(), vec![0, 1, 2]);
        assert_eq!(parse_selection("all", 3).unwrap(), vec![0, 1, 2]);
        assert!(parse_selection("6", 5).is_err());
        assert!(parse_selection("0", 5).is_err());
        assert!(parse_selection("", 5).is_err());
    }

    #[test]
    fn parses_since() {
        assert_eq!(parse_since("12h").unwrap(), chrono::Duration::hours(12));
        assert_eq!(parse_since("2d").unwrap(), chrono::Duration::days(2));
        assert_eq!(parse_since("3").unwrap(), chrono::Duration::days(3));
        assert!(parse_since("nope").is_err());
    }
}
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p trace-commons-contributor picker -v`
Expected: FAIL.

- [ ] **Step 3: Implement** `picker.rs`, the command fns, and bin wiring. Reading the selection: `std::io::stdin().read_line`. All human output goes through plain `println!`; no content, paths beyond project basenames, or tokens are ever printed.

- [ ] **Step 4: Run tests + smoke the binary**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor && cargo run -p trace-commons-contributor -- list`
Expected: tests PASS; `list` prints your real local sessions table (manual eyeball: no full paths).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/src
git commit -m "Add session picker, filters, and list/submit/status wiring"
```

---

### Task 12: End-to-end test against the real issuer router

**Files:**
- Create: `crates/trace-commons-contributor/tests/e2e_enroll_and_submit.rs`

**Interfaces:**
- Consumes: the whole lib; from `trace_commons_server`: `trace_upload_claim_issuer::{trace_upload_claim_issuer_router, TraceUploadClaimIssuerConfig}` and the `Database` trait from its db module.
- Produces: one `#[tokio::test]` proving: mint-grant → login (against the real issuer's `/v1/enroll`) → claim mint (real issuer verifies the device signature) → submit (stub ingest) → receipts written, and `tenant_id == derive_user_tenant_id(instance_id, user_subject)`.

**Test skeleton (adapt imports/config fields to what compiles — the authoritative field list is `TraceUploadClaimIssuerConfig` at trace_upload_claim_issuer.rs:101; the reference in-memory Database impl to copy is `PerUserTestDeviceKeyDb` at `crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs:70425`, adapted so `reserve_instance_enrollment` returns Ok, `enroll_instance_user` stores the device key record, and `get_device_key` returns it):**

```rust
use std::sync::Arc;

// 1. In-memory Database impl: copy PerUserTestDeviceKeyDb from
//    crates/trace-commons-server/src/bin/trace_commons_ingest_internal/tests.rs:70425
//    and make three methods real:
//    - reserve_instance_enrollment(..) -> Ok(())
//    - enroll_instance_user(provision) -> insert DeviceKeyRecord for (tenant_id, device_key_id)
//    - get_device_key(tenant, key_id) -> return stored record
//    Everything else keeps the stub/todo shape from the reference.

#[tokio::test]
async fn enroll_mint_submit_round_trip() {
    // Instance keypair + allowlist file registering it.
    let instance_doc =
        ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new()).unwrap();
    let instance_kp = ring::signature::Ed25519KeyPair::from_pkcs8(instance_doc.as_ref()).unwrap();
    use ring::signature::KeyPair as _;
    use base64::Engine as _;
    let instance_pk_b64 =
        base64::engine::general_purpose::STANDARD.encode(instance_kp.public_key().as_ref());

    let tmp = tempfile::tempdir().unwrap();
    let allowlist_path = tmp.path().join("allowlist.json");
    std::fs::write(
        &allowlist_path,
        serde_json::json!({
            "version": 1,
            "generated_at": chrono::Utc::now(),
            "policy_label": "e2e",
            "entries": [{
                "kind": "instance",
                "instance_id": "instance-e2e",
                "instance_public_key": instance_pk_b64,
                "max_enrollments": 10,
                "rate_per_min": 60
            }]
        })
        .to_string(),
    )
    .unwrap();

    // Stub ingest first (its URL goes into the issuer config as onboarding_ingest_url).
    let received = Arc::new(std::sync::Mutex::new(Vec::<serde_json::Value>::new()));
    let ingest_url = {
        use axum::{routing::post, Json, Router};
        let sink = received.clone();
        let router = Router::new()
            .route(
                "/v1/traces",
                post(move |headers: axum::http::HeaderMap, Json(body): Json<serde_json::Value>| {
                    let sink = sink.clone();
                    async move {
                        // Real EdDSA claim minted by the real issuer rides in here.
                        let auth = headers.get("authorization").unwrap().to_str().unwrap();
                        assert!(auth.starts_with("Bearer "));
                        assert!(auth.len() > "Bearer ".len() + 20);
                        sink.lock().unwrap().push(body);
                        Json(serde_json::json!({
                            "status": "accepted",
                            "credit_points_pending": 0.0,
                            "explanation": []
                        }))
                    }
                }),
            )
            .route(
                "/v1/contributors/me/submission-status",
                post(|Json(req): Json<serde_json::Value>| async move {
                    let first = req["submission_ids"][0].clone();
                    Json(serde_json::json!([{
                        "submission_id": first,
                        "trace_id": "00000000-0000-0000-0000-000000000000",
                        "status": "accepted",
                        "credit_points_pending": 0.0,
                        "explanation": []
                    }]))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        url
    };

    // Real issuer router. Use the TEST_EDDSA PEM constants exported from
    // trace_upload_claim_issuer.rs (they are pub(crate) in tests today; if not
    // reachable from an external test, generate a keypair via
    // trace_commons_server::trace_upload_claim_issuer::generate_upload_claim_keypair()
    // and use its private/public PEMs for signing_private_key_pem / signing_public_key_pem
    // and workload_public_key_pem).
    let db = Arc::new(InMemoryEnrollDb::new());
    let keys = trace_commons_server::trace_upload_claim_issuer::generate_upload_claim_keypair().unwrap();
    let config = trace_commons_server::trace_upload_claim_issuer::TraceUploadClaimIssuerConfig {
        bind: "127.0.0.1:0".into(),
        signing_private_key_pem: keys.private_key_pem.clone(),
        signing_public_key_pem: keys.public_key_pem.clone(),
        signing_kid: keys.suggested_kid.clone(),
        issuer: "trace-commons-upload-issuer".into(),
        audience: "trace-commons-upload".into(),
        max_ttl_seconds: 300,
        workload_public_key_pem: keys.public_key_pem.clone(),
        workload_issuer: None,
        workload_audience: None,
        tenant_access_grant_db: None,
        require_tenant_access_grants: false,
        shutdown_grace_seconds: 30,
        request_timeout_seconds: 10,
        max_request_bytes: 64 * 1024,
        allowlist_source: Some(format!("file:{}", allowlist_path.display())),
        allowlist_refresh_interval_seconds: 60,
        allowlist_max_stale_seconds: 3600,
        onboarding_device_key_db: Some(db.clone()),
        onboarding_ingest_url: Some(ingest_url.clone()),
        admin_bind: None,
        // NOTE: this field list mirrors the reference literal in
        // trace_commons_ingest_internal/tests.rs (search "TraceUploadClaimIssuerConfig {").
        // If the struct has gained fields since, copy the reference test's values for
        // them; the overrides above (allowlist_source, onboarding_device_key_db,
        // onboarding_ingest_url) are the ones that matter for this e2e.
    };
    let router = trace_commons_server::trace_upload_claim_issuer::trace_upload_claim_issuer_router(config).unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let issuer_url = format!("http://{}", listener.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });

    // CLI-side flow, all through lib functions.
    let store = trace_commons_contributor::config::ConfigStore::open(tmp.path().join("cfg")).unwrap();
    let device = trace_commons_contributor::identity::DeviceIdentity::load_or_generate(&store).unwrap();
    let grant = trace_commons_contributor::identity::mint_grant(
        instance_doc.as_ref(), &issuer_url, "instance-e2e", "alice@example.com",
        "trace-commons-upload", &device.device_key_id, 300, chrono::Utc::now(),
    ).unwrap();
    trace_commons_contributor::commands::login(&store, Some(&grant.encode())).await.unwrap();

    let cfg = store.load_config().unwrap().unwrap();
    assert_eq!(
        cfg.tenant_id,
        trace_commons_protocol::onboarding::derive_user_tenant_id("instance-e2e", "alice@example.com")
    );
    assert_eq!(cfg.ingest_url, ingest_url);

    // Submit the Claude Code fixture through the real claim path.
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("fixtures/claude-code");
    let src = trace_commons_contributor::source::claude_code::ClaudeCodeSource::new(root.clone());
    let r = src.discover().unwrap().remove(0);
    let outcomes = trace_commons_contributor::submit::submit_sessions(
        &store, &cfg,
        vec![(Box::new(trace_commons_contributor::source::claude_code::ClaudeCodeSource::new(root)) as _, r)],
        &trace_commons_contributor::submit::SubmitOptions { dry_run: false, pii_filter: None },
    ).await.unwrap();
    assert!(matches!(outcomes[0], trace_commons_contributor::submit::SubmitOutcome::Submitted { .. }));
    assert_eq!(received.lock().unwrap().len(), 1);
    assert_eq!(store.load_receipts().unwrap().len(), 1);
}
```

- [ ] **Step 1: Write the test** (it will not compile — that is the failing state). The only adaptation point is the in-memory Database impl (copy from `PerUserTestDeviceKeyDb` at the cited line and make the three named methods real). If `Database`/`DeviceKeyRecord` are not importable from `trace_commons_server` (they live under its db module — check `trace_commons_server::db` exports), adjust imports; if a needed type is `pub(crate)`-only, make it `pub` in the server crate with a one-line doc comment noting the e2e consumer — that visibility bump is the only permitted server-crate edit, and it must not change behavior.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p trace-commons-contributor --test e2e_enroll_and_submit`
Expected: FAIL to compile (missing InMemoryEnrollDb etc.).

- [ ] **Step 3: Implement the in-memory Database impl + config literal, iterate until green.** Also add the submission-status route to the stub ingest and assert `submit::status` returns one row (extend the stub: `/v1/contributors/me/submission-status` returns `[{"submission_id": <echo first id>, "trace_id": "00000000-0000-0000-0000-000000000000", "status": "accepted", "credit_points_pending": 0.0, "explanation": []}]`).

- [ ] **Step 4: Run the full crate suite**

Run: `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor`
Expected: PASS, all tests including e2e.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/tests crates/trace-commons-server/src 2>/dev/null || git add crates/trace-commons-contributor/tests
git commit -m "Add end-to-end enroll, claim, and submit test against real issuer router"
```

---

### Task 13: Docs and final verification

**Files:**
- Create: `crates/trace-commons-contributor/README.md`
- Modify: `README.md` (repo root — add the contributor CLI to the binaries/components list, one short paragraph)
- Modify: `docs/trace-commons-roadmap.md` (mark the contributor-CLI item if one exists in the Production Gap Queue; otherwise add a "shipped" line referencing the spec)

**Interfaces:** none (documentation + verification gate).

- [ ] **Step 1: Write `crates/trace-commons-contributor/README.md`** covering: what it is (three sentences); install (cargo build for now; GitHub Releases binaries are a follow-up); quickstart (`login` without grant → get grant from instance → `login --grant` → `submit`); the consent model (debugging_evaluation scope in v1, deterministic local secret redaction, optional `--pii-filter near-ai` with `TRACE_NEAR_AI_PRIVACY_API_KEY`, server rescrub); config/keystore file locations and permissions; every subcommand with one line each; the `mint-grant` operator flow for dogfooding. State the v1 scope caps (device-key claims are debugging/evaluation-scoped server-side).

- [ ] **Step 2: Update root `README.md` and roadmap** as described in Files.

- [ ] **Step 3: Full verification sweep**

Run, in order, all must pass:
```bash
cargo fmt --all -- --check
RUSTFLAGS="-D warnings" cargo check -p trace-commons-contributor
RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins
RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor
RUSTFLAGS="-D warnings" cargo test -p trace-commons-operator-client
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo clippy --workspace --all-targets -- -D warnings -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
```
Expected: all green. If fmt complains about pre-existing files outside this plan's touch set, scope to `cargo fmt -p trace-commons-contributor -- --check` and note it.

- [ ] **Step 4: Manual smoke (optional but recommended)**

Run: `cargo run -p trace-commons-contributor -- submit --dry-run --source claude-code --since 1d`
Expected: picker lists your real recent sessions; selecting one prints its submission id and byte size; nothing uploads.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-contributor/README.md README.md docs/trace-commons-roadmap.md
git commit -m "Document contributor CLI and complete verification sweep"
```
