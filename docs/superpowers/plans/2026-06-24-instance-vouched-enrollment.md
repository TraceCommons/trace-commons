# Instance-Vouched Enrollment Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let an operator authorize an Ironclaw instance once, after which every user on it self-enrolls into their own per-user Trace Commons tenant with no per-user invite code.

**Architecture:** A new `POST /v1/enroll` on the upload-claim issuer accepts an Ed25519 attestation the instance signs over a user's device key. The issuer verifies it against a registered instance public key, derives a per-user tenant id (`"tenant-" + sha256(instance_id ‖ user_subject)`), enforces a per-instance user cap via a new instance-scoped control-plane ledger, then provisions the tenant (row + contribution policy) and registers the device key — all idempotently. The existing invite `/v1/onboard` path is untouched.

**Tech Stack:** Rust, axum, PostgreSQL (deadpool-postgres), `ring` ED25519, `sha2`, `serde`. Server crate `crates/trace-commons-server`, protocol crate `crates/trace-commons-protocol`.

## Global Constraints

- PostgreSQL-only. Single `cargo check -p trace-commons-server` is sufficient; no libsql.
- Verify with warnings-as-errors before claiming green:
  `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins` and
  `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`.
- Clippy with the repo allow-list: `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`.
- Hash-only / label-only: never store or log the raw instance public key, `user_subject`, nonce, signature, device public key bytes, URLs, or contributor identity. `tenant_id` is itself a hash.
- Fail-closed: any verification, rate, cap, replay, provisioning, or DB error returns one generic refusal; never fall through.
- Forced RLS on every Trace Commons table. New ledger uses an instance-scoped predicate (`trace_current_instance_subject()`); no `BYPASSRLS`.
- No new dependencies (all of `ring`, `sha2`, `serde`, `serde_json`, `base64`, `hex`, `chrono` are already direct deps).
- No emojis; short imperative commit subjects (no `feat:`/`fix:` prefixes) — the example commit messages below use the repo style.
- **Dependency / fallback:** the V30 `trace_accounts` tables are NOT on this branch. This plan uses the no-account fallback: enroll registers the device key (the principal) without an account row. Account creation is back-filled when contributor-account Slice 1 (V30) merges. The new migration is **V31** (gaps are fine; the runner gates each version independently).

---

### Task 1: Protocol types, attestation encoder, and tenant derivation

**Files:**
- Modify: `crates/trace-commons-protocol/src/onboarding.rs`

**Interfaces:**
- Produces:
  - `const TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION: &str = "trace_commons.instance_enroll_request.v1"`
  - `struct TraceInstanceEnrollAttestation { device_key_id: String, aud: String, instance_id: String, user_subject: String, nonce: String, exp: i64 }` (Serialize/Deserialize/Clone/Debug/PartialEq/Eq)
  - `struct TraceInstanceEnrollRequest { schema_version: String, instance_public_key: String, device_public_key: String, attestation: TraceInstanceEnrollAttestation, attestation_sig: String, client_info: TraceOnboardClientInfo }`
  - `fn instance_enroll_attestation_signing_bytes(a: &TraceInstanceEnrollAttestation) -> Vec<u8>`
  - `fn derive_user_tenant_id(instance_id: &str, user_subject: &str) -> String`
  - `fn user_subject_hash(user_subject: &str) -> String`
  - New `TraceOnboardErrorCode` variants: `EnrollMalformed`, `EnrollNotAuthorized`, `EnrollRateLimited`, `EnrollCapExceeded`.

- [ ] **Step 1: Write the failing test**

Add to the `tests` module at the bottom of `crates/trace-commons-protocol/src/onboarding.rs`:

```rust
#[test]
fn derive_user_tenant_id_is_stable_and_separator_safe() {
    let a = derive_user_tenant_id("inst", "user-1");
    assert_eq!(a, derive_user_tenant_id("inst", "user-1"));
    assert!(a.starts_with("tenant-"));
    // The 0x1F separator prevents (a,bc) colliding with (ab,c).
    assert_ne!(
        derive_user_tenant_id("a", "bc"),
        derive_user_tenant_id("ab", "c")
    );
}

#[test]
fn user_subject_hash_is_sha256_shaped() {
    let h = user_subject_hash("user-1");
    assert!(h.starts_with("sha256:"));
    assert_eq!(h.len(), "sha256:".len() + 64);
    assert_eq!(h, user_subject_hash("user-1"));
    assert_ne!(h, user_subject_hash("user-2"));
}

#[test]
fn attestation_signing_bytes_are_unambiguous() {
    let base = TraceInstanceEnrollAttestation {
        device_key_id: "sha256:aa".into(),
        aud: "trace-commons-ingest".into(),
        instance_id: "inst".into(),
        user_subject: "user-1".into(),
        nonce: "n".into(),
        exp: 100,
    };
    let mut moved = base.clone();
    moved.device_key_id = "sha256:a".into();
    moved.aud = "atrace-commons-ingest".into();
    // Field-boundary shift must change the signing bytes.
    assert_ne!(
        instance_enroll_attestation_signing_bytes(&base),
        instance_enroll_attestation_signing_bytes(&moved)
    );
}

#[test]
fn instance_enroll_request_round_trips() {
    let req = TraceInstanceEnrollRequest {
        schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.to_string(),
        instance_public_key: "cHVia2V5".into(),
        device_public_key: "ZGV2a2V5".into(),
        attestation: TraceInstanceEnrollAttestation {
            device_key_id: "sha256:aa".into(),
            aud: "trace-commons-ingest".into(),
            instance_id: "inst".into(),
            user_subject: "user-1".into(),
            nonce: "n".into(),
            exp: 100,
        },
        attestation_sig: "c2ln".into(),
        client_info: TraceOnboardClientInfo { agent: "ironclaw".into(), version: "0.x".into() },
    };
    let encoded = serde_json::to_string(&req).unwrap();
    let decoded: TraceInstanceEnrollRequest = serde_json::from_str(&encoded).unwrap();
    assert_eq!(decoded, req);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-protocol onboarding:: -- --nocapture`
Expected: FAIL — `derive_user_tenant_id`, `user_subject_hash`, `instance_enroll_attestation_signing_bytes`, and the new structs are not defined.

- [ ] **Step 3: Write minimal implementation**

Add near the top of `onboarding.rs` (after the existing `TRACE_ONBOARD_*` consts):

