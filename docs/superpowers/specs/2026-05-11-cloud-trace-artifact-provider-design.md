# Cloud Trace Artifact Provider — Design

Date: 2026-05-11
Status: Draft (pre-implementation)

> **Update 2026-05-11:** Slice 2 (migration / backfill tooling) is deferred. There is no production data in the standalone tracedao-server repo to migrate; building migration workers speculatively violates YAGNI. When a real deployment needs to move bytes between providers, the right shape is a one-off CLI against the actual data shape we observe — not generic worker routes invented blind. Slice 1A (GCS provider) and Slice 1B (KEK trait + local impl + production refusal) shipped; see commits `1c0eb7b` through `d092d19` on `feat/cloud-artifact-provider`.

Owner: Trace Commons / Storage lane (Lane C in `docs/trace-commons-roadmap.md`)
Roadmap item: "Replace service-local encrypted artifact storage with a service-owned object-store provider abstraction, KMS/key-ref strategy, tenant-hashed object keys, hash/decrypt verification, and migration/backfill tooling" (Phase 2, Lane C).

## Decision frame

Trace Commons needs a production object backend before it can finish Phase 2. An earlier draft of this spec collapsed two independent decisions — object backend and key-encryption-key (KEK) provider — into a single "S3 + AWS KMS" bundle. This version separates them.

- **Object backend**: where envelope-encrypted bytes sit. First concrete cloud impl is **GCS**, matching the project's default cloud. The backend is fungible because bytes leaving the server are always already encrypted.
- **KEK provider**: who can unwrap the per-object data encryption key (DEK). The trust model lives here. This spec ships the **trait surface plus a local-only KEK impl**, refuses local KEK in production, and defers the real KEK (TEE-rooted, cloud KMS, or hybrid) to a sister design once the trust-model decision is made.

The Trace Commons threat model — hash-only everything, central-issuer fail-closed, no plaintext fallback — already pushes toward an operator-constrained trust posture. A TEE-rooted KEK is the architecturally consistent answer, but it is a larger slice with platform questions (dstack / GCP Confidential Space / AWS Nitro Enclaves) that are not settled. This spec keeps the door open without committing.

## Goal

Land the object-storage abstraction in production-shape so that:

1. New tenants can store encrypted artifacts in GCS instead of the file-backed pilot path.
2. The KEK is pluggable; production refuses to start with the local KEK.
3. Migration tooling can move existing local-encrypted artifacts to GCS once a real KEK is configured.
4. Choosing between TEE-rooted and cloud KMS KEKs later is a drop-in trait impl, not a refactor.

## Non-goals

- AWS S3 / Azure Blob object providers (same trait, deferred until a deployment needs them).
- Cloud KMS adapters (AWS KMS, GCP Cloud KMS). The trait is ready; the impls are out of scope for this slice.
- TEE-rooted KEK implementations (dstack, Confidential Space, Nitro Enclaves). Trait surface only.
- Cross-region replication, multi-bucket sharding. Operator concerns.
- Replacing `SecretsCrypto` for secrets/keychain code.
- Migrating Ironclaw's local-sidecar artifacts in the field — rehearsed via filesystem-remote, not by this slice.

## Current shape

```
TraceArtifactStore (trait)                  crates/tracedao-server/src/trace_artifact_store.rs:172
├── LocalEncryptedTraceArtifactStore        (line 745)   — file-backed sidecar, dev default
└── ServiceOwnedTraceArtifactStore<P>       (line 487)   — generic wrapper over a remote provider
        P: RemoteTraceArtifactProvider      (line 223)
        └── FileRemoteTraceArtifactProvider (line 257)   — filesystem-remote rehearsal
```

Wired into `tracedao-ingest` via `TraceRemoteObjectStoreConfig` (`bin/tracedao-ingest.rs:1467`). `aws_s3`, `gcs`, `azure_blob` provider names parse but resolve to `DisabledRemoteTraceArtifactStore` (line 1600) that bails closed. Env vars `TRACE_COMMONS_REMOTE_OBJECT_STORE_{PROVIDER,BUCKET,KMS_KEY_ID,CREDENTIAL_REF}` already validated. Admin drill `/v1/admin/object-store-migration-drill` already round-trips put/read/delete/restore against whatever provider is plugged in.

