# Cloud Trace Artifact Provider Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship the GCS object backend + pluggable KEK trait so Trace Commons can move encrypted artifacts to cloud storage in canary deployments, with production blocked until a real KEK lands separately.

**Architecture:** Three independent layers added beside existing code, not refactoring it. (1) `KmsKeyWrapper` trait + `LocalMasterKeyWrapper` impl in a new module. (2) `GcsRemoteTraceArtifactProvider` implementing the existing `RemoteTraceArtifactProvider` trait. (3) Migration worker routes that page `trace_object_refs`, re-encrypt under the destination KEK + provider, transactionally flip the alias, and audit hash-only. Legacy on-disk record shape continues to read; new writes use the envelope format with `wrapped_dek`.

**Tech Stack:** Rust, Axum, PostgreSQL, AES-256-GCM (`aes-gcm`), HKDF (`hkdf`), GCS client TBD (dependency-approval gate before Task 11).

**Spec:** `docs/superpowers/specs/2026-05-11-cloud-trace-artifact-provider-design.md`

---

## File Map

**New files**

| Path | Responsibility |
|------|----------------|
| `crates/tracedao-server/src/trace_artifact_kek.rs` | `KmsKeyWrapper` trait, `KekContext`, `WrappedDek`, `LocalMasterKeyWrapper` |
| `crates/tracedao-server/src/trace_artifact_gcs.rs` | `GcsObjectClient` trait, `InMemoryGcsObjectClient`, `GcsRemoteTraceArtifactProvider` |
| `crates/tracedao-server/src/trace_artifact_migration.rs` | Migration page+copy logic shared between worker handlers |
| `crates/tracedao-server/tests/trace_artifact_kek.rs` | KEK contract tests |
| `crates/tracedao-server/tests/trace_artifact_gcs.rs` | GCS provider contract tests against in-memory + (opt-in) fake-gcs-server |
| `crates/tracedao-server/tests/trace_artifact_migration.rs` | Migration end-to-end tests |

**Modified files**

| Path | What changes |
|------|--------------|
| `crates/tracedao-server/src/lib.rs` | Add `pub mod trace_artifact_kek; pub mod trace_artifact_gcs; pub mod trace_artifact_migration;` |
| `crates/tracedao-server/src/trace_artifact_store.rs` | Extend `EncryptedTraceArtifact` with optional `wrapped_dek`; add schema_version "v2"; `verify_encrypted_artifact` gains context-hash match; `ServiceOwnedTraceArtifactStore` accepts a `KmsKeyWrapper`; legacy reader path preserved |
| `crates/tracedao-server/src/bin/tracedao-ingest.rs` | `TraceRemoteObjectStoreConfig` adds region/endpoint; `gcs` provider becomes constructable; new `/v1/workers/object-store/migrate{,/cleanup,/revert}` routes; drill reports KEK fields; rollout-smoke gains `object_store_kek_production_trust_boundary` check; startup gate `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY` |
| `Cargo.toml` (workspace + crate) | GCS client deps added behind approval gate (Task 11 only) |

**Out of scope**

