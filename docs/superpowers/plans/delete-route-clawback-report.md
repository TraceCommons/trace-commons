# Delete route: credit clawback and content deletion

Scope: `DELETE /v1/traces/{submission_id}` and `POST /v1/traces/{submission_id}/revoke`,
both funnelling into `revoke_submission` in
`crates/trace-commons-server/src/bin/trace-commons-ingest.rs`.

## The question that decided the change

**Is the `credit_points_final = Some(0.0)` write in `revoke_submission` load-bearing
for credit settlement, or is settlement already gated on status?**

Answer: **settlement is gated on status. The zeroing was belt-and-braces, and as a
display value it read as a penalty.**

### (a) Does anything select rows for settlement by `credit_points_final` value, or by status?

By status, never by the value.

`run_credit_settlement` builds its candidate set from the credit *ledger event*
table, not from `credit_points_final`. The filters are:

- `trace_credit_event_type_is_settlement_eligible(event.event_type)`
- `event.credit_points_delta > 0.0`
- event not already in a `Finalized` settlement batch
- the event's submission record has `status == TraceCorpusStatus::Accepted`
  **and** `delayed_credit_applies_to_record(record)` (i.e. `!record.is_terminal()`)
- the account is not on a credit hold

`is_terminal()` covers `Revoked | Expired | Purged`. So the moment revocation flips
status to `Revoked`, the trace's credit events fall out of the settlement candidate
set on two independent predicates. The value of `credit_points_final` is never
consulted anywhere in that path.

The same is true of the aggregate surfaces:

- `TraceCommonsTenantCreditResponse::from_records_events_and_settlements` sums
  `credit_points_final` **only in the `Accepted` arm**. A `Revoked` record only
  increments the `revoked` counter; its `credit_points_final` is never added.
- Per-contributor caps (`run_recompute_contributor_caps_pass` /
  `contributor_cap::increment_micros`) run off gate-decision rows
  (`credit_quality_micros`, `dedup_cluster_size`). They never read
  `credit_points_final`.
- The delayed-credit ledger projection is excluded for terminal records via
  `delayed_credit_applies_to_record`.

The only place the value surfaced after revocation was the per-submission receipt /
status update (`receipt_from_record`, `submission_status_from_record`) and the
contributor CLI's rendering of it. That is a display value, not a control.

### (b) If a trace already settled on-chain before revocation, does the zeroing do anything?

Nothing. It is cosmetic after the fact.

The real, load-bearing clawback is a **separate mechanism**:
`mirror_revocation_to_db` calls
`enqueue_credit_settlement_reversal_items_for_revocation`, which walks the tenant's
credit events for the submission, keeps only those with
`settlement_state == Final` that appear in a `Finalized` settlement batch, and
enqueues a `ReverseCreditSettlement` revocation-propagation item per event
(idempotency-keyed, hash-only reason). The revocation-propagation worker then
executes the reversal and the NEAR reverse-receipt outbox entry.

That pipeline is untouched by this change and is unaffected by the value of
`credit_points_final`.

### (c) Does any invariant, reconciliation, or cap computation assume a revoked trace reads 0.0?

No invariant found. See (a): the tenant credit summary skips the field entirely for
revoked records, caps use a different signal, and the ledger is excluded via
`is_terminal()`. One test asserted the zero
(`statuses_after_revoke[0].credit_points_final == Some(0.0)`); it is updated.

## What survived a revocation before this change

`revoke_submission` tombstoned and relabelled. It did not delete anything locally.

Surviving after a revocation, on the pre-change code:

1. **The stored envelope body.** Outside object-primary submit/review mode,
   `store_envelope` writes a plaintext JSON envelope to
   `state.root/tenants/<key>/objects/<status>/<submission_id>.json`. Nothing in the
   revocation path deleted it. The retention sweep's purge branch only fires for
   `status == Expired`, and revoked records `continue` before reaching it, so the
   sweep never removed it either. It survived indefinitely.
2. **The encrypted artifact.** Deletion was deferred to the revocation-propagation
   outbox (`enqueue_object_payload_delete_items_for_revocation` ->
   `DeleteObjectPayload`), which only fires when *all* of: a DB mirror is
   configured, the propagation worker is running, the object ref is registered, the
   object store is service-owned (`is_service_owned_trace_object_store` excludes the
   local file store), and the artifact kind is in the supported set. With no DB
   mirror, nothing was enqueued and the ciphertext stayed.
