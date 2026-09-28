# Pipeline activation and migration

> **Status: design stage.** This runbook describes tooling that is not in this
> repository yet: the activation routes and the `versioned_pipeline_pg` suite.
> It arrives with the pipeline runtime and qualification changes, and its
> commands can change before then. Do not follow it on a deployment.

Pipeline activation routes the qualified pipeline for new submissions. Retained
legacy records stay readable. The switch assigns each receipt to one
implementation.

The pipeline now has these properties:

- A receipt ownership record selects the legacy executor or the pipeline.
- A retry of a pre-switch receipt returns the original operation.
- It does not start a new pipeline run.
- Unique ledger source keys prevent both paths from awarding the same receipt.
- Tenant routing is an explicit record. Timestamps do not select the bundle.
- Activation and expansion require current drills, readiness, corpus
  evidence, credit reconciliation, index checks, and invalidation checks.
- Rollback selects an earlier qualified bundle for later runs only.
- An unsafe bound policy must be suspended. The run keeps its package.
- First-rollout containment stops new pipeline receipts. Workers and
  packages remain available.
- Legacy writers disable only after their pending owned work completes.
- Destructive schema cleanup is not part of this phase.

## Pipeline receipt routing before activation

`TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` lists the tenants whose new
receipts go to the pipeline. It has an effect only when the ingest build
injects a pipeline runtime. The repository binary injects none.

- Unset or empty (the default): every receipt takes the legacy path.
- Tenants listed with a runtime: those tenants' receipts go to the pipeline.
- Tenants listed without a runtime: ingest refuses to start with
  `pipeline_receipts_configured_without_runtime`.
- Removing a tenant from the list also stops the worker for that tenant.
  Its in-flight pipeline runs stay unprocessed until the tenant is listed
  again.

Activation replaces this list with qualified routing.

## Fail-closed dependency qualification

`assemble_ingest_pipeline_runtime` refuses to start an injected pipeline
runtime whose scorer, embedder, index, or any registered settlement adapter
is not production-qualified (`pipeline_runtime_is_production_qualified`),
with the safe label `pipeline_runtime_dependencies_not_production_qualified`,
whenever either is true:

- `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` lists at least one tenant, or
- `TRACE_COMMONS_PIPELINE_RUNTIME_REQUIRED` is set.

Tenants routed to the pipeline is, on its own, enough to trigger the
refusal -- an assembly that lists tenants without also setting
`TRACE_COMMONS_PIPELINE_RUNTIME_REQUIRED` no longer runs real receipts
through a non-production-qualified dependency (the Reference scorer, the
in-memory `IsolatedPipelineIndex`, `RecordingSettlementAdapter`, or the
like) just because that flag was left unset.

`TRACE_COMMONS_PIPELINE_ALLOW_TEST_DEPENDENCIES` is the only way past this
refusal. **It is for tests and local development only. Production must
never set it.** Setting it:

- Lets an injected runtime with a non-production-qualified dependency start
  even when tenants are routed or the runtime is required, and logs one
  label-only warning (`pipeline_runtime_test_dependencies_allowed`) at
  startup when it does.
- Never combines with `TRACE_COMMONS_PIPELINE_RUNTIME_REQUIRED`: both set
  refuses startup at once with
  `pipeline_test_dependencies_not_allowed_when_required`, regardless of
  whether the injected dependency is actually qualified.

## Per-phase claim lease

Each pipeline phase claims its run under its own lease length, sized for how
long that phase can actually run rather than one fixed lease every phase
shared. Score, in particular, runs the injected scorer and embedder inside
the lease -- a chunked NEAR AI perplexity scorer or a CPU-bound embedder can
exceed a short lease on the pilot.

- `TRACE_COMMONS_PIPELINE_LEASE_SECONDS_REVIEW` -- whole seconds, default 300
  (5 minutes).
- `TRACE_COMMONS_PIPELINE_LEASE_SECONDS_SCORE` -- whole seconds, default 1800
  (30 minutes).
- `TRACE_COMMONS_PIPELINE_LEASE_SECONDS_SETTLE` -- whole seconds, default 300
  (5 minutes).

Each variable is optional; an unset variable keeps its phase's default.
Each configured value must be between 1 second and 2 hours (7200 seconds)
inclusive. A value that does not parse as a non-negative integer, or that
falls outside that range, refuses ingest startup with the safe label
`pipeline_lease_config_invalid`.

`lease_expired` records exactly one situation: the worker that is still
holding the run's lease comes back and writes again -- the phase's own
commit, or a follow-up retry write -- after its own lease has already
expired (the scorer or embedder ran longer than the configured lease). That
write is recorded with `last_error_label = lease_expired` and returned to
`retry` without charging the attempt the claim took. A phase that always
overruns its lease this way shows `lease_expired` every time and never
reaches `failed`/`attempts_exhausted` on that account alone.

