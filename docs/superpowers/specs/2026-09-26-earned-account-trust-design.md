# Earned Account Trust (Flow 3) — Design

Date: 2026-09-26
Status: draft, spec for Zaki's review
Item: Z9 in the #991 work split ("The earned-trust mechanism for Flow 3")
Extends: [`2026-09-23-connect-and-forget-consent-design.md`](2026-09-23-connect-and-forget-consent-design.md),
"The trust model" and "The three paths"
Builds on: #1020 (account admission, V77), #1016 (invite trust, V75),
#1036 (merges carry invite trust, V80), all merged
Scope: `trace-commons-server` (`account_trust.rs`, `admission_ledger.rs`,
`db/postgres_account_trust.rs`, the ingest admission module, a new worker
route), migrations listed but not written. No production code in this PR.

## Problem

The consent spec settles *where* earned trust lives and leaves *what* it is
open. Under its trust model, every contributor has a NEAR AI login, an invite
is full trust, and a contributor without an invite "may still arm any folder,
and sends within server-side limits set by how far the server trusts their
account. That trust grows over time; it is Flow 3's mechanism"
(`2026-09-23-connect-and-forget-consent-design.md:146-148`). Its Open list
still carries "Earned-trust signals and thresholds": "which signals count,
what they accumulate toward and where the limits sit are not" settled.

#1020 built the limit and deliberately left growth out:

- A policy is `BoundedPolicy { version, processing_cost_bound,
  bounded_allowance, period }` (`crates/trace-commons-server/src/account_trust.rs:109-114`).
  `parse_bounded_policy` refuses any `growth_rule` other than `"none"`
  (`account_trust.rs:175`).
- Every non-invited account has one allowance: `policy.bounded_allowance()`,
  compared in the status read (`crates/trace-commons-server/src/admission_ledger.rs:337`)
  and at reservation (`admission_ledger.rs:483`).
- V77 created `trace_account_trust_facts` "for a future reviewed growth
  policy. This table is not consulted by admission and never changes the
  allowance" (`migrations/V77__account_admission.sql:40-56`), with a
  definer function that verifies a fact before recording it
  (`V77__account_admission.sql:160-203`) and a Rust seam over it
  (`crates/trace-commons-server/src/db/postgres_account_trust.rs:18`). The
  operator doc is explicit: "Trust facts remain an unused storage seam; no
  production worker records them" (`docs/operator/account-trust.md:132`), and
  "No quality-based growth is enabled" (`account-trust.md:24`).

So today a non-invited account on an ingest with account admission on gets
the same fixed allowance on day 300 as on day 1. This spec defines what makes
that allowance grow, what makes it shrink, and what it can never change.

## Goal

A deterministic, auditable rule that maps hash-only, server-observed facts
about an account to an allowance multiplier, run in shadow first and switched
on only against stated conditions.

## Non-goals

- **Not credit.** Trust changes how much an account may *send*. It does not
  touch `credit_quality` (`crates/trace-commons-server/src/credit_quality.rs:167`),
  the contributor cap (`contributor_cap.rs:39-52`), or settlement. The
  reputation factor on credit that the credit pipeline has as sub-project #4 is
  a separate item; coupling the two would let credit farming buy volume and
  volume farming buy credit.
- **Not arming.** Under rev 8 any contributor may arm any folder; trust sets
  volume. See "Reconciling with the consent spec" below.
- **Not Sybil resistance on its own.** Trust growth is bounded so that it does
  not *worsen* Sybil exposure; the base allowance that every new account gets
  is Z6's (server-side spam control) problem, not this spec's.
- **Not invite issuance.** Earned trust does not mint, imply, or substitute
  for an invite (see "What trust buys").
- No numeric values. #1020's operator doc says "There are no default numeric
  allowances" (`account-trust.md:20`), and this spec keeps that: every
  threshold below is a named policy parameter, set from calibration.

## The principle

The consent spec's rule, restated for this mechanism: **trust relaxes what may
be sent, never what may be said** (`2026-09-23-connect-and-forget-consent-design.md:166`).

Earned trust changes exactly one number: the account's effective allowance in
processing-cost units per policy period. Everything else is outside its reach:

| Unchanged by trust, at any tier | Why |
|---|---|
| R1: which disclosure an armed folder gets | the model-scrub wording depends on a certified pipeline having run, not on who the account is |
| R2: the gate at the `AutoUpload` decision | a client rule |
| R5: per-entry holds | a held session is held because of what is in it |
| R6: the void rule | a grant voided by widened terms is voided for everyone |
| R7: data-use scope | the contributor's choice |
| The privacy gates: redaction, PII backstop, quarantine, residual-risk | these judge the trace, not the sender |
| The hourly submission quota, request limits, witness capacity (Z5) | #1020 exempts only cumulative allowance even for invitees (`account-trust.md:122-123`); earned trust exempts less than an invite does |
| Credit, the contributor cap, settlement | see Non-goals |
| The `authority` label on `/v1/account/contribution-status` | stays `bounded`; see "Mechanics" |

A high-trust account whose trace carries a secret is quarantined exactly as a
new account's would be.

## What earns trust

### Facts, not scores

The growth rule reads **facts**: rows in `trace_account_trust_facts`, each
naming a server-owned source row (a credit event, a gate decision, a status
transition), recorded once, verified by a definer function, and never
supplied by the client. It does not read live scores on every evaluation;
scores enter only through the fields a fact's source row carries, under an
allowlisted calibration version.

V77 records two kinds today (`account_trust.rs:20-23`,
`V77__account_admission.sql:45-48`):

- `accepted_submission`, outcome `accepted`: the submission is `accepted` and
  has an `accepted` credit-ledger event (`V77__account_admission.sql:167-177`).
- `gate_evaluation`, outcome `evaluated_passed` / `evaluated_failed` /
  `evaluated_not_accepted`: `evaluated_passed` requires `perplexity_passed AND
  novelty_passed` on an accepted submission (`V77__account_admission.sql:178-192`).

This spec adds four (migration M1):

| New kind | Source row | Recorded when |
|---|---|---|
| `submission_withdrawn` | `trace_submissions.withdrawn_at` set (V43) | the contributor withdraws |
| `submission_revoked` | `status = 'revoked'` with `withdrawn_at IS NULL` | an operator or policy revokes (`migrations/V43__trace_withdrawal.sql:13-18` is what separates the two) |
| `submission_quarantined` | `status = 'quarantined'` | the privacy pipeline quarantines |
| `abuse_penalty` | a `trace_credit_ledger` event of type `abuse_penalty` (`crates/trace-commons-server/src/trace_corpus_storage.rs:246`) | a reviewer issues one (`crates/trace-commons-server/src/bin/trace-commons-review.rs:195`) |

Facts are append-only. A later transition does not rewrite an earlier fact;
it adds a new one, and the rule reads both. That matters because V77's
`ON CONFLICT DO NOTHING` (`V77__account_admission.sql:196-199`) means an
`accepted` fact persists after its submission is withdrawn or revoked. The
rule must therefore net later facts against earlier ones, not trust the first.

### The qualified contribution

The unit that grows trust is a **qualified contribution**: one submission for
which, at evaluation time, all of these hold.

1. **Accepted.** An `accepted_submission` fact exists.
2. **Not netted out.** No later `submission_withdrawn`, `submission_revoked`
   or `submission_quarantined` fact for the same submission. (Retention
   `expired` and `purged` do not net a contribution out: the contribution was
   real and left under policy, not because of anything the contributor did.)
3. **Passed the gate under an allowlisted evaluator.** A `gate_evaluation`
   fact with outcome `evaluated_passed` whose `evaluator_version` (the gate
   policy version) is on the growth policy's allowlist.
4. **Cleared the quality threshold under an allowlisted calibration era.**
   `credit_quality_micros >= q_min` on the gate decision, with
   `credit_quality_calibration_version` on the allowlist (V39 columns). `q`
   is a *threshold*, not a weight: it is shadow-only and its calibration is
   still moving (`constants_at`, `credit_quality.rs:115`), and a weight would
   hand the credit function's calibration choices a second job.
5. **First in its dedup cluster, under an allowlisted dedup signal.** The
   submission is the earliest member of its `dedup_cluster_id` (V40), and the
   decision's `dedup_signal_version` (V57) is on the allowlist.

Signals deliberately **not** read in v1:

- **Per-author perplexity (V73).** Its own spec says the evidence "is not
  enough to gate on it" and defers any use until backfilled values have been
  checked against human labels
  (`2026-09-18-per-author-perplexity-shadow-design.md:62-63, 293-299`). The
  growth policy's allowlist mechanism can admit it later.
- **Raw perplexity or novelty magnitudes.** The gate's pass/fail already
  encodes them under a versioned floor, and the novelty floor is a holding
  value pending recalibration.
- **The submit-time credit estimate** (`credit_points_pending`). It is a
  client-visible heuristic, not a server judgement.