```rust
pub const TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION: &str =
    "trace_commons.instance_enroll_request.v1";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceInstanceEnrollAttestation {
    pub device_key_id: String,
    pub aud: String,
    pub instance_id: String,
    pub user_subject: String,
    pub nonce: String,
    pub exp: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TraceInstanceEnrollRequest {
    pub schema_version: String,
    pub instance_public_key: String,
    pub device_public_key: String,
    pub attestation: TraceInstanceEnrollAttestation,
    pub attestation_sig: String,
    pub client_info: TraceOnboardClientInfo,
}

/// Canonical, unambiguous signing bytes for an enrollment attestation. Each
/// field is length-prefixed (u64-le) so no field-boundary shift can collide.
/// This is the single source of truth shared by the Ironclaw signer and the
/// issuer verifier — keep it the only encoder.
pub fn instance_enroll_attestation_signing_bytes(
    a: &TraceInstanceEnrollAttestation,
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(b"trace_commons.instance_enroll.v1\n");
    for field in [
        a.device_key_id.as_str(),
        a.aud.as_str(),
        a.instance_id.as_str(),
        a.user_subject.as_str(),
        a.nonce.as_str(),
    ] {
        out.extend_from_slice(&(field.len() as u64).to_le_bytes());
        out.extend_from_slice(field.as_bytes());
    }
    out.extend_from_slice(&a.exp.to_le_bytes());
    out
}

/// Derive the per-user tenant id. `0x1F` (unit separator) between the two
/// fields makes the concatenation injective. The result is a hash, so it is
/// non-identifying. One function, no drift — shared by signer and server.
pub fn derive_user_tenant_id(instance_id: &str, user_subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(instance_id.as_bytes());
    hasher.update([0x1F]);
    hasher.update(user_subject.as_bytes());
    format!("tenant-{}", hex::encode(hasher.finalize()))
}

/// Hash-only form of the per-user subject for the enrollment ledger.
pub fn user_subject_hash(user_subject: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"user_subject:");
    hasher.update(user_subject.as_bytes());
    format!("sha256:{}", hex::encode(hasher.finalize()))
}
```

Add the four variants to the `TraceOnboardErrorCode` enum and its `as_wire_str` match:

```rust
    EnrollMalformed,
    EnrollNotAuthorized,
    EnrollRateLimited,
    EnrollCapExceeded,
```
```rust
            Self::EnrollMalformed => "EnrollMalformed",
            Self::EnrollNotAuthorized => "EnrollNotAuthorized",
            Self::EnrollRateLimited => "EnrollRateLimited",
            Self::EnrollCapExceeded => "EnrollCapExceeded",
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p trace-commons-protocol onboarding::`
Expected: PASS (all four new tests plus existing ones).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-protocol/src/onboarding.rs
git commit -m "Add instance-enroll protocol types and tenant derivation"
```

---

### Task 2: Instance subject hashing and allowlist schema extension

**Files:**
- Modify: `crates/trace-commons-server/src/trace_upload_claim_allowlist.rs`

**Interfaces:**
- Consumes: nothing new.
- Produces:
  - `fn hash_instance_subject(public_key_bytes: &[u8]) -> String`
  - `struct InstancePolicyTemplate { policy_version, allowed_consent_scopes, allowed_uses }` — the `Deserialize` file shape.
  - `struct InstancePolicyTemplateSnapshot { policy_version: String, allowed_consent_scopes: Vec<String>, allowed_uses: Vec<String> }` — the owned snapshot shape.
  - `struct InstanceSnapshotEntry { instance_subject_hash: String, instance_id: String, instance_public_key: Vec<u8>, max_enrollments: u32, rate_per_min: Option<u32>, policy_template: InstancePolicyTemplateSnapshot, contributor_label: Option<String> }`
  - `AllowlistSnapshot::instance_entry(&self, instance_subject_hash: &str) -> Option<&InstanceSnapshotEntry>`

- [ ] **Step 1: Write the failing test**

Add to the `tests` module of `trace_upload_claim_allowlist.rs`:

```rust
#[test]
fn hash_instance_subject_is_namespaced_against_invites() {
    let pk = [7u8; 32];
    let h = hash_instance_subject(&pk);
    assert!(h.starts_with("sha256:"));
    assert_eq!(h.len(), "sha256:".len() + 64);
    // "instance:" prefix cannot collide with "invite:"-prefixed codes.
    assert_ne!(h, hash_invite_code(&String::from_utf8_lossy(&pk)));
}

#[test]
fn snapshot_parses_instance_entry_with_policy_template() {
    let pk_b64 = base64::engine::general_purpose::STANDARD.encode([3u8; 32]);
    let file: AllowlistFile = serde_json::from_str(&format!(
        r#"{{
            "version": 1,
            "generated_at": "2026-06-24T00:00:00Z",
            "policy_label": "pilot",
            "entries": [{{
                "kind": "instance",
                "instance_id": "ironclaw-acme-prod",
                "instance_public_key": "{pk_b64}",
                "max_enrollments": 5000,
                "rate_per_min": 60,
                "policy_template": {{
                    "policy_version": "ironclaw-pilot-v1",
                    "allowed_consent_scopes": ["pilot_research"],
                    "allowed_uses": ["model_training"]
                }},
                "note_label": "ironclaw-acme-prod"
            }}]
        }}"#
    ))
    .expect("instance allowlist JSON parses");
    let snap = AllowlistSnapshot::from_file(file, "test".into(), Instant::now()).expect("ok");
    let subject = hash_instance_subject(&[3u8; 32]);
    let entry = snap.instance_entry(&subject).expect("instance entry by hash");
    assert_eq!(entry.instance_id, "ironclaw-acme-prod");
    assert_eq!(entry.max_enrollments, 5000);
    assert_eq!(entry.rate_per_min, Some(60));
    assert_eq!(entry.policy_template.policy_version, "ironclaw-pilot-v1");
    assert_eq!(entry.instance_public_key, vec![3u8; 32]);
}

#[test]
fn existing_invite_only_file_still_parses_without_kind() {
    let h = hash_invite_code("INV-1");
    let file: AllowlistFile = serde_json::from_str(&format!(
        r#"{{
            "version": 1,
            "generated_at": "2026-05-17T00:00:00Z",
            "policy_label": "pilot",
            "entries": [{{"subject_hash": "{h}", "tenant_id": "t"}}]
        }}"#
    ))
    .expect("legacy invite JSON parses");
    let snap = AllowlistSnapshot::from_file(file, "test".into(), Instant::now()).expect("ok");
    assert!(snap.contains(&h));
    assert!(snap.instance_entry(&h).is_none());
}

#[test]
fn instance_entry_rejects_bad_pubkey_len() {
    let short = base64::engine::general_purpose::STANDARD.encode([1u8; 16]);
    let file: AllowlistFile = serde_json::from_str(&format!(
        r#"{{
            "version": 1, "generated_at": "2026-06-24T00:00:00Z", "policy_label": "p",
            "entries": [{{
                "kind": "instance", "instance_id": "i", "instance_public_key": "{short}",
                "max_enrollments": 1,
                "policy_template": {{"policy_version": "v", "allowed_consent_scopes": [], "allowed_uses": []}}
            }}]
        }}"#
    ))
    .unwrap();
    assert!(matches!(
        AllowlistSnapshot::from_file(file, "test".into(), Instant::now()),
        Err(AllowlistError::Malformed(_))
    ));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --lib trace_upload_claim_allowlist:: -- --nocapture`
Expected: FAIL — `hash_instance_subject`, `instance_entry`, instance fields not defined.

- [ ] **Step 3: Write minimal implementation**

Add the hashing function next to `hash_invite_code`:

```rust
/// Canonical instance-subject hashing. The `"instance:"` prefix namespaces the
/// digest so an instance public key can never collide with an invite-code
/// subject hash. Single source of truth for ledger keys, denial accounting,
/// and audit actor labels.
pub fn hash_instance_subject(public_key_bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(b"instance:");
    hasher.update(public_key_bytes);
    format!("sha256:{}", hex::encode(hasher.finalize()))
}
```

Make `hex` and `base64::Engine` available — add `use base64::Engine as _;` to the test module if missing (the crate already depends on both).

Replace the `AllowlistEntry` definition with a tagged form that keeps invites the default. Add the instance shapes:

```rust
fn default_entry_kind() -> String {
    "invite".to_string()
}

