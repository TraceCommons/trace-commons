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

---

# Part 2: stop reversing settled credit on revocation

Project-owner decision: **credit already earned and settled stays earned.**
Revocation removes the trace from the commons and deletes its content, but does
not reach back onto the chain. A contributor who is uneasy about a trace must
not be financially penalised for pulling it back, or the trace stays in place.

## What triggered the reversal, and what does now

`mirror_revocation_to_db` called `enqueue_credit_settlement_reversal_items_for_revocation`,
which walked the tenant's `Finalized` settlement batches and enqueued one
`ReverseCreditSettlement` propagation item per settled credit event bound to the
revoked submission. The worker then wrote a negative credit event and, when the
original settlement carried a NEAR contract id, a `reverse_credit_receipt`
outbox row.

The enqueue call and the function are removed. Nothing on any revocation path
enqueues a reversal item any more.

## Flows changed

All three callers of `mirror_revocation_to_db` are revocation, and all three
stop clawing back:

1. `revoke_submission` — the contributor/reviewer `DELETE` route. The act the
   decision is about.
2. `run_maintenance`, the two revoked-record arms — a record already marked
   revoked in the file store, and a record reconciled against an existing
   tombstone. Both are the same act arriving late, not a separate policy.
3. `backfill_tenant_to_db`, the `record.is_revoked()` arm — re-mirroring an
   already-revoked record. Enqueuing a clawback from a *backfill* was the
   sharpest edge of the old behaviour.

## Flows deliberately left alone

- **Retention expiry.** `run_maintenance`'s `record.is_expired_at(now)` arm goes
  through `mirror_expiration_to_db`, which is a separate function and never
  enqueued credit reversals in the first place. Expiry is the operator's
  retention clock, not a contributor withdrawing; it was not touched and did not
  need to be.
- **Purge.** Same — no credit reversal on that path, before or after.
- **Review reject.** `credit_points_final = Some(0.0)` on rejection stands.
  Credit was never awarded there, so nothing is being clawed back.

## Reversal callers that survive

- `reverse_credit_settlement_for_revocation_propagation` — the worker that
  executes a `ReverseCreditSettlement` item. **Kept, unchanged.**
- `ensure_near_credit_reversal_outbox_item_for_revocation` — the NEAR reverse
  receipt it writes. **Kept, unchanged.**
- The `ReverseCreditSettlement` propagation action, its target variant, its
  `CreditSettlementReversalFailed` error class, and the `credit_points_reversed`
  netting in the contributor credit summary. **All kept.**

Keeping them matters for two reasons. It preserves the only mechanism by which
an operator can reverse a fraudulent or mistakenly settled credit — removing
that while removing the automatic clawback would be a worse outcome than doing
nothing. And any item enqueued before this change (the pilot may hold some)
still drains deterministically instead of becoming permanently unprocessable.

There is no API route that enqueues a `ReverseCreditSettlement` item; the
removed function was the only producer. An operator correction is therefore
currently an out-of-band row insert. Worth knowing; not worth inventing a route
inside this change.

## Reconciliation invariants checked

- **`reconcile_db_mirror` / `db_reconciliation_drill`** compare the file store
  against the DB mirror, symmetrically. With no reversal events written on
  either side, nothing drifts. Unaffected.
- **Contributor credit summary** (`TraceCommonsTenantCreditResponse`) sums
  settled credit from `Finalized` settlement line items, which are keyed by
  credit account and event id, not by submission status. A revoked trace's
  settled credit therefore stays counted — which is exactly the intended
  outcome. `credit_points_reversed` simply reads 0 unless an operator raised a
  reversal. No invariant assumed the two would move together.
- **No on-chain-versus-in-corpus reconciliation exists.** Nothing reconciles
  NEAR settlement totals against the set of traces currently in the corpus, so
  the scenario the decision worried about (settled credit for a trace no longer
  in the corpus) has no checker to break.
- **`revocation-effects-drill` did assume the clawback**, and this is the one
  place that needed a call. `delayed_credit_reversal_ready` required
  `credit_reversal_item_count > 0`, so with the clawback gone the check would
  have gone permanently red and blocked the rollout-smoke required check. The
  `> 0` requirement is dropped: zero reversal items is now the ready state, and
  the check reads as "the withdrawing contributor was not charged". It is not
  vacuous — any reversal item that *does* exist (operator correction, or an item
  enqueued before this change) must still be fully drained, with its reversal
  credit event and NEAR reverse receipt present, before the check goes green.

## Documentation and copy

- `docs/trace-commons.md` — worker description, revocation-effects-drill row,
  and both Phase-status table rows.
