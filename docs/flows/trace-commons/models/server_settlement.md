# Settlement and payout source mapping

Baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`. This domain models current
source behavior, with explicit finite abstractions. The owner selected P9 from
[promise candidates](../promise-candidates.md): retries must not create a second
internal credit, and payout retries retain their operation identity. `P9` is
registered in the TLC configuration. Typechecking alone is not behavioral proof.
The selected source files remain unchanged at `a2e0aa56608f8c403deff679f8b51e76ac2f5ca7`.

## Identities and state

`Runs={0,1}` represents two independent tenant/run identities. `Legs={0,1,2,3}`
is `(run * 2 + instrument)`: even legs are Trace Credit, odd legs are a second
external instrument. The operation key is the immutable leg identity, never an
attempt counter. `selected` stores immutable Settle selection presence;
`indexDone` is the projected result of the separate index domain; `liveLease`
is a valid-current-lease boundary input from `server_pipeline`. `leaseToken`
ranges over three finite token identities per run; `flightToken` captures the
call's actual attempt token. Reclaim requires an expired lease, changes its
token, and excludes any outstanding call's token. The third token permits
repeated reclaims without reusing an outstanding older token. No old reply can
become authoritative merely because `liveLease` becomes true again. Durable
adapter-result updates require `flightLeaseCurrent`; a stale result is discarded
without erasing its possible external effect. The replacement worker may retry
a still-dispatched operation using its unchanged operation key.

`resultReady` records an adapter response; `externalEffect` records an external
instrument effect only. Trace Credit receives an adapter receipt too, but its
actual local payment is `ledger`. `batch` and `near` are separate. Compatibility
`NoveltyUtility` credit can ledger without a batch; the graded and PipelineScore
families can batch. `audited` models delayed mirrored credit audit completion.
A duplicate can complete withheld without a ledger event. `leg` distinguishes
pending, dispatched, retry, complete, complete_withheld, mismatch, unreconciled
and forfeited. Terminal and ambiguous failures are not treated as paid credit.

A disjoint account payout namespace reuses numerical indices only as
`account * 2 + stream`: even is earned, odd referral. `accountLine`,
`accountEffect` and `accountConfirmed` never alias `leg`, `externalEffect`,
`ledger`, or the pipeline NEAR outbox. Two accounts each have one link and at
most one bounded line per stream. `accountHold` represents shared standing;
`streamHold` represents a single stream's error hold. `referralHold` is the
separate abuse trip. Finite per-pass budget is `0..2`; it represents zero-read
plus send versus pending-line recovery, not real amounts or rate epochs.

`nearContract` retains one of two stored contract identities; `nearMode` is
disabled/dry_run/http and `submittedMode` pins confirmation mode. Repeated
external calls reuse their operation identity. Effect sets are environmental
logical effects under these idempotent interfaces, not a proof that an actual
provider implements the contract. Calls may still fail or leave uncertain local
records. In-flight internal, pipeline NEAR, and account transfer slots are
independent, permitting cross-rail interleavings; one concurrent call per slot
is a bound, not the production concurrency limit.

## Source anchors and modeled transitions

Paths are relative to the repository root; all line anchors refer to the pinned
baseline above.

| Source anchor | Modeled behavior/actions |
| --- | --- |
| `crates/trace-commons-server/src/versioned_pipeline.rs:4223` and `:4789` | Score seeds per-award legs; `TheScoreCommitsTwoInstrumentAwards`, `TheSettleSelectionIsPersisted`. Stored selection is reused, not recomputed on retry. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:14643` | Settle validates stored Score/award/operation correspondence before effects and waits for index dispatch. Validation details are a boundary assumption; `TheIndexDispatchIsResolved` supplies its projection. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:14950` through `:15498` | Per-instrument dispatch and receipt handling, cap/config refusal, duplicate withholding and independent leg progress; `TheWorkerDispatchesAnInstrumentOperation`, `TheAdapterReturnsAReceipt`, `TheReturnedReceiptDoesNotAnswerTheOperation`. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:15848` and `:16000` | Internal ledger transaction rechecks submission and policy under locks; ledger, batch and completed leg commit together. `TheInternalCreditTransactionCommits`, `TheReturnedCreditFindsWithdrawal`, `TheInternalCreditTransactionIsSuspended`. Hold semantics differ for compatibility unbatched credit. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:874`, `:917`, `:13670` | Failure reconciliation and terminal-versus-retry dispatch rules. `TheFailurePathCannotReconcileAnExternalLeg` records uncertainty; a live retry can repeat an unreconciled operation. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:1828`, `:14950` | Inoperable submissions forfeit unfinished legs, including an already dispatched leg whose effect may have happened; completed legs are retained. `TheWorkerForfeitsAnUnfinishedLeg`. |
| `crates/trace-commons-server/src/versioned_pipeline_credit.rs:449`, `:456`, `:465`, `:472` | Deterministic event/source/batch/outbox identities and request-key separation. Numerical IDs abstract these hashes and UUIDs. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:4566`, `:4616` | Credit audit mirroring is separate from ledger commit: `TheCreditAuditIsMirrored`. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:16659`, `:16758` through `:17112` | Complete eligible Trace Credit legs pay even after later withdrawal/run failure. Preparation, submit, local recording, crash, explicit retry and chain confirmation are separate actions. Policy gates dispatch, not submitted confirmation. Stored contract/mode are retained; changed contract is refused; chain failure cannot be retried. |
| `crates/trace-commons-server/src/payout_worker.rs:1`, `:58`, `:138` | Only the configured `near_ai` account rail is constructed; disabled mode creates no worker; settlement notify and retry tick wake an enabled worker. `TheAccountWorkerRetryTickArrives` abstracts notify/tick; modes and deployment availability are inputs, not assertions of live enablement. |
| `crates/trace-commons-server/src/payout_pass.rs:147`, `:159`, `:1024` | Shared standing versus earned/referral stream holds; `TheAccountStandingChanges`, `TheCommittedAccountTransferNeedsAnOperator`, `TheReferralAbuseControlTrips`. |
| `crates/trace-commons-server/src/payout_pass.rs:679`, `:732`, `:787` | Preflight budget, accrual and referral isolation; `TheAccountAccrualPassFindsOwedCredit` abstracts accrual and reversals. |
| `crates/trace-commons-server/src/payout_pass.rs:1052`, `:1209` | Waiting/active/closing/closed link lifecycle; closing is represented at its reconciled boundary, not every zero-send/blocked-close detail. |
| `crates/trace-commons-server/src/payout_pass.rs:1388`, `:1535`, `:1672` | Standing, ceilings, stream-program totals and pending-line persistence. `TheAccountPayoutLineIsPersisted` assumes these route-specific eligibility checks succeeded; target arithmetic is omitted. |
| `crates/trace-commons-server/src/payout_pass.rs:1798`, `:1948` | Commit before local record, matching-receipt recovery, ended-line/unrecorded-receipt operator hold; `TheAccountRailCommitsTheTransfer`, `TheAccountSendCrashesBeforeLocalCommit`, `TheAccountPassRecoversTheMatchingReceipt`. |