#[derive(Debug, Deserialize)]
pub struct InstancePolicyTemplate {
    pub policy_version: String,
    #[serde(default)]
    pub allowed_consent_scopes: Vec<String>,
    #[serde(default)]
    pub allowed_uses: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct AllowlistEntry {
    #[serde(default = "default_entry_kind")]
    pub kind: String,
    // Invite fields (kind = "invite").
    #[serde(default)]
    pub subject_hash: Option<String>,
    #[serde(default)]
    pub tenant_id: Option<String>,
    #[serde(default)]
    pub note_label: Option<String>,
    #[serde(default = "default_allowlist_max_uses")]
    pub max_uses: u32,
    // Instance fields (kind = "instance").
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub instance_public_key: Option<String>,
    #[serde(default)]
    pub max_enrollments: Option<u32>,
    #[serde(default)]
    pub rate_per_min: Option<u32>,
    #[serde(default)]
    pub policy_template: Option<InstancePolicyTemplate>,
}

#[derive(Debug, Clone)]
pub struct InstancePolicyTemplateSnapshot {
    pub policy_version: String,
    pub allowed_consent_scopes: Vec<String>,
    pub allowed_uses: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct InstanceSnapshotEntry {
    pub instance_subject_hash: String,
    pub instance_id: String,
    pub instance_public_key: Vec<u8>,
    pub max_enrollments: u32,
    pub rate_per_min: Option<u32>,
    pub policy_template: InstancePolicyTemplateSnapshot,
    pub contributor_label: Option<String>,
}
```

> NOTE for the implementer: `tenant_id` and `subject_hash` became `Option` — fix the existing invite-parsing loop in `AllowlistSnapshot::from_file` to read `entry.subject_hash.as_deref()` / `entry.tenant_id.as_deref()` and reject `kind == "invite"` rows missing either (preserving today's `Malformed` behavior). Skip invite parsing for `kind == "instance"` rows.

Extend `AllowlistSnapshot` with an instance map and lookup. In the struct add:

```rust
    instances_by_hash: HashMap<String, InstanceSnapshotEntry>,
```

In `from_file`, build it alongside the invite loop:

```rust
let mut instances_by_hash: HashMap<String, InstanceSnapshotEntry> = HashMap::new();
for entry in &file.entries {
    if entry.kind != "instance" {
        continue;
    }
    let instance_id = entry.instance_id.as_deref().map(str::trim).unwrap_or("");
    if instance_id.is_empty() {
        return Err(AllowlistError::Malformed("instance_id must be non-empty".into()));
    }
    let pk_b64 = entry.instance_public_key.as_deref().unwrap_or("").trim();
    let pk = base64::engine::general_purpose::STANDARD
        .decode(pk_b64)
        .map_err(|_| AllowlistError::Malformed("instance_public_key not base64".into()))?;
    if pk.len() != 32 {
        return Err(AllowlistError::Malformed("instance_public_key must be 32 bytes".into()));
    }
    let max_enrollments = entry.max_enrollments.unwrap_or(0);
    if max_enrollments == 0 {
        return Err(AllowlistError::Malformed("max_enrollments must be > 0".into()));
    }
    let tmpl = entry
        .policy_template
        .as_ref()
        .ok_or_else(|| AllowlistError::Malformed("instance entry requires policy_template".into()))?;
    if tmpl.policy_version.trim().is_empty() {
        return Err(AllowlistError::Malformed("policy_version must be non-empty".into()));
    }
    let subject = hash_instance_subject(&pk);
    let contributor_label = entry
        .note_label
        .as_deref()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(ToString::to_string);
    instances_by_hash.entry(subject.clone()).or_insert(InstanceSnapshotEntry {
        instance_subject_hash: subject,
        instance_id: instance_id.to_string(),
        instance_public_key: pk,
        max_enrollments,
        rate_per_min: entry.rate_per_min,
        policy_template: InstancePolicyTemplateSnapshot {
            policy_version: tmpl.policy_version.trim().to_string(),
            allowed_consent_scopes: tmpl.allowed_consent_scopes.clone(),
            allowed_uses: tmpl.allowed_uses.clone(),
        },
        contributor_label,
    });
}
```

Add `instances_by_hash` to the returned `Self { ... }` and the lookup method:

```rust
    pub fn instance_entry(&self, instance_subject_hash: &str) -> Option<&InstanceSnapshotEntry> {
        self.instances_by_hash.get(instance_subject_hash)
    }
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p trace-commons-server --lib trace_upload_claim_allowlist::`
Expected: PASS — including the existing invite tests (regression for the `Option` refactor).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/trace_upload_claim_allowlist.rs
git commit -m "Add instance subject hashing and instance allowlist entries"
```

---

### Task 3: V31 enrollment-ledger migration and runner wiring

**Files:**
- Create: `migrations/V31__trace_instance_enrollments.sql`
- Modify: `crates/trace-commons-server/src/db/postgres.rs` (migration sequence, after the V29 block)
- Test: `crates/trace-commons-server/tests/trace_corpus_pg_store.rs` (or the existing pg contract test file — add a migration/RLS case)

**Interfaces:**
- Produces: table `trace_instance_enrollments`, function `trace_current_instance_subject()`, applied as migration version `31`.

- [ ] **Step 1: Write the migration SQL**

Create `migrations/V31__trace_instance_enrollments.sql`:

```sql
-- V31: instance-enrollment ledger (control plane above per-user tenants).
-- Isolated on a parallel INSTANCE predicate, not the tenant predicate: this
-- table is intentionally cross-tenant (an instance maps to many per-user
-- tenants), so tenant RLS would defeat its purpose. Hash-only columns.

CREATE TABLE trace_instance_enrollments (
    instance_subject_hash TEXT NOT NULL
        CHECK (instance_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    user_subject_hash     TEXT NOT NULL
        CHECK (user_subject_hash ~ '^sha256:[0-9a-f]{64}$'),
    tenant_id             TEXT NOT NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (instance_subject_hash, user_subject_hash)
);

CREATE INDEX idx_trace_instance_enrollments_instance
    ON trace_instance_enrollments (instance_subject_hash);

ALTER TABLE trace_instance_enrollments ENABLE ROW LEVEL SECURITY;
ALTER TABLE trace_instance_enrollments FORCE ROW LEVEL SECURITY;

CREATE OR REPLACE FUNCTION trace_current_instance_subject()
RETURNS TEXT
LANGUAGE SQL
STABLE
AS $$
    SELECT NULLIF(current_setting('trace_commons.instance_subject', true), '');
$$;

DROP POLICY IF EXISTS trace_instance_isolation ON trace_instance_enrollments;
CREATE POLICY trace_instance_isolation ON trace_instance_enrollments
    USING      (instance_subject_hash = trace_current_instance_subject())
    WITH CHECK (instance_subject_hash = trace_current_instance_subject());

GRANT SELECT, INSERT ON trace_instance_enrollments TO trace_commons_runtime;
```

> NOTE: confirm the runtime role name by grepping existing migrations for `GRANT ... TO` (e.g. `grep -rn "GRANT" migrations/V28__device_keys.sql`); use whatever role those grants target. If migrations rely on default ownership grants and have no explicit `GRANT`, drop the `GRANT` line to match.

- [ ] **Step 2: Wire the migration into the runner**

In `crates/trace-commons-server/src/db/postgres.rs`, find the last migration block (version `29`) in the `run_migrations`/setup function and append, copying the exact shape of the preceding block:

```rust
        if client
            .query_opt(
                "SELECT 1 FROM _trace_commons_migrations WHERE version = $1",
                &[&31_i32],
            )
            .await?
            .is_none()
        {
            client
                .batch_execute(include_str!(
                    "../../../../migrations/V31__trace_instance_enrollments.sql"
                ))
                .await?;
            client
                .execute(
                    "INSERT INTO _trace_commons_migrations (version, name) VALUES ($1, $2)",
                    &[&31_i32, &"trace_instance_enrollments"],
                )
                .await?;
        }
```

- [ ] **Step 3: Write the failing RLS test**

Add to the PostgreSQL-backed contract test (the file holding existing `trace_corpus_pg` cases; gate it the same `#[ignore]`/env way the repo gates pg tests):

```rust
#[tokio::test]
async fn instance_enrollment_ledger_is_instance_scoped() {
    let backend = pg_test_backend().await; // use the repo's existing pg test harness
    let mut client = backend.trace_pool().get().await.unwrap();
    // Insert under instance A's context.
    let tx = client.transaction().await.unwrap();
    tx.execute("SELECT set_config('trace_commons.instance_subject', $1, true)",
        &[&format!("sha256:{}", "a".repeat(64))]).await.unwrap();
    tx.execute(
        "INSERT INTO trace_instance_enrollments (instance_subject_hash, user_subject_hash, tenant_id)
         VALUES ($1, $2, $3)",
        &[&format!("sha256:{}", "a".repeat(64)),
          &format!("sha256:{}", "b".repeat(64)),
          &"tenant-deadbeef"]).await.unwrap();
    tx.commit().await.unwrap();
    // Under instance B's context the row is invisible.
    let tx = client.transaction().await.unwrap();
    tx.execute("SELECT set_config('trace_commons.instance_subject', $1, true)",
        &[&format!("sha256:{}", "c".repeat(64))]).await.unwrap();
    let rows = tx.query("SELECT 1 FROM trace_instance_enrollments", &[]).await.unwrap();
    assert!(rows.is_empty(), "instance B must not see instance A rows");
}
```

- [ ] **Step 4: Run the test (requires PostgreSQL)**

Run: `cargo test -p trace-commons-server --test trace_corpus_pg_store instance_enrollment_ledger_is_instance_scoped -- --ignored`
Expected: PASS (table + policy applied; cross-instance read returns empty).

- [ ] **Step 5: Commit**

```bash
git add migrations/V31__trace_instance_enrollments.sql crates/trace-commons-server/src/db/postgres.rs crates/trace-commons-server/tests/
git commit -m "Add V31 instance-enrollment ledger with instance-scoped RLS"
```

---

### Task 4: Ledger DB ops — reserve enrollment with atomic cap

**Files:**
- Modify: `crates/trace-commons-server/src/db/mod.rs` (trait `Database`)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` (impl)
- Test: PostgreSQL contract test file

**Interfaces:**
- Consumes: `trace_instance_enrollments` (Task 3).
- Produces on `trait Database`:
  - `enum InstanceEnrollmentOutcome { NewlyEnrolled, ExistingUser, CapExceeded }`
  - `async fn reserve_instance_enrollment(&self, instance_subject_hash: &str, user_subject_hash: &str, tenant_id: &str, max_enrollments: i64) -> Result<InstanceEnrollmentOutcome, DatabaseError>`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn reserve_instance_enrollment_dedups_and_caps() {
    let backend = pg_test_backend().await;
    let inst = format!("sha256:{}", "1".repeat(64));
    let u1 = format!("sha256:{}", "2".repeat(64));
    let u2 = format!("sha256:{}", "3".repeat(64));
    // cap = 1.
    assert!(matches!(
        backend.reserve_instance_enrollment(&inst, &u1, "tenant-u1", 1).await.unwrap(),
        InstanceEnrollmentOutcome::NewlyEnrolled));
    // same user again -> existing, no cap consumption.
    assert!(matches!(
        backend.reserve_instance_enrollment(&inst, &u1, "tenant-u1", 1).await.unwrap(),
        InstanceEnrollmentOutcome::ExistingUser));
    // a second distinct user exceeds cap = 1.
    assert!(matches!(
        backend.reserve_instance_enrollment(&inst, &u2, "tenant-u2", 1).await.unwrap(),
        InstanceEnrollmentOutcome::CapExceeded));
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --test trace_corpus_pg_store reserve_instance_enrollment_dedups_and_caps -- --ignored`
Expected: FAIL — method not defined.

- [ ] **Step 3: Write minimal implementation**

In `db/mod.rs` add to the `Database` trait and the outcome enum:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstanceEnrollmentOutcome {
    NewlyEnrolled,
    ExistingUser,
    CapExceeded,
}

// inside trait Database:
    async fn reserve_instance_enrollment(
        &self,
        instance_subject_hash: &str,
        user_subject_hash: &str,
        tenant_id: &str,
        max_enrollments: i64,
    ) -> Result<InstanceEnrollmentOutcome, DatabaseError>;
```

In `db/trace_corpus_pg.rs` implement it with a single instance-scoped transaction:

```rust
    async fn reserve_instance_enrollment(
        &self,
        instance_subject_hash: &str,
        user_subject_hash: &str,
        tenant_id: &str,
        max_enrollments: i64,
    ) -> Result<InstanceEnrollmentOutcome, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await.map_err(DatabaseError::Postgres)?;
        tx.execute(
            "SELECT set_config('trace_commons.instance_subject', $1, true)",
            &[&instance_subject_hash],
        ).await.map_err(DatabaseError::Postgres)?;