This does **not** cover a worker that crashes mid-phase. A crashed worker
never comes back to write anything, so nothing is recorded for it -- the run
simply stays `leased` until `lease_expires_at` passes on its own, at which
point the next claim (by any worker) reclaims it as an ordinary charged
attempt with `last_error_label` cleared. Repeated crashes still exhaust
`max_attempts` and end in `failed`/`attempts_exhausted`, with no record of
why. The same gap applies whenever more than one worker races the same run:
if a second worker reclaims an expired lease before the first worker's own
stale write runs, that write's lease token no longer matches anything (the
token-only fence in `record_lease_expired`), so it changes nothing -- that
attempt is silently lost, not recorded as `lease_expired` and not otherwise
un-charged.

Lease renewal (extending a lease a phase still holds, mid-phase) is PR 4
work and not implemented yet. It is what will close both gaps above -- a
live worker renewing its lease before it expires, rather than a phase
finding out only after the fact (or a crash never finding out at all).

## Quarantined runs awaiting review

A run Review quarantines with no human assessment yet is parked in the
`awaiting_review` state, with `last_error_label = review_assessment_required`.
No claim query selects that state, so a parked run is not claimed and does
not retry hourly forever, and it is not charged: parking gives the claim's
attempt back the same way a transient retry does. Nothing in this release
moves a parked run back to `pending` -- the human review route that reads
the assessment and unparks the run comes later. A parked run whose
submission is later withdrawn, expired, or purged stays parked; that later
route must itself handle an inoperable submission when it runs.

## Settle failures and settlement legs

A run waits in `retry` without a charge to its attempts when the failure is
not the trace's fault:

- `settlement_cap_missing`: no per-instrument cap is configured for an
  award's instrument. The leg's adapter is not called. Configure the cap;
  the next attempt settles the leg.
- `settlement_adapter_missing`: the service holds no adapter for the
  instrument.
- `database_unavailable` (any phase): the database could not serve a
  statement -- no pool connection, a lost connection, a serialization
  failure, a deadlock, a shutdown, or too many connections. If a leg's
  adapter call returned before the failure, the next attempt calls the
  adapter again with the same operation reference. The adapter must answer
  that call from the first one.

An amount above a configured cap is different: the cap refuses the payment,
the leg fails as `credit_cap_exceeded`, and the attempt is charged.

The adapter's answer decides what happens to a dispatched leg:

- A receipt that answers the request completes the leg. The leg records the
  receipt's result reference and, for an effect with an external record,
  its `external_receipt_hash`. One external receipt answers one leg of a
  tenant.
- `settlement_adapter_unavailable`: the adapter could not complete the
  effect now, or cannot say whether it did. The leg waits in `retry`, the
  run retries without a charge, and the next attempt sends the same
  request.
- `settlement_request_conflict` or `settlement_request_rejected`: the
  adapter says no effect happened and the operation must not be sent
  again. The leg is `failed` with that label.
- `settlement_result_mismatch`: the receipt does not answer the request (a
  different result reference, or an external receipt another leg already
  recorded). The effect is unknown. The leg is `failed` with that label.

A leg `failed` with one of the last three labels is never dispatched again.
While one exists, every Settle attempt is charged, so the other legs still
settle and the run's attempts run out. A request that cannot be formed
(`settlement_request_invalid`) is refused before any adapter call, and the
attempt is charged.

Before each adapter call, the leg moves to `leased` under the run's lease,
and `dispatched_at` records that the leg was sent. When a Settle run fails
(`attempts_exhausted`, or a terminal label such as `bundle_package_invalid`
or `index_key_conflict`), no leg stays open:

- A leg that was never dispatched is `forfeited` with `run_failed`. So is a
  Trace Credit leg that is not `complete`, one `failed` with
  `settlement_result_mismatch` included: it pays only through the ledger
  row that commits with its completion, so it paid nothing.
- A leg `failed` with `settlement_request_conflict` or
  `settlement_request_rejected` is `forfeited` and keeps its label: no
  effect happened. A leg of another instrument `failed` with
  `settlement_result_mismatch` stays `failed` with its label: its effect is
  unknown. Neither gets another adapter call.
- Any other dispatched leg of another instrument gets one more adapter call
  from the worker that fails the run. A result equal to the selected result
  reference makes the leg `complete`. Any other outcome makes it `failed`
  with `settlement_unreconciled`. The worker does not make the call when
  the submission is no longer operable, the adapter or cap is missing, the
  amount is above the cap, or the persisted selection has no result for the
  leg -- because none was ever seeded, or because the persisted selection
  does not decode.
