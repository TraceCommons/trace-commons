# A pipeline bundle that pays trace credit on the pilot

Status: approved 2026-11-03. The owner accepted all ten recommendations in
section 9 and asked for deployment in the three stages of section 7.2.
Stage 1 (the code, with the rule off) is being implemented.
Date: 2026-11-03. Code read at `main` 96ba35043. The pilot runs 0eea14e2c.

## 1. Summary

The pilot's NEAR AI payout rail can now pay, but uploads earn nothing it can
pay. The question was whether to raise the flat award in the signed
compatibility bundle, or to let the scoring pipeline set the award from the
scores.

**Raising the flat award alone pays nothing.** A compatibility run's credit
is written as a ledger row that nothing settles. The pipeline never puts it
in a settlement batch (Ruling S10), `main`'s settlement skips it on purpose,
and the NEAR AI rail pays only from finalized batches. A delta above 0 would
put points in contributors' ledgers that are never paid. Section 3 shows the
code.

So every way to pay needs code. A leg can be paid only if it is both
**batched** and still passes `main`'s credit checks (the `model_training`
consent check, the issuer check, the production-gate check). Today no leg
type does both: the `NoveltyUtility` leg is checked but never batched, and
the `PipelineScore` leg is batched but never checked.

**Recommendation: option C.** Add a graded credit rule to the compatibility
policy family:

- Score sets the award from the trace's own perplexity and anomaly terms,
  capped by the configured delta, and records each factor in
  `award_adjustments`.
- Settle batches that leg as a `NoveltyUtility` event, so `main`'s checks
  still run and the NEAR AI rail pays it.

The rule is pinned in the signed package and built from the env file, like
the delta. The production assembly, `pipeline.py package`, the promotion
tooling and default routing all keep working without a new binary. Section 4
lays out options A', B and C.

## 2. Facts this rests on

- **Rates and caps.** The pilot's settlement table pays $0.25 per credit
  point. The rail's lifetime ceiling is $20 per account, which is 80 points.
  The settlement policy version is `pilot-2026-11`, and `main`'s scheduler cap
  is 80 points per account.
- **Who can be paid.** 7 accounts are open: 1 NEAR AI account and 6 legacy
  accounts. The legacy accounts are paid through a carried payout link once
  their holders link.
- **Routing.** Every tenant on the pilot is routed to the versioned pipeline
  by default routing: about 20 default-routed tenants plus one activated by
  hand.
  - Their active bundle is the signed compatibility package
    `sha256:973e6171...`, signed with key id `pilot-package-20261009`.
  - Its Score rule is `compatibility_quality_novelty_v1` and its Settle rule
    is `compatibility_membership_from_score_v1`. Its delta,
    `novelty_utility_microcredits`, is 0.
- **The 2026-11-03 dry run.** The dry-run settlement and the risk summary
  reported 0 eligible source events and 0 pending credit.
- **Gate pass rates since the floors were recalibrated.**
  - The perplexity floor has been 1.5 since 2026-09-19. It refuses almost no
    real trace: production p05 is 2.17, p50 5.14 and p95 23.99 under
    Qwen3.8-27B.
  - The novelty floor has been 0.10 since 2026-09-21. It refused about 1% of
    singleton traces in the 2026-09-22 measurement. On the first live batch
    under both floors, 8 of 11 traces passed.
  - A near-duplicate scored after its sibling earns nothing: the per-tenant
    Score lock and `main`'s duplicate short-circuits see to that.
- **The shadow credit quality q** (`credit_quality.rs`, V3 era) is
  `f * g * a`:
  - `f` is the perplexity term: floor 1.5, ceiling 13.73, graded floor 0.25.
  - `g` is the novelty term. Its floor is still 0.5, above almost every live
    novelty value, so `g` sits at its graded floor of 0.30 for nearly every
    trace.
  - `a` is the anomaly term: soft limit 40, hard limit 310.
  - So q runs about 0.08-0.30 today, and its spread comes almost entirely
    from `f` and `a`.
- **Volume.** Upload volume per account per day has not been measured on the
  current build. Section 6 gives figures for several rates instead of one
  guess.

## 3. Why a flat delta on today's bundle pays nothing

