# Pipeline smoke tests

What to run, and what must hold, at each step of moving a deployment from the
legacy scoring path to the versioned pipeline. This page orders the checks;
the routes, labels, and procedures themselves are documented in
[pipeline-activation.md](pipeline-activation.md),
[pipeline-qualification.md](pipeline-qualification.md),
[pipeline-lab.md](pipeline-lab.md), [smoke-test.md](smoke-test.md),
[drills.md](drills.md), and [deployment.md](deployment.md).

There are four stages. Do not start a stage until every check of the one
before it holds. A stage that fails is fixed and run again from its first
check; it is not passed by exception.

| Stage | What changes | Tenants on the pipeline |
|---|---|---|
| 1. Deploy with the pipeline off | New binary and migrations | none |
| 2. Offline replay | Nothing on the deployment | none |
| 3. One throwaway tenant | One routing row | one, created for the test |
| 4. First real tenant | One routing row | one |

## What is not possible yet

Read "Current completion" in [pipeline-activation.md](pipeline-activation.md)
first. In short:

- The repository binary injects no pipeline runtime. With it, stages 3 and 4
  cannot run: no tenant can be served on the pipeline, and no activation can
  pass the gate, because the three promotion-only checks
  (`pipeline_production_adapters`, `pipeline_remote_restore`,
  `pipeline_hf_network_canary`) need the production assembly.
- There is no shadow or dual-run mode. Each receipt has exactly one owner. A
  tenant on the pipeline is not scored by the legacy path, and its
  pipeline-owned receipts never move back to it. The only side-by-side
  comparison is the offline replay of stage 2.

Stages 1 and 2 can run today. Stages 3 and 4 are written down now so that the
production assembly is built to pass them.

## Stage 1: deploy with the pipeline off

Goal: the new build behaves as the old one did for every tenant, and the
schema is ready for a later activation.

### 1a. Rehearse the migrations on a restored copy

Before touching the deployment, restore its most recent backup to a scratch
instance on the **same PostgreSQL major version** as production. A rehearsal
on another major version does not count: catalog behaviour differs between
versions, and a migration that passed locally has failed on the production
version before.

On the scratch instance, as the migrator role (never the runtime login; see
"First: does this build carry a migration the database does not have?" in
[deployment.md](deployment.md)):

1. Apply every migration the deployment does not have yet, in one boot or with
   `psql --single-transaction` per file.
2. Record the wall-clock time of each. Migrations that rewrite or scan large
   tables under an exclusive lock are the ones that decide the maintenance
   window: the ones that add constraints or unique indexes to the credit
   ledger, the settlement batches, and the NEAR outbox, and the ones that
   backfill data or add triggers on account and gate-decision tables.
3. Confirm the migrator could create and grant every role the migrations
   need. A refusal here is a refusal at deploy time.
4. Run the `has_table_privilege` checks listed in
   [deployment.md](deployment.md) for the runtime login, including the
   pipeline routing and ownership tables. The new build reads the routing row
   on every upload, so a missing grant fails every upload, not only pipeline
   ones.
5. Run the [db-reconciliation drill](drills.md) on the scratch copy and keep
   the result as the baseline for 1c.

Pass: every migration applied, the timings fit the window, every privilege
check is true.

### 1b. Deploy

Stop ingest, apply the migrations as the migrator, install the binaries, start
ingest. Leave `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` and
`TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` unset on every replica. Before
starting, read [deployment.md](deployment.md) for any configuration the new
build reads for the first time, and pin every value whose change is
user-visible (for example a WebAuthn relying-party id: changing it invalidates
registered passkeys).

### 1c. Smoke the legacy path

1. Run [smoke-test.md](smoke-test.md) in full (`scripts/operator/smoke-gate.sh`).
2. `GET /v1/admin/pipeline/routing` for each live tenant: `routing_state` is
   null.
3. A client upload end to end on a throwaway invite and tenant, with an
   isolated client directory: the upload is accepted, scored by the legacy
   gate, and reaches the same status it would have reached on the previous
   build. Clean up the invite and the tenant afterwards.
4. The required drills, compared with the baseline from 1a and with the last
   recorded run: `audit-chain`, `postgres-rls`, `db-reconciliation`,
   `rollback`. A drill that was not ready before the deploy may still be not
   ready; a gap count that grew is a failure.
5. Logs: zero `ERROR` lines in the application log for the first hour, and no
   upload answered `500`.

Pass: all five hold. Stage 1 is also the rollback point: keep the previous
binaries and the pre-migration backup until stage 4 is done.

## Stage 2: offline replay

Goal: the pipeline reaches the same decisions as the legacy gate on traces
like the deployment's, before any tenant depends on it. This runs on a
workstation or a scratch host against a throwaway PostgreSQL; nothing is sent
to the deployment.

1. Build a corpus that covers the cases that matter, not only clean traces:
   short single-turn sessions, chunk-capped long sessions, two sessions that
   share their opening events (the duplicate case), a session with residual
   PII the classifier must catch, a session outside the tenant's consent scope,
   and a session with a tool the redactor does not know.
