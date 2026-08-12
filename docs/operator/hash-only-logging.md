# Hash-only Logging Classes

Every error class string the system emits in production logs is a stable
identifier — no raw URLs, no IDs, no plaintext. This doc maps each class
to what it means, what to check first, and the most common root cause.

The list comes from grepping the codebase for the class names; verify
periodically via `grep -rEn '"(Kek|Gcs|GcpKms|Perplexity|Embedder|Vector|Dstack|Revocation|Tenant|AuditChain|PrivilegedAction)[A-Za-z]+"' crates/`.

| Class | Meaning | Check first | Common root cause |
|---|---|---|---|
| `KekWrapFailed` | Cloud KMS encrypt call refused or timed out | Workload identity, KMS endpoint reachability, key version state | Service account lost `cryptoKeyEncrypterDecrypter`; key version disabled |
| `KekUnwrapFailed` | Cloud KMS decrypt call refused | Same as above plus: is the key version that wrapped the DEK still enabled? | Old key version disabled during rotation |
| `KekContextMismatch` | AAD on unwrap differs from AAD at wrap (`tenant_ctx` mismatch) | Did any tenant label env change recently? Did the wrap and unwrap routes both feed the same canonical `tenant_ctx`? | Misconfigured tenant_ctx in a worker route; KEK adapter swap that changed canonicalization |
| `KekDowngradeRejected` | A path tried to use a less-trusted KEK provider after a more-trusted one was bound | Check `TRACE_COMMONS_KEK_PROVIDER` and `TRACE_COMMONS_KEK_REQUIRE_PRODUCTION_TRUST_BOUNDARY` | Manual env change snuck `local_master_key` into a prod path |
| `KekProviderUnknown` | `TRACE_COMMONS_KEK_PROVIDER` is set to an unrecognized value | Env var typo | Typo |
| `KekProviderUnavailable` | KEK adapter could not be constructed at startup | Workload identity + KMS endpoint reachability | Network rules or IAM not applied |
| `KekDekEncryptFailed` | Local AEAD encrypt on the DEK failed | RNG / nonce state | Extremely rare; usually a host issue |
| `KekDekDecryptFailed` | Local AEAD decrypt on the DEK failed | Ciphertext corruption or AAD mismatch | Tampered envelope, or `tenant_ctx` mismatch (often paired with `KekContextMismatch`) |
| `GcsClientInit` | GCS client could not be constructed at startup | ADC / Workload Identity present? `gcs-client` feature compiled in? | Workload Identity not bound |
| `GcsPutFailed` | Upload to GCS failed | Bucket existence, IAM (`storage.objectAdmin`), versioning enforcement | IAM denied; or `TRACE_COMMONS_OBJECT_STORE_REQUIRE_VERSIONING=true` and bucket lacks versioning |
| `GcsGetFailed` | Download from GCS failed | Object existence, generation matches the ref row in PG | Object purged outside the audit-aware path (operator deleted it manually) |
| `GcsDeleteFailed` | Soft-delete (delete-marker) write failed | IAM, bucket retention policy | Bucket has a retention lock that prevents delete |
| `GcsRestoreFailed` | Restoring a soft-deleted object failed | Generation IDs, soft-delete retention window | Soft-delete window elapsed |
| `GcpKmsClientInit` | KMS client init failed | ADC / Workload Identity | Same as GCS init failures |
| `GcpKmsEncryptFailed` | KMS `encrypt` API call failed | Permissions, key state | Workload identity lost encrypter role |
| `GcpKmsDecryptFailed` | KMS `decrypt` API call failed | Permissions, key version enabled | Workload identity lacks `cryptoKeyEncrypterDecrypter` on the version that wrapped the DEK |
| `PerplexityScorerInit` | Candle model load failed at startup | Model files present at `TRACE_COMMONS_PERPLEXITY_MODEL_PATH`, CUDA device available, tail cutoff finite-negative | Missing weights; nvcc not installed; CUDA driver mismatch |
| `PerplexityScorerInferenceFailed` | Per-request scoring threw | Token count vs. `max_tokens`; GPU memory | Context overflow; GPU OOM; corrupted weights |
| `PerplexityScorerInputTruncated` | Input exceeded `TRACE_COMMONS_PERPLEXITY_MAX_TOKENS` | Caller submitted oversized plaintext | Operator should consider raising the limit if legitimate inputs are clipped |
| `EmbedderInferenceFailed` | fastembed inference threw | ONNX runtime errors; tokenizer failure | Model file missing or corrupt under `TRACE_COMMONS_EMBEDDER_CACHE_DIR` |
| `VectorInvalidationFailed` | Vector index could not be invalidated on revocation | Gate service ready? Index file writable? | Gate service unavailable or disk full |
| `MetadataInvalidationFailed` | Trace metadata / derived-record invalidation on revocation failed | DB mirror reachable? RLS predicate intact? | PG mirror unavailable; deadlock under load |
| `ExportInvalidationFailed` | Export manifest or manifest-item invalidation on revocation failed | DB mirror reachable? Export manifest tables writable? | PG mirror unavailable; constraint deadlock |
| `DerivedRecordInvalidationFailed` | Derived record invalidation on revocation failed (same DB path as `MetadataInvalidationFailed`) | DB mirror reachable? | PG mirror unavailable |
| `BenchmarkArtifactInvalidationFailed` | Benchmark artifact invalidation on revocation failed | DB mirror reachable? Benchmark manifest tables writable? | PG mirror unavailable |
| `RankerArtifactInvalidationFailed` | Ranker artifact invalidation on revocation failed | DB mirror reachable? Ranker manifest tables writable? | PG mirror unavailable |
| `CreditSettlementReversalFailed` | Operator-raised credit settlement reversal failed (revocation never raises one) | NEAR outbox configured? Credit ledger writable? Settlement batch list readable? | NEAR outbox dependency unavailable; settlement batch read failed |
| `WorkerQueueInvalidationFailed` | Worker queue invalidation on revocation failed | External worker cache invalidator configured (`TRACE_COMMONS_REVOCATION_WORKER_CACHE_INVALIDATOR_REQUIRED`)? Endpoint reachable? | Required invalidator missing or returning non-2xx |
| `ObjectDeletionFailed` | Physical object payload deletion on revocation failed | Service-owned encrypted store configured? Remote object deleter (e.g., GCS) reachable? Verification before deletion passed? | Missing GCS deleter; encrypted store unavailable; tenant payload verification mismatch |
| `PhysicalDeleteReceiptRecordFailed` | Recording the physical-delete receipt row failed after the underlying delete | DB mirror reachable? Receipt row upsert path healthy? | PG mirror unavailable post-delete |
| `DstackKekUnavailable` | (Phase B) dstack-rooted KEK not reachable | n/a in v1 | Phase B issue |
| `DstackGateServiceUnavailable` | (Phase B) dstack-attested gate not reachable | n/a in v1 | Phase B issue |
| `RevocationPropagationFailure` | A revocation effect failed and was not retried successfully | Look at `revocation_propagation_terminal_failed_<kind>` counters (vector_entries / object_refs / export_manifests / export_manifest_items / derived_records / benchmark_artifacts / ranker_artifacts / credit_settlements / worker_queues / physical_delete_receipts) in operational summary; cross-reference the typed `<Action>Failed` error class | See the per-kind rows above; the kind-specific class identifies the failing subsystem |
| `TenantDriftRejected` | A request's resolved tenant_id drifted from the AAD path | Auth path correctness | Worker route assembled `tenant_ctx` from the wrong source |
| `AuditChainDriftRejected` | Audit chain hash chain broken | Run `audit-chain-drill`; likely PG was restored mid-write | Backup restore at a bad point |
| `PrivilegedActionAbacDenied` | ABAC denied a privileged action | Actor/principal context | Acting principal lacks the required grant |

## How to triage in production

1. **Note the class.** Just the class — no IDs, no specifics. Logs are
   hash-only by design.
2. **Pull `GET /v1/admin/operational-summary`** to get the surrounding
   counter state.
3. **Match the class to this table** to know which subsystem.
4. **Run the relevant drill** (`/v1/admin/<subsystem>-drill`) to
   reproduce + isolate.
5. **Apply the fix from this table or
   [`troubleshooting.md`](troubleshooting.md).**

The audit chain itself (`trace_audit_events`) is your forensic record;
[`audit-trail-forensics.md`](audit-trail-forensics.md) covers reading
it during incidents.