        // Already enrolled? Idempotent, no cap consumption.
        let existing = tx.query_opt(
            "SELECT 1 FROM trace_instance_enrollments
              WHERE instance_subject_hash = $1 AND user_subject_hash = $2",
            &[&instance_subject_hash, &user_subject_hash],
        ).await.map_err(DatabaseError::Postgres)?;
        if existing.is_some() {
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(InstanceEnrollmentOutcome::ExistingUser);
        }

        // Lock-free cap check: count, then insert ON CONFLICT DO NOTHING.
        let count: i64 = tx.query_one(
            "SELECT COUNT(*)::BIGINT FROM trace_instance_enrollments
              WHERE instance_subject_hash = $1",
            &[&instance_subject_hash],
        ).await.map_err(DatabaseError::Postgres)?.get(0);
        if count >= max_enrollments {
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(InstanceEnrollmentOutcome::CapExceeded);
        }
        let inserted = tx.execute(
            "INSERT INTO trace_instance_enrollments
                 (instance_subject_hash, user_subject_hash, tenant_id)
             VALUES ($1, $2, $3)
             ON CONFLICT (instance_subject_hash, user_subject_hash) DO NOTHING",
            &[&instance_subject_hash, &user_subject_hash, &tenant_id],
        ).await.map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        // A racing insert of the SAME user resolves to ExistingUser.
        Ok(if inserted == 1 {
            InstanceEnrollmentOutcome::NewlyEnrolled
        } else {
            InstanceEnrollmentOutcome::ExistingUser
        })
    }
```

> NOTE: a concurrent burst of DISTINCT new users could each read `count < cap` and all insert, overshooting the cap by the concurrency width. For the pilot's per-instance rate limit this is acceptable; if strict capping is later required, take an advisory lock on `hashtext(instance_subject_hash)` at the top of the tx. Document this in the method doc-comment.

Add a matching stub to any in-memory/mock `Database` impl used by issuer unit tests (search for other `impl Database for` blocks; return `NewlyEnrolled` or a test-controlled value).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p trace-commons-server --test trace_corpus_pg_store reserve_instance_enrollment_dedups_and_caps -- --ignored`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/db/
git commit -m "Add reserve_instance_enrollment ledger op with atomic cap"
```

---

### Task 5: Provisioning DB op — ensure tenant, stamp policy, register device key

**Files:**
- Modify: `crates/trace-commons-server/src/db/mod.rs` (trait)
- Modify: `crates/trace-commons-server/src/db/trace_corpus_pg.rs` (impl)
- Test: PostgreSQL contract test file

**Interfaces:**
- Consumes: `ensure_trace_tenant`, `begin_trace_tenant_transaction` (existing), `DeviceKeyWrite` (existing).
- Produces on `trait Database`:
  - `struct InstanceUserProvision { device_key_id: String, tenant_id: String, public_key: String, instance_subject_hash: String, client_info: serde_json::Value, policy_version: String, allowed_consent_scopes: serde_json::Value, allowed_uses: serde_json::Value }`
  - `async fn enroll_instance_user(&self, p: InstanceUserProvision) -> Result<(), DatabaseError>`

- [ ] **Step 1: Write the failing test**

```rust
#[tokio::test]
async fn enroll_instance_user_provisions_tenant_and_device_key() {
    let backend = pg_test_backend().await;
    let tenant = "tenant-prov-test";
    let p = InstanceUserProvision {
        device_key_id: format!("sha256:{}", "d".repeat(64)),
        tenant_id: tenant.to_string(),
        public_key: "ZGV2a2V5".to_string(),
        instance_subject_hash: format!("sha256:{}", "e".repeat(64)),
        client_info: serde_json::json!({"agent":"ironclaw","version":"0.x"}),
        policy_version: "ironclaw-pilot-v1".to_string(),
        allowed_consent_scopes: serde_json::json!(["pilot_research"]),
        allowed_uses: serde_json::json!(["model_training"]),
    };
    backend.enroll_instance_user(p.clone()).await.unwrap();
    // Re-running is idempotent and does not overwrite the policy.
    backend.enroll_instance_user(p).await.unwrap();

    // Tenant row, policy row, and device key all exist under tenant context.
    let mut client = backend.trace_pool().get().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.execute("SELECT set_config('trace_commons.trace_tenant_id', $1, true)", &[&tenant]).await.unwrap();
    assert_eq!(tx.query_one("SELECT COUNT(*) FROM trace_tenants WHERE tenant_id=$1", &[&tenant]).await.unwrap().get::<_, i64>(0), 1);
    assert_eq!(tx.query_one("SELECT policy_version FROM trace_tenant_policies WHERE tenant_id=$1", &[&tenant]).await.unwrap().get::<_, String>(0), "ironclaw-pilot-v1");
    assert_eq!(tx.query_one("SELECT COUNT(*) FROM device_keys WHERE tenant_id=$1", &[&tenant]).await.unwrap().get::<_, i64>(0), 1);
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --test trace_corpus_pg_store enroll_instance_user_provisions_tenant_and_device_key -- --ignored`
Expected: FAIL — method/struct not defined.

- [ ] **Step 3: Write minimal implementation**

In `db/mod.rs`:

```rust
#[derive(Debug, Clone)]
pub struct InstanceUserProvision {
    pub device_key_id: String,
    pub tenant_id: String,
    pub public_key: String,
    pub instance_subject_hash: String,
    pub client_info: serde_json::Value,
    pub policy_version: String,
    pub allowed_consent_scopes: serde_json::Value,
    pub allowed_uses: serde_json::Value,
}

// trait Database:
    async fn enroll_instance_user(&self, p: InstanceUserProvision) -> Result<(), DatabaseError>;
```

In `db/trace_corpus_pg.rs`:

```rust
    async fn enroll_instance_user(&self, p: InstanceUserProvision) -> Result<(), DatabaseError> {
        self.ensure_trace_tenant(&p.tenant_id).await?;
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &p.tenant_id).await?;

        // Stamp the contribution policy once; never overwrite an existing one.
        tx.execute(
            "INSERT INTO trace_tenant_policies
                 (tenant_id, policy_version, allowed_consent_scopes, allowed_uses,
                  updated_by_principal_ref)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (tenant_id) DO NOTHING",
            &[
                &p.tenant_id,
                &p.policy_version,
                &p.allowed_consent_scopes,
                &p.allowed_uses,
                &format!("instance-enroll:{}", p.instance_subject_hash),
            ],
        ).await.map_err(DatabaseError::Postgres)?;

        // Register the device key (the principal). Idempotent on device_key_id.
        tx.execute(
            "INSERT INTO device_keys
                 (device_key_id, tenant_id, public_key, invite_subject_hash, client_info)
             VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (device_key_id) DO NOTHING",
            &[
                &p.device_key_id,
                &p.tenant_id,
                &p.public_key,
                &p.instance_subject_hash,
                &p.client_info,
            ],
        ).await.map_err(DatabaseError::Postgres)?;

        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(())
    }
```

> NOTE: confirm the `device_keys` primary key / unique column name by reading `migrations/V28__device_keys.sql` (the `ON CONFLICT` target must match it — `device_key_id` per the onboard insert at `db/postgres.rs:1242`). Confirm `trace_tenant_policies` column list against `migrations/V1__trace_commons_schema.sql:17-25`; add any NOT NULL column it requires with a sensible default.

Add the stub to any mock `Database` impl (return `Ok(())`).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p trace-commons-server --test trace_corpus_pg_store enroll_instance_user_provisions_tenant_and_device_key -- --ignored`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/db/
git commit -m "Add enroll_instance_user provisioning op (no-account fallback)"
```

---

### Task 6: Process-local replay cache and per-instance rate limiter

**Files:**
- Create: `crates/trace-commons-server/src/instance_enroll_guard.rs`
- Modify: `crates/trace-commons-server/src/lib.rs` (add `pub mod instance_enroll_guard;`)

**Interfaces:**
- Produces:
  - `struct ReplayCache` with `fn new() -> Self` and `fn consume(&self, key: &str, ttl: Duration, now: Instant) -> bool` (true = fresh/accepted, false = replay)
  - `struct InstanceRateLimiter` with `fn new() -> Self` and `fn try_acquire(&self, subject: &str, rate_per_min: u32, now: Instant) -> bool`

- [ ] **Step 1: Write the failing test**

Create `crates/trace-commons-server/src/instance_enroll_guard.rs` with only the tests first:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    #[test]
    fn replay_cache_blocks_second_use_until_ttl() {
        let cache = ReplayCache::new();
        let t0 = Instant::now();
        assert!(cache.consume("inst|nonce", Duration::from_secs(300), t0));
        assert!(!cache.consume("inst|nonce", Duration::from_secs(300), t0));
        // After TTL the key is evicted and a (hypothetical) reuse is fresh again.
        let later = t0 + Duration::from_secs(301);
        assert!(cache.consume("inst|nonce", Duration::from_secs(300), later));
    }

    #[test]
    fn rate_limiter_refuses_past_budget() {
        let rl = InstanceRateLimiter::new();
        let t0 = Instant::now();
        // rate_per_min = 2 -> 2 tokens available at start.
        assert!(rl.try_acquire("inst", 2, t0));
        assert!(rl.try_acquire("inst", 2, t0));
        assert!(!rl.try_acquire("inst", 2, t0));
        // Half a minute later ~1 token refilled.
        assert!(rl.try_acquire("inst", 2, t0 + Duration::from_secs(31)));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --lib instance_enroll_guard:: -- --nocapture`
Expected: FAIL — types not defined.

- [ ] **Step 3: Write minimal implementation**

Prepend to `instance_enroll_guard.rs`:

```rust
//! Process-local enrollment guards: nonce replay cache + per-instance token
//! bucket. Restart-resets (matching the allowlist DenialCounter posture); the
//! attestation `exp` and derived-tenant idempotency are the durable backstop.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub struct ReplayCache {
    seen: Mutex<HashMap<String, Instant>>, // key -> expiry
}

impl ReplayCache {
    pub fn new() -> Self {
        Self { seen: Mutex::new(HashMap::new()) }
    }

    /// Returns true if `key` is fresh (and records it until `now + ttl`);
    /// false if it is a replay within its TTL. Evicts expired keys on each call.
    pub fn consume(&self, key: &str, ttl: Duration, now: Instant) -> bool {
        let mut seen = self.seen.lock().expect("ReplayCache poisoned");
        seen.retain(|_, &mut expiry| expiry > now);
        if seen.contains_key(key) {
            return false;
        }
        seen.insert(key.to_string(), now + ttl);
        true
    }
}

impl Default for ReplayCache {
    fn default() -> Self { Self::new() }
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

pub struct InstanceRateLimiter {
    buckets: Mutex<HashMap<String, Bucket>>,
}

impl InstanceRateLimiter {
    pub fn new() -> Self {
        Self { buckets: Mutex::new(HashMap::new()) }
    }

    /// Token bucket: capacity = `rate_per_min`, refill = `rate_per_min`/60 per
    /// second. Returns true if a token was available and consumed.
    pub fn try_acquire(&self, subject: &str, rate_per_min: u32, now: Instant) -> bool {
        if rate_per_min == 0 {
            return false;
        }
        let cap = rate_per_min as f64;
        let refill_per_sec = cap / 60.0;
        let mut buckets = self.buckets.lock().expect("InstanceRateLimiter poisoned");
        let bucket = buckets.entry(subject.to_string()).or_insert(Bucket { tokens: cap, last: now });
        let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_per_sec).min(cap);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }
}

impl Default for InstanceRateLimiter {
    fn default() -> Self { Self::new() }
}
```

Add `pub mod instance_enroll_guard;` to `crates/trace-commons-server/src/lib.rs` (alphabetical with the other `pub mod` lines).

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p trace-commons-server --lib instance_enroll_guard::`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/instance_enroll_guard.rs crates/trace-commons-server/src/lib.rs
git commit -m "Add process-local enroll replay cache and rate limiter"
```

---

### Task 7: Issuer enroll handler and `/v1/enroll` route

**Files:**
- Modify: `crates/trace-commons-server/src/trace_upload_claim_issuer.rs`

**Interfaces:**
- Consumes: Task 1 protocol types + `derive_user_tenant_id` + `user_subject_hash` + `instance_enroll_attestation_signing_bytes`; Task 2 `hash_instance_subject` + `instance_entry`; Task 4 `reserve_instance_enrollment` + `InstanceEnrollmentOutcome`; Task 5 `enroll_instance_user` + `InstanceUserProvision`; Task 6 `ReplayCache` + `InstanceRateLimiter`.
- Produces: `POST /v1/enroll` handler returning `TraceOnboardResponse`; `IssuerState::enroll`; verify helper `verify_instance_attestation_signature`.

- [ ] **Step 1: Write the failing test**

Add to the issuer test module (mirror `onboard_*` tests; use the same in-test `IssuerState` builder + mock `Database`). Generate an Ed25519 keypair with `ring` (the test module already constructs keypairs — reuse that helper):

```rust
#[tokio::test]
async fn enroll_happy_path_provisions_user_tenant() {
    let (signing, verifying_pk_bytes) = test_ed25519_keypair(); // returns (Ed25519KeyPair, [u8;32])
    let state = issuer_state_with_instance_entry(&verifying_pk_bytes, "ironclaw-acme", 100, 60).await;
    let device_pk = [9u8; 32];
    let device_key_id = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(&device_pk);
    let attestation = TraceInstanceEnrollAttestation {
        device_key_id: device_key_id.clone(),
        aud: state.audience.clone(),
        instance_id: "ironclaw-acme".into(),
        user_subject: "user-1".into(),
        nonce: "nonce-1".into(),
        exp: (Utc::now() + Duration::minutes(4)).timestamp(),
    };
    let sig = signing.sign(&instance_enroll_attestation_signing_bytes(&attestation));
    let req = TraceInstanceEnrollRequest {
        schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.into(),
        instance_public_key: b64(&verifying_pk_bytes),
        device_public_key: b64(&device_pk),
        attestation,
        attestation_sig: b64(sig.as_ref()),
        client_info: TraceOnboardClientInfo { agent: "ironclaw".into(), version: "0.x".into() },
    };
    let resp = state.enroll(req).await.expect("enroll succeeds");
    assert_eq!(resp.tenant_id,
        trace_commons_protocol::onboarding::derive_user_tenant_id("ironclaw-acme", "user-1"));
    assert_eq!(resp.device_key_id, device_key_id);
}

#[tokio::test]
async fn enroll_rejects_bad_signature_uniformly() {
    let (_signing, verifying_pk_bytes) = test_ed25519_keypair();
    let state = issuer_state_with_instance_entry(&verifying_pk_bytes, "ironclaw-acme", 100, 60).await;
    let device_pk = [9u8; 32];
    let attestation = TraceInstanceEnrollAttestation {
        device_key_id: trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(&device_pk),
        aud: state.audience.clone(),
        instance_id: "ironclaw-acme".into(),
        user_subject: "user-1".into(),
        nonce: "nonce-1".into(),
        exp: (Utc::now() + Duration::minutes(4)).timestamp(),
    };
    let req = TraceInstanceEnrollRequest {
        schema_version: TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION.into(),
        instance_public_key: b64(&verifying_pk_bytes),
        device_public_key: b64(&device_pk),
        attestation,
        attestation_sig: b64(&[0u8; 64]), // garbage signature
        client_info: TraceOnboardClientInfo { agent: "ironclaw".into(), version: "0.x".into() },
    };
    let err = state.enroll(req).await.unwrap_err();
    assert_eq!(err.status(), StatusCode::FORBIDDEN); // generic, non-enumerating
}
```

> The implementer adds small test helpers `test_ed25519_keypair`, `issuer_state_with_instance_entry`, and `b64` next to the existing onboard test helpers. `issuer_state_with_instance_entry` builds an `IssuerState` whose allowlist snapshot contains one instance entry (write a temp allowlist JSON file and point a `FileAllowlistSource` at it, mirroring `onboard` tests) and whose `onboarding_device_key_db` is the mock backend.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p trace-commons-server --lib trace_upload_claim_issuer::tests::enroll_ -- --nocapture`
Expected: FAIL — `enroll`, `/v1/enroll`, verify helper not defined.

- [ ] **Step 3: Write minimal implementation**

Add the verify helper next to `verify_device_claim_signature`:

```rust
fn verify_instance_attestation_signature(
    instance_public_key: &[u8],
    signing_bytes: &[u8],
    signature: &[u8],
) -> Result<(), IssuerError> {
    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, instance_public_key)
        .verify(signing_bytes, signature)
        .map_err(|_| {
            IssuerError::onboard_error(StatusCode::FORBIDDEN, TraceOnboardErrorCode::EnrollNotAuthorized)
        })
}
```

Add `ReplayCache` + `InstanceRateLimiter` fields to `IssuerState` (construct with `::new()` wherever `IssuerState` is built, including the test builder and `build_state`), plus an env-config default for the rate:

```rust
// In IssuerState struct:
    instance_replay_cache: Arc<crate::instance_enroll_guard::ReplayCache>,
    instance_rate_limiter: Arc<crate::instance_enroll_guard::InstanceRateLimiter>,
    instance_enroll_default_rate_per_min: u32,