2. Run it through `pipeline.py run --bundle compatibility --corpus <file>`
   ([pipeline-lab.md](pipeline-lab.md)) with the gate floors, top-k, chunk
   settings, and credit delta of the deployment.
3. Score the same traces through the legacy gate with the same settings.
4. Compare, per trace: admitted or refused, the refusal label, the credit
   quality, and the pending credit.

Pass:

- Every trace that the legacy gate refuses for privacy or consent is refused
  by the pipeline. This is the one comparison with no tolerance: a pipeline
  gate that is weaker than the legacy gate is a release blocker.
- Credit quality agrees within the scorer's own run-to-run variation, and
  every disagreement in admit/refuse is explained in writing.

Note the known difference: pipeline submissions write no legacy gate-decision
row, so features that read that table (duplicate clustering, the contributor
cap, per-author scoring, account trust) do not see them. Decide whether that is
acceptable before stage 3, not after.

## Stage 3: one throwaway tenant

Preconditions:

- A build with the production assembly, which passes startup without
  `pipeline_runtime_dependencies_not_production_qualified`,
  `pipeline_runtime_main_gate_config_mismatch`, `compatibility_zero_floor`, or
  `pipeline_privacy_filter_required`.
- A full set of 22 signed check results for that build's revision
  ([pipeline-qualification.md](pipeline-qualification.md)).
- The tenant is created for the test, holds no real contributor's work, and
  is in `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` on **every** replica.
- NEAR settlement mode is `disabled` (see [settlement-mode.md](settlement-mode.md)).
- Vector indexing is running for the tenant: the pipeline's Score phase needs
  a live embedder and index.

Activate the tenant ("Activate, roll back, contain, deactivate" in
[pipeline-activation.md](pipeline-activation.md)), then run these checks.

### 3a. The happy path

Upload a small set of traces from stage 2's corpus. Each reaches a terminal
state, the decisions match stage 2's for the same traces, and
`GET /v1/admin/pipeline/operational-summary` shows no stuck work. The client
shows the same pending credit the pipeline recorded.

### 3b. The gates refuse what they must

Upload the PII, out-of-scope-consent, and duplicate traces. Each is refused or
quarantined with the same label as in stage 2, and none earns credit.

### 3c. Failure drills

| Drill | How | Must hold |
|---|---|---|
| Object store fault | Point the tenant's store at a missing object, or remove one object a run needs | The run fails as an integrity failure on spaced, charged attempts and ends after its per-phase budget; it does not end within seconds; a transport error is not charged |
| Worker lost mid-Score | Stop the worker while a run is in Score | The lease expires; a commit from the old holder is refused as stale; another worker completes the run |
| Restart with parked runs | Restart ingest while runs are parked | Parked runs are released in bounded batches and complete |
| Revocation and withdrawal | Revoke one submission and withdraw another at each phase (before Score, after Score, after Settle) | Each follow-up is applied, export snapshot items are invalidated, the tombstone carries the caller's reason, and a lost follow-up is recovered by the worker |
| Containment | `POST /v1/admin/pipeline/contain` | New uploads are `503` and write nothing; work already received completes; uploads are accepted again after re-opening (this is the test `containment_refuses_new_receipts_and_keeps_pending_work` against the deployment) |
| Deactivation | `POST /v1/admin/pipeline/deactivate`, then move the tenant to `TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` | New uploads take the legacy path; in-flight runs and settlement legs complete on the drain list; `GET /v1/admin/pipeline/legacy-drain` reaches zero |

The store-fault, lease, revocation, and parked-run behaviour described above is
what the open pipeline follow-ups specify; run these drills on a build that
includes them.

### 3d. Settlement, last

Only after 3a to 3c hold: switch NEAR settlement to dry-run, then to the live
mode, for this tenant only. A dry-run confirmation never replaces a real
transaction hash, and a line submitted in one mode is confirmed only in that
mode.

### 3e. Reconcile

Run `db-reconciliation` and `rollback` for the throwaway tenant. Every gap is
either zero or a known, documented pipeline-row difference.

Pass: every check in 3a to 3e holds. Then contain, deactivate, drain, and
remove the throwaway tenant.

## Stage 4: first real tenant

Agree on the abort rule before activating, and write it in the change record:
**any credit mismatch with stage 2's expectations, any trace that passes a gate
it must fail, or any stuck run older than its phase budget means
`contain` at once, then `deactivate`.**

Then, for the first week:

- Daily: `GET /v1/admin/pipeline/operational-summary`, run counts by state,
  settled credit compared with the recorded pending credit, the audit chain
  drill, and the application log for the pipeline's log labels.
- After each deploy: `GET /v1/admin/pipeline/routing` shows
  `active_bundle_qualified_on_revision: true`. Every new build revision needs
  a new qualification, including a build that changes only documentation;
  without one the tenant's uploads are refused with `pipeline_bundle_not_qualified`.
- Never install a build older than the routing rule while any tenant's row
  says `pipeline` or `contained`: it ignores the row. Deactivate first
  ("Binary rollback to an older build" in [deployment.md](deployment.md)).