The pass-level `settleGuard` is a saved operability snapshot. `guardRead` starts
empty, so the initialized false value cannot authorize dispatch or forfeiture.
Only `TheWorkerReadsTheSubmissionGuard` makes a snapshot valid. Selection,
lease expiry, lease reclaim and `TheWorkerEndsTheSettleAttempt` invalidate it.
`attempted` permits at most one dispatch per leg per pass; a new pass reads the
guard again before retrying. A valid snapshot can still become stale after its
read: no fresh operability check is invented at external dispatch. Source
`versioned_pipeline.rs:15235` explicitly permits a non-Trace-Credit adapter call
after withdrawal raced that read; the model preserves this through
`TheWorkerReadsTheSubmissionGuard`. Internal credit still rechecks the current
`operable` value under its transaction. `TheExternalInstrumentMayTakeEffectWithoutAReceipt`
followed by `TheAdapterReturnsUnavailable` represents an unavailable/ambiguous
adapter response whose effect may already have happened. Neither path is
silently converted into no effect.

The captured-token fence maps to
`crates/trace-commons-server/src/versioned_pipeline.rs:15857`
(`required_lease_token`) and `:15946` (`ensure_current_lease` before internal
credit writes), plus `:8208` and `:8593` for settlement updates/current-lease
validation. `TheStaleAdapterResultIsDiscarded` releases only the observed stale
in-flight slot, not durable progress or external-effect evidence.

