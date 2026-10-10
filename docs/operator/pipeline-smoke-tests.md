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

## Limits

Read "Current completion" in [pipeline-activation.md](pipeline-activation.md)
first. In short:

- Only a `near-ai-scorer` build started with
  `TRACE_COMMONS_PIPELINE_RUNTIME=production` injects a pipeline runtime ("The
  production assembly" in [pipeline-activation.md](pipeline-activation.md)).
  Any other build serves no tenant on the pipeline, and no activation can pass
  the gate on it, because the three promotion-only checks
  (`pipeline_production_adapters`, `pipeline_remote_restore`,
  `pipeline_hf_network_canary`) need the production assembly. Stages 3 and 4
  run on that build only. Stage 3 first ran on the pilot on 2026-10-09/10;
  what it taught is folded into the stage 3 checks below.
- There is no shadow or dual-run mode. Each receipt has exactly one owner. A
  tenant on the pipeline is not scored by the legacy path, and its
  pipeline-owned receipts never move back to it. No tool compares the two
  paths on the same traces; stage 3 compares the pipeline's decisions with
  what the legacy gate recorded for the same synthetic traces on another
  tenant.

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
- `TRACE_COMMONS_PIPELINE_CREDIT_ISSUER_PRINCIPAL_REF` is set on every replica,
  to a canonical hashed principal ref (`principal_sha256:` and 64 hexadecimal
  characters). Once a tenant is on the receipts list, a runtime that serves
  the compatibility bundle without it refuses to start
  (`pipeline_credit_issuer_principal_missing`); any other form of the value
  is refused too.
- NEAR settlement mode is `disabled` (see [settlement-mode.md](settlement-mode.md)).
- The production assembly's embedder and index are configured: the
  pipeline's Score phase needs a live embedder and index, and it takes both
  from the assembly's own environment. It does not use `main`'s vector index
  worker. The `vector-index` drill's `scheduler_enabled` reports that worker
  (`TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_ENABLED`), so do not require it to be
  true for the pipeline.

### Before activation: the boot and the qualification

These caught the first stage 3 run on the pilot (2026-10-09/10).

- **The adapters check variables are for one boot.** The deployed host emits
  `pipeline_production_adapters` at startup when
  `TRACE_COMMONS_PIPELINE_CHECK_RESULT_DIR`, `TRACE_COMMONS_PIPELINE_CHECK_RUN_ID`
  and `TRACE_COMMONS_PIPELINE_CHECK_CODE_REVISION_HASH` are set
  ("`pipeline_production_adapters`" in
  [pipeline-activation.md](pipeline-activation.md), #1295). Remove all
  three from the env file once that boot has written its result. Left in
  place, a restart of the same revision only logs
  `pipeline_production_adapters_already_emitted` and continues, but the first
  boot of a new revision is refused (`pipeline_check_revision_mismatch`),
  because the variables still name the old one.
- **The result directory must be writable by the service.** A unit with
  `ProtectSystem=strict` and no `ReadWritePaths=` for that directory refuses
  the start. Builds since #1315 name this `pipeline_check_write_failed`;
  earlier builds reported it as `pipeline_check_already_emitted`, which is
  what the pilot showed on 2026-10-09, with no result file anywhere.
- **Every new revision needs a new qualification for every routed tenant.**
  After any deploy, `GET /v1/admin/pipeline/routing` for each tenant whose row
  says `pipeline` must show `active_bundle_qualified_on_revision: true`. Until
  the bundle is qualified on the running revision, that tenant's new uploads
  are `503 pipeline_bundle_not_qualified`, and ingest logs
  `pipeline_active_bundle_not_qualified` at start, once for each such tenant.
  This holds for the throwaway tenants of this stage as much as for a real one.
- **`qualify` against a shared server leaves roles behind.** A
  `pipeline.py qualify --postgres-admin-url ...` run leaves three cluster-wide
  owner-probe roles from `migration_atomicity_pg` on that PostgreSQL server,
  and the next `qualify` on it fails at `postgres_migration_atomicity`. Until
  #1335 is fixed, drop them by hand before qualifying again (`DROP OWNED BY`
  the role in each database, then `DROP ROLE`), or qualify against a
  disposable container.

Before activating, build a synthetic trace set that covers the cases that
matter, not only clean traces: short single-turn sessions, chunk-capped long
sessions, two sessions that share their opening events (the duplicate case), a
session with residual PII the classifier must catch, a session outside the
tenant's consent scope, and a session with a tool the redactor does not know.
No real contributor's trace goes in it.

Upload the whole set to a **second** throwaway tenant that stays on the legacy
path, and record per trace, once it is terminal (below): admitted or refused,
the refusal label, the credit quality, and the pending credit. That is the
reference the checks below compare with, scored by the deployment's own
legacy gate and settings.

Then activate the pipeline tenant ("Activate, roll back, contain, deactivate"
in [pipeline-activation.md](pipeline-activation.md)), and run these checks.

### When a submission is terminal

Do not record a submission the moment it reads `accepted`. A pipeline
submission reads `accepted` as soon as Admission admits it, and its credit
figure arrives later, when the run has been scored. Until then its status
shows 0.00 pending and the line "Scoring in progress; credit is assigned when
the gate's evaluation completes." A comparison that records at that point
records a zero that is not the pipeline's answer.

A submission is terminal when its status is not `accepted` (refused,
quarantined, rejected), or when it is `accepted` and carries a credit basis
line: either "Credit reflects the gate's scoring ..." (the scored figure), or
"This trace duplicates an earlier submission under your account and earns no
separate credit." Poll until every submission of the set is terminal before
comparing. The same rule applies to the legacy reference tenant.

### Re-running the comparison

Submission ids are derived from content (the contributor derives each from
the session hash), so uploading the same set again to the same pipeline
tenant replays the earlier receipts and their results; it does not run the
new code. To re-run after a code change, create a fresh pipeline tenant (a
new run id), add it to `TRACE_COMMONS_PIPELINE_RECEIPTS_TENANT_IDS` on every
replica, qualify the bundle for it on the running revision, and activate it.

### 3a. The happy path

Upload the clean traces of the set. Each reaches a terminal state and
`GET /v1/admin/pipeline/operational-summary` shows no stuck work. Compared
with the legacy tenant's record: the same admit/refuse decision, credit quality
within the scorer's own run-to-run variation, and every disagreement explained
in writing. The client shows the same pending credit the pipeline recorded.

### 3b. The gates refuse what they must

Upload the PII and duplicate traces. Each is refused, quarantined, or (the
duplicate) withheld with the same label or line the legacy tenant recorded,
and none earns credit. This comparison has no tolerance: a trace the legacy
gate refuses for privacy or consent and the pipeline admits is a release
blocker.

Upload the out-of-scope-consent and unknown-tool traces too, and compare them
with the legacy record the same way. Do not expect them to earn nothing: on
the pilot (2026-10-09/10) both were admitted and credited 3.00 on **both**
paths. The earlier expectation of "no credit" for them was wrong, not the
pipeline. What must hold is agreement with the legacy path; a difference in
either direction is a finding.

### 3c. Failure drills

| Drill | How | Must hold |
|---|---|---|
| Object store fault | Point the tenant's store at a missing object, or remove one object a run needs | The run fails as an integrity failure on spaced, charged attempts and ends after its per-phase budget; it does not end within seconds; a transport error is not charged |
| Worker lost mid-Score | Freeze the worker while a run is in Score (`SIGSTOP` the ingest process past the run's `lease_expires_at`, then `SIGCONT`); lower `TRACE_COMMONS_PIPELINE_LEASE_SECONDS_SCORE` first so the freeze is minutes, keeping four times the lease above a real Score | The lease expires; the old holder's commit is refused as stale (`lease_expired`, attempt not charged); the run is reclaimed and completes. On a single host the same process reclaims it. A killed worker (`SIGKILL`) writes nothing, so its run stays `leased` until the lease expires and is then reclaimed as a charged attempt |
| Restart with held and parked runs | Restart ingest while runs are held by a suspended phase (`POST /v1/admin/pipeline/policy-interventions`), one is leased in Score, and any are parked (`awaiting_review`) | Held runs keep their state across the restart and complete once the phase is resumed; the leased run is reclaimed after its lease and completes. A parked run whose submission is still operable stays parked until a reviewer assesses it -- a restart does not release it. Parked runs whose submission is no longer operable are released in batches of 32 and end `failed` with `submission_inoperable` |
| Revocation and withdrawal | Revoke one submission and withdraw another at each phase (before Score, after Score, after Settle) | Each follow-up is applied, export snapshot items are invalidated, the tombstone carries the caller's reason, and a lost follow-up is recovered by the worker |
| Containment | Upload a large in-flight probe, then `POST /v1/admin/pipeline/contain` while its run is still in progress (see below) | New uploads are `503` and write nothing; work already received completes; uploads are accepted again after re-opening (this is the test `containment_refuses_new_receipts_and_keeps_pending_work` against the deployment) |
| Deactivation | `POST /v1/admin/pipeline/deactivate`, then move the tenant to `TRACE_COMMONS_PIPELINE_DRAIN_TENANT_IDS` | New uploads take the legacy path; in-flight runs and settlement legs complete on the drain list; `GET /v1/admin/pipeline/legacy-drain` reaches zero, except as noted below |

"Work in flight at the contain" is judged by the rule in "When a submission
is terminal": an `accepted` submission without a credit basis line is still
in flight, and it must reach a terminal status while the tenant is contained.
Since #1324 moved the classifier rescrub to a privacy pass at the start of
Review, a small trace clears the whole pipeline in seconds, so a small probe
is usually finished before the contain lands and proves nothing. Use the
long, chunk-capped body of the synthetic set (about 356 KB) as the in-flight
probe, and confirm it is not yet terminal when the contain returns. On the
pilot it completed while contained in about five minutes.

Resuming a suspended phase does not wake the runs it held: each waits for its
`next_attempt_at`, which is the phase's age clamped to between 1 second and
1 hour.

On the pilot (2026-10-10, build f6f76f0c), no client-side trace could be made
to park (`awaiting_review`). Admission's Medium quarantine is unreachable
from a client (#1344), and the privacy pass only escalates at High. The
parked half of this drill is covered in CI by
`releasing_parked_runs_is_bounded_per_call` and
`parked_run_with_an_inoperable_submission_is_released`.

On a deployment with `main`'s vector index worker off
(`TRACE_COMMONS_VECTOR_INDEX_SCHEDULER_ENABLED` unset), the legacy drain report
keeps `vector_index_pending` above zero for the tenant's legacy uploads (the
reference tenant's, and any taken after deactivation). That is legacy work
for a worker that is not running, not pipeline work; the drain is judged on
the other counts.

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

A run held for review does not drain by itself: a run Admission quarantined,
or one the Review-start privacy pass escalated (its submission reads
`quarantined` since #1332), stays held until a reviewer assesses it, and the
tenant does not drain cleanly until then. Reject each synthetic probe that is
held, as a reviewer for the tenant:

1. `GET /v1/review/pipeline/quarantine` lists the held runs and what holds
   each (`hold_reason`).
2. `POST /v1/review/pipeline/runs/{run_id}/claim`, which returns the lease
   token.
3. `POST /v1/review/pipeline/runs/{run_id}/assessment` with that
   `lease_token`, `"recommendation": "reject"`, and a `reason` that is a
   reason code: 1 to 64 characters of lowercase letters, digits and `_` (for
   example `synthetic_probe_teardown`). Free text answers `422 invalid reason
   code`; a recommendation other than `approve` or `reject` answers
   `422 invalid review recommendation`.

"Quarantined runs and human review" in [pipeline-activation.md](pipeline-activation.md)
describes the routes in full.

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