- `docs/trace-commons-storage.md` — effects-drill contract, propagation-item
  description, drill runbook line, propagation-completeness line, test-coverage
  line, and the "after revocation" acceptance line ("credit finalizes or
  reverses according to policy" was the sentence most directly contradicted).
- `docs/trace-commons-roadmap.md` — Phase A capability list.
- `docs/operator/troubleshooting.md`, `hash-only-logging.md`,
  `operational-summary.md` — `CreditSettlementReversalFailed` is now labelled as
  an operator-raised correction, with an explicit note that revocation never
  raises one.
- `crates/trace-commons-contributor/src/consent.rs` — the consent prompt now
  reads "you can revoke submitted traces later; credit you have already earned
  is kept". This is the promise the code now actually keeps.

## Tests

Three PG-backed tests were inverted and renamed. Each seeds real settled credit
(delayed training credit, benchmark-conversion credit, ranking-utility credit),
revokes, drains the propagation worker, and proves no clawback:

- `revocation_leaves_settled_credit_and_enqueues_no_near_reverse_receipt`
- `revocation_leaves_settled_benchmark_conversion_credit_intact`
- `revocation_leaves_settled_ranking_utility_credit_intact`

Each asserts: no `ReverseCreditSettlement` propagation item, no
`revocation_credit_reversal:*` credit event, no `reverse_credit_receipt` NEAR
outbox row, and — for the latter two — that the contributor's
`credit_points_settled` is unchanged and `credit_points_reversed` is zero.

`revocation_effects_drill_records_remote_credit_reversal_and_object_delete_evidence`
was updated for the new drill semantics (zero reversal items, drill still green).

### PostgreSQL baseline

`--test-threads=1`, `TRACE_COMMONS_PG_TEST_DATABASE_URL=postgres://localhost/trace_commons_test`.

| | passed | failed | ignored |
|---|---|---|---|
| Baseline (7c575ea, Part 1 committed) | 805 | 99 | 1 |
| After Part 2 | 808 | 96 | 1 |

Failure sets compared by name. **Zero new failures.** The three that left the
failure set are exactly the three inverted tests, under their old names — they
were failing at baseline (they asserted the clawback against a store whose
settlement path is part of the known pre-existing PG breakage) and pass under
their new names and new assertions. The remaining 96 are the known pre-existing
PG failures CI never runs.

`revocation_effects_drill_records_remote_credit_reversal_and_object_delete_evidence`
fails in both runs. Its `worker.failed` count dropped from 2 to 1 — the credit
reversal item that used to fail is simply no longer created; the remaining
failure is the pre-existing remote-object-delete one.

Also clean: `RUSTFLAGS='-D warnings' cargo check -p trace-commons-server --bins`,
`RUSTFLAGS='-D warnings' cargo test -p trace-commons-server --no-run`,
`cargo fmt --all`, clippy with the repo allow-list (same two pre-existing
`partialeq_to_none` warnings in an unrelated community-snapshot test).

## Still open: `canonical_summary` survives revocation

Not closed in this change. It is not a one-line fix, and here is precisely what
it needs.

The natural-language `canonical_summary` derived from the trace body survives
revocation in both the file derived record and the mirrored DB row; only
`status` is flipped. Closing it requires all four of:

1. **File record.** `TraceCommonsDerivedRecord.canonical_summary` is a
   non-optional `String`, so "cleared" has to mean empty string, and empty
   string already carries meaning elsewhere — the dedup candidate filter tests
   `!candidate.canonical_summary.trim().is_empty()`. Either the field becomes
   `Option<String>` (a serde-compatibility change across every stored derived
   record) or every consumer of the empty case is audited.
2. **DB row.** `invalidate_trace_submission_artifacts` already runs an `UPDATE
   trace_derived_records SET status = $3`, so the write path exists — but it is
   shared by six callers including the retention-expiry path. Nulling
   `canonical_summary` there must be conditional on the derived status being
   `Revoked`, or expiry silently inherits a content-deletion behaviour the owner
   explicitly separated from withdrawal.
3. **Backfill ordering.** `backfill_tenant_to_db` re-mirrors the derived record
   from the file store. If the file copy still holds the summary, a backfill
   re-populates the DB column after it was cleared. The file-side clear must
   land first and be durable.
4. **Tombstone invariant.** `canonical_summary_hash` must keep flowing into the
   tombstone. It is read from the derived record before the clear, so the
   ordering that Part 1 established for `redaction_hash` has to be extended to
   cover it.

Scoped as its own piece of work.
