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

## Per-phase claim lease (decision D4)

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

Before each adapter call, the leg moves to `leased` under the run's lease,
and `dispatched_at` records that the leg was sent. When a Settle run fails
(`attempts_exhausted`, or a terminal label such as `bundle_package_invalid`
or `index_key_conflict`), no leg stays open:

- A leg that was never dispatched is `forfeited` with `run_failed`. So is a
  Trace Credit leg: it pays only through the ledger row that commits with
  its completion.
- A dispatched leg of another instrument gets one more adapter call from the
  worker that fails the run. A result equal to the selected result
  reference makes the leg `complete`. Any other outcome makes it `failed`
  with `settlement_unreconciled`. The worker does not make the call when
  the submission is no longer operable, the adapter or cap is missing, or
  the amount is above the cap.
- When the next claim fails a crashed worker's run (`attempts_exhausted`
  after its lease expired), no worker can call the adapter. Every
  dispatched leg of another instrument is then `settlement_unreconciled`.

`settlement_unreconciled` means the external payment may have happened.
Find the leg's `operation_ref_hash` in the adapter's records and reconcile
it by hand. Nothing retries it.

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
