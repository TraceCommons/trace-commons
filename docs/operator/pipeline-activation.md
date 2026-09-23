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