3. **The submission metadata record**, status-flipped to `Revoked`, still pointing at
   `object_key` and holding `artifact_receipt`. Intentional.
4. **The derived record**, status-flipped to `Revoked`, still holding
   `canonical_summary` — a natural-language summary derived from the trace. See
   "Left open" below.
5. Credit ledger events (retained for audit, hidden from contributor projections),
   audit rows, and the hash-only tombstone. Intentional.

## What changed

`crates/trace-commons-server/src/bin/trace-commons-ingest.rs`:

1. **Stopped zeroing `credit_points_final` in `revoke_submission`.** Credit already
   awarded stays. Withdrawal is a contributor's right, not an offence; treating it
   as one deters honest use.
2. **Same in the retention-sweep sibling site** (`run_maintenance`, the branch that
   reconciles a file record against an existing tombstone). Same decision, same
   reason.
3. **Fail-closed precheck**, before any mutation: if the record carries an
   `artifact_receipt` but `state.artifact_store` is `None`, the request is refused
   with `503` and the missing-control name
   `trace_artifact_store_unconfigured`. Tombstoning a record while leaving its
   ciphertext in an unreachable store would make "revoked" a lie.
4. **Content deletion**, as the last step of `revoke_submission`:
   `delete_trace_objects_for_record` removes the stored envelope body and the
   encrypted artifact. It runs last deliberately — `redaction_hash_for_record` reads
   the stored envelope to build the tombstone, so deleting earlier would strip the
   tombstone of the hashes it exists to carry. Errors propagate; a store that refuses
   deletion (the disabled remote object store returns an error from
   `delete_artifact`) fails the request rather than leaving the payload behind.
   Deleting an already-absent object is a no-op, so re-revoking is idempotent.

The tombstone is unchanged and remains hash-only: tenant/submission keys plus
`redaction_hash` and `canonical_summary_hash` sha256 digests. No identity, no path,
no content.

### Not changed, deliberately

- `credit_points_final = Some(0.0)` on the **review-reject** path is a different
  decision — credit was never awarded, not clawed back. Left alone.
- `enqueue_credit_settlement_reversal_items_for_revocation` and the whole
  `ReverseCreditSettlement` propagation pipeline. See "Escalate".

## Tests

Added to `trace_commons_ingest_internal/tests.rs`:

- `revocation_leaves_awarded_final_credit_unchanged`
- `revocation_deletes_stored_envelope_body_and_encrypted_artifact`
- `revocation_fails_closed_when_artifact_store_is_unconfigured`
- `revocation_tombstone_carries_no_identity_path_or_content`
- `revoking_twice_is_idempotent`

Updated: the `Some(0.0)` assertion in the delayed-credit revocation test is now
`None`.

### PostgreSQL baseline

The DB-backed ingest suite requires `--test-threads=1`.

| | passed | failed | ignored |
|---|---|---|---|
| Baseline (HEAD b7dbcd8, untouched) | 800 | 99 | 1 |
| After this change | 805 | 99 | 1 |

Failure sets compared by test name: **identical**. Zero new failures, zero fixed.
The 99 are the known pre-existing PG failures that CI never runs.

Also clean: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins`,
`RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --no-run`,
`cargo fmt --all`, and clippy with the repo allow-list (the only two clippy
warnings are pre-existing `partialeq_to_none` hits in an unrelated
community-snapshot test).

## Escalate

**The reversal pipeline is now the only clawback, and it is a policy contradiction
with this change.** Revocation still enqueues `ReverseCreditSettlement` propagation
items for credit that has already settled on-chain, and a NEAR reverse-receipt
outbox entry. So after this change a contributor who withdraws a trace sees the
awarded figure preserved in their receipt while any *settled* credit for that trace
is still reversed on-chain behind them.

That is a deliberate, separate mechanism with real on-chain consequences, and
deciding whether "withdrawal is not a punishment" extends to it is a project-owner
call, not one to make inside this handler. It was left untouched.

**Second, smaller finding:** the derived record's `canonical_summary` — a
natural-language summary derived from the trace body — survives revocation in both
the file record and the mirrored DB row. Only its status is flipped. Clearing it
consistently would need a DB-side write path that does not currently exist, so it
was reported rather than half-fixed. It also feeds a dedup candidate filter
(`!candidate.canonical_summary.trim().is_empty()`), so clearing it has behavioural
reach beyond storage hygiene.
