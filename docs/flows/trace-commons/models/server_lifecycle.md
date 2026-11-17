# Index, access, withdrawal, retention and export mapping

Baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`. These are bounded current
source transitions. The owner selected P10 (revocation/expiry prevents a new
export completion) and P11 (tenant isolation, export portion) from
[promise candidates](../promise-candidates.md). Both are registered in the TLC
configuration. The selected export source file remains unchanged at
`a2e0aa56608f8c403deff679f8b51e76ac2f5ca7`.

## Finite identities and independent state

Three immutable submissions use IDs `0,1,2`. IDs 0 and 1 belong to tenant/owner
0 and initially map to source session 0; ID 2 belongs to tenant/owner 1 and
session 1. `actor` and auth-derived `tenant` each range over `{0,1}`, including
mismatches that cannot authorize withdrawal. A missing account source-session
mapping falls back to one-submission withdrawal. Session tombstones are distinct
from submitted-version tombstones. Submissions enter this domain already
accepted; receipt and phase approval are owned by other models.

`status`, database-clock `expired`, `legalHold`, approved `payload`, sealed
`command`, logical `member`, `write`, physical index entries, and `completeRun`
are independent. `live(i)` requires accepted status and unexpired database-clock
evidence. A hold does not extend live eligibility. `readEligible` records a
consumer read projection of physical entries whose logical membership and live
submission still permit use. Its cached last-read result is not updated
retroactively on withdrawal.

One physical write can be in flight, from Settle or rebuild; `locksHeld` is
separate from `late`. Releasing row locks on deadline/lost DB session does not
cancel blocking index work. `fence` is a tenant set with an unexpired rebuild
fence. `invalidation` and `due` separate durable removal state and scheduling.
One claim is observed, with fresh/stale token relation; this abstracts lease
identity, claim expiry and retry backoff without encoding seconds. Two tenant
identities keep a fence or process routing change scoped. The production
rebuild refusal checks only this process's routing/draining configuration;
`processActive` does not assert global quiescence.

Two export IDs represent two immutable idempotency keys. A snapshot has state,
item IDs, owner and tenant; `delivered` survives subsequent invalidation.
`allowedUse` and `exportGrant` are validated authorization evidence, not raw
tokens. Snapshot creation can select zero to three eligible IDs (the tenant
partition limits the effective item count to two). Export operation context may
change between creation and completion and must still name the stored owner
and tenant.

## Source anchors

| Source anchor, relative to repository root | Transitions and observations |
| --- | --- |
| `crates/trace-commons-server/src/versioned_pipeline.rs:80` | `live_submission_sql!` and accepted predicate exclude withdrawn/revoked/purged and database-clock-expired rows. `TheDatabaseClockPassesExpiry` changes eligibility before maintenance updates status. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:4223`, `:4513`, `:4789`, `:4869` | Sealed Score command, persisted Settle membership and independent write progress. `TheScoreSealsAnIndexCommand`, `TheSettleSelectionIncludesARevision`. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:14643`, `:17888` | Settle's locked dispatch, deadline, partial/uncertain write, complete/cancelled/failed states; `TheSettleIndexCallStarts`, `TheIndexDeadlineOrDatabaseSessionEnds`, `TheIndexWriteReturnsBeforeTheDeadline`. One-entry abstraction represents a whole sealed command. |
| `crates/trace-commons-server/src/versioned_pipeline_index.rs:242`, `:298` | Same-key/same-content idempotent upsert, content conflict, idempotent revision invalidation. `TheIndexWriteReportsAContentConflict`, `TheInvalidationWorkerRemovesPhysicalEntries`. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:13003`, `:13157`, `:5005` | Rebuild commits its fence before taking run/submission locks, rechecks rebuildability and authoritative command, skips invalidated/inoperable runs, honors deadline, and does not clear a fence while a write remains in flight. `TheRebuildCommitsItsFenceAndStartsAWrite`, `TheRebuildCannotPersistItsFence`, `TheFenceMarginExpires`. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:5070`, `:5113`, `:5190`, `:5233`, `:5273` | Due invalidation claims are tenant-fenced; completion is claim-fenced; index unavailable/uncertain leaves uncharged retry. A late write reopens only pending/complete removal, not a terminal failed row. `TheLateIndexWriteReturns`, `TheIndexIsUnavailableDuringRemoval`, `TheInvalidationClaimExpires`. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:5446` | Owner check before and after locks; mapped account source-session withdrawal covers all that session's owned versions; otherwise only requested submission. Session, runs and submissions are locked in that order. `YouWithdrawAnOwnedSubmission`, `TheSourceSessionBindingIsAbsent`, `TheWithdrawalRequestNamesAnotherOwner`. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:5671`, `:5725`, `:5744`, `:5790` | Withdrawal/revocation cancel pending membership, queue physical cleanup for writes that may have happened, release parked review, and recover incomplete follow-ups. The withdrawal transaction does not change completed settlement legs; settlement model owns later forfeiture. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:6003`, `:8772`, `:8909` | Retention follow-up invalidates exports, queues deletion on purge and index removal; withdrawal/revocation invalidate derived records and export membership. Snapshot invalidation preserves historical completion time. |
| `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:75724`, `:75836`, `:76059` | DB-only pipeline submissions participate in maintenance; legal hold prevents maintenance marking expiry and purge. Purge queues payload deletion; a row's purged status is separate from physical deletion. `TheRetentionWorkerMarksExpiry`, `TheRetentionWorkerMarksPurge`. |
| `crates/trace-commons-ingest/src/bin/trace-commons-ingest.rs:24302`, `:68070` | Source-session tail reconciliation receives held policy IDs; the queued object-ref deletion helper is a separate path. The model does not impose a new blanket legal-hold gate on all queued propagation. `ThePropagationWorkerDeletesPayloads` means an already-authorized queued item executes. |
| `crates/trace-commons-server/src/versioned_pipeline_product.rs:457`, `:740` | Export selection uses completed runs with approved live object refs, permitted uses, consent allowlists and privacy filter; request key serializes selection and checks stored requester/use/purpose metadata on replay. `YouRequestAnExportSnapshot`, `TheExportIdempotencyKeyIsReplayed`, `TheExportKeyNamesConflictingMetadata`. |
| `crates/trace-commons-server/src/versioned_pipeline_product.rs:952` | Completion checks requester ownership, invalidation and live/permitted-use source eligibility under submission locks, then commits manifest/items/snapshot together. Already complete is idempotent. `YouCompleteYourExportSnapshot`, `TheExportCompletionRevalidationRefusesDelivery`. |

