# Pipeline privacy rescrub out of the receipt

Status: draft for owner review, 2026-10-09.
Scope: `trace-commons-server` only. No client or protocol change. The
recommended storage option adds one migration (see "What changes in
storage").

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
proposal changes a contract rather than restoring one. Owner's call
(open question 0).

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
2. Run the boundary's classifier rescrub on them (`rescrub`).
3. Store the result as a new encrypted object, keyed by content hash, as
   REV-002 requires for transformed content. Record its object ref and the
   merged residual-risk conditions on the run, in one transaction.
4. Hand the Review policy that object's bytes as `source_artifact`, in place
   of the raw source.

Review's approved content is then derived from scrubbed content. Score
reads only approved content (`load_approved_bytes`, VP:11415). Exports are
not confirmed yet; the implementation plan must check that every export
reader takes approved content, and fix any that reads the source.

Review's evidence hashes its input as `source_content_hash` (VP:11222).
After this change that is the hash of the pass's output, not the raw
source. The pass's own record keeps the raw source hash, so both stay
traceable.

The pass is a server control, not part of the bundle, for the same reason
the boundary is one at the receipt today. A bundle cannot opt out of it, and
the package and its qualification do not change.

**Escalation.** The pass can raise the risk above what Admission saw. This
matters in practice. A trace whose only PII is prose, like stage 3's
`pii-residual` probe, has nothing the deterministic redactor recognises, so
Admission now admits it at Low. Today the receipt-time classifier is what
quarantines it. Under this proposal the pass must.

Admission's decision is committed and is not rewritten, and the bundle's
Review policy only knows to wait for a human when Admission said
Quarantine. So the escalation is the server's, not the policy's.

When the pass's merged risk is Medium or High, the server holds the run
itself before it calls the Review policy:
- **No human assessment recorded yet** (`load_review_assessment`): the
  server parks the run as `AwaitingReview` with `privacy_review_required`,
  through the same state and queue as an Admission quarantine. The policy
  is not called.
- **Assessment recorded:** `record_review_assessment` moves the run back to
  `pending`. The next dispatch finds the pass already recorded and an
  assessment present. An approval goes on to the Review policy with the
  pass's output. A rejection ends the run as the reviewer's rejection does
  today.
- **Low, or no change:** the run goes straight to the Review policy.

High is held for a human, not rejected automatically. Legacy's backstop
quarantines and never rejects. And a classifier-only finding with no
deterministic signal is the case where a human look is worth most. Open
question 1 asks whether High should reject instead.

This keeps the bundle untouched: `ReviewInput`, the Review policy and the
qualification fixtures do not change.

**Failure.** Any classifier error is transient: the run goes to `Retry`
with backoff, as legacy's backstop leaves an item held. After the retry
budget is spent, the run fails with `privacy_classification_failed` and
stays out of the corpus, with nothing approved. The pass never falls back to
the deterministic result alone: a configured control whose dependency fails
refuses the path (the repository's fail-closed rule).

**Crash safety.**
- A crash between the classifier call and the transaction repeats the call.
  The object is content-addressed, so a repeated store is idempotent.
- A crash after the transaction finds the pass recorded and skips straight
  to the Review policy.
- The pass runs at most once per run to completion.

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
and outcome. Two options:
- **(a) New columns on `pipeline_runs`, behind one migration.**
  Recommended: the record sits beside the run's other phase state, under
  the same RLS. The deploy becomes a Route B migration deploy instead of
  binary-only.
- **(b) A row in an existing per-run object table**, with no schema change.
  Only if the plan finds one that fits without overloading its meaning.

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
- **The pass runs once.** A Review dispatch runs the pass once and stores
  one content-addressed object. A crash point after the classifier and
  before the transaction repeats the call and stores no second object. A
  crash after the transaction does not call the classifier again.
- **Escalation.** A run that Admission admitted at Low and the pass finds
  Medium or High parks `AwaitingReview` with `privacy_review_required`, and
  the Review policy is not called. After an approving assessment the next
  dispatch calls the policy once, with the pass's output and no second
  classifier call. After a rejecting assessment the run ends rejected.
- **Failure.** A classifier error leaves the run in `Retry`. Exhausted
  retries fail it with `privacy_classification_failed` and no approved
  object.
- **Downstream reads.** Score's input is derived from the pass's output,
  never from the source. A source carrying a planted prose-PII span that the
  double redacts must not appear in the bytes Score loads.
- **Restore drill.** It covers a run stopped after the pass and before the
  Review policy: the restore keeps the pass's object and record, and the
  resume does not call the classifier again.
- **Stage 3 rerun.** On the pilot, `long-chunk-capped` is stored with
  status `processing` in under 2 s and reaches a terminal state. The
  `pii-residual` case is still quarantined.

## Rollout

The change moves the code revision, so it ships as one promote cycle: a
build and a Route B deploy for the migration in "What changes in storage"
(a); then `qualify`, the promote checks, sign and assemble; then requalify
the pipeline tenant's bundle on the new revision. With option (b) the deploy
is binary-only.

## Open questions for the owner

0. **SUB-005's intent.** Does "MUST own synchronous privacy-risk handling"
   mean the bounded, local handling this proposal keeps at Admission? Or
   must the classifier's verdict be known before the receipt commits? If
   the latter, this proposal amends SUB-005, and the contracts document
   changes in the same PR as the code.
1. **High.** Hold it for a human, as recommended and as legacy does, or
   reject automatically with `privacy_risk_rejected`?
2. **Client status while the pass is pending.** Should the submission
   status say something like legacy's "Held pending an automated privacy
   check" rather than "processing"? That needs a status value the clients
   know, so it is a client change.
3. **Retry budget and backoff for classifier errors.** Default: legacy's
   backstop budget.
