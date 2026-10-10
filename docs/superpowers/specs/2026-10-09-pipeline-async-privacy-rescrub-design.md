# Pipeline privacy rescrub out of the receipt

Status: revised after owner review, 2026-10-09; escalation rule amended by
owner decision 2026-10-10 (see "Decision 2026-10-10: PII found and removed
is accepted" under "Escalation"). The implementation plan is
`docs/superpowers/plans/2026-10-09-pipeline-async-privacy-rescrub.md`; see
"Corrections from the implementation plan" at the end.
Scope: `trace-commons-server` only. No client or protocol change. One
migration, V117 (see "What changes in storage").

## Problem

`PipelineService::submit` (versioned_pipeline.rs, step 3, VP:9081-9116 on
`main` at 3c7d4a239) runs the privacy boundary's `rescrub` inside the HTTP
upload request, before it stages the run. On the pilot the boundary is
`ClassifierRedactorPipelinePrivacyBoundary`. It runs the deterministic
redactor, then the NEAR AI prose-PII classifier over every window of the
trace. The protocol notes put one round trip at about 4.5 s, and throughput
at windows times round trip.

Stage 3 on the pilot (2026-10-09, run 20261009a, build 3c4581ba) measured
the cost at the proxy:

| Trace | Legacy path | Pipeline path |
|---|---|---|
| `long-chunk-capped`, 372 KB request | 200 in 0.77 s | 3 attempts, each dropped by the client at 29.9 s; never stored |
| the five small traces, 3.6-8 KB | 200 in 0.3-0.5 s | 200 in 3-13 s |

The client gives up at about 30 s, and the server's handler is dropped
with the connection, so the trace is lost. Any real session of that size
fails the same way. Every pipeline upload is also 5-15x slower than legacy.

The contracts bear on the ordering. SUB-005
(2026-09-11-versioned-pipeline-behavioral-contracts.md:386) says, among
other things:

> - Admission MUST use bounded local work.
> - Admission MUST own synchronous privacy-risk handling.

The design spec's §4 says the same: Admission "runs in the request path. It
uses only bounded local work so that it can complete before the response",
and it "can quarantine a trace because of synchronous PII risk".

My reading is that the synchronous privacy handling Admission owns is the
bounded, local kind. That is the deterministic redactor and the risk it
derives, which Admission keeps under this proposal. A network classifier
with a 4.5 s round trip per window is neither bounded nor local, so it does
not belong in the request path. If the intent was the opposite, that the
classifier's verdict must be known before the receipt commits, then this
proposal changes a contract rather than restoring one. The owner confirmed
the first reading (owner decision 0).

The design spec's receipt path (2026-09-09-versioned-pipeline-design.md
§6) lists seven steps and has no scrub step. The receipt-time rescrub came
in later, with the activation work. pipeline-activation.md, "Authority and
privacy at the receipt", describes it as current behaviour; it is not a
contract.

## What legacy does

`main`'s handler runs the deterministic `rescrub_trace_envelope`
synchronously, with no classifier. It stores that envelope encrypted, under
status `awaiting_pii_backstop`, and answers the client at once. The client
sees "Held pending an automated privacy backstop verdict; not yet in the
corpus." Then `process_one_pii_backstop` runs the classifier
asynchronously. It stores a separate `rescrubbed-envelope` object and moves
the record to Accepted or Quarantined. A classifier error leaves the item
held and counts an attempt.

## Recommendation: a server-owned privacy pass at the start of Review

### Receipt (synchronous)

- Steps 1 and 2 are unchanged. The boundary must still exist
  (`privacy_control_missing`), and replay, conflict, tombstone, quota and
  routing refusals still come before anything else.
- Step 3 runs only the deterministic part: `rescrub_trace_envelope`, the
  bounded, local redactor that the classifier boundary already runs first.
  Its conditions merge into `residual_risk_basis` exactly as today. The
  legacy handler has already run the same redactor before it routes the
  upload to the pipeline (ING:15317), so this is a second,
  defence-in-depth pass, not new work. Its conditions may already be in
  `request.residual_risk_basis`.