```

Read the default from env in the config builder (default 60):
`std::env::var("TRACE_COMMONS_INSTANCE_ENROLL_RATE_PER_MIN").ok().and_then(|v| v.parse().ok()).unwrap_or(60)`.

Implement `enroll` on `IssuerState` (mirror `onboard`'s structure):

```rust
    async fn enroll(
        &self,
        request: TraceInstanceEnrollRequest,
    ) -> Result<TraceOnboardResponse, IssuerError> {
        use crate::trace_upload_claim_allowlist::hash_instance_subject;
        use trace_commons_protocol::onboarding::{
            device_key_id_from_public_key_bytes, derive_user_tenant_id,
            instance_enroll_attestation_signing_bytes, user_subject_hash,
            TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION,
            TRACE_ONBOARD_RESPONSE_SCHEMA_VERSION,
        };

        let malformed = || IssuerError::onboard_error(
            StatusCode::BAD_REQUEST, TraceOnboardErrorCode::EnrollMalformed);
        let denied = || IssuerError::onboard_error(
            StatusCode::FORBIDDEN, TraceOnboardErrorCode::EnrollNotAuthorized);

        if request.schema_version != TRACE_INSTANCE_ENROLL_REQUEST_SCHEMA_VERSION {
            return Err(malformed());
        }
        let b64 = base64::engine::general_purpose::STANDARD;
        let instance_pk = b64.decode(request.instance_public_key.trim()).map_err(|_| malformed())?;
        let device_pk = b64.decode(request.device_public_key.trim()).map_err(|_| malformed())?;
        if instance_pk.len() != 32 || device_pk.len() != 32 {
            return Err(malformed());
        }
        let sig = b64.decode(request.attestation_sig.trim()).map_err(|_| malformed())?;

        // Resolve the registered instance entry.
        let subject = hash_instance_subject(&instance_pk);
        let snapshot = self.onboard_allowlist_snapshot()?;
        let entry = match snapshot.instance_entry(&subject) {
            Some(e) => e.clone(),
            None => { self.denial_counter.record(); return Err(denied()); }
        };

        // Verify the signature against the REGISTERED key bytes.
        let signing_bytes = instance_enroll_attestation_signing_bytes(&request.attestation);
        verify_instance_attestation_signature(&entry.instance_public_key, &signing_bytes, &sig)?;

        // Bind aud, instance, freshness, device id. All misses -> uniform deny.
        let a = &request.attestation;
        let now = Utc::now().timestamp();
        let device_key_id = device_key_id_from_public_key_bytes(&device_pk);
        if a.aud != self.audience
            || a.instance_id != entry.instance_id
            || a.exp <= now
            || a.exp > now + 300
            || a.device_key_id != device_key_id
        {
            return Err(denied());
        }

        // Replay dedupe (process-local).
        let replay_key = format!("{}|{}", subject, a.nonce);
        let ttl = std::time::Duration::from_secs((a.exp - now).max(0) as u64);
        if !self.instance_replay_cache.consume(&replay_key, ttl, std::time::Instant::now()) {
            return Err(denied());
        }

        // Rate limit.
        let rate = entry.rate_per_min.unwrap_or(self.instance_enroll_default_rate_per_min);
        if !self.instance_rate_limiter.try_acquire(&subject, rate, std::time::Instant::now()) {
            self.denial_counter.record();
            return Err(IssuerError::onboard_error(
                StatusCode::TOO_MANY_REQUESTS, TraceOnboardErrorCode::EnrollRateLimited));
        }

        // Derive tenant + reserve against the cap.
        let tenant_id = derive_user_tenant_id(&entry.instance_id, &a.user_subject);
        let user_hash = user_subject_hash(&a.user_subject);
        let db = self.onboarding_device_key_db.as_ref()
            .ok_or_else(IssuerError::onboard_registry_not_configured)?;
        let outcome = db.reserve_instance_enrollment(
            &subject, &user_hash, &tenant_id, entry.max_enrollments as i64,
        ).await.map_err(|_| IssuerError::internal())?;
        if matches!(outcome, crate::db::InstanceEnrollmentOutcome::CapExceeded) {
            self.denial_counter.record();
            return Err(IssuerError::onboard_error(
                StatusCode::FORBIDDEN, TraceOnboardErrorCode::EnrollCapExceeded));
        }

        // Provision tenant + policy + device key (idempotent).
        let client_info = serde_json::to_value(&request.client_info).map_err(|_| malformed())?;
        db.enroll_instance_user(crate::db::InstanceUserProvision {
            device_key_id: device_key_id.clone(),
            tenant_id: tenant_id.clone(),
            public_key: request.device_public_key.trim().to_string(),
            instance_subject_hash: subject.clone(),
            client_info,
            policy_version: entry.policy_template.policy_version.clone(),
            allowed_consent_scopes: serde_json::to_value(&entry.policy_template.allowed_consent_scopes).map_err(|_| IssuerError::internal())?,
            allowed_uses: serde_json::to_value(&entry.policy_template.allowed_uses).map_err(|_| IssuerError::internal())?,
        }).await.map_err(|_| IssuerError::internal())?;

        let ingest_url = self.onboarding_ingest_url.clone()
            .ok_or_else(IssuerError::onboard_tenant_config_missing)?;
        tracing::info!(
            instance_subject = %subject, tenant_id = %tenant_id,
            outcome = ?outcome, "instance_enroll");
        Ok(TraceOnboardResponse {
            schema_version: TRACE_ONBOARD_RESPONSE_SCHEMA_VERSION.to_string(),
            tenant_id,
            ingest_url,
            issuer_url: self.issuer.clone(),
            audience: self.audience.clone(),
            device_key_id,
            contributor_label: entry.contributor_label.clone(),
            community_url: self.onboarding_community_url.clone(),
            profile_url: self.onboarding_profile_url.clone(),
            leaderboard_url: self.onboarding_leaderboard_url.clone(),
        })
    }
