# Abstract composition

Source baseline: `8218cc8ae408eb9c2c8a16d5768d9f6e0f7f83d6`.
This is a standalone, finite **abstract composition**, not an import-based
product of the detailed modules. It connects selected manual contribution
boundaries. Matching actions and the correspondence in [composition.md](../composition.md)
are review aids; no simulation relation or refinement has been established.
Separate domain checks do not establish the behavior of this composition, and
this composition does not establish every detailed transition.

All frames are internal composition panels (`kind: server`). They project
client knowledge alongside server boundaries and are not replicas of native
screens or proposed app UI. Their human-readable controls drive simulation
macros; for example, `Change account` includes a complete account-change
ceremony, and `Approve preview` abstracts the native approval action. Actual
client copy and intermediate screens belong to the client domain frames.
The frame prioritizes visible history rather than current server state: a
withdrawn or credited server trace can remain visually pending until reread.
Closing the window hides the panel without stopping daemon/server actions.

## State and finite bounds

The sole variable is `s`; `vars` exposes it. The model has two source/trace
slots, two account identities, two device identities, two worker identities,
two immutable bundle labels, and one fixed tenant. The selected slot is
independent from in-flight upload, worker and history work. Each source has
one permitted content change (`revision` 0 to 1); consent is one of two
fingerprint classes. Clock relations, retries and policy availability are
finite categorical inputs, not elapsed wall-clock time. There is no unbounded
counter or fairness assumption.

Within each trace record:

- `revision`, `consent`, `preview`, `pinRevision`, `pinConsent`, `pinAccount`,
  `pinDevice`, `pinAvailable` separate source identity, input fingerprint and
  readable stored approval. Trace slot is also its stable submission/key
  identity; new keys for the same source and cross-key conflicts are omitted.
- `sent`, `requestAccount`, `requestDevice`, `staged`, `objectPresent`, `orphan`,
  `receipt`, `receiptAccount`, `receiptDevice`, `route`, `localReceipt` separate
  request transport, attempt storage, durable acceptance and client receipt.
- `admission`, `submissionStatus`, `phase`, `runState`, `worker`, `expiredWorker`, `privacy`,
  `reviewApproved`, `reviewAssessed`, `scoreReviewed` distinguish synchronous Admission,
  worker phases, privacy processing, human holds and expired work. The durable
  submission verdict is independent of run progress: an accepted submission
  stays accepted through a Score hold or later processing failure; an existing
  quarantined verdict is not rewritten by retry exhaustion. Only live received
  submissions move to rejected for the modeled generic exhausted failure.
  A human assessment resolves a hold without rewriting immutable Admission.
- `indexCommand`, `indexWriteInFlight`, `indexPresent`, `eligible` distinguish
  persisted inclusion, external writes, physical presence and logical use.
- `instrumentKind`, `settleGuardRead`, `settleGuardOperable` preserve the pass
  snapshot of operability. `instrumentInFlight`, `instrumentDispatched`,
  `instrumentResultReady`, `instrumentAuthorityCurrent`, `instrumentExternalAccepted`, `instrumentRecorded`,
  `instrumentForfeited`, `instrumentUnreconciled` distinguish a selected
  instrument, possible external effect, matching reply, persisted result and
  forfeiture without pretending an ambiguous external effect was undone. The
  captured `instrumentAuthorityCurrent` flag is invalidated by lease expiry or
  release and is never restored by a replacement worker claim. An old callback
  may still reveal an external effect, but cannot record a receipt under the
  replacement lease or move that worker to retry. A discarded stale reply
  leaves the effect uncertainty intact for a later same-operation retry.
  `earned` is a 0/1 internal-credit category; `pipelineNear` and
  `nearExternalAccepted` track the separate pipeline NEAR outbox.
- `withdrawRequest`, `withdrawLocal`, `withdrawn`, `cleanupDue` separate an account-authenticated
  request, its local response, logical exclusion and later physical index cleanup.
- `cached`, `cachedCredit`, `cacheStale`, `visible`, `visibleCredit` separate
  daemon history from app knowledge.