- Steps 4-7 are unchanged. The stored source object is the envelope after
  the deterministic rescrub, encrypted as today, and Admission's
  `privacy_risk` comes from it.

The client keeps the same receipt: `status: "processing"`, "Accepted for
pipeline processing." Replay is unchanged: `request_content_hash` is the hash
of the raw request bytes, the receipt never calls the classifier, and so a
replay never does either.

The trait changes to match. `PipelinePrivacyBoundary` (VPA:86) has one
`rescrub` today, and `ClassifierRedactorPipelinePrivacyBoundary` runs the
deterministic redactor and then the classifier inside it (VPA:149-168). It
splits into:
- `rescrub_deterministic(&mut envelope)`, called at the receipt;
- `rescrub_classifier(&mut envelope)`, called by the pass below.

`DeterministicPipelinePrivacyBoundary` implements the second as a no-op.
`classifies_prose_pii()` and `production_qualified()` keep their meaning,
so ingest still refuses a runtime whose boundary does not classify while
`TRACE_COMMONS_REQUIRE_PRIVACY_FILTER` is set.

### Privacy pass (asynchronous, server-owned)

When `process_claimed` dispatches a run whose next phase is Review, and the
run has no recorded privacy pass, the server does the following before it
calls the bundle's Review policy:

1. Load the source bytes (`load_source_bytes`, as Review does today).
2. Run the boundary's classifier rescrub on them (`rescrub_classifier`).
3. Store the result as a new encrypted object. Its object key is per
   attempt (`pipeline_attempt_object_id("privacy-pass", run_id,
   lease_token)`), staged in `pipeline_attempt_artifacts` like Review's
   approved object, so an attempt that never commits is deleted by the
   attempt sweep. Its object ref id is derived from the run id alone. A
   content-addressed key is not possible: every write encrypts with a fresh
   salt and nonce (ruling FR1). REV-002's "stored by content hash" is met by
   recording the plaintext hash. In one transaction, record the object ref,
   the hashes of the pass's input and output, the merged residual-risk
   conditions and the outcome on the run, and write the post-classifier
   `privacy_risk`, `residual_risk_basis`, `redaction_counts` and
   `redaction_pipeline_version` back to the submission row.
4. Hand the Review policy that object's bytes as `source_artifact`, in place
   of the raw source.

Review's approved content is then derived from scrubbed content. Score
reads only approved content (`load_approved_bytes`, VP:11415). Exports are
confirmed to read approved content only: the pipeline export snapshot
selects by `approved_object_ref_id` and reads no bytes, and the index
rebuild reads only sealed index commands. Its privacy filter reads the
submission's `privacy_risk`, which is why step 3 writes it back. Legacy
readers that can reach a pipeline submission refuse it only because the
pipeline's object wrapper does not decode as an envelope; a test pins that,
and moving them to the reviewer metadata view is a separate follow-up.

Review's evidence hashes its input as `source_content_hash` (VP:11222).
After this change that is the hash of the pass's output, not the raw
source. The pass's own record keeps the raw source hash, so both stay
traceable.

The pass is a server control, not part of the bundle, for the same reason
the boundary is one at the receipt today. A bundle cannot opt out of it, and
the package and its qualification do not change.

**Escalation.** The pass can raise the risk above what Admission saw. A
trace whose only PII is prose, like stage 3's `pii-residual` probe, has
nothing the deterministic redactor recognises, so Admission admits it at
Low, and only the pass's classifier finds it.

