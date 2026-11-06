# Versioned processing, leases and human holds

Current-behavior source baseline:
`8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`. This model starts at the durable
receipt boundary. Admission has **already executed synchronously**; it is never
a leased worker phase. No owner-selected promises, product invariants, temporal
properties, guarantee tests or TLC checks are present. Typechecking is not a
proof of the product behavior.

## Variables and finite identity abstraction

- Two durable run identities, two tenants, two immutable bundle ids, two
  workers, two human reviewers. Each run snapshots tenant and bundle at receipt.
  No action migrates it when activation or rollback selects another bundle.
- Run `phase` (`review`, `score`, `settle`, `none`) is independent from `state`
  (`absent`, `pending`, `leased`, `retry`, `awaiting_review`,
  `awaiting_operator`, `complete`, `failed`). `submissionStatus` is separate
  from run state: Admission quarantine and privacy escalation can be
  `quarantined`, Review approval makes `accepted`, and Review rejection is a
  normal Complete run with `rejected` submission status.
- `attempt` is a reduced finite budget of 2 per phase, reset on successful
  phase advancement. Production's configured/stored `max_attempts` is not
  asserted to be 2. Claim charges, suspension/parking refunds, operational
  failures retain the charge. `due` abstracts clock ordering; a world event
  makes backoff due without assuming it ever happens.
- `workers[worker]` preserves a request snapshot with run id, phase and token
  after expiry/reclaim. `run.live` and `run.token` are the authoritative
  database lease relation. A stale response on the same expired token can
  record an uncharged Retry; after reclaim replaced the token it changes no run
  state. `renewals` 0..2 abstracts a finite cap, not two real renewal calls:
  production renews until four configured lease durations from start.
- Worker tokens are **symbolic identities**, chosen fresh from `1..16`, not
  ordered counters. A token excludes the run's current token, every outstanding
  worker snapshot for that run and all staged/committed artifact identities.
  There are at most 8 artifact slots, 2 worker snapshots and 1 current token,
  so this identity abstraction does not disable a legitimate fresh claim when
  numeric generations reach a ceiling. Deleted artifact slots can be reused;
  their identity is no longer observable. The bound does not model an external
  observer retaining an arbitrarily old token after every modeled reference
  has disappeared.
- Human claim fields (`reviewOwner`, `reviewToken`, `reviewLive`) and
  `humanRequests[reviewer]` are distinct from worker leases. Tokens from `1..4`
  exclude current claim plus both outstanding human request snapshots. A
  matching live claim may assess; stale/expired/replaced requests are refused.
  Reclaim by the same reviewer issues a new token just as the upsert does.
- `privacy` (`none`, `cleared`, `escalated`) is a committed Review-start pass.
  `reviewAssessment` and `scoreAssessment` are phase-bound and separate.
  `reviewAfterPass` preserves assessment freshness. All modeled receipts are
  current V117+ receipts: humans cannot claim Review until the privacy pass is
  recorded. Legacy pre-V117 receipts and their pre-pass assessments are an
  explicit omission, not an idealized freshness repair.
- Eight recyclable attempt artifact slots hold run/token/kind/staging/object
  identity. Privacy-pass and approved artifacts are separate; Score's command
  and neighbor pair is merged as `score_commands`. Committed object deletion
  belongs to withdrawal, while due staged orphan deletion belongs to the
  attempt sweeper. Source object bytes themselves belong to `server_receipt`.
- `runnable` is per tenant/bundle/phase. `dependency` and `privacyControl`
  represent current runtime conditions; configuration changes/restarts enter
  as world inputs. They do not claim any compiled feature is deployed.
- `revision`, `award`, `selection`, `effectsTerminal`, `paid`, `finishGranted`
  and `heldBefore` are the coarse settlement interface. `finishable` requires
  persisted selection and terminal progress; it does not mean a phase commit
  has already happened. Detailed index/ledger/instrument/payout variables belong
  to `server_settlement`.
