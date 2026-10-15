# Pipeline duplicate-basis repair (#1313) -- design

Status: implemented on `pipeline-duplicate-basis-repair` for case (b) and
for the gap the owner ruled on. For case (a), the bases are recorded and the
rescore is deferred.

Issue: #1313 (item L2-6 of #1185). Rulings come from the #1143 review. The
duplicate ruling is "near-duplicates are dropped after the first one". N-7
is "the index holds only revisions of runs that completed".

Owner decisions of 2026-10-14:

1. Case (b) as ruled. Keep N-7, with no short form.
2. The gap (a paid run whose Settle commit is refused for good) is held for
   operator review and not failed.
3. Case (a): record the bases now, and defer the rescore.
4. The migration is V128. V127 is #1237's.

Code read on `origin/main` at `91f37668d`.

## Migration V128

`migrations/V128__pipeline_settle_finish_and_score_bases.sql`:

- `pipeline_runs.state` gains `awaiting_operator`. The check constraint is
  widened.
- `pipeline_runs.operator_held_at TIMESTAMPTZ` records when the run was
  last held for an operator. Every hold sets it and nothing clears it. The
  runtime gets `UPDATE` on this column only.
- `pipeline_runs.settle_finish_granted_at TIMESTAMPTZ` records when a
  failure path gave a finishable Settle run its one extra attempt. The
  runtime gets `UPDATE` on this column only. `max_attempts` keeps no update
  grant, because the extra attempt is an attempt given back:
  `attempt_count = max_attempts - 1`.
- `pipeline_score_bases (tenant_id, run_id, basis_run_id, basis_revision_id,
  recorded_at)`, with primary key `(tenant_id, run_id, basis_run_id)`:
  - a foreign key to the scored run, which cascades;
  - no foreign key on the basis run, so the record outlives a basis run that
    is deleted with its submission;
  - an index on `(tenant_id, basis_run_id)`;
  - `ENABLE` and `FORCE ROW LEVEL SECURITY`, with the
    `trace_corpus_tenant_isolation` policy on `trace_current_tenant_id()`;
  - for the runtime, `SELECT` and `INSERT` only.

  The table is listed in `TRACE_COMMONS_RLS_TABLES`, so the pipeline's
  tenant-isolation control in the operational summary covers it.

## Case (b): a paid run whose Settle failed for good

### What happened

The Trace Credit leg commits `complete` with its ledger row in one
transaction. The run completes in a later one (`commit_settle`). Three paths
could fail the run between the two:

- the sweep in `claim_next`, on an expired lease with the attempts spent
  (for example a crash after `AfterInstrumentOperation` on the last attempt);
- `mark_retry` when it exhausts the attempts;
- `mark_failed`.

N-7 (`invalidate_index_writes_of_failed_runs_on_tx`) then removed the paid
revision from the index, and the next near-duplicate was paid again.

### Rule

A run is **finishable** (`settle_run_is_finishable_sql!`) when all of these
hold:

- `next_phase = 'settle'`;
- the selection is persisted;
- `index_write_state` is `none`, `complete` or `cancelled`. It is not
  `pending`, and not `failed`, because partly written entries must leave the
  index under N-7;
- every leg is `complete` or `forfeited`. A run with no leg qualifies.

A failure path that would fail a finishable run whose
`settle_finish_granted_at` is still NULL (`finishable_settle_run_sql!`)
does not fail it. Instead it:

- moves the run to `retry`, due now;
- sets `attempt_count = max_attempts - 1`;
- sets `settle_finish_granted_at`;
- sets the label `settle_finish_granted`.

The next claim runs Settle through the normal path:

- Step 4 reuses the selection.
- Step 5 is skipped, because the index write is not `pending`.
- Step 6 skips every terminal leg.
- `commit_settle_from_progress` commits.

The run ends `complete`, so N-7 never applies to it.
`invalidate_index_writes_of_failed_runs_on_tx` is unchanged.

Where the rule applies:

- **`claim_next`**: one statement before the sweep grants the extra attempt
  to expired, exhausted, finishable runs. It uses the sweep's own lock rule
  (`FOR UPDATE OF p SKIP LOCKED`, in run-id order). The sweep then sees them
  as `retry` and leaves them, and the same claim can pick one up.
- **`mark_retry`**: after its UPDATE fails a Settle run, the same
  transaction converts a finishable run to the extra attempt. In that case it
  resolves no legs, queues no invalidation and rejects no submission.