What is missing:

1. A GCS implementation of `RemoteTraceArtifactProvider`.
2. A `KmsKeyWrapper` trait so DEK wrapping is decoupled from the legacy `SecretsCrypto` HKDF derivation.
3. A `LocalMasterKeyWrapper` impl gated against production.
4. Startup gating that refuses production deployments with the local KEK.
5. Migration / backfill tooling.

## Architecture: three independent layers

### Layer 1 — Object backend (`RemoteTraceArtifactProvider`)

Existing trait. Implementations in scope:

- `FileRemoteTraceArtifactProvider` — exists, rehearsal/tests
- `GcsRemoteTraceArtifactProvider` — **new**, production

The trait stays unchanged. Bytes flowing through it are already envelope-encrypted by the layer above, so a backend swap does not affect ciphertext.

### Layer 2 — KEK provider (`KmsKeyWrapper`) — new

```rust
// crates/tracedao-server/src/trace_artifact_store/kek.rs (new module)

pub trait KmsKeyWrapper: Send + Sync {
    /// Wrap a 32-byte data encryption key. `context` is an integrity-binding
    /// set (tenant_storage_ref + artifact_kind) that the wrapper MUST mix
    /// into the wrap, so a wrapped DEK from object A cannot unwrap object B.
    fn wrap_dek(&self, dek: &[u8; 32], context: &KekContext) -> anyhow::Result<WrappedDek>;

    fn unwrap_dek(&self, wrapped: &WrappedDek, context: &KekContext) -> anyhow::Result<[u8; 32]>;

    /// Safe diagnostic shape for /v1/admin/config-status — never the raw
    /// key ref, ARN, attestation quote, or wrapping key material.
    fn safe_status(&self) -> KekWrapperStatus;

    /// Returns true if this KEK can be trusted as a production trust
    /// boundary. `LocalMasterKeyWrapper` returns false; future TEE / KMS
    /// impls return true. Startup gate uses this.
    fn is_production_trust_boundary(&self) -> bool;
}

pub struct KekContext {
    pub tenant_storage_ref: String,
    pub artifact_kind: TraceArtifactKind,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct WrappedDek {
    pub wrapper_kind: String,        // "local_master_key" | (later) "gcp_kms" | "tee_dstack" | ...
    pub key_ref_hash: String,        // sha256 of the configured key id; never the raw ref
    pub ciphertext_base64: String,   // wrapped DEK bytes
    pub context_hash: String,        // sha256 of canonical(KekContext); cross-check on unwrap
}
```

Implementations in scope for this slice:

- `LocalMasterKeyWrapper` — wraps DEKs using the existing `SecretsCrypto` master key. Returns `is_production_trust_boundary() = false`. Refuses to construct when `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY=true`.

Implementations carried as concrete designs in **separate** specs (not in this slice):

- `TeeKekWrapper` (dstack / Nitro / Confidential Space). Attested enclave holds an unsealed wrapping key; unwrap requires running inside the attested image. This is the architecturally aligned answer and the most likely production target.
- `CloudKmsKeyWrapper` (GCP Cloud KMS or AWS KMS). Operator-trusted; useful for deployments that explicitly accept that trust posture.
- Hybrid (cloud KMS releases a sealing key only to attested code).

The point: nothing in slice 1 commits to a KEK vendor. The trait surface lets the right answer drop in.

### Layer 3 — Object key partitioning + DEK lifecycle (vendor-independent)

- Per-object random 32-byte DEK encrypts ciphertext (AES-256-GCM, same nonce-prefixed format `SecretsCrypto` already produces).
- DEK wrapped via `KmsKeyWrapper::wrap_dek(dek, context)`.
- Wrapped DEK lives in the stored JSON record alongside the ciphertext.
- Object key shape: `{object_store_alias}/{tenant_storage_ref_hash}/{artifact_kind}/{object_key}` — same partitioning as filesystem-remote (already validated by `validate_file_remote_object_ref`). No raw tenant ids, contributor ids, or paths in keys.

## Slice 1A — GCS object backend

### New types