## Composition boundaries

| Boundary | Detailed action/fields | Projection to composition |
| --- | --- | --- |
| Score awards durable | `TheScoreCommitsTwoInstrumentAwards`; `leg` pending | Score-complete boundary can begin settlement. |
| Immutable selection | `TheSettleSelectionIsPersisted`; `selected` | A retry retains selected membership/operations. |
| Index resolved | `TheIndexDispatchIsResolved`; `indexDone` | Inbound from lifecycle; no new index effect hidden here. |
| Withdrawal/expiry | `TheSubmissionBecomesInoperable`; `operable` | Inbound lifecycle event; existing completed legs survive. |
| Internal credit committed | `TheInternalCreditTransactionCommits`; `ledger`, `batch`, `leg` | Credit read-back is supported by a committed ledger event. |
| Pipeline payout submitted/confirmed | `ThePipelineNearSubmitIsRecorded`, `ThePipelineNearTransactionIsConfirmed`; `near` | Dispatch and confirmation remain distinct. |
| Account owed / transfer / receipt | `TheAccountAccrualPassFindsOwedCredit`, `TheAccountRailCommitsTheTransfer`, `TheAccountReceiptIsRecorded` | Separate account-rail state; cannot stand for pipeline NEAR issuance. |
| Client feedback | `ledger`, `audited`, `near`, `accountConfirmed` | Composition chooses which committed server observation is read back; a send alone is insufficient. |

## Explicit coarse logic and omissions

The bound includes one positive award per instrument, one immutable Score
outcome per run, one batch/outbox line per run and one account link/line per
stream. It omits numeric amounts, cap arithmetic/overflow, batch line splitting,
ledger reversal/accrual formulas, detailed NoveltyUtility runtime filters,
provider registration/qualification, recipient sealing/opening, receipt hash
collision or reuse across different legs, account/link successor migration,
carry release, send-window timestamps, referral shared-versus-dedicated
credential capacity, and provider-specific HTTP refusal classes. A mismatch is
terminal at the leg boundary but the model does not reproduce charged attempt
exhaustion; `server_pipeline` owns operational attempts/run failure. It does not
model transaction internals or prove the adapter contract.

Automatic account-rail modes are initially live as an exploration scenario,
not a claim about deployment defaults (source defaults disabled). Pipeline
configuration may change between calls; no deployment state is inferred. A
worker/receipt result or operator intervention must actually occur for progress;
no fairness or eventual-payment assumption is added. Server panels are internal
review frames and their controls are sentence-named model events, not proposed
application buttons.

## Selected safety observation

`creditCommits` counts actual modeled internal ledger commits per operation,
saturating at two so a second commit remains a visible violation. `P9` requires
at most one and requires each ledger member to have a recorded commit. The
instrument, pipeline NEAR and account-rail first/last key observations record
dispatch and receipt-query identity in separate namespaces. Pipeline NEAR also
captures the stored contract at first dispatch. Retry observations must retain
these identities, including after an uncertain external outcome.

These observations add fields to `s`; no action guard depends on them. Removing
them projects every transition back to the previous model. They do not add a
retry restriction. Numeric immutable IDs abstract source-derived keys; collision
resistance and the external provider's idempotency contract remain assumptions.
See [proof notes](../proofs/credits-exports-notes.md) for bounds and limitations.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees. P9 is now selected; its current checker outcome is recorded
separately in the proof evidence.
