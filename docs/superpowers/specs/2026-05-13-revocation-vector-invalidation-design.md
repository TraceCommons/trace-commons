# Revocation → Vector Invalidation Hook — Design (Phase A6)

Date: 2026-05-13
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets + Revocation lanes
Predecessors:
- `2026-05-11-private-vector-system-design.md` (rephased)
- `2026-05-13-vector-index-design.md` (A4 — adds the tenant_storage_ref param to `VectorIndex::delete`)
- PR #12 (shipped `EnclaveGateService::invalidate_vector_entry` no-op + `GateDecision.vector_entry_id`)
- Migration V24 (added `vector_entry_id` column on `trace_gate_decisions`)

## Goal

When a trace is revoked, the existing revocation-propagation worker must
also invalidate the corresponding vector index entry — otherwise the
embedding stays searchable in the corpus and a future contributor
could be denied novelty credit because their trace looks "similar to"
a revoked trace that should no longer count.

The schema for this already exists. The worker action enum has
`InvalidateVector` (per `TraceRevocationPropagationAction`); the
target enum has `VectorEntry { vector_entry_id }` (per
`TraceRevocationPropagationTarget`). What doesn't exist is the
runtime hook that processes those targets by calling
`gate_service.invalidate_vector_entry`, plus a propagation-failure
audit-row shape that the previous implementer correctly flagged as
underspecified.

## Why this is needed

Three scenarios where vector invalidation matters:

1. **Contributor self-revocation.** A contributor revokes their
   submission post-credit. The credit ledger gets a negative entry
   (via existing revocation propagation) and the vector index entry
   should also disappear so the trace no longer affects future
   novelty scores.

2. **Operator-initiated revocation.** Privacy review, content abuse,
   etc. Same propagation requirements.

3. **Retention expiration.** A trace's retention window expires; the
   trace gets a tombstone and the vector entry should follow.

In all three, the existing revocation-propagation worker already
enqueues a `VectorEntry` propagation item per the schema's
`TraceRevocationPropagationItemWrite`. A6 makes the worker
*process* those items by calling into the gate service.

## Non-goals

- New schema. The existing propagation tables and enums cover this.
- Real-time invalidation. Propagation is async by design; eventual
  consistency is acceptable (and architecturally correct given
  how revocation already works).
- Cross-tenant invalidation. Each propagation item is tenant-scoped.
- Vector-index garbage collection beyond per-entry deletion. Compaction
  is a future operational concern.
- Vector-index rebuild from audit trail on corruption — A4 spec
  handles this as a manual operator command.

## Existing propagation pipeline (where we plug in)

Find the existing revocation propagation worker in
`bin/trace-commons-ingest.rs`. There's a worker route + helper functions
that:

1. **Page pending propagation items** (`status = Pending` or
   `status = Failed` with `next_attempt_at` past due) for the
   authenticated tenant.
2. **Dispatch each item** to a handler based on its `target.kind()`.
3. **Update the item's status** to `Done` on success or `Failed` on
   error, with `attempt_count` incremented and `next_attempt_at` set
   for retry.
4. **Append a hash-only audit row** capturing the propagation action.
5. **Return a summary** for the operational summary endpoint.

The handlers for `ObjectRef`, `ExportManifest`, `DerivedRecord`, etc.
already exist. The `VectorEntry` handler today is either missing or
emits a no-op success. A6 makes it real.

## The implementation

### 1. Hook into the revocation-propagation worker

In `bin/trace-commons-ingest.rs`, find the propagation dispatch (search
for matches on `TraceRevocationPropagationTarget::VectorEntry`).
Replace the no-op with a call into `state.gate_service`:

```rust
TraceRevocationPropagationTarget::VectorEntry { vector_entry_id } => {
    let tenant_ctx = gate_tenant_ctx_from_auth(&tenant);
    match state.gate_service.invalidate_vector_entry(&tenant_ctx, *vector_entry_id) {
        Ok(()) => RevocationPropagationOutcome::Done,
        Err(e) => {
            let error_hash = sha256_text_hex(&format!("{e}"));
            tracing::warn!(
                tenant_id = %tenant.tenant_id,
                propagation_item_id = %item.propagation_item_id,
                error_class = "VectorInvalidationFailed",
                error_hash = %error_hash,
                "revocation propagation vector invalidation failed"
            );
            RevocationPropagationOutcome::Failed {
                last_error_class: "VectorInvalidationFailed".to_string(),
                last_error_hash: error_hash,
            }
        }
    }
}
```

