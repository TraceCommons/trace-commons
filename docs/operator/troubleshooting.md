# Troubleshooting

Common failure modes by symptom. Each entry follows the pattern:
observed behavior → hash-only log signature → root cause → fix.

## Binary refuses to start

**Symptom:** `tracedao-ingest` exits non-zero immediately.
**Signature:** First line of stderr is a hash-only class name plus an
env-var hint.

Walk through the env vars in dependency order
(see [`deployment.md`](deployment.md)) and confirm:

1. `DATABASE_URL` set and reachable.
2. `TRACE_COMMONS_KEK_PROVIDER` and `TRACE_COMMONS_KEK_GCP_KMS_KEY_NAME`
   set; workload identity has decrypter+encrypter role.
3. `TRACE_COMMONS_GATE_PERPLEXITY_FLOOR_MICROS`,
   `_TAIL_FRACTION_FLOOR_MICROS`, `_NOVELTY_FLOOR_MICROS` all set, at
   least one positive.
4. `TRACE_COMMONS_GATE_POLICY_VERSION` set, non-empty.
5. `TRACE_COMMONS_PERPLEXITY_MODEL_PATH` exists, contains `config.json`,
   `tokenizer.json`, `model.safetensors*`.

## GPU OOM during gate evaluation

**Symptom:** `PerplexityScorerInferenceFailed` for large inputs;
`/v1/admin/operational-summary` shows the counter ticking.
**Root cause:** Llama-3.1-8B + KV cache exceeds H100 80GB on
near-context-window inputs. Most often happens when
`TRACE_COMMONS_PERPLEXITY_MAX_TOKENS` is raised above 16384 or when
multiple concurrent evaluations hit the GPU.
**Fix:**
- Lower `TRACE_COMMONS_PERPLEXITY_MAX_TOKENS` (the gate worker is
  single-threaded inside the candle scorer's mutex; concurrency isn't
  the issue, input size is).
- Verify `nvidia-smi` shows headroom at idle.
- If you've moved to a different model, confirm it fits.

## `KekContextMismatch` (most common rotation/config failure)

**Symptom:** Unwrap fails on artifacts that were wrapped earlier today.
**Signature:** `KekContextMismatch`.
**Root cause:** The `tenant_ctx` AAD bytes used at wrap differ from the
bytes used at unwrap. Almost always one of:
- A tenant-ID label env var was changed (tenant labels are part of the
  AAD canonicalization in v1).
- A KEK adapter swap changed canonical AAD format.
- A worker route is constructing tenant_ctx from a different field
  (e.g. envelope vs auth-derived) than the original wrap.
**Fix:** Restore the previous tenant label value; if you genuinely
intend to migrate AAD format, that's a re-wrap procedure, not a
config change.

## `EmbedderInferenceFailed` after deploy

**Symptom:** Every gate evaluation fails on embedding.
**Signature:** `EmbedderInferenceFailed` or `EmbedderModelIdUnrecognized`
at startup.
**Root cause:**
- ONNX file missing or partially-downloaded under
  `TRACE_COMMONS_EMBEDDER_CACHE_DIR`.
- `TRACE_COMMONS_EMBEDDER_MODEL_ID` is not in fastembed's supported
  list.
**Fix:** Re-run `scripts/operator/stage-models.sh`. Verify the model id
against fastembed's enum.

## `GcpKmsDecryptFailed`

**Symptom:** Specific tenants' artifacts fail to decrypt; new uploads
work fine.
**Signature:** `GcpKmsDecryptFailed`.
**Root cause:**
- Workload identity has `cryptoKeyEncrypter` but not `Decrypter` role.
- The key version that wrapped the DEK has been disabled (rotation
  retired it too early).
**Fix:** Check IAM bindings. Re-enable the disabled version if the
claim-lifetime window hasn't elapsed for all in-flight wrapped DEKs.

## `VectorInvalidationFailed`

**Symptom:** Revocation events queue up but vector entries remain in
the index.
**Signature:** `VectorInvalidationFailed`;
`revocation_propagation_terminal_failed_vector_entries` increments.
**Root cause:**
- Gate service is not ready (no model loaded → no orchestrator → no
  index access).
- Disk full at `TRACE_COMMONS_VECTOR_INDEX_ROOT`.
- Vector scheduler is disabled.
**Fix:**
- Check `gate_service_status.ready` in operational summary.
- `df -h $TRACE_COMMONS_VECTOR_INDEX_ROOT`.
- Verify `TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_ENABLED=true` and that
  the scheduler bearer token is set.

## `ObjectDeletionFailed`

**Symptom:** Physical object payloads remain after revocation; receipts
are not recorded.
**Signature:** `ObjectDeletionFailed`;
`revocation_propagation_terminal_failed_object_refs` increments.
**Root cause:**
- Remote object deleter (e.g., GCS deleter) is required but not
  configured (`TRACE_COMMONS_REQUIRE_REMOTE_OBJECT_DELETER=true` with no
  deleter wired).
- Service-owned encrypted store is unreachable (KMS unwrap fails).
- Pre-deletion tenant/payload verification mismatch — the object's
  embedded `tenant_id` no longer matches.
