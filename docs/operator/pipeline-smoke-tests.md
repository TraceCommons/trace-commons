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
| 2. Lab corpus runs | Nothing on the deployment | none |
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
  pipeline-owned receipts never move back to it. No tool compares the two
  paths on the same traces; stage 3 compares the pipeline's decisions with
  what the legacy gate recorded for the same synthetic traces on another
  tenant.

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

## Stage 2: lab corpus runs

Goal: the pipeline build behaves as specified on the repository's corpora,
before any tenant depends on it. This runs on a workstation in a disposable
PostgreSQL container ([pipeline-lab.md](pipeline-lab.md)); nothing is sent to
the deployment, and no deployment trace is used.

Run both from a checkout of the commit the deployment runs:

```bash
python3 scripts/operator/pipeline.py run --bundle compatibility
python3 scripts/operator/pipeline.py run --bundle compatibility \
  --corpus crates/trace-commons-server/tests/fixtures/pipeline-hf-jsonl/pin-local.json
```

Pass, from the reports under `.local/`: every fixture completed,
`failure_count` 0, every fixture's `mismatches` empty, `replay_same_run_count`
and `changed_content_refused_count` equal to the fixture count, and
`tenant_isolation` true. The built-in corpus covers an admitted plan, a locally
redacted secret, an approved and a rejected privacy quarantine, and a
high-risk rejection; the harness also refuses any report that carries a
fixture's secret probe.

What this does not show:

- **Agreement with the legacy gate on the deployment's settings.** The lab
  scores with the reference scorer, embedder and index (the report's
  `safe_blockers` say so), and takes no gate floors, top-k, chunk settings or
  credit delta. Credit quality and pending credit from these runs say nothing
  about the deployment's. That comparison moves to stage 3 (3a), where the
  production assembly scores with the deployment's own adapters.
- **Independent privacy enforcement.** In the compatibility bundle, Admission
  and Review are pass-through: privacy and consent refusals come from the
  ingest admission both paths share. A pipeline gate weaker than the legacy one
  would need a separate implementation to exist; stage 3b checks the
  refusals on the deployment.
- **Consent-scope, unknown-tool, and shared-opening (duplicate) cases.** Corpus
  expectations are keyed by fixture label in the harness's Rust, so these need
  new fixtures and expectation code, not only a corpus file.

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

Before activating, build a synthetic trace set that covers the cases that
matter, not only clean traces: short single-turn sessions, chunk-capped long
sessions, two sessions that share their opening events (the duplicate case), a
session with residual PII the classifier must catch, a session outside the
tenant's consent scope, and a session with a tool the redactor does not know.
No real contributor's trace goes in it.

Upload the whole set to a **second** throwaway tenant that stays on the legacy
path, and record per trace: admitted or refused, the refusal label, the
credit quality, and the pending credit. That is the reference the checks below
compare with, scored by the deployment's own legacy gate and settings.

Then activate the pipeline tenant ("Activate, roll back, contain, deactivate"
in [pipeline-activation.md](pipeline-activation.md)), and run these checks.

### 3a. The happy path

Upload the clean traces of the set. Each reaches a terminal state and
`GET /v1/admin/pipeline/operational-summary` shows no stuck work. Compared
with the legacy tenant's record: the same admit/refuse decision, credit quality
within the scorer's own run-to-run variation, and every disagreement explained
in writing. The client shows the same pending credit the pipeline recorded.

### 3b. The gates refuse what they must

Upload the PII, out-of-scope-consent, unknown-tool and duplicate traces. Each
is refused or quarantined with the same label the legacy tenant recorded, and
none earns credit. This comparison has no tolerance: a trace the legacy gate
refuses for privacy or consent and the pipeline admits is a release blocker.

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
**any credit mismatch with the legacy reference from stage 3, any trace that passes a gate
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