```rust
// crates/tracedao-server/src/trace_artifact_store/gcs.rs (new)

pub struct GcsRemoteTraceArtifactProvider<C> {
    client: C,                  // abstract GCS client trait
    bucket: String,
    object_store_alias: String, // matches existing alias tracking
    require_versioning: bool,
}

pub trait GcsObjectClient: Send + Sync {
    fn put_object(&self, key: &str, body: bytes::Bytes, metadata: BTreeMap<String, String>)
        -> anyhow::Result<()>;
    fn get_object(&self, key: &str) -> anyhow::Result<GcsObjectFetch>;
    fn delete_object(&self, key: &str) -> anyhow::Result<bool>;
    fn restore_deleted_object(&self, key: &str) -> anyhow::Result<bool>;
}
```

Two `GcsObjectClient` implementations:

1. **Production** — wraps the chosen GCS crate (subject to dependency-approval per `~/.claude/CLAUDE.md`).
2. **In-memory** — for unit tests; mirrors filesystem-remote's test setup.

### Stored record shape

The object stored at `gs://<bucket>/<key>` uses the same JSON record shape as filesystem-remote, extended with the wrapped DEK:

```jsonc
{
  "object_ref": { /* TraceArtifactObjectRef, unchanged */ },
  "artifact":   { /* EncryptedTraceArtifact, ciphertext encrypted by per-object DEK */ },
  "wrapped_dek": {
    "wrapper_kind": "local_master_key",
    "key_ref_hash": "...",
    "ciphertext_base64": "...",
    "context_hash": "..."
  },
  "invalidated_at": null,
  "invalidation_reason": null
}
```

`EncryptedTraceArtifact.ciphertext_base64` is unchanged — same AES-256-GCM nonce-prefixed format. The only change: the key fed to `Aes256Gcm` is the unwrapped DEK, not an HKDF derivation off the master key.

### Versioning + restore

Require GCS **Object Versioning** on. `delete_encrypted_artifact` deletes the live generation (versioning preserves the prior generation). `restore_deleted_encrypted_artifact` rewrites the latest non-deleted generation back to live. If versioning is off, hard `NotFound` — the existing `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING` gate catches this at startup.

Bucket-level CMEK / CSEK can stay configured as defense-in-depth but is not the trust boundary; the wrapped DEK is.

### Config

Reuses `TraceRemoteObjectStoreConfig`:

- `TRACE_COMMONS_REMOTE_OBJECT_STORE_PROVIDER=gcs`
- `TRACE_COMMONS_REMOTE_OBJECT_STORE_BUCKET=<bucket name>`
- `TRACE_COMMONS_REMOTE_OBJECT_STORE_CREDENTIAL_REF=<service account email or ADC label>` — audit-only label; resolution uses the standard GCP credentials chain
- `TRACE_COMMONS_REMOTE_OBJECT_STORE_ENDPOINT` — optional, for fake-gcs-server in tests
- `TRACE_COMMONS_REMOTE_OBJECT_STORE_KMS_KEY_ID` — repurposed: opaque key-ref label hashed into `WrappedDek.key_ref_hash`

`DisabledRemoteTraceArtifactStore` keeps its bail-out for `aws_s3` / `azure_blob`. Only `gcs` (and `file_system`) become constructable in this slice.

### Status surface

`/v1/admin/config-status` adds (all hash-only or label-only — never the bucket name, service-account email, or raw key ref):

- `object_store.alias`
- `object_store.provider` — `"gcs"`
- `object_store.kek.kind` — `"local_master_key"`
- `object_store.kek.key_ref_hash`
- `object_store.kek.is_production_trust_boundary` — `false` for local KEK
- `object_store.require_versioning`

### Error / log shape

Hash-only error / reason logging, matching recent commits (`5f9a4e9` and family). Stable error class names: `GcsPutFailed`, `GcsGetFailed`, `GcsDeleteFailed`, `GcsRestoreFailed`, `KekWrapFailed`, `KekUnwrapFailed`, `KekContextMismatch`, `WrappedDekMissing`. Raw GCS request IDs hashed before logging.

### Failure semantics