Five links, all on `main` 96ba35043:

1. **Score awards the flat delta when both floors pass.**
   `versioned_pipeline_compat.rs:495-507`. Ruling P3-D6 says the award is the
   configured delta and never q.
2. **Settle writes that leg as `final` and never batches it.**
   - `versioned_pipeline.rs:15781-15814`: the ledger row's `external_ref` is
     the pipeline's ledger source key, and its state is `final`.
   - `versioned_pipeline.rs:16008-16014` (Ruling S10): a `NoveltyUtility` leg
     gets `batch_id = None`. Only a `PipelineScore` leg reaches
     `finalize_pending_credit_batch`.
3. **`main`'s settlement skips it on purpose.**
   - `trace-commons-ingest.rs:42416-42445`: a `NoveltyUtility` event counts
     only with the legacy gate's `novelty_utility:` `external_ref` prefix, at
     or after `SETTLE_FROM`.
   - `db/postgres.rs:2301-2304`: the all-tenants enumerator applies the same
     rule.
   - The comment there gives the reason: "`main` paying it would pay what the
     pipeline chose not to."
4. **The NEAR AI rail accrues only from finalized settlement batches.**
   `payout_accrual.rs:1-20` and `:264-276`. The filter is the batch status
   alone, with no policy-version or instrument filter, so a batch the pipeline
   writes would be paid like any other.
5. **The operator runbook says the same.** `pipeline-activation.md:3026-3031`
   and `:3277-3279`: the compatibility leg is "never paid" and reports
   `not_settlement_eligible`.

So a delta above 0 on today's bundle would add ledger points to each
contributor's status (`credit_points_ledger`) that no batch, no settlement
run and no rail ever pays. That is worse than 0. The 2026-11-03 dry run
showing 0 eligible events is consistent with this, and would still show 0
with any delta.

Two more constraints shape every option:

- **Checks run only on `NoveltyUtility` legs.** At `versioned_pipeline.rs:15926-15931`,
  Settle applies `main`'s credit checks (`novelty_utility_withheld_reason`)
  only to a `NoveltyUtility` leg. One of those checks withholds credit
  unless the submission's allowed uses include `model_training`
  (`pipeline-activation.md:3379`). A `PipelineScore` leg skips them, so
  moving to that event type would pay traces whose contributor never allowed
  model training.
- **The default package is always the compatibility package built from
  env.** `production_pipeline_builder` (`versioned_pipeline_production.rs:735-790`)
  builds it from `main`'s gate configuration. The open `trace-commons-ingest`
  binary registers no Score policies, and a test enforces that
  (`crates/trace-commons-ingest/tests/ingest_library_entry.rs:212-213`).

## 4. Options

### Option A': the flat award, batched

The smallest change that pays. The compatibility Score policy is unchanged,
apart from a non-zero delta. Settle stops treating the compatibility leg as
unsettled.

- **Code.**
  - Add a `settle_into_batch` term to the compatibility configuration. It is
    serialized only when true, so today's package bytes and the hash
    `973e6171...` do not change.
  - When it is set, Settle writes the `NoveltyUtility` row `pending`, calls
    `finalize_pending_credit_batch` with event type `novelty_utility`
    (`versioned_pipeline.rs:16008-16040`), and applies credit holds as it
    does for `PipelineScore`.
  - `main`'s settlement exclusion (`is_gate_scoring_award`) stays as it is,
    so nothing is paid twice.
  - The term joins `MainGateConfig`, `matches_main_gate`
    (`versioned_pipeline_compat.rs:237-248`), the env parser and the package
    allowlist (`scripts/operator/pipeline_tooling/envfile.py:31-49`).
- **Rulings it changes.** S10, for bundles that set the term.
- **What it does not do.** The scores do not set the award. A pass is a pass.

### Option B: a registered Score policy

What the registry seam was built for: an embedding binary registers a
`ScorePolicyFactory` (`versioned_pipeline_score_registry.rs:111-140`). Its
policy prices each trace from the scores and records `award_adjustments`
(`trace-commons-gate-api/src/pipeline.rs:1772-1835`, at most 32 factors, each
from 0 to 1,000,000).

On the pilot it is blocked several ways:

- **A second binary.** The open binary must not register policies
  (`ingest_library_entry.rs:212-213`), so the pilot would run a second
  binary built on `trace_commons_ingest::run_with_score_policies`.
- **Not the default package.** The default package stays the
  env-built compatibility package (`versioned_pipeline_production.rs:735-790`).
  Default routing would keep putting every new tenant on the unpaid
  compatibility bundle, and each tenant would have to be moved by hand and
  then requalified by hand after every deploy.
- **The promotion tooling cannot build or check it.**
  - `pipeline.py package --bundle production` builds only the compatibility
    package.
  - The `promote package-checks` harness runs in the ingest library's tests
    with an empty registry. The qualification validator refuses a package
    whose Score policy has no registered factory
    (`bundle_implementation_unknown`, `pipeline-qualification.md` "Package
    trust").
- **The batching problem is unsolved.**
  - If the factory declares `NoveltyUtility`, its leg is still never batched
    (S10 keys on the event type), so it still pays nothing.
  - If it declares `PipelineScore`, it is batched but skips the consent,
    issuer and production-gate checks (section 3).
  - Either way it needs the option A' Settle change as well.
- **Use.** The seam is the place for a proprietary scorer. It is the wrong
  vehicle for the pilot's first payout.

### Option C (recommended): a graded credit rule in the compatibility family

What "the scoring pipeline determines this" asks for, without a new binary.

**Configuration.**
- Add `credit_rule` to `CompatibilityBundleConfig`
  (`versioned_pipeline_compat.rs:122-147`). It is an enum: `flat`, today's
  rule, or `graded_perplexity_v1`.
- It is serialized only when it is not `flat`, so today's package bytes and
  hash do not change.
- `graded_perplexity_v1` implies the option A' batching. A graded award that
  is never paid would be pointless.
- It is set from a new env variable,
  `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_RULE` (`flat` by default). The
  variable joins `MainGateConfig`, `matches_main_gate`, the env parser and
  `PACKAGE_VARIABLES`, exactly as the delta does. So `pipeline.py package
  --bundle production --env-file` builds the graded package with no tooling
  change.

**Score** (`versioned_pipeline_compat.rs:386-566`). Under `graded_perplexity_v1`:

- Floors, membership, the index command and the shadow q are unchanged.
- When both floors pass, the award is

  `award = delta * f * a`

  computed on integer micros:
  `floor(delta_micro * f_micros / 1e6 * a_micros / 1e6)`. `delta_micro` is
  `novelty_utility_microcredits`, which becomes the per-trace ceiling.
  - `f` is the V3 perplexity term on the trace's representative perplexity:
    graded floor 0.25 at the 1.5 floor, rising on a log curve to 1.0 at
    13.73.
  - `a` is the anomaly term on peak / representative: 1.0 up to 40, then
    falling linearly to 0 at 310.
  - Both come from `credit_quality.rs` (`saturating_term`, `graded_floor`
    and the anomaly branch), under the calibration era in force at Score
    (`constants_at`).
  - The novelty term `g` is left out (see section 5).
- `award_adjustments` records, in order:
  - `perplexity_term`: factor `f`, with the representative perplexity in
    micros as the basis (clamped to `u32`);
  - `anomaly_term`: factor `a`, with the ratio in micros as the basis
    (clamped).

  An auditor recomputes the award from the delta in the signed package.
- The evaluation `rule_id` is `compatibility_graded_perplexity_v1`, so the
  ledger reason is `novelty_utility:compatibility_graded_perplexity_v1`.
- An award that rounds to 0 is no award.

**Settle.** As in option A':

- The leg is written `pending` and batched.
- `main`'s credit checks run first, unchanged. A withheld leg writes no row.
- Credit holds apply to the batched leg.
- `main`'s settlement exclusion stays, so the legacy scheduler never touches
  these rows.
- The Settle cap stays at the delta (`versioned_pipeline_production.rs:683-697`),
  which is also the graded ceiling, so the cap code does not change.

**Contributor status.** A graded leg settles into a finalized batch in the
same Settle transaction, so its points read as settled credit awaiting the rail, instead of 0 and `not_settlement_eligible` (the
rules at `pipeline-activation.md:3277-3279` and `:3431-3433`). Any wording change goes
through `docs/ui-copy-guide.md`; no new notice.

**Rulings it changes.** P3-D6 ("the award is the configured delta, never q")
and S10, both only for bundles under the new rule. Flat bundles keep both
rulings as they are.

**Tests (TDD, in the implementation plan).**
- A graded award equals `delta * f * a` at the floor, at p50 and at the
  ceiling, and the adjustments recompute it.
- An anomaly at or above 310 gives no award.
- A flat package's bytes and hash are unchanged.
- A graded leg is batched and paid by the NEAR AI accrual planner.
- A graded leg whose submission lacks `model_training` is withheld and
  batched nowhere.
- `main`'s settlement never selects a graded pipeline row.
- `matches_main_gate` refuses a rule mismatch.
- The PostgreSQL runtime suite covers a held account and a duplicate.
- `f` and `a` feed a signed, audited award now, not only shadow evidence.
  So the factors are either computed in fixed point or pinned by test
  vectors, and the qualification harness and production agree to the
  microcredit. The implementation plan decides which.

### Comparison

| | A' flat, batched | B registered | C graded, batched |
|---|---|---|---|
| Scores set the award | no | yes | yes |
| Pays on the NEAR AI rail | yes | only with A''s Settle change | yes |
| Keeps the consent, issuer and production-gate checks | yes | only if it declares `NoveltyUtility` | yes |
| New binary | no | yes | no |
| Default routing and promotion tooling unchanged | yes | no | yes |
| Today's package hash unchanged | yes | yes | yes |
| Rulings changed | S10 | S10 | S10, P3-D6 |

## 5. Why perplexity and anomaly, and not full q

The owner's credit model says traces a frontier model finds surprising earn
more. The perplexity term is that signal, calibrated for Qwen3.8-27B (V3).

Full q multiplies in the novelty term `g`. Its floor of 0.5 sits above
bge-large's whole live range (novelty p50 0.165, p95 0.29), so `g` is
0.30 for nearly every trace. Multiplying by it scales every award by 0.30
and adds no ranking. Novelty still decides whether a trace is paid at all,
through the 0.10 gate floor and the duplicate rule.

Add a novelty term in a later era, once novelty is recalibrated against the
refilled index. The `constants_at` schedule exists for that, and the rule
name carries `_v1` so a `_v2` can add it.

## 6. Award amounts

Recommended ceiling: a delta of **1 point**
(`TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=1`, 1,000,000
microcredits), or $0.25 at the pilot rate.

Graded award per passing trace (`a = 1`, which covers the observed capped
median ratio of 8.18):

| Representative perplexity | `f` | Points | USD |
|---|---|---|---|
| 1.65 (observed minimum) | 0.29 | 0.29 | $0.07 |
| 2.17 (p05) | 0.40 | 0.40 | $0.10 |
| 4.62 (capped-trace median) | 0.66 | 0.66 | $0.17 |
| 5.14 (p50) | 0.70 | 0.70 | $0.17 |
| 13.73 and above (about p90) | 1.00 | 1.00 | $0.25 |

At a typical 0.68 points per passing trace, an account reaches the 80-point
rail ceiling after about 118 passing traces. That is 80 at the best case
and about 275 at the floor.

Days to the ceiling:

| Passing traces per day | Graded, typical 0.68 | Flat 1 point (A') | Flat 2.5 points (A') |
|---|---|---|---|
| 3 | 39 days | 27 days | 11 days |
| 8 (the 09-21 batch rate) | 15 days | 10 days | 4 days |
| 20 | 6 days | 4 days | 2 days |

A typical account earns about $0.50 to $3.40 a day at those rates. The most
the 7 open accounts can ever be paid is 7 x $20 = $140, and only accounts
that have linked can be paid at all.

Two limits matter:

- **The binding cap is the rail ceiling.** `MAX_POINTS_PER_ACCOUNT=80` is
  `main`'s scheduler cap, and the pipeline's own batches do not read it.
  Each leg settles in its own batch (`pipeline-activation.md:3070`). So the
  cap that actually binds is the rail's `ACCOUNT_CEILING_NANO` ($20). Points
  past 80 still accrue in the ledger and are not paid (decision D8).
- **Consent comes before any of this.** A trace whose allowed uses lack
  `model_training` is withheld (`policy_mismatch`). The default consent scope
  does not include it. How many pilot traces carry it has not been measured;
  it should be counted before go-live (D9).

## 7. Build, sign and promote

### 7.1 Where the key is

The package signing key is in the stage-3 key directory on the pilot host
(mode 0700). It sits beside the check-signing key (`pilot-check-20261009`)
and the current signed package. Its key id is `pilot-package-20261009`.

- The ingest's package trust store under `/etc/tracecommons/` trusts that
  key, so a new package signed with it needs no trust-store change.
- The key's file name was not read for this design. The operator confirms it
  with `ls` before step 3.
- No step below prints the key.

### 7.2 Sequence

Each step names what it changes and why uploads keep working. The owner's
approval splits the steps into three stages:

- **Stage 1, the code (step 0).** Ship option C and D6 with the rule off:
  `flat`, delta 0, package hash unchanged.
- **Stage 2, the rule, checked before anything is sent (steps 1-8).** The
  pilot's NEAR AI rail went `live` on 2026-11-03, before this design. So
  stage 2 begins by returning the rail to `dry_run` (step 1), and the first
  graded awards are batched, priced and checked (step 8) while nothing can
  be sent.
- **Stage 3, payment (step 9).** Only after step 8 passes does the rail go
  back to `live`.

**Step 0. Ship the code with the rule off.**
- Deploy the option C build with the env unchanged: `flat`, delta 0.
- The default package's bytes are the same, so its hash is still
  `973e6171...`. The deploy is the 2026-11-03 shape (phase A, R1-R4), and the
  operator scripts' "signed package unchanged" check passes as is.
- This keeps the risky change (code) apart from the credit switch.

**Step 1. Return the rail to `dry_run`, then pre-flight.**
- The rail is `live` today. Set `TRACE_COMMONS_NEAR_AI_PAYOUT_MODE=dry_run`
  and `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_DRY_RUN=true`, and restart
  (or halt through the control-token route first, then restart). Readiness
  must show `near_ai` `"mode":"dry_run"` before step 2. Nothing below may
  run while the rail can send.
- Count the accepted pipeline submissions that carry `model_training`, per
  tenant (D9).
- Record `GET /v1/admin/config-status`: armed, routed tenant count.

**Step 2. Build the graded env, without a restart.**
- Copy the live env file to a 0600 temp file. In the copy, set:
  - `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=1`
  - `TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_RULE=graded_perplexity_v1`
- `pipeline.py package` reads only `PACKAGE_VARIABLES` from it.

**Step 3. Build and sign the package**, on the host, from the promote
checkout at the deployed revision:

```bash
python3 scripts/operator/pipeline.py package --bundle production \
  --env-file <temp env> \
  --signing-key <stage-3 package key> --key-id pilot-package-20261009 \
  --output <keys dir>/PKG-graded.json --public-key-output <keys dir>/PKG-graded-KEY.json
# expect: PipelinePackageOK: bundle=sha256:<new> package=sha256:<new>
```

- Check that `PKG-graded-KEY.json` matches the trusted key already in the
  package trust store.
- Shred the temp env.

**Step 4. Phase A on the new package.**
- Run the phase A promotion with `promote init --package PKG-graded.json`.
- The host's promotion wrapper (not in the repository) pins the old
  package's path, so it needs a package argument or a pinned copy. The owner
  runs that edit.
- `promote package-checks` reads the gate settings and the delta back from
  the package itself (`CompatibilityBundleConfig::main_gate`,
  `versioned_pipeline_compat.rs:216-219`), so the checks run against the
  graded configuration.
- Record `M`, `R` and `REV`. `REV` must equal the running build's.

**Step 5. Edit the live env, without a restart.** Ingest reads env only at
start, so this edit changes nothing until step 6.

```bash
TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_POINTS_DELTA=1
TRACE_COMMONS_NOVELTY_UTILITY_CREDIT_RULE=graded_perplexity_v1
TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE=true
TRACE_COMMONS_NOVELTY_UTILITY_SETTLE_FROM=<RFC 3339 UTC, the edit time>
# plus TRACE_COMMONS_PIPELINE_CHECK_* for one boot, as on 2026-11-03 R1
```

- `REQUIRE_PRODUCTION_GATE` and `SETTLE_FROM` are boot-required once the
  delta is above 0 (`novelty_utility_credit_requires_production_gate`,
  `novelty_utility_settle_from_missing`).
- `SETTLE_FROM` gates only `main`'s legacy settlement. Pipeline-batched
  credit ignores it, so it has no effect on what this design pays.

**Step 6. Restart (R1).**
- The new default package is the graded one.
- Startup checks every routed tenant's active bundle against the new gate
  configuration (`pipeline-activation.md:3457-3480`). Those bundles are the
  old delta-0 package, so **without D6 this start is refused**
  (`pipeline_runtime_main_gate_config_mismatch`). See section 7.3.
- With D6, the old bundle is tolerated, because a tenant already selected it
  and it differs only by a lower award, and the start proceeds.
- D6 never applies to the armed set. Arming runs the tenant-bundle checks on
  the set's package, which still names the old package, so it fails and
  default routing stays unarmed until R3 swaps in the graded set. No new
  tenant lands on the delta-0 package in the window.
- The old bundle is still qualified on this revision, so routed tenants keep
  uploading on it, at delta 0, until R3 moves them.

**Step 7. R2 and R3.**
- R2: `promote adapters`, `sign` and `assemble`. The adapters result written
  at this boot names the graded package.
- R3: re-arm default routing with the new set, using the script's
  package-change override.
- With D6, the requalify loop moves each default-routed tenant whose active
  bundle is the old delta-0 package to the armed graded bundle. Today it
  skips such a tenant with `pipeline_default_routing_requalify_bundle_differs`
  (`pipeline-activation.md:1345-1347`).
- The tenant activated by hand moves through `POST /v1/admin/pipeline/activate`
  with the graded package. Today's R4 script only requalifies.

**Step 8. Verify in dry run.**
- The first graded run's forensic trace
  (`GET /v1/admin/pipeline/runs/{run_id}/forensic`) shows:
  - an award equal to the recomputation from `award_adjustments`;
  - a leg that is `complete` with a `settlement_batch_id`;
  - a finalized batch.
- The rail's dry-run pass shows a priced accrual (`dry_run_units`) for that
  account.
- A trace without `model_training` shows `withheld` / `policy_mismatch`.

**Step 9 (stage 3). Go live**, only after step 8 passed.
- Resume the rail, or set `TRACE_COMMONS_NEAR_AI_PAYOUT_MODE=live` with
  `TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_DRY_RUN=false`, and restart.
- Confirm one real payout end to end on the owner's own linked account.

### 7.3 Without D6: moving each tenant by hand

If D6 is declined, every routed tenant must leave the delta-0 bundle before
step 6 (`pipeline-activation.md:3482-3527`). That is about 21 tenants. For
each one, under the old configuration:

1. Deactivate it.
2. Add it to the drain list.
3. Wait until it has no run in flight.
4. As the database owner, delete its `pipeline_active_bundles` row inside a
   transaction that sets the tenant. Expect `DELETE 1`.

After the restart, qualify and activate each tenant again by hand.

Two costs follow:

- Default routing never re-activates a tenant whose routing row exists
  (`pipeline-activation.md:1247-1250`). So all of these tenants become
  hand-activated, and after every later deploy each one needs its own
  requalification, the R4 step, instead of the loop doing it.
- Deactivated tenants upload through the legacy path until they are
  activated again. That path awards nothing on the pilot.

D6 avoids both costs.

## 8. Rollback

Restoring the env backup is not a rollback here. Once tenants are bound to
the graded bundle, a restart with delta 0 is refused for the same reason as
in step 6.

- **Stop paying, instantly, no restart.**
  - Halt the rail through the payout control token's halt route
    (`payout_rails.rs`). Undo it with the matching resume.
  - Then set `TRACE_COMMONS_NEAR_AI_PAYOUT_MODE=disabled` at the next
    restart, with the delta and the rule unchanged.
  - Awards keep being written and batched, and accruals are priced, but
    nothing is sent.
- **Stop awarding.**
  1. For each tenant, roll back to the old delta-0 bundle with
     `POST /v1/admin/pipeline/rollback`.
     - The route selects a bundle that an earlier `activate` or `rollback`
       of this tenant selected (`pipeline-activation.md:495-504`). The
       default-routing activation counts.
     - It runs the gate on fresh evidence: a signed check set for the old
       package on the running revision. That is the 2026-11-03 set while it
       is inside its 7-day maximum age, or a new set after that.
     - Under the graded configuration, D6's tolerance is what lets the gate
       pass on the old package.
  2. Wait until no graded run is in flight.
  3. Set the delta to 0 and the rule to `flat`.
  4. Restart.

  D6's tolerance covers only bundles with a lower award, so the graded bundle
  must be gone from every tenant first.
- **Code.** Step 0's build with the rule `flat` behaves exactly as today,
  because the flat package bytes are unchanged. A binary rollback below it
  first requires stopping the awards.

What stays:

- Ledger rows and finalized batches. The ledger is append-only, and settled
  credit is not clawed back (`docs/trace-commons-roadmap.md:37`).
- Accruals and rate epochs.
- Sent NEAR AI transfers, which cannot be undone.

A correction is an operator-raised reversal, which the rail prices against
the line it reverses.

## 9. Owner decisions

All ten were approved as recommended by the owner on 2026-11-03.

D1. **Which option.** Recommend C: graded, batched, in the compatibility
family. A' is the fallback if a flat award is wanted first. It is the same
Settle change without the Score change, and C can follow as a second package.
Do not use B for the pilot.

D2. **Lift Ruling S10 for the new rule:** the pipeline batches its own
`NoveltyUtility` leg. Recommend yes, scoped to bundles that opt in. `main`'s
settlement exclusion stays, so nothing is paid twice.

D3. **Supersede Ruling P3-D6 for the new rule:** the award may depend on the
scores. Recommend yes. Flat bundles keep P3-D6.

D4. **The graded formula.** Recommend `delta * f * a`, recorded as two
`award_adjustments`, with the novelty term left out until it is recalibrated
(section 5). The alternative is full q, which scales every award by about
0.30.

D5. **The per-trace ceiling (delta).** Recommend 1 point ($0.25). A typical
trace then earns about 0.68 points and the 80-point ceiling takes about 118
passing traces.

D6. **Tolerate and move old bundles.** Two parts:
- Startup tolerates a compatibility bundle that a tenant already selected,
  or that a run in flight is bound to, when it differs from `main`'s gate
  configuration only by a lower award (delta 0, rule `flat`). The tolerance
  never reaches the default package or the armed set, which must match
  exactly.
- The default-routing requalify loop moves such a tenant to the armed
  bundle.

Recommend yes, built in the same PR. Without it, about 21 tenants must be
moved by hand, and they become hand-activated for good (section 7.3).

D7. **Credit holds on the batched leg.** Recommend that holds apply, as for
`PipelineScore` and as `main` leaves held accounts out of its batches. Today
the compatibility leg ignores holds because it never pays.

D8. **The cap.** Recommend relying on the rail's $20 lifetime ceiling for the
pilot. It equals 80 points at $0.25. Ledger points past 80 keep accruing,
unpaid.
- The alternative is a pipeline-side lifetime cap per account. That is new
  code, and it would make the pilot's 80-point policy hold even if the rate
  changes.
- If ledger points past the paid ceiling are shown to contributors, that
  needs wording review.

D9. **The consent precondition.** Recommend counting the pilot traces that
carry `model_training` before go-live, read-only. Add no prompt or notice
without separate approval (copy guide, rule 9).

D10. **Go-live order.** Recommend:
1. Ship the code with the rule off (step 0).
2. Switch the rule and verify in dry run (steps 2-8).
3. Then go live (step 9).

That puts two restarts and one re-arm between now and the first payout.

## 10. Not established

- Upload volume per account per day on the current build. Section 6 gives a
  range instead.
- How many pilot submissions carry `model_training`.
- The package key's file name. It is confirmed on the host before step 3.
- That D6's tolerance covers every path that checks a tenant bundle (startup,
  the activation and rollback gates, the requalify loop) and no path that
  checks the armed set. The staging rehearsal of steps 0-8 and of the
  rollback confirms it.