**Fix:**
- Confirm the deleter dependency is wired and reachable.
- Run the encrypted-store smoke; check KMS bindings.
- For verification mismatches, treat as a data-integrity incident —
  do not bypass the check.

## `CreditSettlementReversalFailed`

**Symptom:** Revocation events queue up; credit reversals never make
it to the NEAR outbox.
**Signature:** `CreditSettlementReversalFailed`;
`revocation_propagation_terminal_failed_credit_settlements` increments.
**Root cause:**
- NEAR credit outbox is unreachable or the settlement batch list
  read fails.
- The contributor's settlement batch is missing the source line item
  (data drift).
- DB mirror writes blocked while the reversal ledger row is appended.
**Fix:**
- Check NEAR outbox connectivity and the credit-settlement-batch read
  path.
- Run the credit-settlement drill to confirm batch state.
- Verify the DB mirror is healthy (the audit row stamps the failure
  hash; raw NEAR error text never leaks).

## `WorkerQueueInvalidationFailed`

**Symptom:** Process-evaluation or ranking-training queues retain
work items for revoked traces.
**Signature:** `WorkerQueueInvalidationFailed`;
`revocation_propagation_terminal_failed_worker_queues` increments.
**Root cause:**
- External worker cache invalidator required
  (`TRACE_COMMONS_REVOCATION_WORKER_CACHE_INVALIDATOR_REQUIRED=true`)
  but no endpoint is wired.
- The configured invalidator returns non-2xx or fails the evidence-hash
  validation.
**Fix:**
- Confirm the invalidator endpoint is configured and healthy.
- If it's the in-tree placeholder, swap the surface for a real cache
  invalidator before re-enabling the requirement flag.

## `MetadataInvalidationFailed` / `ExportInvalidationFailed` / `DerivedRecordInvalidationFailed` / `BenchmarkArtifactInvalidationFailed` / `RankerArtifactInvalidationFailed` / `PhysicalDeleteReceiptRecordFailed`

**Symptom:** Revocation effects on derived artifacts or manifest
membership fail to land.
**Signature:** Matching class above;
`revocation_propagation_terminal_failed_<kind>` increments in the
operational summary (kind = `derived_records`, `export_manifests`,
`export_manifest_items`, `benchmark_artifacts`, `ranker_artifacts`,
`physical_delete_receipts`).
**Root cause:** Almost always a DB mirror outage or a constraint
deadlock under load — every one of these handlers is a thin call to
the PG mirror.
**Fix:**
- Verify DB mirror connectivity and that RLS predicates (forced via
  `trace_current_tenant_id()`) are intact.
- For deadlocks, the retry cap (`TRACE_COMMONS_REVOCATION_PROPAGATION_MAX_ATTEMPTS`)
  usually clears transient drift. If a specific item stays terminal-failed,
  inspect the DB row for unexpected state.

## `PerplexityScorerInferenceFailed` (non-OOM)

**Symptom:** All gate evaluations fail, GPU memory looks fine.
**Root cause:**
- Model files corrupted (failed download).
- CUDA driver / candle build mismatch.
- Tokenizer file missing or wrong revision.
**Fix:**
- Re-run `stage-models.sh`; the SHA256 check catches partial downloads.
- `nvidia-smi` shows driver version; rebuild with matching
  `cargo build --features local-gpu-models-cuda`.

## Audit chain drill fails after PG restore

**Symptom:** `AuditChainDriftRejected` after restoring from backup.
**Root cause:** The backup ended mid-write — the chain has an entry
that references a `prev_audit_event_hash` that doesn't match.
**Fix:** Restore to an earlier PITR point. If using daily snapshots
only, accept the drift loudly: open an incident, set
`TRACE_COMMONS_REQUIRE_DB_RECONCILIATION_CLEAN=false` temporarily, and
manually mark the chain as re-anchored. **Document this clearly in the
audit table.**

## Smoke gate exits 1 with an unfamiliar class

**Symptom:** `scripts/operator/smoke-gate.sh` returns non-zero with a
class name not in this doc.
**Fix:** Check
[`hash-only-logging.md`](hash-only-logging.md) for the broader table,
then `grep -rn "<class>" crates/` to find where it's emitted. Update
both that doc and this one once the root cause is known.

## "Everything looks fine but nothing's happening"

**Symptom:** Submissions arrive but no gate decisions appear.
**Root cause:** Gate worker route is not being driven. Check:
- Worker scheduler env vars (each worker has its own
  `_SCHEDULER_ENABLED` + `_SCHEDULER_TOKEN`).
- `trace_audit_events` for recent rows — is anything moving?
- Are submissions in `accepted` state? Gate evaluates after acceptance.

## Calibration pass rate way off target

**Symptom:** After Phase 2 re-cal, pass rate is 80% (or 5%) instead of
30%.
**Root cause:** Distribution shift between bootstrap and pilot. The
floors set from OASST2 don't apply to the pilot's domain.
**Fix:** Re-run `analyze-calibration.sh` on **pilot** data only (filter
the CSV to recent gate_version_hash rows). The pilot-derived floors
will be different and are the correct values.