- When the next claim fails a crashed worker's run (`attempts_exhausted`
  after its lease expired), no worker can call the adapter. The first two
  rules still apply; every other dispatched leg of another instrument is
  then `settlement_unreconciled`.

`settlement_unreconciled`, and `settlement_result_mismatch` on a leg of
another instrument, mean the external payment may have happened. Find the
leg's `operation_ref_hash` in the adapter's records and reconcile it by
hand. Nothing retries it once the run has failed. On a run that is still
live, Settle dispatches a `settlement_unreconciled` leg again on its next
attempt -- the label only means the *last* reconciling call did not confirm
a match, not that the leg is done being tried. A leg a withdrawal forfeits
(below) can carry this same label, for the same reason.

A withdrawal (or another way a submission stops being operable -- revoked,
purged, or expired) forfeits every leg that is not `complete` without an
adapter call. A dispatched leg of an external instrument -- `dispatched_at`
is set, and the instrument is not `trace_credit` -- is forfeited as
`settlement_unreconciled` instead of `submission_inoperable`: its adapter
was called at least once, so it may have taken effect, whatever its state or
label. That includes a leg already `settlement_unreconciled` on a live run
and a leg dispatched earlier and later refused by a lowered cap. The leg
keeps its `dispatched_at`; find its `operation_ref_hash` in the adapter's
own records and reconcile it by hand, the same as a `settlement_unreconciled`
leg a failed run leaves behind (above) -- nothing in this release reconciles
it automatically. The exception is a leg whose own label already says no
effect happened (`settlement_request_conflict` or
`settlement_request_rejected`): it stays `submission_inoperable`. An
undispatched leg had no effect, and a Trace Credit leg that is not
`complete` paid nothing -- it pays only through the ledger row that commits
with its completion -- so both of those stay `submission_inoperable` too.

## Receipt staging and the orphan sweep

A pipeline receipt records its envelope object as a `staged` row in
`pipeline_receipt_artifacts` before it writes the object. The transaction that
stores the receipt marks the row `committed`. If the receipt fails after the
write, the row stays `staged`. On each pass, the pipeline worker sweeps every
listed tenant: for up to 32 `staged` rows whose `cleanup_after` (one hour after
staging) has passed, it deletes the object and then the row. If a delete
fails, the row stays for the next pass and the worker logs
`pipeline_receipt_sweep_delete_failed`. A failed receipt stays counted against
the quota. A retry with the same submission id is not counted again.

A tenant removed from `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` stops
being swept the same pass it stops being worked: the sweep runs only over
listed tenants. Its staged objects (and their rows) stay exactly as they
were until the tenant is listed again or an operator removes them by hand.

After a failed receipt attempt leaves a staged row, a later receipt with the
same idempotency key and different content is refused with the
content-conflict 409 until the sweeper removes that staged row -- up to
about one hour after it was staged. A retry with the *same* content is
unaffected: the failed attempt never created a run, so the retry is a new
attempt that creates the run itself, and the caller gets the same 200 it
would have gotten on a first success.

## Submission quota at switch-over

The pipeline counts only pipeline receipts against the hourly submission
quota. It does not count legacy submission records. In the first hour after a
tenant moves to the pipeline, that tenant can therefore receive up to one
extra hourly quota. This is accepted behavior.

The legacy quota and tombstone checks still run before ingest routes a
receipt to the pipeline. A tenant with recent legacy submissions can
therefore receive a legacy 429 for a pipeline receipt.

## Rehearse the switch

Set `TRACE_COMMONS_PG_TEST_DATABASE_URL` to a PostgreSQL test database.
Then run this command:

```bash
cargo test -p trace-commons-server --test versioned_pipeline_pg pipeline_activation
```

The tests cover mixed legacy and pipeline records, exact replay of a
pre-switch receipt, unique ledger sources, tenant expansion gates,
rollback, containment, suspension instead of rebinding, and writer
retirement after pending work completes.

The [local pipeline lab](pipeline-lab.md) `qualify` command runs this suite as part of pipeline qualification. Lab corpus and package evidence remains in local files;
these PostgreSQL integration tests remain separate schema and recovery checks.

## Local operator routes

`trace-commons-pipeline-local` adds these routes:

- `POST /v1/pipeline/switched-submissions`
- `GET /v1/admin/pipeline-routing`
- `POST /v1/admin/pipeline-contain`
- `POST /v1/admin/pipeline-retire-legacy-writer`

The existing corpus paths still submit directly to the pipeline executor.
The switched route is the dual-path receipt used during migration.

## Current completion

None of the redesign is in this repository yet. The versioned pipeline
contracts are defined; the runtime, qualification, and activation work
follows in later changes. `SCR-005` stays deferred until the external
valuation protocol exists. New valuation rules use a later bundle through
the same qualification and activation process.