- Frames project the latest event across all runs into nine internal panels.
  Controls represent independent reviewer/admin API operations, not navigation
  restrictions or proposed UI buttons. Eligibility is still guarded per run.

## Exact source anchors and transition correspondence

Paths and line anchors are pinned to the baseline above.

| Actions / state | Source anchor | Source behavior represented |
| --- | --- | --- |
| `TheReceiptCommitsItsAdmission` | `crates/trace-commons-server/src/versioned_pipeline.rs:11385-11649`, `8376-8401`, `13835-13845` | Admission is synchronous; Reject creates Complete/none, Admit or Quarantine creates Pending/Review. Worker explicitly refuses asynchronous Admission. |
| Immutable bundle / `runnable` | `crates/trace-commons-server/src/versioned_pipeline.rs:2158-2254`, `2395-2530`, `11293-11360` | Registration rejects a reused id with different package bytes. Bound bundle is loaded per run, never replaced by the active selection; mutable policy flags can suspend it. |
| `TheWorkerClaimsOrReclaimsTheRun`, `TheExpiredExhaustedLeaseIsSwept` | `crates/trace-commons-server/src/versioned_pipeline.rs:2633-2803`, `2854-2901` | Claims select due Pending/Retry or expired Leased work, under skip-locked row locks, increment attempt and issue a new token. Exhausted expired work is swept, with finish-grant/rehold exceptions before failure. Selection ordering/skip-lock scheduling are coarse nondeterministic choices. |
| `TheWorkerLeaseExpires`, `TheWorkerRenewsWithinItsCap` | `crates/trace-commons-server/src/versioned_pipeline.rs:983-1017`, `2946-3024`, `10291-10344` | Database-clock expiry; renewal is a best-effort background helper bounded at four lease durations. It cannot substitute for a phase's commit fence. |
| `TheStaleWorkerReturns` | `crates/trace-commons-server/src/versioned_pipeline.rs:7149-7180`, `13345-13363` | Expired same-token response refunds claim and schedules Retry using a token-only fence. Reclaimed token/phase transition/terminal state means no mutation. |
| Attempt stage/publish/sweep actions | `crates/trace-commons-server/src/versioned_pipeline.rs:7695-7801`, `12256-12552`, `17686-17701` | Per run/lease-token objects are staged before publication; cleanup becomes due after maximum lease-renewal bound plus commit margin. Sweep owns only staged orphan artifacts. Object refs recorded atomically at commit belong to withdrawal. |
| `ThePrivacyPassCommits` | `crates/trace-commons-server/src/versioned_pipeline.rs:12715-12851`, `3336-3495` | Classifier works on stored deterministic source, validates/merges risk basis, stores a separate pass artifact and its source/output hash linkage. Escalation moves received submission to quarantined. Pass record has a lease/submission fence but **no policy lock**, because it is a server control. |
| `TheWorkerParksForHumanReview` | `crates/trace-commons-server/src/versioned_pipeline.rs:13845-13896`, `7109-7143` | Escalated pass needs a post-pass assessment regardless of Admission; Admission quarantine also needs a human. Park AwaitingReview, refund claim, no hourly retry while waiting. |
| `ReviewerClaimsTheHold`, `TheHumanReviewLeaseExpires` | `crates/trace-commons-server/src/versioned_pipeline.rs:137-163`, `3590-3690` | Idle quarantined/escalated Review or Score hold; current receipts need a recorded pass. Human exclusive claim has separate identity and maximum 30-minute real lease. Same reviewer can renew/reclaim; another may take over after expiry. |
| `ReviewerApprovesTheHold`, `ReviewerRejectsTheHold`, `TheStaleHumanAssessmentIsRefused` | `crates/trace-commons-server/src/versioned_pipeline.rs:3758-3960` | Matching principal/token/live claim, eligible idle phase and current operability are rechecked under locks. Approval resolves all phase hold reasons; phase-bound assessment and claim deletion commit with unparking to Pending/due now. Worker-leased state refuses late assessments. Unresolved-reason failures are a coarse rejected-request interface; approval events represent fully resolved reasons. |
| `TheReviewApprovesAndCommits`, `TheReviewRejectsAndCommits` | `crates/trace-commons-server/src/versioned_pipeline.rs:3028-3304`, `13845-14018` | Review uses privacy-pass bytes. Escalated Admission-Admit plus human Reject is rejected by server without policy call. Approval stores a separately staged approved revision and sets submission accepted, advances Score and resets attempts; rejection ends Complete/none, rejected. |
| `TheScorePolicyHoldsTheTrace` | `crates/trace-commons-server/src/versioned_pipeline.rs:7024-7094`, `14315-14370` | Score hold parks AwaitingReview and refunds claim before any award/settlement is seeded. Unpublished prestaging rows are deleted under the same lease fence. |
| `TheScoreCommits`, repeated hold branch of `TheDeterministicRefusalEndsOrHoldsTheRun` | `crates/trace-commons-server/src/versioned_pipeline.rs:14121-14234`, `14315-14370`, `4223-4478` | Human Reject yields zero Score via server; Approve is fed to the pinned policy; repeated hold after approval fails. Commit stores optional index/neighbor artifacts, outcomes and award legs, advances Settle with reset attempts. |
| `TheDependencyOrPolicySuspensionReleasesTheAttempt` | `crates/trace-commons-server/src/versioned_pipeline.rs:6916-6963`, `13321-13344`, `13463-13475`, `13534-13549` | Missing/deactivated bound controls, bundle-store outage, missing privacy/settlement adapter/cap are uncharged suspension with phase-age backoff. Delay clamps to 1 s..1 h, no terminal retry budget for suspension. |
| `TheTransientOutageOrScoreLockBusyReleasesTheAttempt` | `crates/trace-commons-server/src/versioned_pipeline.rs:6965-7022`, `13364-13401`, `13476-13531` | Transient DB/authority/policy/outage failures are uncharged; compatibility Score lock busy is also uncharged but uses its distinct fixed delay and removes unpublished rows. Exact delay classes are merged to `due`. |
| `TheOperationalFailureChargesTheAttempt` | `crates/trace-commons-server/src/versioned_pipeline.rs:6772-6914`, `13550-13576`, `13626-13643` | Permanent policy/general operational/integrity/classifier failure consumes bounded attempts. Generic 50-ms doubling, integrity hourly and classifier 30-s doubling are distinct source timings merged to a not-due/due relation. Exhausted classification retains its label and held submission status; generic failure rejects a still-received submission. |
| `TheDeterministicRefusalEndsOrHoldsTheRun` | `crates/trace-commons-server/src/versioned_pipeline.rs:13403-13461`, `6689-6741`, `7990-8040` | Inoperable Review/Score, repeated Score hold, index conflict/unembeddable input and deterministic gate evidence failures terminate rather than spending ordinary retries. Error-label classes are coarse. A finishable paid Settle with gate conflict/incomplete evidence, or a previously held paid run, parks AwaitingOperator instead. |
| `TheSettleSelectionAndTerminalEffectsAreRecorded`, `TheSettleCommitsFromProgress` | `crates/trace-commons-server/src/versioned_pipeline.rs:14643-14747`, `15499-15691`, `215-260` | Persisted selection and terminal instrument/index progress precede commit. Detailed dispatch, external success/local failure, credit holds and payout confirmation are delegated to settlement; this boundary does not expand those effects or assert atomic external payment. |
| Finish grant, rehold, `OperatorReleasesThePaidRun`, `OperatorFailsThePaidRun` | `crates/trace-commons-server/src/versioned_pipeline.rs:2639-2707`, `6630-6685`, `6843-6905`, `7990-8040` | Exhausted finishable work gets one automatic commit-only attempt. Finishability allows no paid leg. Paid permanent refusal holds. Operator release gives one more attempt and records finish grant; failure/exhaustion after prior hold reholds, while explicit operator Fail permits failed-run cleanup. |
| `TheSubmissionBecomesInoperable`, `TheInoperableHumanHoldIsReleased` | `crates/trace-commons-server/src/versioned_pipeline.rs:3650-3661`, `3842-3851`, `4062-4126`, `13435-13462` | Withdrawal/revocation/expiry/purge may happen during every phase/hold. Inoperable parked work is released so Review/Score can fail under their permanent guard. Settle follows its own forfeit/complete rules rather than the Review/Score failure branch. |
| `TheRuntimeDependencyOrPrivacyControlChanges` | `crates/trace-commons-ingest/src/bin/trace_commons_ingest_internal/production_assembly.rs:42-108`, `117-145`; `crates/trace-commons-server/src/versioned_pipeline_authority.rs:16-45` | Configured runtime/controls are explicit environmental input, not evidence of live activation. Production selection requires feature + gate compatibility; default unset runtime is None outside this model's starting receipt boundary. |