**Decision 2026-10-10: PII found and removed is accepted.** Owner decision
(Zaki): "Once PII removed we should go to accepted", for the pipeline as
for legacy. The rule as first written (quoted below where it is amended) escalated any pass
risk above Admission's, so a post-pass Medium -- the classifier found prose
PII and removed it -- held the run for a human. On the pilot, stage 3's
`pii-residual` probe (names, a date of birth, an address) was held on the
pipeline path. The rule is now: **the pass escalates only when its risk is
High and above the receipt's** (`privacy_pass_outcome` in
`versioned_pipeline.rs`). A post-pass Medium is the found-and-removed (or
consent-flag) floor, and the envelope the pass hands Review is the redacted
one, so the pass records `cleared` -- with its merged basis (for example
`found_and_removed`), the redaction counts and the raw `medium` risk written
back, so the audit trail keeps what was removed -- and the run goes on to
the Review policy with the scrubbed bytes. An Admission-admitted run ends
accepted on the redacted content, with its gate decision and credit like any
accepted run, and Score and exports read only that approved content. High
(a key finding, a coverage gap, a survivor, or a residual scan that could
not run: what redaction did not resolve) is still held for a human and the
submission stored `quarantined` (#1326), never rejected automatically.
Admission is unchanged: a receipt-time Medium that Admission quarantined
keeps Admission's hold whatever the pass clears, and a receipt-time High is
still rejected by Admission. The text below describes the hold mechanics,
which apply unchanged to a High escalation.

Admission's decision is committed and is not rewritten, and the bundle's
Review policy only knows to wait for a human when Admission said
Quarantine. So the escalation is the server's, not the policy's.

The pass escalates when its merged risk is High and above the risk
Admission saw (as amended 2026-10-10; first written as "above the risk
Admission saw: Medium or High for a run Admission admitted, High for a run
Admission quarantined at Medium"). Both sides are on Admission's scale. The submission
row stores the envelope's raw residual risk, and a Medium whose only basis
is the consent content flag is Low to Admission; so the receipt-time side
is that raw value mapped through the same rule, with the stored basis, and
the pass side is its own raw risk mapped with the merged basis. A
consent-flag-only trace admitted at Low whose classifier then finds prose
PII, and removes it, now clears (2026-10-10); it escalated as first
written. The pass writes the raw post-classifier risk back to the
submission row, the scale the row holds everywhere else. When it escalates, the server holds the run
itself before it calls the Review policy:
- **No human assessment recorded after the pass** (`load_review_assessment`,
  compared with the pass's `privacy_pass_recorded_at`): the server parks the
  run as `AwaitingReview` with `privacy_pass_review_required`. That label is
  the pass's own, so a reviewer can tell it from Admission's
  `privacy_review_required`. The review queue's predicate, which today
  requires an Admission quarantine, also admits a run the pass escalated,
  and lists the hold reason. The policy is not called.
- **Assessment recorded after the pass:** `record_review_assessment` moves
  the run back to `pending`. The next dispatch finds the pass already
  recorded and an assessment present. An approval must resolve
  `privacy_pass_review_required` (and the Admission reason, if there is
  one); it goes on to the Review policy with the pass's output, and the
  server records the approving assessment's evidence hash and resolved
  reasons on the pass record, so the approved outcome is linked to the human
  decision. A rejection of a run Admission admitted is committed by the
  server under rule id `privacy_pass_human_review_rejected_v1`, because the
  bundle's Review policy ignores an assessment when Admission admitted.
- **Not escalated:** the run goes to the Review policy, which holds an
  Admission-quarantined run for a human as today.

A run that needs the pass is claimable for human review only once its pass
is recorded, so no assessment can be given before the classifier has seen
the trace. An assessment that predates the pass (possible only for a run
received before this change) is ignored when the pass escalates; the run
stays held, fail-closed.

High is held for a human, not rejected automatically (owner decision).
Legacy's backstop quarantines and never rejects. And a classifier-only
finding with no deterministic signal is the case where a human look is worth
most. A receipt-time High is still rejected by Admission.

This keeps the bundle untouched: `ReviewInput`, the Review policy and the
qualification fixtures do not change.

**Failure.** Any classifier error or timeout is retried: the run goes to
`Retry` with backoff, as legacy's backstop leaves an item held. The budget is
the run's own (5 attempts, legacy's count) and the backoff is legacy's, 30 s
doubling. The call is bounded by legacy's 900 s, or less when the Review
lease's renewal cap is shorter. After the retry budget is spent, the run
fails with `privacy_classification_failed` and stays out of the corpus, with
nothing approved. A contributor sees such a run as `quarantined`. The pass never falls back to
the deterministic result alone: a configured control whose dependency fails
refuses the path (the repository's fail-closed rule).

**Crash safety.**
- A crash between the classifier call and the transaction repeats the call.
  The crashed attempt's object stays staged and the attempt sweep deletes
  it; only one object ref is ever committed.
- A live worker that loses its lease mid-pass may finish the classifier
  while a second worker runs it again. Only one result is recorded; the
  loser's commit is refused and its object deleted or swept.
- A crash after the transaction finds the pass recorded and skips straight
  to the Review policy.
- The pass is recorded at most once per run; the classifier call can run
  more than once.

**Customer access.** Until the pass has run, the trace has no approved
content, so it is in no corpus, score or export. SYS-004, "unresolved
privacy risk MUST prevent customer access", holds without a new gate.

### What changes in storage

The stored source object now holds the envelope after the deterministic
rescrub but before the classifier, which is what legacy stores too. It is
encrypted under the tenant's key, as today.

One rule moves with it. pipeline-activation.md:1598-1599 and 1629 say the
stored source is the post-boundary content. That becomes: "the stored source
is the content after the deterministic rescrub; the approved content, which
is all that Score and exports read, is after the classifier."

Recording the pass needs a place on the run for its object ref, conditions
and outcome. Two options were considered:
- **(a) New columns on `pipeline_runs`, behind one migration (V117).**
  Taken: the record sits beside the run's other phase state, under the same
  RLS. The deploy becomes a Route B migration deploy instead of
  binary-only.
- **(b) A row in an existing per-run object table**, with no schema change.
  Not possible: staging the pass object needs a new
  `pipeline_attempt_artifacts.artifact` value, and V108 pins that set in a
  CHECK, so any option needs a migration.

V117 also adds `privacy_pass_required`, FALSE on every existing run and TRUE
by default from then on, and a CHECK that refuses an approved object on a
run that requires the pass and has none. `commit_review` refuses the same
approval with a safe label first. A binary that predates this change,
rolled back onto a V117 database, therefore cannot approve a run that has
no pass recorded. The CHECK sees only that a pass exists, not which object
Review read, so it does not stop that binary approving a run whose pass is
recorded but whose Review has not committed: the old Review reads the
run's source, the deterministic envelope the classifier never saw, and
approves it with any prose PII the pass removed. That binary does not hold
a run that has no pass, either: it sees the
CHECK violation as a raw, non-transient database error, charges it as
`minimal_policy_failed` on its 50 ms doubling backoff, and the run uses up
its five attempts in about a second and ends `failed` with
`attempts_exhausted`, which is terminal. Every run received after V117 that
reaches an approval at Review under the old binary is lost that way. A
rollback below this revision must first suspend the Review policy of the
affected bundles, which are those of every run at Review that requires a
pass and has no approval, pass recorded or not (an uncharged wait,
`bundle_policy_not_runnable`), or stop the pipeline workers; the plan's
Task 10 gives the count query and the runbook steps.

The submission row's `redaction_counts` and `redaction_pipeline_version`
are updated from the pass. Its `redaction_hash` stays the deterministic
envelope's: a withdrawal's tombstone is written from that column and a later
receipt matches tombstones against the hash of its own deterministic
envelope, and a changed `redaction_hash` revokes the submission's token
bundles (V68). Legacy's backstop does refresh the hash; this is a
deliberate difference.

## Alternatives considered

1. **Speed up the receipt-time classifier** (concurrent windows up to NEAR
   AI's rate limit, plus caching). This narrows the gap but leaves an upload
   time that grows with trace size. A larger trace still times out, and
   SUB-005's bounded-local-work rule stays broken.
2. **Raise the client's timeout.** This needs a client release and leaves
   the server holding long requests. It hides the cost without removing it.
3. **A new pipeline phase between Admission and Review.** `Phase` lives in
   `trace-commons-gate-api`. A new variant changes the gate contract, every
   bundle's phase list and the qualification fixtures. Putting the pass at
   the start of Review gets the same ordering without touching the contract.
4. **Make the bundle's Review policy do the scrub.** Design §4 allows a
   production Review policy to scrub PII. But a bundle could then ship
   without it, and the privacy control would move from the server into
   qualified content. That is the wrong side of the trust boundary.

## Tests

- **Receipt time.** The receipt makes no classifier call. A boundary
  double whose `rescrub_classifier` counts calls and sleeps 60 s must not
  delay `submit`. Its `rescrub_deterministic` is still called once, and the
  receipt returns `processing`.
- **The pass runs once.** A Review dispatch runs the pass once and commits
  one object ref. A crash point after the classifier and before the
  transaction repeats the call and commits one ref; the orphaned attempt
  object is swept. A crash after the transaction does not call the
  classifier again. A lease lost mid-pass records one result.
- **Escalation.** A run that Admission admitted at Low and the pass finds
  High parks `AwaitingReview` with `privacy_pass_review_required`, the
  submission is stored `quarantined`, and the Review policy is not called.
  A run the pass finds Medium because it found and removed PII (a
  consent-flag-only Medium included) is not held: the pass records
  `cleared` and the run completes accepted on the redacted content, which
  lacks the PII (amended 2026-10-10). After an approving assessment the
  next dispatch calls the policy once, with the pass's output and no second
  classifier call, and the run records the assessment's hash. After a
  rejecting assessment the run ends rejected. A quarantined run is not
  claimable before its pass, and an assessment recorded before the pass
  never releases an escalated run.
- **Approval guard.** An approval of a run that requires the pass and has
  none is refused, by `commit_review` and by the CHECK.
- **Failure.** A classifier error leaves the run in `Retry`. Exhausted
  retries fail it with `privacy_classification_failed` and no approved
  object.
- **Downstream reads.** Score's input is derived from the pass's output,
  never from the source. A source carrying a planted prose-PII span that the
  double redacts must not appear in the bytes Score loads.
- **Resume after the pass.** Runtime crash-matrix tests cover a run stopped
  after the pass and before the Review policy: the resume does not call the
  classifier again. The restore drill keeps its one pending run; its
  database fingerprint gains the pass record, which both seeded runs now
  carry.
- **Legacy readers.** A pipeline submission whose stored source carries a
  planted prose marker is refused by the legacy envelope readers and
  exports, or never appears in their output.
- **Stage 3 rerun.** On the pilot, `long-chunk-capped` is stored with
  status `processing` in under 2 s and reaches a terminal state. The
  `pii-residual` case is still quarantined, but only after a worker
  dispatch has run the pass; until then it reads `accepted`.

## Rollout

The change moves the code revision, so it ships as one promote cycle: a
build and a Route B deploy for the migration in "What changes in storage"
(a); then `qualify`, the promote checks, sign and assemble; then requalify
the pipeline tenant's bundle on the new revision.

## Owner decisions

0. **SUB-005's intent.** "MUST own synchronous privacy-risk handling" means
   the bounded, local handling this proposal keeps at Admission. The
   contracts document gets a clarifying sentence, not an amendment.
1. **High.** Held for a human, as legacy does. Not rejected by the pass.
2. **Client status while the pass is pending.** The receipt keeps
   `processing`; no client change and no new status value. One server-side
   mapping changes inside the existing vocabulary: a run that failed with
   `privacy_classification_failed` reads `quarantined`, not `accepted`.
3. **Retry budget and backoff for classifier errors.** Legacy's backstop
   budget: 5 attempts, 30 s doubling, a 900 s call bound.
4. **Rejection of an escalated Admit run.** Committed by the server, rule id
   `privacy_pass_human_review_rejected_v1`. An approval is still the bound
   policy's, linked to the assessment on the pass record.
5. **Legacy readers.** Their refusal of pipeline submissions is pinned by a
   test now; moving them to the reviewer metadata view is a follow-up.

## Corrections from the implementation plan

The plan's "Spec corrections" list the places where this document was wrong
or impossible as first written: a content-addressed object (now per-attempt
keys with staging and sweep), the review queue's predicate (widened), the
policy's handling of an assessment on an Admit run (server-committed
rejection), the terminal label on retry exhaustion (new code), and storage
option (b) (impossible). A recheck of the plan corrected two more: the
escalation comparison must put the stored raw risk on Admission's scale
before comparing, and a rollback onto V117 fails post-V117 runs terminally
at Review rather than holding them. The text above has been updated to
match.