The `RevocationPropagationOutcome` is whatever the surrounding helper
returns to drive the status update — read the existing handler shape
and match it.

### 2. The propagation-failure audit-row shape

The previous implementer (PR #12 era) flagged this as underspecified.
Walking the requirements:

- The audit row records that a propagation attempt failed
- Hash-only — no raw error strings, no tenant policy state
- Stable error class for grep-ability
- Bound to the `propagation_item_id` so the operator can find it
- Bound to the `submission_id` (revocation origin) for forensics

The shape, mapping to the existing typed-audit-metadata enum:

```rust
TraceAuditMetadata::RevocationPropagationFailure {
    propagation_item_id: Uuid,
    source_submission_id: Uuid,
    target_kind: TraceRevocationPropagationTargetKind,
    action: TraceRevocationPropagationAction,
    error_class: String,           // stable label, e.g. "VectorInvalidationFailed"
    error_hash: String,             // sha256:<hex> of the raw error text
    attempt_count: u32,             // the attempt that just failed
}
```

The existing `TraceRevocationPropagationItemRecord` already has
`last_error: Option<String>` — but that's only the most recent
attempt's error and lives on the working row, not in audit. The new
audit metadata is the durable record.

If the project's typed audit metadata enum is in
`trace_corpus_storage.rs`, add the variant there. Add a matching
storage-mirror variant. Add migration coverage if the audit-metadata
column is typed in PostgreSQL.

**Naming convention:** the action-specific error classes follow the
pattern `<Action>Failed`. For the four most likely propagation-failure
targets:

| Action | Error class |
|--------|-------------|
| `InvalidateVector` | `VectorInvalidationFailed` |
| `InvalidateMetadata` | `MetadataInvalidationFailed` |
| `InvalidateExportMembership` | `ExportInvalidationFailed` |
| `DeleteObjectPayload` | `ObjectDeletionFailed` |

Reuse existing strings where the codebase already has them.

### 3. Retry semantics

The existing propagation infrastructure already handles retries:
`attempt_count` increments, `next_attempt_at` slides on failure with
some backoff, and a `Failed` item with `next_attempt_at` past is
re-claimed on the next worker tick.

What A6 adds: a per-action retry cap. After N failures (default 5)
the item goes to a terminal `Failed` state and the operator must
intervene. This prevents an infinitely-failing propagation item from
filling the worker's claim queue.

New env: `TRACE_COMMONS_REVOCATION_PROPAGATION_MAX_ATTEMPTS` (default 5).
Apply in the worker's claim logic: items with `attempt_count >= max`
are reported as terminal-failed in operational summary and not
retried.

### 4. Gate-service safe-status check

When `gate_service.safe_status().kind == "legacy_deterministic"` (the
default, no real gate wired in), the gate service's
`invalidate_vector_entry` is a no-op success. The propagation item
gets marked `Done` with a hash-only audit row, even though nothing
actually happened (there was no vector entry to invalidate). This is
correct behavior — the audit trail records "the gate service was
asked to invalidate and said OK."

When `gate_service.kind == "dstack_stub"`, the call bails (per the
stub's contract). The propagation item correctly fails and retries.

When `gate_service.kind == "enclave_mock"` or `"enclave_local_gpu"`,
the call actually deletes the entry from the index.

### 5. Tests

Add caller tests in `bin/trace-commons-ingest.rs`:

1. **Happy path:** seed a tenant + a `VectorEntry` propagation item +
   configure `InMemoryGateService`. Run the worker tick. Assert:
   - The propagation item moves to `Done`
   - A propagation-success audit row lands (existing shape)
2. **Gate service bails (`DstackGateService`):** same seed but with
   the dstack stub. Run the worker tick. Assert:
   - Item moves to `Failed` with `attempt_count = 1`,
     `last_error` set, `next_attempt_at` in the future
   - A `RevocationPropagationFailure` audit row lands with
     `error_class = "VectorInvalidationFailed"`
3. **Retry cap exhausted:** seed an item with `attempt_count = 4`,
   `max = 5`. Run the worker. Assert the next attempt either
   succeeds or fails — if fails, item moves to terminal `Failed` and
   subsequent ticks don't re-claim it.
4. **Tenant isolation:** seed identical propagation items under two
   tenants. Run worker under tenant A. Assert tenant B's item is
   untouched.

These are unit-style caller tests, no PG required for the happy path
(the mocks suffice). Add a PG variant for the typed audit-metadata
column round-trip.

### 6. Operational summary

`/v1/admin/operational-summary` already exposes propagation aggregates
(per the existing `revocation_propagation_*` fields). Extend to
report:

- `revocation_propagation_terminal_failed_vector_entries` —
  count of items at `attempt_count >= max` with target kind
  `VectorEntry`

The other propagation target kinds get the same treatment but that's
out of scope here; A6 only owns the vector path.

## Hash-only logging discipline

All failure logs hash the raw error text. Stable error classes
(`VectorInvalidationFailed`, `RevocationPropagationFailure`, etc.)
are the only stable labels. The `propagation_item_id` and
`source_submission_id` are server-generated UUIDs (not user state),
safe to log.

## What this does NOT change

- The revocation-propagation worker route itself
- The propagation item schema (already V22)
- The `TraceRevocationPropagationTarget` / `Action` /
  `ItemStatus` enums (already complete for our needs)
- The credit-event reversal path (already wired in PR #12 + earlier)
- The existing scheduler / cron loop

## Failure modes summary

| Situation | Behavior |
|-----------|----------|
| Vector entry exists, gate service ready | `Done`, audit success row, item gone |
| Vector entry doesn't exist (already deleted) | `delete()` returns `Ok(false)`. We treat this as `Done` — the postcondition is "entry is gone" and that's already satisfied. Audit row notes "vector_entry_not_found" via a sub-field. |
| Gate service unavailable (dstack stub) | `Failed` with `VectorInvalidationFailed`. Item retries per backoff. |
| Gate service errors out (real impl) | Same: `Failed` with hash-only error class + retry. |
| Retry cap exhausted | Terminal `Failed`. Item appears in operational summary; no automatic retry. |
| Network partition between worker and gate service (when gate becomes a separate process) | Same as gate-service-errors-out; falls into retry path. (Today the gate service is in-process — this is forward-looking.) |

## Open questions

1. **Should `Ok(false)` (already-deleted) count as Done or as a
   distinguished "no-op" outcome?** Recommendation: **Done**. The
   contract is "make sure the entry is gone"; if it's gone already,
   we're satisfied. Add a sub-field to the audit row noting "no entry
   was present at invalidation time" so the operator can audit.

2. **Should the retry cap be per-target-kind?** Different
   propagation actions have different failure characteristics
   (object deletion may fail on network blip; vector invalidation
   may fail because the index is rebuilding). Recommendation:
   **single cap in v1** (env-controlled), refine per-kind if real
   failure patterns appear.

3. **Should the failure audit row include the action attempted
   AND the target kind?** Yes (both are in the typed metadata
   above).

4. **Should the existing per-action handlers (object-ref,
   export-manifest, etc.) be retrofitted to use the new typed
   audit-failure shape?** Recommendation: **yes**, but as a
   separate follow-up. A6 owns the vector path only, but the
   shape generalizes; once it works for vector, retrofit the
   others in a single mechanical PR.

## Cost estimate

| Item | Estimate |
|------|----------|
| Typed audit-metadata variant + storage mirror | 1 day |
| Worker dispatch hook for `VectorEntry` | <1 day |
| Retry cap env + worker logic | 1 day |
| Tests (4 cases above) | 1-2 days |
| Operational summary extension | <1 day |
| Documentation | <1 day |
| **Total** | **~4-5 days of focused work** |

Code-only, no hardware dependency.

## What this spec commits to

- Worker dispatch hook for `TraceRevocationPropagationTarget::VectorEntry`
- New typed audit metadata variant `RevocationPropagationFailure`
- Per-attempt error-class strings (`VectorInvalidationFailed`, etc.)
- Retry cap via `TRACE_COMMONS_REVOCATION_PROPAGATION_MAX_ATTEMPTS`
- Operational-summary extension for vector-entry terminal failures
- Reuse of every existing propagation helper

## What this spec does not commit to

- Retrofit of other propagation target kinds to the new failure
  audit shape (separate follow-up)
- Per-target retry caps (single cap in v1)
- Real-time invalidation (eventual consistency stays)
- Vector-index garbage collection / compaction