## Composition boundary events

1. `TheReceiptCommitsItsAdmission` consumes
   `server_receipt.TheFinalReceiptTransactionRuns` with Created. It projects
   tenant, pinned bundle, Admission outcome and initial submission/run state.
   Actual receipt key/byte/principal/object identity remains in receipt model;
   processing does not choose new bytes or a new bundle.
2. `ThePrivacyPassCommits`, Review commit actions and human assessment actions
   own the processing/hold fields and observable submission states. World
   withdrawal input composes with the lifecycle model's current operability.
3. `TheScoreCommits` produces the settlement boundary: immutable approved
   revision, zero/nonzero award class and persisted Score/index-command result.
   `TheSettleSelectionAndTerminalEffectsAreRecorded` projects detailed
   settlement's selection, all-effects-terminal and paid-leg facts. It is a
   **coarse interface**, not the body of an external dispatch.
4. `TheSettleCommitsFromProgress`, AwaitingOperator and failed-run actions feed
   status/credit/index read-back in composition. Physical index contents,
   logical eligibility, invalidation delivery, ledger/outbox and both payout
   systems must remain separately owned by the settlement/lifecycle models.
5. `OperatorSuspendsOrResumesThePolicy` affects a phase's bound policy without
   migrating the run; receipt activation changes only later run selection.