The account-level `accountRail` map has independent `accrued`, `linked`,
`state`, `externalAccepted` fields. Its earned/referral stream is represented
by one independent accrual event. It is not derived directly from the selected
trace's `earned`; real accrual queries/batching/eligibility remain detailed
settlement behavior. `pipelineDispatchAllowed` coarsens the fresh Settle-policy permission used
for instrument/ledger/final commit and pipeline NEAR dispatch. `accountRailEnabled` and
`accountRailAllowed` are environment inputs, not live deployment assertions.

`app`, `window` and `daemon` separate process lifetime from window closure.
`accountSession` authorizes account withdrawal; `enrolled` and
`deviceAuthority` represent upload credentials. `allowance` coarsens the
pre-send/admission authority and allowance checks; it does not model byte
quota accounting. `historyRead`, `readTrace`, `readAccount`, `readStatus`,
`readCredit` retain a captured IPC result across selection/account changes.

## Source correspondence and deliberate coarsening

| Source anchor | Modeled boundary | Limit |
| --- | --- | --- |
| `macos/Sources/TraceCommonsApp/AppDelegate.swift:76`, `:108`; `AppModel.swift:1687`, `:1924` | Window close versus confirmed quit, restart and daemon loss | Quit confirmation, bounded teardown, attached external daemon ownership and separate compute are detailed shell behavior; `YouQuitApp` is successful completion of an in-process quit. |
| `crates/trace-commons-contributor/src/daemon/uploader.rs:378`, `:433`, `:475`, `:527`, `:557` | Enrollment recheck, source rehash, approval fingerprint and pin availability before send | Manual preview branch only. Composition retry denotes a transient scheduling/transport condition over a retained approval; the same pin/input invalidation action covers approved and retry. Actual QueueState stays Approved on this source path (`daemon/mod.rs:1682`). Durable server receipt replay remains separate. Automatic grants, witness certificates, undo/keep/expiry, pacing and eight exact queue states remain in `client_contribution`. Account/device equality projects the fingerprint inputs rather than claiming the implementation compares integer IDs. |
| `daemon/approved_envelope.rs:1`; `daemon/uploader.rs:441` | Stored approval artifact identity | The digest is validated against stored bytes. `approved_envelope_for` then deliberately applies an approved contributor verdict to `outcome.task_success`; the composition collapses that bounded overlay. It makes no raw upload-equals-preview byte theorem. Cryptography/redaction quality are outside scope. |
| `crates/trace-commons-server/src/versioned_pipeline.rs:11805`, `:11982`, `:12199` | Staging, encrypted object publication, receipt commit/orphan cleanup | `TheServerStagesCiphertext` merges preparation/staging/publication; `TheServerCommitsReceiptAndAdmission` merges synchronous Admission computation with its final atomic commit. Attempt UUIDs, byte-conflict variants, reservation ledgers and final control rechecks remain in `server_receipt`. |
| `versioned_pipeline.rs:11982`, `:1302`, `:2633`, `:2946`, `:8034` | Receipt-bound bundle, worker lease/expiry/retry | Two versioned bundles only. Activation changes new staging; an existing receipt keeps `route`. Legacy, containment and rollback refusal detail remain in `server_receipt`. One expired-worker slot coarsens lease tokens, and one external-call slot serializes replacement adapter calls until an older callback is consumed; this does not model every concurrent adapter flight. External result authority is captured at dispatch and invalidated on expiry/release rather than inferred from any currently leased worker; all `TheExpiredWorkerReturns` events represent a result already rejected by the source's current-token check. Claims remain possible after withdrawal: Review/Score terminate inoperable, while Settle can forfeit open legs and finish from actual progress. Submission verdict is retained independently from failed/held run state; generic exhaustion is still possible after withdrawal and changes only a live received verdict; revoked/accepted/quarantined verdicts are preserved. Privacy-classification failure is not merged into this generic exhaustion event. |
| `versioned_pipeline.rs:3336`, `:3590`, `:3738`, `:7024` | Review-start privacy pass, Review hold and separate Score hold | Human reviewer claims, privacy attempt artifacts and stale assessment races are retained only in `server_pipeline`. Privacy-policy result is external input; human assessment macros represent successful source-authorized completion. |
| `versioned_pipeline.rs:4223`, `:4789`, `:6165`, `:13670` | Score/index command, persisted instrument selection, external effect and settlement completion | One chosen internal or external instrument coarsens the real multi-instrument set. Step 6 snapshots operability before its loop (`:14944`); a non-Trace-Credit adapter may start after a withdrawal races that read (`:15235`). Its matching receipt may be recorded after withdrawal (`:15396`) under that call's captured live lease (`update_settlement_on_tx:8208`, `ensure_current_lease:8593`), never a replacement worker's lease. Internal credit uses a fresh locked recheck. Recording a credit/result is separate from completing the run; paid effects survive later forfeiture/withdrawal, and ambiguous dispatched external effects retain an unreconciled marker. |
| `versioned_pipeline.rs:15848`, `:16679`, `:16719`, `:16758`; `versioned_pipeline_credit.rs:449` | Internal credit, pipeline NEAR send, local persistence failure and confirmation | A nondeterministic `payoutSelected` input coarsens eligibility/rail selection: earned credit alone need not create a NEAR payout. `ThePipelineNearConfirmationArrives` is allowed under dispatch suspension. The already-completed payout-eligible credit leg can be dispatched and confirmed after withdrawal; there is no fresh withdrawal veto in this outbox path. Mode/contract migration, holds, terminal chain failure and batching belong to `server_settlement`. |
| `crates/trace-commons-server/src/payout_worker.rs:1`, `:75`; `payout_pass.rs:592`, `:1052` | Independently accrued and linked account NEAR AI/referral payout | Link lifecycle, main/referral separate lines, rate epochs and receipt matching remain detailed. Composition `submitted/confirmed/failed` categories describe uncertain/persisted/reconciled outcome, not a claim that this provider exposes the pipeline NEAR chain-confirmation API. |
| `crates/trace-commons-contributor/src/daemon/withdraw.rs:1`, `:128`; `versioned_pipeline.rs:5446`, `:5744` | Account-authenticated withdrawal and previous-account boundary | Request captures owning account. Device authority is deliberately not the withdrawal grant. Web withdrawal is an independently authenticated owner event. Multiple submissions per source session, renewal/token rotation and refusal copy are detailed domain behavior. |
| `versioned_pipeline.rs:4932`, `:5233`, `:6003`, `:10788`, `:17778` | Logical exclusion, late index write, later cleanup/purge | Object purge action means retention/legal-hold checks have permitted it; those checks are not represented as a separate composition state. Withdrawal never sets `indexPresent` false by itself. Cleanup is not assumed to happen eventually. |
| `crates/trace-commons-contributor/src/daemon/history.rs:1`, `:36`, `:67`; `macos/Sources/TraceCommonsApp/AppModel.swift:2018`, `:3905`, `:4317` | Receipt status, server poll, cached IPC snapshot and later UI publication | `processing` receipt normalizes to submitted; it is not a verdict. Refresh requests a poll and reads cache, so may remain stale. `perform` has no client/account-generation completion guard: history snapshots may publish after account change or window closure while the app lives. `readAccount` records that fact, not a protective guard. |
| `AppModel.swift:978`, `:1453`, `:3930`, `:3984` | Distinct guarded balance/onboarding/detail operations | Not projected into composition history. Those request generations/scopes cannot be inferred from `perform`; detailed client services own them. A possible stale path here is a modeling/source-review lead, not a reproduced production defect. |

All source paths abbreviated as `daemon/...` above are under
`crates/trace-commons-contributor/src/`; `versioned_pipeline...` paths are under
`crates/trace-commons-server/src/`.

Additional omissions: first-run/navigation, native browser ceremonies,
managed tools/private inference, learning/Insights, exports, public interfaces,
legacy processing, production runtime qualification, exact worker accounting,
physical index recovery/rebuild, and detailed platform differences. Consult
the suite coverage inventory and domain notes rather than interpreting the
name `end_to_end` as complete product coverage.

## Verification

`quint typecheck docs/flows/trace-commons/models/end_to_end.qnt` passed with
Quint `0.33.0` after the initial model was written. The parent integration run
owns final typechecking, view/control checks and finite simulation evidence.
No invariant, temporal property, product promise, TLC run, or guarantee test
was added or executed by this model's author. Typechecking establishes static
well-formedness only. Simulation, when recorded by integration, samples paths;
it is not exhaustive exploration or a product proof.

## Integrated validation

Earlier typecheck-only status notes describe the model-writing stage. The later
[integrated validation report](../validation.md) records the suite-level
frame/control comparisons and sampled traces. Those checks do not establish
product guarantees; owner promise selection and TLC checks remain pending.