## Composition correspondence

| Boundary | Detailed owner/action and fields | Composition meaning |
| --- | --- | --- |
| Approved/sealed revision available | Inbound `TheScoreSealsAnIndexCommand`; `command` | Processing has committed an authoritative command. |
| Settle index decision/finish | `TheSettleSelectionIncludesARevision`, `TheIndexWriteReturnsBeforeTheDeadline`; `member`, `write` | Settle model sees index resolved only after its write, exclude or cancellation boundary. |
| Withdrawal durable | `YouWithdrawAnOwnedSubmission`; `status`, `withdrawnSessions` | Receipt history/status becomes inoperable; mapped source-session uploads are refused. |
| Logical access removed | `live`, `member`; `TheConsumerReadsLogicalIndexEligibility` | Absence from consumer eligibility is distinct from physical deletion. |
| Physical cleanup | `TheInvalidationWorkerRemovesPhysicalEntries`, `ThePropagationWorkerDeletesPayloads`; `physical`, `payload`, `propagation` | Cleanup completion can lag withdrawal and can be reopened by a late index call. |
| Export ready/delivered | `YouRequestAnExportSnapshot`, `YouCompleteYourExportSnapshot`; `snapshot`, `snapshotItems`, `delivered` | Created snapshot is not a delivered manifest. Revalidation happens at delivery. |
| Settlement notification | `status`/`expired` changes | Inbound `TheSubmissionBecomesInoperable` in settlement; completed legs are not clawed back. |

## Coarse and omitted behavior

Each index command is one physical entry. One call and one cleanup claim are
observed at a time, although worker identities and leases are modeled in the
processing domain. Deadline/fence/backoff relations replace durations. The model
omits ANN vectors, content/hash computation, multiple revisions per submission,
partial command-entry counts, exact lock wait scheduling, terminal malformed
invalidation claims, rebuild command-set reports, and object-store ciphertext.
A physical index-loss action represents restored/missing derived state, not an
allowed arbitrary consumer mutation.

Retention models one successful authorized maintenance boundary and one purge
cutoff relation; dry-run counts, backfill mode, audit mirroring failure and every
legacy file/database reconciliation step remain coarse service interfaces.
Legal holds are operator configuration inputs. Queued deletion success is not a
proof of external downstream deletion; only the bounded local payload flag is
removed. External export consumers and their later use are outside the model.

Export scope allowlists/privacy filters are successful/failing evidence through
`allowedUse` and `exportGrant`; no ABAC interpreter or policy mutation semantics
are invented. The illustrative `TheAllowedUseEvidenceChanges` is an external
revalidation input, not a new server mutation route. The model collapses
source-object invalidation and derived cleanup into the recorded follow-up
state, and does not reproduce every account previous-owner merge case. Snapshot
response-loss and malformed manifest bytes are in the coarse service boundary.

These server frames explain state during review. Their sentence action controls
are model events, not proposed product UI.

## Selected safety observations

`P10` observes live and permitted-use eligibility when a new export completes.
`P11` observes requester/tenant authorization at that same transition and checks
that every stored snapshot contains only its tenant's objects. The receipt
portion of P11 lives in `server_receipt`. Completion facts are saved in
`completionChecked`, `eligibleAtCompletion` and
`tenantAuthorizedAtCompletion`; later withdrawal, expiry or an authenticated
context change does not rewrite the historical fact. A previously delivered
export is not recalled by these promises.

No observer is read by an action guard or changes an original state assignment;
erasing these fields recovers the previous transition relation. In particular,
they do not suppress an export attempt after revocation. They observe the
existing completion revalidation. See [proof notes](../proofs/credits-exports-notes.md)
for source-locking semantics, bounds and excluded guarantees.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees. P10 and the export portion of P11 are now selected; current
checker outcomes are recorded separately in the proof evidence.