## Coarse behavior and known omissions

The model covers the current privacy-required receipt population. Pre-V117
privacy-pass-optional runs and assessment timestamp replacement paths require
additional initialization/transitions before making claims about historical
backlogs. Cryptographic/hash validation and policy quality are symbolic input
classes. Source integrity/read errors, input-unembeddable and deterministic
index conflicts share failure classes rather than complete label vocabularies.
A rejected approval with unresolved reasons does not mutate durable state and
is documented as a coarse API refusal rather than a separate action.

Score's command/neighbor object pair is merged; optional artifacts can be absent
on valid outputs. The model includes publication and staging but does not prove
hash consistency, physical index writes, tenant Score-lock serialization or
unapplied-command reader correctness. Stale worker phase results are fenced;
an out-of-process service crash is represented by lease expiry and retained
artifact/request state, not a host/process lifecycle model. Cleanup success
and retries are possibilities, with no fairness or eventual-cleanup assumption.

`TheSettleSelectionAndTerminalEffectsAreRecorded` collapses all terminal
instrument progress into one environment boundary. Its `paid` input only
represents completed paid progress, never an unconfirmed external request;
partial legs, suspension while dispatch is in flight, credit account authority,
credit-rule selection, and detailed index progress are covered by the separate
settlement model. `submissionStatus` is a logical state class; inoperability
keeps prior status here and does not expand lifecycle tombstones/purged rows.
No result from these separate abstractions proves their composition.

The reduced attempt cap, two runs, finite artifact capacity and symbolic time
are declared bounds. Artifact slot availability can serialize extensive orphan
production until sweeping frees a slot; the model makes no claim about disk,
queue or PostgreSQL capacity. Identity tokens can repeat only once no modeled
observer/artifact can confuse the old and new identities. All promises remain
pending owner selection.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees; owner promise selection and TLC checks remain pending.