- Refactoring the 2096-line `trace_artifact_store.rs` into a submodule directory (per `~/.claude/CLAUDE.md`: don't refactor beyond what the task requires).
- AWS S3 / Azure Blob impls. They keep failing closed via `DisabledRemoteTraceArtifactStore`.
- Any real KEK (TEE / cloud KMS). Separate spec.

---

## Pre-flight

- [ ] **Run the existing test suite** to establish a green baseline.

```bash
cargo check -p tracedao-server --bins
cargo test -p tracedao-server --test trace_corpus_storage_contract --test trace_corpus_pg_store
```

Expected: clean. If anything fails, stop and fix before starting.

---

## Slice 1B — KEK trait + local impl + production refusal

### Task 1: KEK module skeleton — types + trait

**Files:**
- Create: `crates/tracedao-server/src/trace_artifact_kek.rs`
- Modify: `crates/tracedao-server/src/lib.rs` (add `pub mod trace_artifact_kek;`)
- Test: `crates/tracedao-server/tests/trace_artifact_kek.rs` (created in Task 2)

- [ ] **Step 1: Add module declaration to `lib.rs`**

```rust
pub mod trace_artifact_kek;
```

- [ ] **Step 2: Write trait + types in `trace_artifact_kek.rs`**

```rust
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::trace_artifact_store::TraceArtifactKind;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KekContext {
    pub tenant_storage_ref: String,
    pub artifact_kind: TraceArtifactKind,
}

impl KekContext {
    pub fn canonical_hash(&self) -> String {
        let canonical = serde_json::json!({
            "schema": "trace_commons_kek_context.v1",
            "tenant_storage_ref": self.tenant_storage_ref,
            "artifact_kind": self.artifact_kind.as_path_segment(),
        });
        let mut hasher = Sha256::new();
        hasher.update(canonical.to_string().as_bytes());
        format!("sha256:{:x}", hasher.finalize())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WrappedDek {
    pub wrapper_kind: String,
    pub key_ref_hash: String,
    pub ciphertext_base64: String,
    pub context_hash: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct KekWrapperStatus {
    pub kind: String,
    pub key_ref_hash: String,
    pub is_production_trust_boundary: bool,
}

pub trait KmsKeyWrapper: Send + Sync {
    fn wrap_dek(&self, dek: &[u8; 32], context: &KekContext) -> anyhow::Result<WrappedDek>;
    fn unwrap_dek(&self, wrapped: &WrappedDek, context: &KekContext) -> anyhow::Result<[u8; 32]>;
    fn safe_status(&self) -> KekWrapperStatus;
    fn is_production_trust_boundary(&self) -> bool;
}
```

- [ ] **Step 3: Verify compile**

```bash
cargo check -p tracedao-server
```

Expected: clean.

- [ ] **Step 4: Commit**

```bash
git add crates/tracedao-server/src/trace_artifact_kek.rs crates/tracedao-server/src/lib.rs
git commit -m "Add KmsKeyWrapper trait skeleton"
```

---

### Task 2: LocalMasterKeyWrapper TDD — wrap/unwrap round-trip

**Files:**
- Modify: `crates/tracedao-server/src/trace_artifact_kek.rs`
- Create: `crates/tracedao-server/tests/trace_artifact_kek.rs`

- [ ] **Step 1: Write failing test** in `crates/tracedao-server/tests/trace_artifact_kek.rs`

```rust
use secrecy::SecretString;
use tracedao_server::secrets::SecretsCrypto;
use tracedao_server::trace_artifact_kek::{KekContext, KmsKeyWrapper, LocalMasterKeyWrapper};
use tracedao_server::trace_artifact_store::TraceArtifactKind;

fn fixture_crypto() -> SecretsCrypto {
    SecretsCrypto::new(SecretString::new("a".repeat(32).into())).unwrap()
}

#[test]
fn local_master_key_wrapper_round_trips_dek() {
    let wrapper = LocalMasterKeyWrapper::new(fixture_crypto(), "local-test".into());
    let dek = [7u8; 32];
    let ctx = KekContext {
        tenant_storage_ref: "tenant-a-storage-ref".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let wrapped = wrapper.wrap_dek(&dek, &ctx).unwrap();
    let recovered = wrapper.unwrap_dek(&wrapped, &ctx).unwrap();
    assert_eq!(dek, recovered);
    assert_eq!(wrapped.wrapper_kind, "local_master_key");
    assert_eq!(wrapped.context_hash, ctx.canonical_hash());
    assert!(!wrapper.is_production_trust_boundary());
}
```

- [ ] **Step 2: Run test — expect compile failure**

```bash
cargo test -p tracedao-server --test trace_artifact_kek -- local_master_key_wrapper_round_trips_dek
```

Expected: FAIL — `LocalMasterKeyWrapper` not found.

- [ ] **Step 3: Implement `LocalMasterKeyWrapper`** in `trace_artifact_kek.rs`

```rust
use crate::secrets::SecretsCrypto;
use base64::{Engine, engine::general_purpose::STANDARD};

pub struct LocalMasterKeyWrapper {
    crypto: SecretsCrypto,
    key_ref_label: String,
}

impl LocalMasterKeyWrapper {
    pub fn new(crypto: SecretsCrypto, key_ref_label: impl Into<String>) -> Self {
        Self { crypto, key_ref_label: key_ref_label.into() }
    }

    fn key_ref_hash(&self) -> String {
        let mut h = Sha256::new();
        h.update(self.key_ref_label.as_bytes());
        format!("sha256:{:x}", h.finalize())
    }
}

impl KmsKeyWrapper for LocalMasterKeyWrapper {
    fn wrap_dek(&self, dek: &[u8; 32], context: &KekContext) -> anyhow::Result<WrappedDek> {
        // AAD-bind the wrap to the context: prepend the context hash bytes
        // as an integrity tag, then AES-GCM-encrypt the (tag || dek).
        let context_hash = context.canonical_hash();
        let mut plaintext = Vec::with_capacity(32 + 32);
        plaintext.extend_from_slice(context_hash.as_bytes());
        plaintext.extend_from_slice(dek);
        let (encrypted, salt) = self.crypto.encrypt(&plaintext)
            .map_err(|e| anyhow::anyhow!("KekWrapFailed: {e}"))?;
        // Pack salt + ciphertext for self-describing decrypt.
        let mut packed = Vec::with_capacity(1 + salt.len() + encrypted.len());
        packed.push(salt.len() as u8);
        packed.extend_from_slice(&salt);
        packed.extend_from_slice(&encrypted);
        Ok(WrappedDek {
            wrapper_kind: "local_master_key".into(),
            key_ref_hash: self.key_ref_hash(),
            ciphertext_base64: STANDARD.encode(&packed),
            context_hash,
        })
    }

    fn unwrap_dek(&self, wrapped: &WrappedDek, context: &KekContext) -> anyhow::Result<[u8; 32]> {
        anyhow::ensure!(wrapped.wrapper_kind == "local_master_key", "KekUnwrapFailed: wrapper kind mismatch");
        let expected_ctx = context.canonical_hash();
        anyhow::ensure!(wrapped.context_hash == expected_ctx, "KekContextMismatch");
        let packed = STANDARD.decode(&wrapped.ciphertext_base64)
            .map_err(|_| anyhow::anyhow!("KekUnwrapFailed: base64"))?;
        anyhow::ensure!(packed.len() > 1, "KekUnwrapFailed: short");
        let salt_len = packed[0] as usize;
        anyhow::ensure!(packed.len() > 1 + salt_len, "KekUnwrapFailed: salt len");
        let salt = &packed[1..1 + salt_len];
        let encrypted = &packed[1 + salt_len..];
        let decrypted = self.crypto.decrypt(encrypted, salt)
            .map_err(|e| anyhow::anyhow!("KekUnwrapFailed: {e}"))?;
        let bytes = decrypted.expose_bytes();
        anyhow::ensure!(bytes.len() == expected_ctx.len() + 32, "KekUnwrapFailed: length");
        anyhow::ensure!(&bytes[..expected_ctx.len()] == expected_ctx.as_bytes(), "KekContextMismatch: inner");
        let mut dek = [0u8; 32];
        dek.copy_from_slice(&bytes[expected_ctx.len()..]);
        Ok(dek)
    }

    fn safe_status(&self) -> KekWrapperStatus {
        KekWrapperStatus {
            kind: "local_master_key".into(),
            key_ref_hash: self.key_ref_hash(),
            is_production_trust_boundary: false,
        }
    }

    fn is_production_trust_boundary(&self) -> bool { false }
}
```

Note: `DecryptedSecret::expose_bytes` may need adding if it doesn't already exist — **check `src/error.rs` before writing the test** and front-load the helper add if missing. If present under a different name, prefer that name.

- [ ] **Step 4: Run test — expect pass**

```bash
cargo test -p tracedao-server --test trace_artifact_kek -- local_master_key_wrapper_round_trips_dek
```

Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/tracedao-server/src/trace_artifact_kek.rs crates/tracedao-server/tests/trace_artifact_kek.rs
git commit -m "Implement LocalMasterKeyWrapper with context-bound wrap"
```

---

### Task 3: KEK negative tests — context substitution, wrapper mismatch

**Files:**
- Modify: `crates/tracedao-server/tests/trace_artifact_kek.rs`

- [ ] **Step 1: Add cross-object substitution test**

```rust
#[test]
fn unwrap_rejects_swapped_context() {
    let wrapper = LocalMasterKeyWrapper::new(fixture_crypto(), "local-test".into());
    let dek = [7u8; 32];
    let ctx_a = KekContext {
        tenant_storage_ref: "tenant-a".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let ctx_b = KekContext {
        tenant_storage_ref: "tenant-b".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let wrapped = wrapper.wrap_dek(&dek, &ctx_a).unwrap();
    let err = wrapper.unwrap_dek(&wrapped, &ctx_b).unwrap_err();
    assert!(format!("{err}").contains("KekContextMismatch"));
}

#[test]
fn unwrap_rejects_wrong_wrapper_kind() {
    let wrapper = LocalMasterKeyWrapper::new(fixture_crypto(), "local-test".into());
    let ctx = KekContext {
        tenant_storage_ref: "tenant-a".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let mut wrapped = wrapper.wrap_dek(&[0u8; 32], &ctx).unwrap();
    wrapped.wrapper_kind = "gcp_kms".into();
    let err = wrapper.unwrap_dek(&wrapped, &ctx).unwrap_err();
    assert!(format!("{err}").contains("wrapper kind mismatch"));
}

#[test]
fn unwrap_rejects_inner_context_tampering() {
    // An attacker that swaps `context_hash` to match the requested context
    // but didn't re-encrypt should still be caught by the inner tag.
    let wrapper = LocalMasterKeyWrapper::new(fixture_crypto(), "local-test".into());
    let ctx_a = KekContext {
        tenant_storage_ref: "tenant-a".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let ctx_b = KekContext {
        tenant_storage_ref: "tenant-b".into(),
        artifact_kind: TraceArtifactKind::ContributionEnvelope,
    };
    let mut wrapped = wrapper.wrap_dek(&[0u8; 32], &ctx_a).unwrap();
    wrapped.context_hash = ctx_b.canonical_hash();
    let err = wrapper.unwrap_dek(&wrapped, &ctx_b).unwrap_err();
    assert!(format!("{err}").contains("KekContextMismatch"));
}
```

- [ ] **Step 2: Run — expect all pass**

```bash
cargo test -p tracedao-server --test trace_artifact_kek
```

- [ ] **Step 3: Commit**

```bash
git add crates/tracedao-server/tests/trace_artifact_kek.rs
git commit -m "Cover KEK context substitution and wrapper-kind drift"
```

---

### Task 4: Wire KEK into `EncryptedTraceArtifact` reader path (additive)

**Files:**
- Modify: `crates/tracedao-server/src/trace_artifact_store.rs`

This task adds `wrapped_dek` as an optional field. Legacy records continue to read.

- [ ] **Step 1: Locate `EncryptedTraceArtifact`** (line 165) and `EncryptedTraceArtifactReceipt` (line 25). Read the full struct + serde derives.

- [ ] **Step 2: Add optional `wrapped_dek`**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EncryptedTraceArtifact {
    pub schema_version: String,
    pub receipt: EncryptedTraceArtifactReceipt,
    pub salt_base64: String,
    pub ciphertext_base64: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wrapped_dek: Option<crate::trace_artifact_kek::WrappedDek>,
}
```

Constants: keep the existing `"v1"` schema version string. New writes (Task 8 onward) set `schema_version = "v2"` and populate `wrapped_dek`. Reads accept both: if `wrapped_dek` is `None`, fall through to the legacy `SecretsCrypto.decrypt(ciphertext, salt)` path; if present, unwrap the DEK first and decrypt with it.

- [ ] **Step 3: Update `verify_encrypted_artifact`** to require `wrapped_dek.context_hash` to match the `KekContext` derived from `(tenant_storage_ref, artifact_kind)` **only when `wrapped_dek.is_some()`**. Legacy records skip this check.

- [ ] **Step 4: Compile**

```bash
cargo check -p tracedao-server
```

Expected: clean. No tests yet — the consumer change lands in Task 8.

- [ ] **Step 5: Commit**

```bash
git add crates/tracedao-server/src/trace_artifact_store.rs
git commit -m "Allow optional wrapped_dek on EncryptedTraceArtifact"
```

---

### Task 5: Plumb `KmsKeyWrapper` into `ServiceOwnedTraceArtifactStore`

**Files:**
- Modify: `crates/tracedao-server/src/trace_artifact_store.rs`

Today `ServiceOwnedTraceArtifactStore<P>` holds a `SecretsCrypto`. We add a generic `K: KmsKeyWrapper` so the new write path can wrap a fresh DEK per object. The legacy `SecretsCrypto` field stays for reading legacy records.

- [ ] **Step 1: Add second generic parameter**

```rust
pub struct ServiceOwnedTraceArtifactStore<P, K> {
    config: TraceArtifactProviderConfig,
    crypto: SecretsCrypto,        // legacy reader path
    kek: K,                       // new envelope writer
    provider: P,
}

impl<P: RemoteTraceArtifactProvider, K: KmsKeyWrapper> ServiceOwnedTraceArtifactStore<P, K> {
    pub fn new(config: TraceArtifactProviderConfig, crypto: SecretsCrypto, kek: K, provider: P) -> Self {
        Self { config, crypto, kek, provider }
    }
    // existing methods now use self.kek for writes and fall back to self.crypto for legacy reads
}
```

- [ ] **Step 2: Update `put_serialized_json` / `put_scoped_json`** to:
  1. Generate a 32-byte random DEK.
  2. Build `KekContext { tenant_storage_ref, artifact_kind }`.
  3. Call `kek.wrap_dek(dek, ctx)`.
  4. AES-256-GCM encrypt the serialized JSON with the **raw DEK** (same nonce-prefix format `SecretsCrypto` produces — extract the encrypt helper if needed; OK to call `Aes256Gcm` directly with a random nonce).
  5. Set `schema_version = "v2"`, populate `wrapped_dek`.

- [ ] **Step 3: Update `read_*` paths** to branch on `wrapped_dek`:
  - `Some(w)` → `kek.unwrap_dek(&w, &ctx)`, then AES-256-GCM-decrypt with the DEK.
  - `None` → existing `crypto.decrypt(ciphertext, salt)` path (unchanged).

- [ ] **Step 4: Find all callers** of `ServiceOwnedTraceArtifactStore::new` and update construction sites to pass a `KmsKeyWrapper`.

```bash
grep -rn "ServiceOwnedTraceArtifactStore::new\|ServiceOwnedTraceArtifactStore {" crates/tracedao-server/src
```

For each site, plug in `LocalMasterKeyWrapper::new(crypto.clone(), <label>)` as a transitional default. (The real production wrapper choice happens at config wiring in Task 13.)

- [ ] **Step 5: Compile + run existing tests**

```bash
cargo check -p tracedao-server --bins
cargo test -p tracedao-server --test trace_corpus_storage_contract
```

Expected: green. Legacy reader path unchanged. New writes produce v2 records; existing v1 records still readable.

- [ ] **Step 6: Add a round-trip test** — write a fresh artifact via `put_*`, read it back via `read_*`, assert `wrapped_dek.is_some()` and decrypted bytes match.

- [ ] **Step 7: Commit**

```bash
git add crates/tracedao-server/src/trace_artifact_store.rs
git commit -m "Wrap per-object DEKs through KmsKeyWrapper in service-owned store"
```

---

### Task 6: `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY` startup gate

**Files:**
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`

- [ ] **Step 1: Add env constant**

```rust
const TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY: &str =
    "TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY";
```

- [ ] **Step 2: After KEK construction, gate startup**

Find the place where `SecretsCrypto` / artifact-store wiring happens (around line 1357). After constructing the KEK, add:

```rust
if env_truthy(TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY)
    && !kek.is_production_trust_boundary()
{
    anyhow::bail!(
        "kek_production_trust_boundary_required: configured KEK does not provide a production trust boundary"
    );
}
```

- [ ] **Step 3: Test** by setting both env vars in a temp config harness (use an existing test helper or add one); assert startup bails. Use the same test pattern as existing config-status tests.

- [ ] **Step 4: Commit**

```bash
git add crates/tracedao-server/src/bin/tracedao-ingest.rs
git commit -m "Refuse startup when KEK production-trust-boundary required but missing"
```

---

### Task 7: `/v1/admin/config-status` exposes safe KEK + provider fields

**Files:**
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`

- [ ] **Step 1: Find the config-status JSON builder** (search `config-status` or `config_status`).

- [ ] **Step 2: Add fields under `object_store`**

```jsonc
"object_store": {
  "alias": "<existing>",
  "provider": "<existing>",            // gcs | file_system | local_encrypted
  "require_versioning": <existing bool>,
  "kek": {
    "kind": "local_master_key",
    "key_ref_hash": "sha256:...",
    "is_production_trust_boundary": false
  }
}
```

Populated from `kek.safe_status()`.

- [ ] **Step 3: Add a caller test** — POST to `/v1/admin/config-status` under a fixture tenant, assert the new fields are present and hash-only.

- [ ] **Step 4: Commit**

```bash
git add crates/tracedao-server/src/bin/tracedao-ingest.rs
git commit -m "Expose safe KEK + provider fields in config-status"
```

---

## Slice 1A — GCS object backend

### Task 8: `GcsObjectClient` trait + `InMemoryGcsObjectClient`

**Files:**
- Create: `crates/tracedao-server/src/trace_artifact_gcs.rs`
- Modify: `crates/tracedao-server/src/lib.rs`

- [ ] **Step 1: Add module decl** to `lib.rs`.

- [ ] **Step 2: Write the trait + in-memory impl**

```rust
use bytes::Bytes;
use std::collections::BTreeMap;
use std::sync::Mutex;

pub struct GcsObjectFetch {
    pub body: Bytes,
    pub metadata: BTreeMap<String, String>,
}

pub trait GcsObjectClient: Send + Sync {
    fn put_object(&self, key: &str, body: Bytes, metadata: BTreeMap<String, String>) -> anyhow::Result<()>;
    fn get_object(&self, key: &str) -> anyhow::Result<GcsObjectFetch>;
    fn delete_object(&self, key: &str) -> anyhow::Result<bool>;
    fn restore_deleted_object(&self, key: &str) -> anyhow::Result<bool>;
}

#[derive(Default)]
pub struct InMemoryGcsObjectClient {
    live: Mutex<BTreeMap<String, (Bytes, BTreeMap<String, String>)>>,
    deleted: Mutex<BTreeMap<String, (Bytes, BTreeMap<String, String>)>>,
}

impl GcsObjectClient for InMemoryGcsObjectClient {
    fn put_object(&self, key: &str, body: Bytes, metadata: BTreeMap<String, String>) -> anyhow::Result<()> {
        self.live.lock().unwrap().insert(key.to_string(), (body, metadata));
        Ok(())
    }
    fn get_object(&self, key: &str) -> anyhow::Result<GcsObjectFetch> {
        let live = self.live.lock().unwrap();
        let (body, metadata) = live.get(key).ok_or_else(|| anyhow::anyhow!("GcsGetFailed: not found"))?;
        Ok(GcsObjectFetch { body: body.clone(), metadata: metadata.clone() })
    }
    fn delete_object(&self, key: &str) -> anyhow::Result<bool> {
        if let Some(record) = self.live.lock().unwrap().remove(key) {
            self.deleted.lock().unwrap().insert(key.to_string(), record);
            Ok(true)
        } else { Ok(false) }
    }
    fn restore_deleted_object(&self, key: &str) -> anyhow::Result<bool> {
        if let Some(record) = self.deleted.lock().unwrap().remove(key) {
            self.live.lock().unwrap().insert(key.to_string(), record);
            Ok(true)
        } else { Ok(false) }
    }
}
```

- [ ] **Step 3: Compile**

```bash
cargo check -p tracedao-server
```

- [ ] **Step 4: Commit**

```bash
git add crates/tracedao-server/src/trace_artifact_gcs.rs crates/tracedao-server/src/lib.rs
git commit -m "Add GcsObjectClient trait and in-memory test impl"
```

---

### Task 9: `GcsRemoteTraceArtifactProvider` — put + get TDD

**Files:**
- Modify: `crates/tracedao-server/src/trace_artifact_gcs.rs`
- Create: `crates/tracedao-server/tests/trace_artifact_gcs.rs`

- [ ] **Step 1: Write failing put+get round-trip test** mirroring the filesystem-remote contract test (search `file_remote_trace_artifact` in existing tests for the pattern).

The test should:
1. Build a `GcsRemoteTraceArtifactProvider` over `InMemoryGcsObjectClient`.
2. Construct a fixture `TraceArtifactObjectRef` + `EncryptedTraceArtifact` with `wrapped_dek = Some(...)`.
3. `put_encrypted_artifact`, then `read_encrypted_artifact`.
4. Assert round-trip equality.

- [ ] **Step 2: Run — expect compile failure**

- [ ] **Step 3: Implement the provider**, mirroring `FileRemoteTraceArtifactProvider`:

```rust
pub struct GcsRemoteTraceArtifactProvider<C> {
    client: C,
    bucket: String,
    object_store_alias: String,
    require_versioning: bool,
}

impl<C: GcsObjectClient> RemoteTraceArtifactProvider for GcsRemoteTraceArtifactProvider<C> {
    fn put_encrypted_artifact(&self, object_ref: TraceArtifactObjectRef, artifact: EncryptedTraceArtifact) -> anyhow::Result<()> {
        validate_gcs_object_ref(&object_ref)?;
        verify_encrypted_artifact(&artifact, /* ... */)?;
        let record = serde_json::to_vec(&GcsRecord { object_ref: object_ref.clone(), artifact, invalidated_at: None, invalidation_reason: None })?;
        let key = self.object_key(&object_ref);
        self.client.put_object(&key, record.into(), BTreeMap::new())
            .map_err(|e| anyhow::anyhow!("GcsPutFailed: {e}"))
    }

    fn read_encrypted_artifact(&self, object_ref: &TraceArtifactObjectRef) -> anyhow::Result<RemoteTraceArtifactRecord> {
        let key = self.object_key(object_ref);
        let fetch = self.client.get_object(&key).map_err(|e| anyhow::anyhow!("GcsGetFailed: {e}"))?;
        let record: GcsRecord = serde_json::from_slice(&fetch.body)?;
        anyhow::ensure!(record.object_ref == *object_ref, "GcsGetFailed: ref mismatch");
        Ok(RemoteTraceArtifactRecord {
            object_ref: record.object_ref,
            artifact: record.artifact,
            invalidated_at: record.invalidated_at,
        })
    }
    // delete / restore land in Task 10
}
```

Define `validate_gcs_object_ref` to reuse the existing `validate_file_remote_object_ref` partitioning rules (extract to a shared helper if needed).

- [ ] **Step 4: Run — expect PASS**

- [ ] **Step 5: Commit**

```bash
git add crates/tracedao-server/src/trace_artifact_gcs.rs crates/tracedao-server/tests/trace_artifact_gcs.rs
git commit -m "Add GCS provider put/get with in-memory test"
```

---

### Task 10: GCS provider — invalidate + delete + restore TDD

**Files:**
- Modify: `crates/tracedao-server/src/trace_artifact_gcs.rs`
- Modify: `crates/tracedao-server/tests/trace_artifact_gcs.rs`

- [ ] **Step 1: Write failing tests** for `invalidate_encrypted_artifact`, `delete_encrypted_artifact` (returns `true` on hit, `false` on miss), and `restore_deleted_encrypted_artifact`.

- [ ] **Step 2: Implement** — invalidate is a read-modify-write of the JSON record; delete calls `client.delete_object`; restore calls `client.restore_deleted_object`. Mirror the filesystem-remote logic.

- [ ] **Step 3: Add a `require_versioning` test** — the production gate is the existing startup env `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING` (not a new path). The unit test here just confirms the provider exposes its versioning support so the startup gate can read it. Keep parity with how filesystem-remote signals this today.

- [ ] **Step 4: Run all GCS tests — green**

- [ ] **Step 5: Commit**

```bash
git add crates/tracedao-server/src/trace_artifact_gcs.rs crates/tracedao-server/tests/trace_artifact_gcs.rs
git commit -m "Add GCS provider invalidate/delete/restore"
```

---

### Task 11: GCS client dependency disclosure + production client impl

**Files:**
- Modify: `Cargo.toml` (workspace) and `crates/tracedao-server/Cargo.toml`
- Modify: `crates/tracedao-server/src/trace_artifact_gcs.rs`

⚠️ **Hard gate per `~/.claude/CLAUDE.md`:** Do not run `cargo add` until the user has approved the dependency.

- [ ] **Step 1: Disclose candidates** — write a short message to the user listing 2-3 GCS crate candidates with: name + version, downloads, transitive dep count, last publish, license, maintenance status. Wait for explicit approval before proceeding.

  Candidates to evaluate: `google-cloud-storage`, `cloud-storage-rs`, or a thin `reqwest` + `google-cloud-auth` wrapper.

- [ ] **Step 2: Once approved**, add to `~/.claude/approved-dependencies.md` and update `Cargo.toml`.

- [ ] **Step 3: Implement `ProdGcsObjectClient`** wrapping the chosen crate. Behind a `cfg(feature = "gcs-client")` feature flag so unit tests on hermetic CI don't need the dep.

- [ ] **Step 4: Compile both feature configurations**

```bash
cargo check -p tracedao-server --bins
cargo check -p tracedao-server --bins --features gcs-client
```

- [ ] **Step 5: Commit**

```bash
git add Cargo.toml crates/tracedao-server/Cargo.toml crates/tracedao-server/src/trace_artifact_gcs.rs
git commit -m "Add production GCS client behind gcs-client feature"
```

---

### Task 12: Wire `gcs` provider into `TraceRemoteObjectStoreConfig`

**Files:**
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`

- [ ] **Step 1: Find `TraceRemoteObjectStoreProvider::parse`** (line 1580) and `DisabledRemoteTraceArtifactStore::disabled_error` (line 1611).

- [ ] **Step 2: Change the wiring** so `TraceRemoteObjectStoreProvider::Gcs` no longer routes to `DisabledRemoteTraceArtifactStore`. Instead construct a `GcsRemoteTraceArtifactProvider` over `ProdGcsObjectClient`, wrap it in `ServiceOwnedTraceArtifactStore` with the configured `KmsKeyWrapper` (`LocalMasterKeyWrapper` for now).

- [ ] **Step 3: Add optional env vars** for `REGION` and `ENDPOINT`:

```rust
const TRACE_COMMONS_REMOTE_OBJECT_STORE_REGION: &str = "TRACE_COMMONS_REMOTE_OBJECT_STORE_REGION";
const TRACE_COMMONS_REMOTE_OBJECT_STORE_ENDPOINT: &str = "TRACE_COMMONS_REMOTE_OBJECT_STORE_ENDPOINT";
```

Plumb through `TraceRemoteObjectStoreConfig`. `ENDPOINT` is optional (default = real GCS); `REGION` is optional (GCS doesn't strictly need it for client construction but record it for audit).

- [ ] **Step 4: Keep `aws_s3` and `azure_blob` routing through `DisabledRemoteTraceArtifactStore`** — unchanged.

- [ ] **Step 5: Add a config-status test** asserting the new fields surface; assert `provider="gcs"` is reported.

- [ ] **Step 6: Commit**

```bash
git add crates/tracedao-server/src/bin/tracedao-ingest.rs
git commit -m "Wire GCS provider as a constructable remote object store"
```

---

### Task 13: Integration test against fake-gcs-server (opt-in)

**Files:**
- Create: `crates/tracedao-server/tests/trace_artifact_gcs_integration.rs`

- [ ] **Step 1: Gate the test** behind `cfg(feature = "gcs-client")` plus a `GCS_FAKE_ENDPOINT` env var. If the env is not set, mark the test skipped (use `#[ignore]` plus a runtime check).

- [ ] **Step 2: Write a contract test** that runs the same suite as `trace_artifact_gcs.rs` but against `ProdGcsObjectClient` pointed at the fake-gcs-server endpoint. Cover put/get/delete/restore + versioning behavior.

- [ ] **Step 3: Document** how to bring up fake-gcs-server in `docs/trace-commons-storage.md` (one paragraph; `docker run -p 4443:4443 fsouza/fake-gcs-server`).

- [ ] **Step 4: Run with the integration env set**

```bash
GCS_FAKE_ENDPOINT=http://localhost:4443 cargo test -p tracedao-server --features gcs-client --test trace_artifact_gcs_integration -- --ignored
```

Expected: green.

- [ ] **Step 5: Commit**

```bash
git add crates/tracedao-server/tests/trace_artifact_gcs_integration.rs docs/trace-commons-storage.md
git commit -m "Add opt-in fake-gcs-server integration coverage"
```

---

### Task 14: Drill + smoke — report KEK + new required check

**Files:**
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`

- [ ] **Step 1: Extend `object_store_migration_drill_handler` response** to include:

```jsonc
"kek": { "kind": "...", "key_ref_hash": "...", "is_production_trust_boundary": false },
"provider": "gcs"
```

Hash these into `object_store_migration_drill_evidence_hash` so existing evidence reproducibility is preserved (bump the schema to `trace_commons_object_store_migration_drill.v2`).

- [ ] **Step 2: Add a new required rollout-smoke check** named `object_store_kek_production_trust_boundary`. Search for the existing required-check list (look for `tenant_canary_isolation` to find it). Add this name. The drill `record_rollout_smoke_evidence` for this check records `passed` only when `kek.is_production_trust_boundary == true`.

- [ ] **Step 3: Add a caller test** that runs the drill under a local-KEK config and asserts the new check is reported as `failed` (or `not yet passed`).

- [ ] **Step 4: Commit**

```bash
git add crates/tracedao-server/src/bin/tracedao-ingest.rs
git commit -m "Report KEK fields in object-store drill and require production-trust-boundary smoke check"
```

---

## Slice 2 — Migration tooling

### Task 15: Migration helper module — page + decide skip

**Files:**
- Create: `crates/tracedao-server/src/trace_artifact_migration.rs`
- Modify: `crates/tracedao-server/src/lib.rs`

- [ ] **Step 1: Add module decl.**

- [ ] **Step 2: Write the iteration helper**

```rust
pub struct MigrationBatch {
    pub from_alias: String,
    pub to_alias: String,
    pub limit: u32,
    pub artifact_kinds: Option<Vec<TraceArtifactKind>>,
    pub dry_run: bool,
}

pub struct MigrationOutcome {
    pub checked: u32,
    pub copied: u32,
    pub skipped: u32,
    pub failed: u32,
    pub manifest_hash: String,
}

pub struct MigrationSkipReason; // hash-only enum: PendingRevocation, PendingRetention, AlreadyMigrated, ArtifactKindFilteredOut
```

`page_candidates` queries `trace_object_refs` for tenant + alias matches, joined against the revocation/retention helpers used by `revocation_propagation` and `retention_dry_run` drills. Skip rows pending lifecycle action.

- [ ] **Step 3: Unit test the skip logic** with mock store inputs.

- [ ] **Step 4: Commit**

```bash
git add crates/tracedao-server/src/trace_artifact_migration.rs crates/tracedao-server/src/lib.rs
git commit -m "Add migration page+skip helper"
```

---

### Task 16: Migration copy step — TDD

**Files:**
- Modify: `crates/tracedao-server/src/trace_artifact_migration.rs`
- Modify: `crates/tracedao-server/tests/trace_artifact_migration.rs`

- [ ] **Step 1: Write a failing test** — fixture: one v2 record in source, no rows in destination. After `migrate_one`, destination has a re-wrapped record with a fresh `wrapped_dek` (different ciphertext + DEK from source), the plaintext bytes round-trip, and source remains untouched.

- [ ] **Step 2: Add the legacy-format reader test** — source has a v1 record (no `wrapped_dek`). After `migrate_one`, destination has a v2 record with `wrapped_dek`. Decryption uses the legacy `SecretsCrypto` path for source, the new KEK path for destination.

- [ ] **Step 3: Implement `migrate_one`**

```rust
pub fn migrate_one<S, D>(
    source_store: &S,
    dest_store: &D,
    object_ref: &TraceArtifactObjectRef,
) -> anyhow::Result<()>
where
    S: TraceArtifactStore,
    D: TraceArtifactStore,
{
    // 1. Read source — yields decoded JSON regardless of v1/v2 format.
    let plaintext = source_store.read_json_by_object_key(
        &object_ref.tenant_storage_ref,
        object_ref.artifact_kind.clone(),
        &object_ref.object_key,
        &object_ref.ciphertext_sha256,
    )?;
    let serialized = serde_json::to_vec(&plaintext)?;
    // 2. Write destination — produces a v2 record with a fresh DEK + wrap.
    dest_store.put_serialized_json(
        &object_ref.tenant_storage_ref,
        object_ref.artifact_kind.clone(),
        &object_ref.object_key,
        &serialized,
    )?;
    // 3. Verify destination read decrypts to the same plaintext hash.
    let round_trip = dest_store.read_json_by_object_key(/* same args */)?;
    anyhow::ensure!(round_trip == plaintext, "migration round-trip mismatch");
    Ok(())
}
```

- [ ] **Step 4: Run tests — green**

- [ ] **Step 5: Commit**

```bash
git add crates/tracedao-server/src/trace_artifact_migration.rs crates/tracedao-server/tests/trace_artifact_migration.rs
git commit -m "Migrate single object across providers with verification"
```

---

### Task 17: Transactional alias flip in `trace_object_refs`

**Files:**
- Modify: `crates/tracedao-server/src/db/trace_corpus_pg.rs`
- Modify: `crates/tracedao-server/src/trace_corpus_storage.rs`
- Modify: `crates/tracedao-server/tests/trace_corpus_storage_contract.rs` (or wherever the corpus contract test lives)

- [ ] **Step 1: Add a store method** `update_object_ref_store_alias(tenant_id, object_ref_id, expected_from_alias, to_alias) -> bool`. Atomic UPDATE with a WHERE clause that checks the current alias; returns whether one row was updated.

  This repo is postgres-only — no libsql build. A single `cargo check -p tracedao-server` is sufficient.

- [ ] **Step 2: Contract test it** — same-id rows in two tenants, ensure tenant-A flip doesn't touch tenant-B.

- [ ] **Step 3: Implement in `PgBackend`** under `trace_current_tenant_id()` context.

- [ ] **Step 4: Add a caller test** that ties `migrate_one` + `update_object_ref_store_alias` together and asserts the row's `object_store` is the destination after success.

- [ ] **Step 5: Commit**

```bash
git add crates/tracedao-server/src/db/ crates/tracedao-server/src/trace_corpus_storage.rs crates/tracedao-server/tests/
git commit -m "Add transactional object-ref store-alias flip"
```

---

### Task 18: Migration worker route — `POST /v1/workers/object-store/migrate`

**Files:**
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`

- [ ] **Step 1: Add the route** with the standard worker-token auth pattern. Find an existing worker handler (e.g. `near_credit_outbox_submit_handler`) and copy the auth shape.

- [ ] **Step 2: Request DTO**

```rust
#[derive(Deserialize)]
struct MigrateRequest {
    from_alias: String,
    to_alias: String,
    limit: u32,                  // 1..=500
    dry_run: bool,
    artifact_kinds: Option<Vec<String>>,
}
```

Validate `limit` 1..=500. Validate `from_alias != to_alias`. Validate alias names against the known set.

- [ ] **Step 3: Handler body**: page candidates, for each row call `migrate_one` + `update_object_ref_store_alias`, accumulate `MigrationOutcome`. On `dry_run`, run the copy + verify but skip the alias flip.

- [ ] **Step 4: Response DTO** — only the hash-only outcome counts + manifest hash. No object refs.

- [ ] **Step 5: Caller test** under a small fixture corpus.

- [ ] **Step 6: Commit**

```bash
git add crates/tracedao-server/src/bin/tracedao-ingest.rs
git commit -m "Add object-store migration worker route"
```

---

### Task 19: Hash-only audit row for migration

**Files:**
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`
- Modify: `crates/tracedao-server/src/trace_corpus_storage.rs` (if audit shape lives there)

- [ ] **Step 1: Find existing audit-row builders** — look for `audit_action = "credit_settlement_approval_recorded"` or similar; copy the shape.

- [ ] **Step 2: Append per migration call**

```jsonc
{
  "action": "object_store_migration",
  "from_alias_hash": "sha256:...",
  "to_alias_hash":   "sha256:...",
  "source_list_hash": "sha256:...",  // canonical hash over migrated object_ref ids
  "manifest_hash":   "sha256:...",
  "counts": { "checked": N, "copied": N, "skipped": N, "failed": N }
}
```

- [ ] **Step 3: Caller test** that the row lands under the correct tenant scope.

- [ ] **Step 4: Commit**

```bash
git add crates/tracedao-server/src/
git commit -m "Append hash-only audit row for object-store migration"
```

---

### Task 20: `migrate/cleanup` route — source-side delete after verification

**Files:**
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`

- [ ] **Step 1: Add the route** with the same worker-token auth.

- [ ] **Step 2: Pull rows** that have already been flipped to the destination alias and where the source byte location can be derived. For each, call the source provider's `delete_encrypted_artifact` and record a physical-delete receipt row using the existing receipt path (search `physical_delete_receipt`).

- [ ] **Step 3: Caller test** — verify cleanup only deletes rows that have already been flipped, not rows still on source.

- [ ] **Step 4: Commit**

```bash
git add crates/tracedao-server/src/bin/tracedao-ingest.rs
git commit -m "Add migrate/cleanup route for source-side deletes"
```

---

### Task 21: `migrate/revert` route — manifest-bound rollback

**Files:**
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`

- [ ] **Step 1: Add the route** taking `{ manifest_hash: String }`.

- [ ] **Step 2: Look up** the migration audit row by `manifest_hash`. If found and recent (e.g. ≤24h), enumerate the rows it covered and flip them back via `update_object_ref_store_alias` with reversed arguments. If older or unknown, fail closed.

- [ ] **Step 3: Audit row** for the revert with `action="object_store_migration_revert"` and the bound `manifest_hash`.

- [ ] **Step 4: Caller test** — apply, revert, verify alias is back to source and source bytes are still readable. Verify revert refuses when manifest is missing.

- [ ] **Step 5: Commit**

```bash
git add crates/tracedao-server/src/bin/tracedao-ingest.rs
git commit -m "Add manifest-bound migrate/revert route"
```

---

### Task 22: Extend `/v1/admin/object-store-migration-drill` to probe both providers

**Files:**
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`

- [ ] **Step 1: Locate** `run_object_store_migration_drill` (line ~31281).

- [ ] **Step 2: Add an optional `probe_destination` flag** to the drill request. When set, the drill:
  1. Writes a synthetic probe record via the source store.
  2. Runs `migrate_one` source → destination.
  3. Reads the probe back via destination, asserts plaintext match.
  4. Deletes both, restores both.
  5. Reports the per-step manifest hash.

  All hash-only in the response. No real tenant data touched (probe storage ref is a fixed `trace_commons_object_store_migration_drill` value).

- [ ] **Step 3: Update the drill evidence hash** to cover the new fields. Bump the schema string to `trace_commons_object_store_migration_drill.v3`.

  ⚠️ Operator-visible churn: the schema bump invalidates any pinned `rollout_smoke` evidence hashes recorded under v2. Call this out in the PR description — operators need to re-run the drill and re-record evidence after the deploy. No code work in the plan; just communication.

- [ ] **Step 4: Caller test** under both providers.

- [ ] **Step 5: Commit**

```bash
git add crates/tracedao-server/src/bin/tracedao-ingest.rs
git commit -m "Drill probes destination provider for migration evidence"
```

---

## Wrap

### Task 23: README + roadmap update

**Files:**
- Modify: `README.md`
- Modify: `docs/trace-commons-roadmap.md`

- [ ] **Step 1: README** — under "Open Production Gaps", reduce gap #1 to note GCS canary is live, KEK strategy is the remaining piece.

- [ ] **Step 2: Roadmap** — in the "Ingestion Storage" section, append progress notes under the existing remote-object-store bullet: "GCS provider now implements `RemoteTraceArtifactProvider` over a pluggable `KmsKeyWrapper`; `LocalMasterKeyWrapper` is dev-only, `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY` blocks production until a real KEK lands."

- [ ] **Step 3: Commit**

```bash
git add README.md docs/trace-commons-roadmap.md
git commit -m "Reflect GCS provider + KEK trait status"
```

---

### Task 24: Final validation

- [ ] **Step 1: Run full test suite**

```bash
cargo test -p tracedao-server
```

Expected: green.

- [ ] **Step 2: Run with the optional feature**

```bash
cargo check -p tracedao-server --bins --features gcs-client
```

- [ ] **Step 3: Smoke the binary builds**

```bash
cargo build -p tracedao-server --release --bin tracedao-ingest
cargo build -p tracedao-server --release --bin tracedao-upload-claim-issuer
```

- [ ] **Step 4: Hand off** — open a PR. Description should call out: production KEK still required (separate spec), GCS canary unblocked, migration tooling lands as worker routes that need explicit operator invocation.

---

## Verification expectations per slice

- **Slice 1B (Tasks 1-7):** new module compiles, KEK tests green, config-status reports KEK fields, startup gate refuses local KEK under the prod env. No production behavior change yet.
- **Slice 1A (Tasks 8-14):** GCS provider passes the same contract tests as filesystem-remote, drill reports new fields, new rollout-smoke required check exists and reads as failing under local KEK.
- **Slice 2 (Tasks 15-22):** migration worker routes pass end-to-end tests under in-memory GCS + a temp local-encrypted source, audit rows land hash-only, revert flips metadata only.
- **Wrap (Tasks 23-24):** docs reflect status, CI green.

## Out of band — what this plan does *not* do

- Implement a real KEK (TEE / cloud KMS). That is a separate spec under `docs/superpowers/specs/`. Until it lands, `object_store_kek_production_trust_boundary` smoke evidence will read as failing and prod cutover is blocked.
- Migrate Ironclaw client-side artifacts. Out of scope; the migration worker operates on server-owned `trace_object_refs` rows only.
- Add AWS S3 / Azure Blob. They keep failing closed via `DisabledRemoteTraceArtifactStore`.