| Situation | Behavior |
|-----------|----------|
| GCS put succeeds, KEK wrap failed earlier | Wrap first, then put. If wrap fails, GCS is never called. |
| GCS put succeeds, then process dies before DB updates `trace_object_refs` | DB reconciliation drill already covers this — orphan GCS objects appear as object-ref store-mismatch blockers. Cleanup via existing physical-delete receipt path. |
| GCS get returns 404 but DB has the ref | Treated like existing `object_ref_missing` blocker; no plaintext fallback. |
| KEK unwrap fails (context mismatch / key gone) | Hard error; no plaintext fallback. Audit row records `KekUnwrapFailed` with the safe context hash. |
| Bucket versioning off when required | Startup gate blocks; matches today's `REQUIRE_VERSIONING` behavior. |

## Slice 1B — Local KEK + production refusal

### `LocalMasterKeyWrapper`

```rust
pub struct LocalMasterKeyWrapper {
    crypto: SecretsCrypto,
    key_ref_label: String,
}
```

Wrap = HKDF-derive a per-object key off the master key salted with the canonical hash of `KekContext`, then AES-256-GCM encrypt the DEK. The wrapped DEK is opaque ciphertext bytes plus the same context hash stored in `WrappedDek.context_hash`.

`is_production_trust_boundary()` returns `false`.

### Startup gate

New env: `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY` (truthy in production deployments).

At startup, if the env is truthy and the configured KEK's `is_production_trust_boundary()` returns `false`, the process aborts with a hash-only operator message naming the safe missing-control: `"kek_production_trust_boundary_required"`. Mirrors how `TRACE_COMMONS_CREDIT_SETTLEMENT_REQUIRE_CENTRAL_ISSUER_PROFILE` already gates the credit path.

The production refusal is the load-bearing piece. Without it, this slice would silently accept an operator-readable KEK in prod.

### Smoke evidence

The existing `/v1/admin/object-store-migration-drill` reports the configured `object_store.kek.kind` and `is_production_trust_boundary` in its hash-only manifest. Rollout-smoke evidence stays bound to that — moving from local KEK to a real KEK requires fresh evidence under the existing `object_store_migration` smoke-check name.

A new required check `object_store_kek_production_trust_boundary` is added to the rollout-smoke gate; the drill records evidence based on `is_production_trust_boundary()`.

## Slice 2 — Migration / backfill tooling

### Worker route

```
POST /v1/workers/object-store/migrate
{
  "from_alias": "trace-commons-local-encrypted" | "trace-commons-service-remote",
  "to_alias":   "trace-commons-service-remote-gcs",
  "limit": 100,
  "dry_run": true | false,
  "artifact_kinds": ["contribution_envelope", ...]   // optional filter
}
```

Returns hash-only summary `{ checked, copied, skipped, failed, manifest_hash }`. No object refs, no key material in response.

### Algorithm (per batch)

1. Page `trace_object_refs` rows under the authenticated tenant where `object_store = from_alias`, not invalidated, not pending-deletion.
2. For each row: read encrypted bytes via the source provider, decrypt with the source KEK, re-encrypt under a fresh destination per-object DEK + wrap under the destination KEK, write to the destination provider with the same `object_key` under the new alias.
3. Verify the destination read decrypts back to the same plaintext hash. If not, mark `failed` and skip; do not touch the source row.
4. On success: update `trace_object_refs.object_store` to the destination alias in a transaction.
5. Append a hash-only audit row with `action="object_store_migration"`, source/destination alias hashes, source-list hash, counts.

Source bytes are not deleted in this step. Cleanup is a separate route.

### Cleanup

`POST /v1/workers/object-store/migrate/cleanup` performs source-side deletes for rows already moved, using the existing physical-delete receipt path. Same revocation/retention semantics — destructive only after destination copies are verified.

### Revert

`POST /v1/workers/object-store/migrate/revert` takes a `manifest_hash`. For rows in that exact prior migration batch, flips `trace_object_refs.object_store` back to the source alias (metadata-only; source bytes still present). Bounded to a single recent manifest to prevent generic rollback shenanigans.

### Drill extension

Extend `/v1/admin/object-store-migration-drill` so an operator can run a probe across both providers in one call: writes a probe record to the source, runs one migrate step, reads from the destination, deletes both, restores both, reports the per-step manifest hash. No real tenant data touched.

### Coordination with revocation / retention

Migration worker skips rows with a pending revocation tombstone or retention purge job. Otherwise we copy bytes the lifecycle layer is about to delete. Uses the same store helpers `revocation_propagation` and `retention_dry_run` drills use.

