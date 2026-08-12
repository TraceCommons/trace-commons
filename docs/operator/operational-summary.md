# `/v1/admin/operational-summary`

The single highest-value endpoint for "is the system healthy right now."
Returns a JSON document with operator-visible labels and hash-only
counters. Read this doc to know which fields should alarm.

## Field categories

The response has roughly three layers:

1. **Reflection** — what the binary read at startup. Operator-visible
   labels: KEK provider, gate service kind, model IDs (the ones that
   were configured, not the contents). Stable across the binary's
   lifetime.
2. **Drill state** — for each `/v1/admin/*-drill`, the timestamp of the
   last successful run and whether it's currently passing. These feed
   `rollout-smoke/evidence`.
3. **Hash-only counters** — per-class error counts since startup.

## Reflection fields

| Field | Meaning | Alarm |
|---|---|---|
| `kek_provider.kind` | `gcp_kms`, `local_master_key`, etc. | Alarm if not `gcp_kms` in prod |
| `kek_provider.active_key_version` | Cloud KMS version label | Should advance after rotation |
| `gate_service_status.kind` | `enclave_local_gpu`, `mock`, `disabled` | Alarm if not `enclave_local_gpu` in prod |
| `gate_service_status.ready` | Models loaded, vector index open | Alarm if false |
| `gate_service_status.gate_version_hash` | Stamped on audit + credit rows | Alarm if changes unexpectedly |
| `gate_service_status.gate_policy_version` | Operator label | — |
| `gate_service_status.perplexity_model_id` | Configured ID | — |
| `gate_service_status.embedder_model_id` | Configured ID | — |
| `central_issuer_profile.principal_refs_present` | Required central-issuer principal refs configured | Alarm if false when `*_REQUIRE_CENTRAL_ISSUER_PROFILE=true` |
| `central_issuer_profile.issuer_approval_required` | Reflects env | — |
| `object_store.bucket_versioning_enforced` | Bucket has versioning on | Alarm if false in prod |
| `audit_chain.last_audit_event_at` | Most recent audit row timestamp | Alarm if more than ~1 hour stale during traffic |

## Drill state

For each drill (see [`drills.md`](drills.md)) the summary surfaces:

- `<drill>.last_run_at`
- `<drill>.last_outcome` (`success` | `failure` | `never_run`)

Alarm if any required-for-promotion drill has `never_run` or `failure`.

## Counters

These are cumulative since binary start. They reset on restart, so
absolute values are less informative than rate.

| Counter | Phase A novelty | Meaning |
|---|---|---|
| `kek_wrap_failed_total` | — | Cumulative `KekWrapFailed` |
| `kek_unwrap_failed_total` | — | Cumulative `KekUnwrapFailed` |
| `kek_context_mismatch_total` | — | Cumulative `KekContextMismatch` |
| `gcp_kms_encrypt_failed_total` | — | Cumulative |
| `gcp_kms_decrypt_failed_total` | — | Cumulative |
| `gcs_put_failed_total` | — | Cumulative |
| `gcs_get_failed_total` | — | Cumulative |
| `perplexity_scorer_inference_failed_total` | new | Phase A2 |
| `embedder_inference_failed_total` | new | Phase A3 |
| `vector_invalidation_failed_total` | new | Phase A4 |
| `revocation_propagation_terminal_failed_vector_entries` | new | Count of vector-entry revocations that exhausted retries and are terminal-failed. **Should be zero**; any non-zero value is an operator action item — investigate and remediate via the vector index drill. |
| `revocation_propagation_terminal_failed_object_refs` | new (A6 retrofit) | Count of object-payload revocations terminal-failed after exhausting retries. Cross-reference `ObjectDeletionFailed` in logs. **Should be zero.** |
| `revocation_propagation_terminal_failed_export_manifests` | new (A6 retrofit) | Count of export-manifest revocations terminal-failed. Cross-reference `ExportInvalidationFailed`. **Should be zero.** |
| `revocation_propagation_terminal_failed_export_manifest_items` | new (A6 retrofit) | Count of export-manifest-item revocations terminal-failed. Cross-reference `ExportInvalidationFailed`. **Should be zero.** |
| `revocation_propagation_terminal_failed_derived_records` | new (A6 retrofit) | Count of derived-record revocations terminal-failed. Cross-reference `DerivedRecordInvalidationFailed`. **Should be zero.** |
| `revocation_propagation_terminal_failed_benchmark_artifacts` | new (A6 retrofit) | Count of benchmark-artifact revocations terminal-failed. Cross-reference `BenchmarkArtifactInvalidationFailed`. **Should be zero.** |
| `revocation_propagation_terminal_failed_ranker_artifacts` | new (A6 retrofit) | Count of ranker-artifact revocations terminal-failed. Cross-reference `RankerArtifactInvalidationFailed`. **Should be zero.** |
| `revocation_propagation_terminal_failed_credit_settlements` | new (A6 retrofit) | Count of operator-raised credit-settlement reversals terminal-failed (often NEAR outbox dependency); revocation never enqueues one. Cross-reference `CreditSettlementReversalFailed`. **Should be zero.** |
| `revocation_propagation_terminal_failed_worker_queues` | new (A6 retrofit) | Count of worker-queue invalidations terminal-failed (typically missing external invalidator). Cross-reference `WorkerQueueInvalidationFailed`. **Should be zero.** |
| `revocation_propagation_terminal_failed_physical_delete_receipts` | new (A6 retrofit) | Count of physical-delete-receipt records terminal-failed after delete. Cross-reference `PhysicalDeleteReceiptRecordFailed`. **Should be zero.** |
| `audit_chain_drift_rejected_total` | — | Cumulative |
| `tenant_drift_rejected_total` | — | Cumulative |
| `privileged_action_abac_denied_total` | — | Cumulative |

## Alarm policy

- **Critical** — alarm immediately:
  - Any drill in `failure` state.
  - Any `revocation_propagation_terminal_failed_*` counter `> 0`
    (`vector_entries`, `object_refs`, `export_manifests`,
    `export_manifest_items`, `derived_records`, `benchmark_artifacts`,
    `ranker_artifacts`, `credit_settlements`, `worker_queues`,
    `physical_delete_receipts`).
  - `gate_service_status.ready == false`.
  - `audit_chain.last_audit_event_at` stale > 1h during business hours.

- **Warning** — investigate within a business day:
  - `*_total` rate > baseline (track per deploy).
  - `kek_context_mismatch_total` increasing (early sign of tenant_ctx
    misconfig).
  - `embedder_inference_failed_total` increasing (model file integrity).

- **Info** — log only:
  - `audit_chain.last_audit_event_at` advancing normally.

## Reading the field shape

Treat field names in this doc as canonical contracts. If the binary's
response renames a field, the rename must be reflected here in the same
PR. Operators have alerting wired to specific field paths.
