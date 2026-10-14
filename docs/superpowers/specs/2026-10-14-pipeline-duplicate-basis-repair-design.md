# Pipeline duplicate-basis repair (#1313) -- design note

Status: design only. Case (b) is ready to build. Case (a), as ruled, needs
a change to the pipeline's run model, and the work stopped here for an owner
decision (see "Case (a): why this stops at the design").

Issue: #1313 (item L2-6 of #1185). Rulings: the #1143 review. The duplicate
ruling is "near-duplicates are dropped after the first one", and N-7 is "the
index holds only revisions of runs that completed". Owner decisions of
2026-10-14: (b) as ruled, keep N-7, no short form; (a) as ruled, in the same
branch.

Code read on `origin/main` at `91f37668d`.

## Case (b): a credited run whose Settle failed for good

### What happens today

The Trace Credit leg commits `complete` with a `credit_event_id`, and the
ledger row commits in the same transaction (`settle_internal_credit`). The
run completes later, in another transaction (`commit_settle_from_progress`
calls `commit_settle`). Three paths can fail the run between the two:

- `claim_next`'s sweep: a lease that expired with `attempt_count >=
  max_attempts`, for example after a crash after
  `PipelineCrashPoint::AfterInstrumentOperation` on the last attempt;
- `mark_retry`, when a charged retry exhausts the attempts;
- `mark_failed`, after a terminal label.

Each of the three calls `resolve_open_settlement_legs_on_tx`, which leaves the
completed leg alone, and then `invalidate_index_writes_of_failed_runs_on_tx`,
which queues the invalidation of the credited revision. The index loses an
entry that was paid for, and the next near-duplicate is credited again.

### Rule

A Settle run that a failure path would fail, and that holds only terminal
legs, is completed, not failed. It ends `complete`, so N-7 keeps its index
entries and no invalidation is queued. `invalidate_index_writes_of_failed_runs_on_tx`
does not change. The owner rejected the short form, a skip in that function,
because it overturns N-7.

A run counts as **finishable** when all of these hold:

- `next_phase = 'settle'`;
- the Settle selection is persisted (`pipeline_runs.settle_selection IS NOT
  NULL`, the column `load_settle_selection` reads);
- `index_write_state` is not `pending`, so the index write is done
  (`none`, `complete`, `cancelled` or `failed`; Step 5 runs before Step 6);
- every leg is `complete` or `forfeited`. A `failed` leg rules this out:
  `commit_settle_from_progress` refuses one, and only the failure path
  resolves it. A run with no legs, a zero award, also qualifies.

### Mechanism: one more claim through the normal path

The commit needs a live lease, the bound bundle (to verify the Score
decision), the persisted selection and a fresh submission guard. Only
`PipelineService::complete_settle_phase` has all four, and the sweep is SQL in
the store. So the sweep does not commit. It leaves a finishable run to be
claimed again, and the next claim runs Settle as usual:

- Step 4 reuses the persisted selection, and the Settle policy does not run
  again.
- Step 5 is skipped, because `index_write_state` is not `pending`.
- Step 6 skips every `complete` and `forfeited` leg, so no adapter is called
  and no ledger row is written.
- `commit_settle_from_progress` builds the outcome from the legs, and
  `write_pipeline_gate_decision_on_tx` is idempotent for its own run.

Changes:

1. **`claim_next` sweep.** The `expired` CTE leaves out finishable runs.
   They stay `leased`, with an expired lease.
2. **Claim predicates** (`claim_next`'s candidate, `claim_run`,
   `claim_run_with_lease_config`). Today a run is claimable when
   `attempt_count < max_attempts`. That becomes
   `attempt_count < max_attempts OR (finishable AND attempt_count <
   max_attempts + 1)`: exactly one extra, charged attempt, and only for the
   commit. The bound stays finite, so the terminal bound of 3 x
   `max_attempts` charged attempts that the #1143 review relied on becomes
   3 x `max_attempts` + 1.
3. **`mark_retry`.** A Settle run that this retry would exhaust, and that is
   finishable, goes to `retry`, due now, not to `failed`. The extra attempt
   then commits it. If that attempt is charged again, `attempt_count`
   reaches `max_attempts + 1` and the run fails as today: invalidated, legs
   kept.
4. **`mark_failed`.** It does not change. Its Settle labels are
   `index_key_conflict`, which comes before any leg, and the two gate-decision
   refusals. Those two are refusals *of the commit itself*, so a commit
   cannot finish such a run (see the residual below). `fail_run` and
   `charged_retry` keep reconciling dispatched legs first.
5. **Uncharged suspensions are unchanged.** A transient database error, a
   policy that is not runnable, or a credit hold in the extra attempt is
   `mark_transient_retry` as today, with no charge and no terminal bound,
   which is the ruling for suspensions.

`unapplied_index_commands_on_tx` does not change. It counts only
`index_write_state IN ('none', 'pending')`, and a finishable run is never
`pending`. A finishable run with state `none` and membership `excluded`
was already left out. One with state `none` and membership `included` cannot
reach Step 6: Step 5 sets `complete` or `cancelled` first.

### Residual, not covered

If `commit_settle` refuses with `pipeline_gate_decision_conflict` (another
run's pipeline row exists for the submission) or
`pipeline_gate_decision_evidence_incomplete`, it refuses on every attempt.
`complete_settle_phase` checks the evidence before any leg settles, so the
second refusal can follow a completed leg only if the outcome changed in
between. A conflict means two runs reached Settle for one submission, which
`insert_pipeline_receipt` exists to prevent. Such a run still fails under
N-7, and its paid revision leaves the index. Fixing that needs a rule this
note does not make.

### Double payment and concurrency (b)

- The extra attempt makes no adapter call and writes no ledger row: Step 6
  skips terminal legs. The ledger's unique index
  `idx_trace_credit_ledger_pipeline_score (tenant, run, score_outcome,
  instrument)` still backs this.
- The claim is the same `FOR UPDATE SKIP LOCKED` claim under a new lease
  token, so two workers cannot both hold the run. A worker whose old lease
  expired is fenced by the lease checks it already has.
- Lock order is unchanged: the run row, then the settlement and
  invalidation rows. The per-tenant Score lock is not involved.

### Tests (b), written first

- `a_credited_settle_run_crashing_on_its_last_attempt_completes_and_keeps_its_index_entries`:
  crash at `AfterInstrumentOperation` on attempt `max_attempts`, expire the
  lease, run `process_one`. Expect `complete`, no
  `pipeline_index_invalidations` row, one `novelty_utility` ledger row and no
  second adapter call. Today the run is `failed`, with an invalidation
  queued.
- The same through `mark_retry`: the last attempt fails with a charged label
  after the leg committed.
- `a_finishable_run_charged_again_fails_and_is_invalidated`: the extra
  attempt is bounded.
- `a_settle_run_with_a_failed_leg_still_fails`: an unchanged control.

## Case (a): a duplicate scored against a run that later fails

### What happens today

B's compatibility Score runs under the tenant's Score lock and reads the
unapplied commands of the tenant's other runs
(`unapplied_index_commands_on_tx`). A's command makes B fail
`novelty_passed`. The Score then returns **no awards and no index command**
(`versioned_pipeline_compat.rs` around lines 466-508), and
`commit_score_in` seeds legs "one per award", so **B has no settlement leg
at all**. B completes with an empty Settle. Its gate decision row records
`novelty_passed = false`. When A fails, A's command drops out, but no B
artifact records that B was counted against it.

### What the ruling needs

"Record the bases, and queue the duplicate's Score again." The record is
small:

- **Migration V127** (V125 and V126 are the last on `main`; other open
  branches are not visible from here):
  `pipeline_score_bases (tenant_id, run_id, basis_run_id, basis_revision_id,
  recorded_at, PRIMARY KEY (tenant_id, run_id, basis_run_id))`, with
  foreign keys to `pipeline_runs` that cascade, an index on
  `(tenant_id, basis_run_id)`, `ENABLE` and `FORCE ROW LEVEL SECURITY`, and
  the `trace_corpus_tenant_isolation` policy on `trace_current_tenant_id()`,
  granted like `pipeline_runs`.
- Written in `commit_score_in`, on the Score-lock transaction, from the rows
  `unapplied_index_commands_on_tx` returned. To keep the record small, only a
  Score that ends with `quality_passed = true` and `novelty_passed = false`
  writes them.
- Read in the fail transactions (the sweep, `mark_retry`, `mark_failed`),
  right after `invalidate_index_writes_of_failed_runs_on_tx`, for a basis run
  that ends failed with its command never applied
  (`index_write_state IN ('none', 'pending', 'cancelled')`). Lock order: the
  run rows the caller holds, then the invalidation rows, then the base rows,
  then the dependant runs, by `run_id`.

The record is not the problem. "Queue B's Score again" is. Nothing in the
schema lets a run be scored a second time:

1. **Score outcomes are immutable and unique per run.** `phase_outcomes` has
   `UNIQUE (tenant_id, run_id, phase)`, and the V92 triggers refuse every
   UPDATE and direct DELETE. No later migration relaxes either. B already
   has a Score outcome (and a Settle outcome, once complete), so B cannot be
   put back to `score`. That holds even while B has not yet completed.
2. **Re-deciding the credit means scoring again, not settling again.** B has
   no leg, no award, no command and no index entries. The credit depends on
   a new perplexity and embedder pass and a new novelty read against the
   current index, under the Score lock. Only that can produce the award, the
   index command (B's entries must enter the index, or the next
   near-duplicate is credited too), the leg and the ledger row.
3. **A second run for B's submission conflicts with three invariants.**
   - `trace_gate_decisions_one_pipeline_row` (V116) allows one pipeline row
     per submission, and `write_pipeline_gate_decision_on_tx` refuses another
     run's row with `pipeline_gate_decision_conflict`. A successor run would
     fail terminally at its own Settle commit.
   - Receipts are keyed one run per `request_idempotency_key`.
   - Five status readers pick a submission's run by `created_at DESC`, and
     about 66 non-test sites read `pipeline_run_settlements`.

So the ruled design needs one of these:

- **(a1) A rescore generation on a run.** Add `score_generation` to
  `pipeline_runs`, `phase_outcomes` (making the unique key `(tenant, run,
  phase, generation)`), the settlement legs (whose primary key is `(tenant,
  run, instrument)`) and the ledger key. Every `outcome_for_phase` caller
  (7), the direct `FROM phase_outcomes` readers (16), the exports and the
  qualification readers then have to pick a generation. The gate decision
  row has to become updatable for its own run.
- **(a2) A successor run for the same submission.** A new run, bound to B's
  bundle, starting at `score` with B's approved revision and a derived
  idempotency key. The gate decision row moves from B to the successor
  (V116 changes from "one row" to "one row per live run"). The status
  readers, the withdrawal and revocation fan-out, the exports and
  `insert_pipeline_receipt` all need to know about successors.

Either is a change to the run model, larger than the issue's "needs a
design", and it touches readers outside this issue (status, exports, gate
consumers). The task said to stop in that case, not to pick a model.

### Double payment (a), whichever model is chosen

- B was never paid: an empty award seeds no leg and no ledger row. The first
  rescore pays at most once through the existing ledger key, because its
  outcome or run id is new.
- The second payment to prevent is a **second rescore**: A1 and A2 both fail,
  or B's rescore counts C and C fails. Proposed rule: a submission is
  rescored only while none of its runs or generations has a `trace_credit`
  leg with a `credit_event_id`. The check runs under the Score lock, in the
  rescore's commit, so two concurrent rescores cannot both pass it.
- **A nondeterministic scorer.** The NEAR AI scorer is not bit-stable, so a
  rescore gives a run a second try at the perplexity floor. Requeuing only a
  Score that already had `quality_passed = true` and `novelty_passed = false`
  limits the rescore to the novelty question the ruling is about.

### Concurrency (a)

- Requeuing happens in the transaction that fails A, which holds A's run row.
  It only marks B (or inserts B's successor) as due. It never takes the
  tenant's Score lock, so it cannot deadlock with a Score that holds the lock
  and reads `pipeline_runs`.
- The rescore takes the Score lock with `pg_try_advisory_xact_lock`, like any
  Score. A busy lock releases the run uncharged.
- Withdrawal of a basis run (A withdrawn, not failed) also drops A's command
  without an index entry. The ruling covers failure only. **Open question.**

### Hash-only and fail-closed

New log lines carry run ids hashed (`sha256_prefixed`), as the
`unapplied_index_command_unreadable` warning does. A basis row that cannot be
read fails closed: no requeue, and an operator-visible label.

## Owner questions

1. Case (a): a rescore generation (a1) or a successor run (a2)? Or accept (a)
   as a known loss, with the bases recorded for a later backfill?
2. Case (a): does withdrawal or revocation of a basis run also requeue?
3. Case (b) residual: what should happen to a run with completed legs whose
   Settle commit refuses for good (a gate decision conflict)?