- **`mark_failed`**: the gap below.

**Bound.** The extra attempt is given once. A run charged again on it fails
exactly as before, and N-7 applies. Uncharged suspensions during the extra
attempt (a transient database error, a policy that is not runnable) still
give the attempt back, as they always did.

## The gap: a paid run whose Settle commit is refused for good

`commit_settle` can refuse permanently with `pipeline_gate_decision_conflict`
(another run's pipeline gate-decision row exists for the submission) or with
`pipeline_gate_decision_evidence_incomplete`. A retry cannot finish such a
run.

**Rule** (`hold_paid_settle_run_for_operator_on_tx`, inside `mark_failed`'s
transaction). A run is held, not failed, when all of these hold:

- the run is in Settle;
- its label is one of those two, or the run was held before
  (`operator_held_at` is set: an operator released it and its attempt
  failed);
- it is finishable, ignoring the grant;
- at least one leg paid: `complete` with a `credit_event_id`, a
  `result_ref_hash` or an `external_receipt_hash`. Every leg that completes
  carries a receipt, so this is every `complete` leg.

A held run moves to **`awaiting_operator`** and keeps its label. Its legs
are left alone (they are terminal), no invalidation is queued, and the
submission is not rejected. Its paid revision stays in the index.

Every other `mark_failed`, and every refusal on a run with nothing paid,
still fails under N-7.

**A released run is never failed by an automatic path** (review of #1413).
`mark_failed`, `mark_retry` exhaustion and the `claim_next` sweep of an
expired lease all hold a run that was held before, if it is still
finishable and paid, instead of failing it. Without this, a worker crash
during a released attempt let the sweep fail the run, and N-7 removed the
paid revision the operator had chosen to keep.

- **Audit.** The hold sets the V118 review audit marker. The worker's review
  audit pass (`append_pipeline_review_audit_events`) appends a system
  `lifecycle_status_change` event:
  - status `accepted`: the submission's status, which the hold does not
    change (Review accepted it and a leg paid). The event records the
    hold as a reason, not a status change. `quarantined` was considered
    and rejected: it would have stated, permanently, a status the
    submission never had;
  - reason label `pipeline_settle_held_for_operator`;
  - event id derived from the run and the hold's time
    (`pipeline-settle-held-for-operator-audit`), so each hold gets one
    event.

  The pass reads `operator_held_at`, not the run's state, so an operator
  who ends the hold before the pass runs does not lose the hold's event.

  The event is label-only, with no run id and no content. A `tracing::warn!`
  logs the label, the reason label, the tenant storage reference and a hash
  of the run id.
- **Operator surface.** `GET /v1/admin/pipeline/operational-summary`
  (`PipelineProductStore::operational_summary`, served by
  `pipeline_operational_summary_handler`). `trace-commons-admin
  operational-summary` reads `main`'s summary, not this one:
  - the `work` rows group by `(phase, state, reason label)`, so a held run
    shows as `settle / awaiting_operator / pipeline_gate_decision_conflict`;
  - the new count `operator_held_run_count` (`serde(default)`, so an older
    reader is unaffected).

  Activation readiness ignores the state: it measures only `pending`,
  `retry` and `leased` work.
- **No claim selects it.** The claim queries list only `pending`, `retry`
  and expired `leased` runs. `runnable_bundle_ids` keeps the run's bundle
  loadable while it is held.
- **Contributor status.** The contributor sees `pending`. Processing has not
  finished, and the credit already shows from the leg.
- **Operator decision**
  (`PgPipelineStore::resolve_operator_held_run`):
  - `Fail` ends the run `failed` (`operator_failed_held_run`) and applies
    N-7 in the same transaction.
  - `Release` gives it one more attempt, due now
    (`operator_released_held_run`). Its legs are terminal, so the attempt
    only commits. The release counts as the run's extra attempt (it sets
    `settle_finish_granted_at`), so no automatic grant follows; if the
    attempt fails in any way, the run is held again.

  Both act only on a run that is still `awaiting_operator`. The route is
  `POST /v1/admin/pipeline/runs/{run_id}/held-run-resolution` with
  `{"resolution": "fail" | "release"}`, behind `main`'s admin credential,
  for the credential's tenant. It answers the run's new state, or `404`
  for a run that is not held. Each decision appends a
  `lifecycle_status_change` row with status `accepted` and the decision's
  label as its reason.
- **Withdrawal** of a held run's submission queues the revision's
  invalidation through the existing path, because the index write is
  `complete`. The run stays held for the operator.

## Double payment and concurrency

- The extra attempt and a `Release` call no adapter and write no ledger row:
  Step 6 skips terminal legs. The ledger's unique index
  `idx_trace_credit_ledger_pipeline_score` (run, Score outcome, instrument)
  still backs this.
- The tests assert one ledger row and no new adapter request after the
  extra attempt.
- Grants and holds run inside transactions that hold the run row: the
  claim's `FOR UPDATE SKIP LOCKED`, and the lease-fenced UPDATE of
  `mark_retry` and `mark_failed`. Two workers cannot both act on one run.
  Lock order is unchanged: the run row, then the settlement and invalidation
  rows.
- None of this takes the per-tenant Score lock.

## Case (a): bases recorded, rescore deferred

### What happens

B's compatibility Score counts A's unapplied index command, fails novelty,
and gets no award, no command and no leg. If A then fails with its command
never applied, nothing ever credits B.

### What is built

`commit_score_phase` keeps the `(run id, revision id)` of every row that
`unapplied_index_commands_on_tx` returned under the tenant's Score lock.
`score_and_commit` passes them to `commit_score_in` only when the Score's
evidence says `quality_passed = true` and `novelty_passed = false`.
`commit_score_in` inserts them into `pipeline_score_bases` on the
Score-lock transaction, after the outcome, so a refused commit records
nothing.

The rule records every unapplied run the Score counted, not only the
nearest. That over-approximates, on purpose: a backfill can recompute.

`PgPipelineStore::list_score_bases` reads them.

The runs affected by (a) are then:

```sql
SELECT b.run_id, b.basis_run_id
  FROM pipeline_score_bases b
  JOIN pipeline_runs a
    ON a.tenant_id = b.tenant_id AND a.run_id = b.basis_run_id
 WHERE b.tenant_id = $1
   AND a.state = 'failed'
   AND a.index_write_state IN ('none', 'pending', 'cancelled');
```

### What is deferred

There is no rescore. Case (a) is a **known, rare loss**: B's contributor is
not credited. A backfill follows once a rescore model is chosen. The
options are recorded in this note's first revision (commit `e245b686b`):

- a rescore generation on the run;
- a successor run.

Either changes the run model, because `phase_outcomes` is immutable and
unique per run and phase, and V116 allows one pipeline gate row per
submission. The double-payment rule proposed for the rescore: rescore a
submission only while it has no leg with a `credit_event_id`, checked under
the Score lock.

## Open questions

1. Should withdrawal or revocation of a basis run (its command dropped, not
   failed) also count toward (a)? Out of scope here.
2. Should holds cover other permanent Settle failures of a paid run (for
   example a bound bundle that no longer loads)? Today only the two
   gate-decision refusals are held.

## Tests

In `crates/trace-commons-server/tests/versioned_pipeline_runtime_pg.rs`:

- `a_paid_settle_run_that_crashes_on_its_last_attempt_completes_and_stays_indexed`
  (the sweep);
- `a_charged_retry_that_exhausts_a_paid_settle_run_gives_it_one_extra_attempt`
  (`mark_retry`);
- `a_finishable_run_charged_again_on_its_extra_attempt_fails_under_n7`
  (the bound);
- `a_paid_run_whose_settle_commit_is_refused_waits_for_an_operator` (the
  gap: hold, audit marker, summary, no claim, `Fail` under N-7);
- `an_operator_released_held_run_completes_without_paying_again`;
- `a_released_run_whose_attempt_crashes_is_held_again` (the sweep) and
  `a_released_run_charged_on_its_attempt_is_held_again` (`mark_retry`);
- `a_paid_run_refused_for_incomplete_evidence_waits_for_an_operator`;
- `a_run_paid_by_another_adapter_waits_for_an_operator` (a leg paid with a
  receipt and no ledger event);
- `a_near_duplicate_score_records_the_unapplied_run_it_counted` (case (a)
  bases).

In the ingest bin's `pipeline_http_pg_tests`:

- `the_review_audit_pass_records_a_run_held_for_an_operator` (status
  `accepted`, and the event survives an operator failing the run first);
- `the_operator_route_resolves_a_held_run`.

The existing N-7 test
`a_run_that_fails_for_good_after_its_index_write_has_its_revision_removed`
is unchanged and still passes: its write is `pending`, so the run is not
finishable.