- **Inference provenance (R4).** The consent spec says R4 "feeds account
  trust" (`2026-09-23-connect-and-forget-consent-design.md:161`). It should,
  but #1005's v2 certificate with signed `inference_provenance` is not yet a
  fact source. It is Open question 5.

### Aggregation

Qualified contributions are aggregated over a sliding **window** `W`
ending at the evaluation's `as_of` time, with a **per-week cap** `c`: in any
one calendar week (the same 7-day bucketing `contributor_cap::epoch_index`
uses, `contributor_cap.rs:30`), at most `c` qualified contributions count.
Three numbers come out:

- `units`: capped qualified contributions in the window;
- `active_weeks`: weeks in the window with at least one qualified contribution;
- `age`: time since the account's first `accepted_submission` fact.

The **tier** is the highest `k` in the policy's tier table for which
`units >= U_k`, `active_weeks >= D_k` and `age >= A_k`, and no penalty
cooldown applies. Tier 0 always exists, with no requirements. The table is
monotone: each tier's thresholds are at least the previous tier's.

A step function over a few tiers, rather than a continuous score, is chosen so
that an operator can explain an allowance in one sentence ("tier 2: 41 units
over 6 active weeks, account 9 weeks old"), a contributor can be told when it
changes, and a calibration can be argued about tier by tier.

### Neutral and negative facts

| Fact | Weight |
|---|---|
| `accepted` + `evaluated_passed` + quality + first-in-cluster | +1 unit (subject to the weekly cap) |
| `evaluated_failed`, `evaluated_not_accepted` | **neutral.** A low-value trace earns nothing; it has already spent allowance, which is its cost |
| `submission_withdrawn` | **neutral: nets out, never penalises.** Withdrawal is a consent right, and a rule that charged for it would chill it. It removes the withdrawn contribution's unit, so submit-accept-withdraw cycles gain nothing |
| `submission_revoked` (operator) | **neutral: nets out.** Revocation reasons are free text (`trace_revocation_reason_for_request`, `crates/trace-commons-server/src/bin/trace-commons-ingest.rs:56512`), cover operator error and policy changes, and cannot be put in a hash-only fact. A punitive revocation should be an `abuse_penalty` |
| `submission_quarantined` | **neutral: nets out** in v1. Quarantine judges the trace's residual privacy risk; the pilot's 114 quarantines examined on 2026-08-31 were verdicts from a NEAR AI degradation window, not contributor conduct. A per-account quarantine *rate* is measured in shadow (Open question 3) |
| `abuse_penalty` | **negative.** The tier is 0 for a cooldown `C` after the latest penalty, and qualified contributions before that penalty stop counting |
| Invite revocation | **neutral for earned trust**; see "What loses trust" |

Only one fact is punitive, and it is the only one a human issues for conduct.
Every other adverse outcome removes credit for the contribution it concerns
and nothing more.

### Gaming, and what resists it

**Farming low-value accepted traces.** A unit needs acceptance, a gate pass,
`q >= q_min` and first-in-cluster; the weekly cap `c` limits how fast units
accrue however many traces are sent; and `active_weeks` and `age` make the
upper tiers cost calendar time that cannot be parallelised within one
account. The attacker's payoff is also bounded: a higher tier buys only
volume, and the credit that volume could earn is capped per identity per epoch
by the contributor cap, which trust does not touch.

**Dedup mega-clusters.** Under dedup v1, 522 of 696 rows sat in one cluster.
First-in-cluster fails *safe* under a mega-cluster: one unit for the whole
cluster, so a broken signal under-counts rather than over-counts. The
allowlist on `dedup_signal_version` keeps rows stamped under a signal known to
be broken out entirely (v1 should not be on it; v2 is
`ACTIVE_DEDUP_ALGORITHM`, `crates/trace-commons-server/src/dedup_simhash.rs:99`).
A rederive that changes cluster membership changes units at the next
evaluation, which is correct, and is recorded by the evaluation's digest.

**Sybil accounts.** A second NEAR account is a second anchored tenant
(`trace_commons_protocol::admission::is_anchored_tenant`,
`crates/trace-commons-protocol/src/admission.rs:219`). Each starts at tier 0,
so N Sybils get N base allowances whether or not this spec exists; that
exposure is Z6's. What this spec must prevent is Sybils *pooling* one corpus
to grow each other: first-in-cluster is evaluated across tenants, so a trace
copied into ten accounts earns a unit for at most the first. That cross-tenant
check must not let one tenant read another's rows; see "Mechanics", M3.

**Self-dealing.** Three routes, three answers:

- *Reviewer approval by the contributor.* An `accepted` fact whose acceptance
  came through review by a principal linked to the same account does not
  qualify. V77's definer function does not check this today; M2 adds it.
- *Merging accounts to pool history.* Facts follow a merge (M4), but the
  weekly cap and `active_weeks` are computed over the union, so two accounts
  merged do not count a week twice.
- *Operator-issued credit events* (`reviewer_bonus`, `training_utility` and
  the rest) are not facts. Only `accepted` and `abuse_penalty` events are read.

## What trust buys

### The allowance multiplier

The growth policy's tier table maps each tier to a multiplier `m_k`
(`m_0 = 1`, monotone). The effective allowance is:

```
effective_allowance = min(bounded_allowance * m_tier, allowance_ceiling)
```

in the same processing-cost units and the same period as the base policy
(`PolicyPeriod`, `account_trust.rs:135-138`). Under a `fixed` period, a higher
tier means more per period; under `lifetime`, it raises the lifetime cap. The
`allowance_ceiling` is mandatory and finite.

### Invite-equivalence: never by accrual

Earned trust **does not reach invite-equivalence on its own**. The top tier's
allowance is finite, and the authority label stays `bounded`. Two things in
the code make that the natural line as well as the safe one:

- `authority = 'invited'` holds if and only if an active invite grant exists.
  Reservation demotes an `invited` row without one
  (`admission_ledger.rs:438-440`), processing refuses any reservation where
  `(authority == "invited") != grant` (`admission_ledger.rs:575`), and V80's
  merge rule re-derives authority from grants alone
  (`migrations/V80__account_trust_merge.sql:11-16`). An earned `invited`
  would break that invariant in three places.
- The client's `AccountAdmission::from_status` maps any authority it does not
  know to `NotAdvertised`, which keeps R3 on
  (`crates/trace-commons-contributor/src/daemon/automatic_gate.rs:151-157`).
  A new `earned` authority label would silently hold every earned account's
  armed folders on every shipped client.

The route to full trust from Flow 3 is therefore an operator issuing an
invite, informed by the account's explain output (below). That keeps a human
decision at the point where volume becomes unlimited. Open question 1 offers
the alternatives.

## What loses trust

- **Decay by window.** Units older than `W` drop out, so an inactive account
  slides down the tiers without any explicit decay step, and returns to tier
  0 once its window is empty. `age` does not decay; it is a floor, not a
  reward.
- **Netting.** Withdrawal, revocation and quarantine remove the unit of the
  submission they concern (above).
- **Penalty.** An `abuse_penalty` sets tier 0 for `C` and discards earlier
  units. After `C`, the account re-earns from facts after the penalty.
- **Invite revocation.** Revoking an invite demotes the account from
  `invited` to `bounded` (`admission_ledger.rs:438-440`) and the account falls
  to its **earned** tier, not to tier 0. Invites are revoked for cohort
  reasons (an expired hackathon code) as often as for conduct, and a
  conduct-driven revocation should come with an `abuse_penalty`. Facts
  recorded while invited count under the same rule, including the weekly cap,
  so an invitee's unlimited volume does not convert into a top tier faster
  than `c` per week allows. Open question 4.
- **Account closure or principal unlink.** Nothing to evaluate: admission
  already refuses a closed account or unlinked principal before any allowance
  is read.

A tier change mid-period does not claw back reservations already charged.
Spend in the current period is compared against the new effective allowance
at the next reservation; a lowered allowance can make `ready` false for the
rest of the period, which clients already handle as `account_limit_reached`.

## Mechanics

### A growth rule beyond `"none"`

`RawPolicy` gains an optional `growth` object (`account_trust.rs:144-152`
uses `deny_unknown_fields`, so this is a parser change, not a configuration
change):

```json
{
  "version": "bounded-growth-v1",
  "processing_cost_bound": 10,
  "bounded_allowance": 100,
  "period": {"mode": "fixed", "seconds": 604800},
  "growth_rule": "tiered-v1",
  "growth": {
    "window_seconds": 0,
    "weekly_cap": 0,
    "q_min_micros": 0,
    "penalty_cooldown_seconds": 0,
    "evaluation_max_age_seconds": 0,
    "allowance_ceiling": 0,
    "evaluator_versions": [],
    "credit_quality_calibration_versions": [],
    "dedup_signal_versions": [],
    "tiers": [
      {"units": 0, "active_weeks": 0, "age_seconds": 0, "multiplier": 1}
    ]
  }
}
```

Every number above is a placeholder for shape, not a proposed value. Parsing
rules:

- `"growth_rule": "none"` with no `growth` object parses exactly as today.
  Every existing fixture (`bin/trace_commons_ingest_internal/admission.rs:886`
  and the pg tests) keeps passing unchanged.
- `"tiered-v1"` requires `growth`; `"none"` forbids it. Any other value is
  refused, as now.
- Tier 0 must have all-zero requirements and multiplier 1; tiers must be
  monotone; `allowance_ceiling >= bounded_allowance`; every multiplication is
  `checked_mul`, refusing the policy on overflow at parse time for the top
  tier; every allowlist must be non-empty. A malformed policy refuses startup
  with the existing `account_admission_policy_invalid` label
  (`admission.rs:87-88`).
- The growth parameters are part of the reviewed policy version. Changing any
  of them is a version bump, supplied through the existing
  `TRACE_COMMONS_ACCOUNT_ADMISSION_POLICY_VERSION` allowlist.

### How admission reads trust: through a materialised evaluation

Admission does not aggregate facts in the reservation transaction. A worker
materialises an **evaluation** per account (M3), and reservation reads the
latest one:

1. `reserve_account_admission` (`admission_ledger.rs:352`) and
   `account_admission_status` (`admission_ledger.rs:275`) read the account's
   latest row in `trace_account_trust_evaluations` for the active policy
   version, in the same transaction that reads `trace_account_trust`.
2. If the row is missing, was computed under a different policy version, is
   `shadow` mode, or is older than `evaluation_max_age_seconds`, the tier is
   **0**. Every failure mode falls to the base allowance, never above it.
3. The comparisons at `admission_ledger.rs:337` and `:483` use
   `effective_allowance` in place of `policy.bounded_allowance()`.
4. The budget-row consistency checks at `admission_ledger.rs:325-327` and
   `:473-475` stay as they are: `cost_limit` keeps recording the **base**
   allowance, so a tier change does not trip them and does not strand a
   budget row.
5. **A tier change does not bump `trust_version`.** Processing refuses a
   reservation whose version moved (`admission_ledger.rs:574`); bumping it on
   every tier change would refuse in-flight work each time an evaluation ran.
   `trust_version` keeps meaning an authority change, as #1016 and V80 use it.
6. The reservation row records the evaluation it relied on (tier and facts
   digest, M5), so an operator can reconstruct every admission decision.
7. `/v1/account/contribution-status` keeps `authority: "bounded"` and adds an
   optional `allowance_tier` (a small integer) and `allowance_tier_changed_at`.
   The client's `StatusBody` does not deny unknown fields
   (`crates/trace-commons-contributor/src/daemon/account_admission.rs:131`),
   so current clients ignore them.

The evaluation is materialised rather than computed inline for three reasons:
the reservation path stays a handful of indexed reads; the runtime role
(`trace_account_admission_runtime`) needs no read on the facts table, which
V77 grants only to the guard (`V77__account_admission.sql:131`); and the
evaluation row is the audit record.

### The worker that records facts and evaluates

Batch first, as the contributor cap and dedup passes were, with inline
recording as a later step:

- `POST /v1/admin/record-account-trust-facts?limit=N&dry_run=` walks
  submissions in anchored tenants whose facts are missing and calls the
  recording function per source. Idempotent: the fact's primary key is
  `(tenant_id, account_id, source_kind, source_id)` and the insert is
  `ON CONFLICT DO NOTHING` (`V77__account_admission.sql:51, 196-199`), so a
  re-run records nothing new. Mirrors `/v1/admin/recompute-contributor-caps`
  (`bin/trace-commons-ingest.rs:8069`): `require_admin`, fail-closed, hash-only
  acknowledgement of counts.
- `POST /v1/admin/evaluate-account-trust?limit=N&mode=shadow|applied&as_of=`
  computes the tier for each account with facts, writes an evaluation row, and
  appends a hash-only `account_trust_tier_changed` row to
  `trace_account_audit` when the tier differs from the previous evaluation.
  Run on a schedule so that decay takes effect without new facts; how often
  bounds `evaluation_max_age_seconds` from below.
- Both routes set tenant context per account (`begin_trace_tenant_transaction`,
  as `record_account_trust_fact` does, `postgres_account_trust.rs:25`) and
  never widen the runtime role.
- Accounts are mapped from submissions exactly as V77's function does:
  `auth_principal_ref` joined to a live `trace_account_principals` row
  (`V77__account_admission.sql:168-172`). Submissions from legacy `tenant-…`
  identities never map, and earn nothing.

### Determinism and the explain route

An evaluation is a pure function of `(growth policy, facts with occurred_at
<= as_of, current gate-decision fields for those submissions, as_of)`. It
stores:

- `growth_policy_version`, `as_of`, `mode`, `tier`, `effective_allowance`;
- label-only counts: `units`, `active_weeks`, `age_weeks`, and the number of
  facts netted out per reason;
- `facts_digest`: `sha256` over the sorted `(source_kind, source_id,
  outcome, evaluator_version, occurred_at)` tuples that entered it, plus the
  dedup and quality fields read for each.

`GET /v1/admin/account-trust/explain?account_ref=<hash>` recomputes the
evaluation at the stored `as_of` and returns the stored and recomputed rows
side by side, with a `reproduced: true|false` flag. It returns counts, tier,
digest and policy version, never submission IDs or trace content. A drill,
`/v1/admin/account-trust-drill`, runs the recomputation over a sample and
reports the reproduced fraction as hash-only evidence for the rollout-smoke
path, per the repo's drill convention.

One source of non-determinism has to be designed out: V77's facts carry only
`recorded_at DEFAULT clock_timestamp()` (`V77__account_admission.sql:50`),
the time the worker ran, not the time the event happened. A backfill would
stamp months of history with today's date and put it all in one week. M1 adds
`occurred_at`, taken from the source row (credit event time, gate
`decided_at`, `withdrawn_at`, `revoked_at`), and the rule reads only that.

### Migrations needed

None written here. Numbering starts after V80 (`migrations/V80__account_trust_merge.sql`),
and each must be registered in `run_migrations`
(`crates/trace-commons-server/src/db/postgres.rs:1426` for V80) and any new
table added to `TRACE_COMMONS_RLS_TABLES` (`postgres.rs:169`).

- **M1. Widen `trace_account_trust_facts`.** Add the four source kinds and
  their outcomes to the `source_kind` and `outcome` CHECKs and the
  kind/outcome consistency CHECK (`V77__account_admission.sql:45-55`). Add
  `occurred_at TIMESTAMPTZ NOT NULL`, backfilled from source rows for
  existing facts (of which the pilot should have none, since nothing records
  them). Index `(tenant_id, account_id, occurred_at)`. Make the foreign key
  to `trace_submissions` (`V77__account_admission.sql:53`) `ON DELETE
  CASCADE`: V43 keeps its withdrawal tombstone free of that foreign key
  precisely so that a future hard delete of the submission row stays possible
  (`V43__trace_withdrawal.sql:34-36`), and a trust fact must not be what
  blocks it. A cascade removes a submission's positive and netting facts
  together, so the account's units come out the same.
- **M2. Extend the recording function.** Replace
  `trace_record_account_trust_fact` (or add a sibling) to verify the new kinds
  from their server rows, to read `occurred_at` from the source, and to
  exclude self-reviewed acceptance. Grant the guard the extra column reads it
  needs: `trace_submissions(withdrawn_at, revoked_at, reviewed_at)`, whatever
  identifies the approving reviewer, and `trace_credit_ledger` rows of type
  `abuse_penalty`. (`trace_submissions.review_assigned_to_principal_ref` names
  the assignee, which need not be the approver; the implementation must find
  the approving principal in the review audit trail, and fail closed, treating
  the acceptance as non-qualifying, where it cannot.) Same ownership pattern as V77 (NOLOGIN, NOBYPASSRLS guard,
  `SECURITY DEFINER`, `search_path=pg_catalog`, tenant check first).
- **M3. `trace_account_trust_evaluations`.** Columns as listed under
  "Determinism"; RLS enabled and forced under `trace_current_tenant_id()`;
  runtime role gets `SELECT` only; writes only through a definer function
  owned by a guard role. Plus a **cross-tenant first-in-cluster check**:
  either a boolean column on `trace_gate_decisions` written by the dedup
  assign and rederive passes (which are already cross-tenant), or a
  boolean-only definer function in the style of
  `trace_account_admission_linkage_ready` (`V77__account_admission.sql:278-297`)
  that answers "is this submission its cluster's earliest member" without
  returning any other tenant's identifiers. The column is simpler and is
  recommended.
- **M4. Merge carry.** A V80-style hook that copies the absorbed account's
  facts onto the survivor inside the executing merge, keyed so a fact is never
  duplicated. Evaluations are not copied; the survivor is re-evaluated.
- **M5. Reservation audit columns.** Nullable `earned_tier` and
  `trust_evaluation_digest` on `trace_account_admission_submissions`.

## Privacy

- **No trace content, ever.** A fact names a server row by UUID and carries a
  closed-set kind and outcome, a version label, and a time. The evaluation
  carries counts, a tier and a digest. Neither carries text, paths, URLs,
  scores' underlying text, revocation reason strings, or principal refs.
- **Hash-only surfaces.** The explain route, the audit row
  (`account_trust_tier_changed`, `safe_metadata` of from/to tier, policy
  version and facts digest, actor via `account_actor_ref`,
  `crates/trace-commons-server/src/account_session.rs:77`), worker
  acknowledgements and logs are counts, labels and hashes.
- **Tenant-scoped under forced RLS.** Facts and evaluations are per tenant
  under `trace_current_tenant_id()`, as every V75/V77 table is. The one
  cross-tenant question, first-in-cluster, is answered as a boolean and never
  as a row.
- **The contributor can see their own tier** through the status route. They
  cannot see any other account's.
- **Withdrawal leaves a fact.** A `submission_withdrawn` fact references a
  submission row that V43 already keeps after withdrawal, and adds only the
  withdrawal time, which the V43 tombstone also holds. It does not extend what
  withdrawal retains beyond account scope.

## Reconciling with the consent spec

The consent spec's rev 7 fixed two structural properties of Flow 3
(`2026-09-23-connect-and-forget-consent-design.md:693-698`): **whatever is
earned is expressed as project mode**, and **nothing is earned silently**.
Rev 8 then made arming the contributor's act and trust a server-side volume
limit. Under rev 8 what Flow 3 earns is volume, not mode, so the first
property no longer describes the mechanism: an armed folder is armed by the
contributor, and earned trust only lets it send more. This spec keeps the
second property in the form rev 8 allows: a tier change is recorded in the
account audit, exposed on the status route, and (once a shell shows it)
announced to the contributor, in both directions. Open question 6 asks
whether the consent spec should be amended to say so.

## Rollout

### 1. Shadow

Same shape as the V73 per-author perplexity shadow: compute and persist,
change no decision.

- Ship M1-M5, the parser change, the two worker routes and the explain route.
  Production policy stays `"growth_rule": "none"`.
- A **candidate** growth policy is supplied separately
  (`TRACE_COMMONS_ACCOUNT_TRUST_SHADOW_POLICY_JSON`, parsed by the same
  function, never read by admission). The evaluation worker runs in
  `mode=shadow` against it.
- Admission ignores `shadow` rows by construction (rule 2 above), so a shadow
  row cannot raise an allowance even if the production policy were
  accidentally switched.
- The status route does not report a shadow tier.
- A label-only counter records reservations refused with
  `account_limit_reached` that the shadow tier would have admitted: the number
  the switch-on decision turns on.

### 2. Calibration against pilot data

What the pilot can and cannot tell us, stated up front: the pilot has on the
order of 1,800 gate decisions, dominated by one contributor, with dedup v2
live since 2026-09-22 and credit-quality V3 only recently in effect. That is
enough to validate **mechanics**: facts recorded idempotently, occurred-at
backfill correct, evaluations reproducible, netting and cooldown behaving.
It is not enough to set **thresholds**, which need several contributors with
some weeks of history each. The calibration report
(`docs/superpowers/reports/`) therefore has two parts: a mechanics part that
can be finished on the pilot, and a threshold part that is re-run as
contributors arrive.

The threshold part reports, per candidate tier table: the tier distribution
of active accounts; how many accounts with any `abuse_penalty` or a
quarantine rate above a stated level reach tier 1 or above (the target is
zero); the weekly-cap binding rate; and the would-have-admitted count.

### 3. Switch-on conditions

The production policy moves from `"none"` to `"tiered-v1"` only when all of
these hold:

1. Account admission itself is enabled on the target ingest, with its
   activation conditions met (`docs/operator/account-trust.md:67-77`);
   growth is meaningless without it.
2. Z6 (spam control) is in place, since the base allowance every account gets
   is the Sybil exposure this spec does not address.
3. Shadow has run for at least one full window `W` on the target deployment,
   with evaluations for every account that has facts.
4. The explain drill reports 100% reproduction over the whole population, not
   a sample, at switch-on.
5. The calibration report's threshold part exists with at least a stated
   minimum number of contributors, and zero penalised or high-quarantine
   accounts at tier 1 or above.
6. The allowlists name only signals known good: dedup v2 or later, the
   credit-quality eras in effect, the gate policy versions in effect.
7. The contributor-facing tier-change notice exists in at least the Tauri
   client, which is the MVP client, so nothing is earned silently.
8. Zaki approves the tier table and ceiling as a reviewed policy version.

Switching off is a policy version bump back to `"none"`. Spend already
recorded stays; `account_period_spend` sums across versions in the same
bucket (`admission_ledger.rs:169-179`), so a switch-off does not refresh an
allowance.

## Testing (for the implementation PRs)

- Parser: `"none"` byte-identical to today; each malformed growth object
  refused; overflow at the ceiling refused.
- Rule (pure, no database): each fact kind's weight, netting order, weekly
  cap, window edges, cooldown, monotone tiers, determinism of the digest
  under row reordering.
- PostgreSQL: facts recorded once under concurrent workers; self-reviewed
  acceptance refused; a runtime role cannot read facts; a shadow evaluation
  never raises an allowance; a stale or missing evaluation yields tier 0; a
  tier change does not refuse an in-flight reservation; a merge does not
  double-count a week; RLS isolation for both new tables.
- The explain drill wired into rollout-smoke evidence.

## Open questions for Zaki

1. **Can earned trust ever reach invite-equivalence?**
   (a) Never by accrual; the top tier is finite, and an operator issues an
   invite to go further. (b) A top tier with no cumulative cap, still labelled
   `bounded`. (c) Automatic invite issuance at the top tier.
   **Recommendation: (a).** It keeps `invited` iff active grant intact in the
   three places that enforce it, needs no client change, and keeps a human
   decision where volume becomes unlimited. (b) is reachable later by raising
   the ceiling in a reviewed version; (c) makes the invite path gameable by
   whatever games the tiers.

2. **Does quality enter as a threshold, a weight, or not at all in v1?**
   (a) Threshold `q >= q_min` under allowlisted eras. (b) Weight units by `q`.
   (c) Gate pass only, no `q`.
   **Recommendation: (a).** A weight couples admission to a still-moving
   credit calibration; (c) lets gate-passing boilerplate count. If Zaki
   prefers to wait for V3 to settle, (c) for the first shadow run and (a)
   before switch-on.

3. **How is quarantine weighted?**
   (a) Neutral, netting out only. (b) A rate threshold: above a stated
   quarantine rate in the window, tier is capped. (c) Each quarantine a
   negative unit.
   **Recommendation: (a) now, measure (b) in shadow.** Quarantine judges the
   trace's residual risk, and the pilot's quarantines came from a scorer
   degradation window rather than from contributors. Punishing it would
   punish contributors for the pipeline's recall. If the shadow shows a
   per-account rate that separates bad actors, (b) is a policy parameter, not
   a redesign.

4. **What happens to earned standing when an invite is revoked?**
   (a) Fall to the earned tier, with facts from the invited period counted
   under the weekly cap. (b) Fall to tier 0 and re-earn. (c) Earned tier, but
   facts from the invited period excluded.
   **Recommendation: (a),** with a conduct-driven revocation accompanied by an
   `abuse_penalty`. Cohort revocations are common (hackathon codes) and should
   not erase real history; the weekly cap stops an invitee's unlimited volume
   from converting into a top tier.

5. **Should inference provenance (R4, #1005) earn trust?**
   (a) Not in v1. (b) A multiplier on units from sessions with verified
   `inference_provenance`. (c) A separate tier requirement ("at least k
   provenance-attested units").
   **Recommendation: (a) in v1, (c) as the follow-up.** The consent spec says
   R4 feeds account trust, and provenance is harder to fake than content
   metrics, but it is not yet a server-recorded fact, and (b) would make
   connected inference a faster route to volume, which the consent spec
   explicitly declines ("one route toward automatic contribution, not the
   door to it").

6. **Amend the consent spec's Flow 3 properties?** Rev 7 says "whatever is
   earned is expressed as project mode"; under rev 8 what is earned is volume.
   (a) Amend the consent spec: Flow 3 earns allowance; "nothing earned
   silently" becomes the tier-change notice. (b) Keep both, and have a tier
   change also offer to switch Ask-me folders to Automatic.
   **Recommendation: (a).** (b) reintroduces the server deciding a folder's
   mode, which rev 8 moved away from, and the offer can be a later client
   feature without being part of trust.

7. **Where do the worker routes' credentials sit?**
   (a) `require_admin`, as the contributor-cap and dedup passes do. (b) A new
   scoped worker bearer, per the repo's scoped-credential convention.
   **Recommendation: (a) for the shadow phase** (batch, operator-run), and
   (b) before an inline or scheduled recorder runs unattended in production.