```

Add the axum handler + route. Next to `onboard_handler`:

```rust
async fn enroll_handler(
    State(state): State<Arc<IssuerState>>,
    Json(request): Json<TraceInstanceEnrollRequest>,
) -> Result<Json<TraceOnboardResponse>, IssuerError> {
    state.enroll(request).await.map(Json)
}
```

In BOTH router builders (`router` at line ~761 and `router_from_state` at ~846) add after the `/v1/onboard` route:

```rust
        .route("/v1/enroll", post(enroll_handler))
```

Import the new protocol type at the top `use trace_commons_protocol::onboarding::{...}` group:
`TraceInstanceEnrollRequest`.

> NOTE: a `Json` extractor rejection (malformed body) yields a 422 by default. To keep enroll non-enumerating, the implementer may add a `DefaultBodyLimit` consistent with `/v1/onboard` and accept the 422 for unparseable JSON (a structural error, not an oracle over secrets) — matching how `/v1/onboard` handles body errors.

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p trace-commons-server --lib trace_upload_claim_issuer::tests::enroll_`
Expected: PASS (happy path + uniform bad-signature deny).

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/trace_upload_claim_issuer.rs
git commit -m "Add /v1/enroll instance-vouched enrollment handler"
```

---

### Task 8: End-to-end regression — multi-device, cap, replay, invite untouched

**Files:**
- Modify: `crates/trace-commons-server/src/trace_upload_claim_issuer.rs` (test module)

**Interfaces:**
- Consumes: everything above.

- [ ] **Step 1: Write the failing tests**

```rust
#[tokio::test]
async fn enroll_second_device_same_user_reuses_tenant_and_does_not_consume_cap() {
    let (signing, pk) = test_ed25519_keypair();
    let state = issuer_state_with_instance_entry(&pk, "inst", 1, 60).await; // cap = 1 user
    let t1 = enroll_user_device(&state, &signing, &pk, "inst", "user-1", [1u8;32], "n1").await.unwrap();
    let t2 = enroll_user_device(&state, &signing, &pk, "inst", "user-1", [2u8;32], "n2").await.unwrap();
    assert_eq!(t1.tenant_id, t2.tenant_id); // same user -> same tenant
    // A different user exceeds cap = 1.
    let err = enroll_user_device(&state, &signing, &pk, "inst", "user-2", [3u8;32], "n3").await.unwrap_err();
    assert_eq!(err.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn enroll_replayed_nonce_is_refused() {
    let (signing, pk) = test_ed25519_keypair();
    let state = issuer_state_with_instance_entry(&pk, "inst", 100, 60).await;
    let device = [7u8;32];
    let att = signed_attestation(&signing, &state.audience, "inst", "user-1", &device, "dup-nonce");
    let req = || enroll_request(&pk, &device, att.clone());
    state.enroll(req()).await.expect("first use ok");
    let err = state.enroll(req()).await.unwrap_err();
    assert_eq!(err.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn onboard_invite_path_unaffected_by_enroll() {
    // Existing onboard happy-path test still passes with enroll wired in.
    onboard_happy_path_smoke().await; // call the existing onboard test body or its helper
}
```

> The implementer factors small helpers (`enroll_user_device`, `signed_attestation`, `enroll_request`) from the Task 7 test code. `onboard_happy_path_smoke` reuses the existing onboard test's setup to prove no shared-state regression.

- [ ] **Step 2: Run tests to verify they fail (then pass after wiring helpers)**

Run: `cargo test -p trace-commons-server --lib trace_upload_claim_issuer::tests::enroll_ -- --nocapture`
Expected: initially FAIL on missing helpers; once helpers compile, all PASS.

- [ ] **Step 3: Implement the test helpers**

Add the factored helpers to the test module (no production code changes expected). If a behavior gap surfaces (e.g. cap not enforced through the mock), fix the mock `Database` to delegate `reserve_instance_enrollment` to a shared in-memory map keyed by `(instance_subject_hash, user_subject_hash)` so dedup/cap behave like Postgres.

- [ ] **Step 4: Run the full enroll suite + warnings-as-errors + clippy**

Run:
```bash
RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run
cargo test -p trace-commons-server --lib trace_upload_claim_issuer::
cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching
```
Expected: PASS / no warnings.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/trace_upload_claim_issuer.rs
git commit -m "Add enroll multi-device, cap, replay, and invite-untouched tests"
```

---

## Final verification (run after all tasks)

- [ ] `RUSTFLAGS="-D warnings" cargo check -p trace-commons-server --bins`
- [ ] `RUSTFLAGS="-D warnings" cargo test -p trace-commons-server --no-run`
- [ ] `cargo clippy -p trace-commons-server --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`
- [ ] `cargo test -p trace-commons-protocol onboarding::`
- [ ] `cargo test -p trace-commons-server --lib instance_enroll_guard:: trace_upload_claim_allowlist:: trace_upload_claim_issuer::`
- [ ] PostgreSQL-backed: `cargo test -p trace-commons-server --test trace_corpus_pg_store -- --ignored` (ledger RLS, reserve cap/dedup, provisioning)
- [ ] `scripts/operator/pilot-bootstrap-smoke.sh` still green (no schema break)

## Deferred (tracked, not in this plan)

- Account/principal creation at enroll — back-fill when contributor-account Slice 1 (V30) merges; add the Slice-1 create-or-reuse into `enroll_instance_user` step.
- Operator tooling: a `generate-instance-registration` helper + docs in `docs/operator/` (parallels `generate-pilot-invites.py`), and an env-reference entry for `TRACE_COMMONS_INSTANCE_ENROLL_RATE_PER_MIN`.
- Cross-tenant aggregate read path for leaderboards over the ledger's tenant list.
- DB audit-row mirror via `append_trace_audit_event` inside the provisioning tx (currently hash-only `tracing` + denial counter).
- TEE attestation and NEAR allowlist source (already-reserved seams).