### Out of scope for slice 2

- Cross-tenant migration (one tenant at a time, like every other admin route).
- Live read fallback that tries destination then source — too easy to mask a real failure.
- Re-wrap-only migration (KEK rotation without changing object backend) — separate spec.

## Verification gates

- **Object gate** — every read verifies `tenant_storage_ref`, `ciphertext_sha256`, `artifact_kind`, `WrappedDek.context_hash` match, and successful unwrap + decrypt before returning bytes. `verify_encrypted_artifact` extended for KEK context match.
- **Audit gate** — migration audit rows tenant-scoped, hash-only, action `object_store_migration`, alias hashes, manifest hash, counts.
- **Parity gate** — DB reconciliation must show no `object_ref_*` blockers before migration promotion.
- **Rollback gate** — `migrate/revert` flips metadata only; source bytes preserved until explicit cleanup.
- **Smoke** — `object_store_migration` is already on the required check list. The GCS drill produces fresh evidence.
- **Production KEK gate** — startup refuses local KEK when `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY=true`; required-check `object_store_kek_production_trust_boundary` added to rollout-smoke.

## Promotion / exit criteria

A tenant is ready to cut over to GCS when:

1. DB reconciliation drill clean for the tenant (no `object_ref_*` blockers).
2. `/v1/admin/object-store-migration-drill` clean with `object_store.provider = "gcs"`.
3. `object_store.kek.is_production_trust_boundary = true` — **gated on a follow-up spec landing a real KEK**.
4. A full backfill batch reports `failed = 0`; manifest hash recorded as smoke evidence.
5. `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING=true` on the destination bucket.
6. Fresh passed evidence for the `object_store_migration` and `object_store_kek_production_trust_boundary` checks (within 24h).

Slice 1 lands GCS in *non-production* canary deployments only. Production cutover is explicitly blocked until a real KEK ships.

## Open questions carried forward

- **KEK vendor.** TEE (dstack / Confidential Space / Nitro), cloud KMS (GCP / AWS), or hybrid. Needs its own design spec; this spec deliberately does not decide.
- **KEK rotation.** With per-object wrapped DEKs, rotation = re-wrap every DEK. The migration worker is the rough shape, but a dedicated re-wrap path that does not change the object backend is cleaner. Defer to the KEK spec.
- **Multi-bucket / multi-alias.** Today the alias is a single config value. Multi-bucket per deployment stays out of scope.
- **GCS client crate choice.** `google-cloud-storage`, `cloud-storage-rs`, or hand-rolled `reqwest` against the JSON API. Dependency disclosure + approval before adoption.

## Rollout

1. Land slice 1A + 1B behind `gcs` provider config. Deployments still default to local-encrypted. No production impact.
2. Stand up a non-prod GCS bucket. Run `/v1/admin/object-store-migration-drill`. Record smoke evidence under local KEK — explicitly *not* passing the production-trust-boundary check.
3. Land slice 2 backfill worker. Run dry-run migration against a canary tenant in the non-prod environment.
4. Decide KEK strategy (separate spec). Implement chosen KEK.
5. Re-run drills under real KEK. Record fresh smoke evidence for both `object_store_migration` and `object_store_kek_production_trust_boundary`.
6. Run live migration for canary tenant. Verify reconciliation clean. Run cleanup.
7. Promote canary, repeat per tenant.

File-backed fallback remains available throughout the per-tenant rollout window.

## Dependencies (require explicit approval per `~/.claude/CLAUDE.md`)

To be disclosed and approved before implementation:

- GCS client crate (final choice TBD)
- ADC / service-account credentials crate
- Fake GCS server for integration tests (e.g. `fsouza/fake-gcs-server` via docker-compose)

No new crypto deps. `aes-gcm` and `hkdf` are already used by `SecretsCrypto`.

## What this slice deliberately does not answer

The KEK trust model. That is the architecturally important question for Trace Commons and deserves its own spec. By shipping the trait surface and refusing local KEK in production, this slice unblocks all the object-storage and migration work without prejudicing the KEK decision. The natural next document is `docs/superpowers/specs/<date>-trace-kek-strategy-design.md` covering operator-trusted (cloud KMS) vs operator-constrained (TEE) postures and recommending a path.
