# Insights private judgements -- addendum to the model-answers design

Date: 2026-10-20. Owner: Zaki. Status: draft, for owner review. Nothing here
is built, and nothing here blocks the model-answers PRs (PR 0, PR 1), which
ship the local design first.

Extends `2026-10-20-insights-model-answers-design.md` (spec PR #1501).

## Why

The model-answers design answers "which model did what" from counts the
counter pass reads on this machine. Its local rules are weakest exactly where
a judgement is needed. This addendum fills those gaps with a decision model on
NEAR AI private inference, called through an attested service. Owner direction
on 2026-10-20: "there is also a decision model coming to near.ai soon" and
"this is basically filling in the gaps insight-model-answers etc".

| Gap in the local design | Local rules (v1, unchanged) | Private judgement (this addendum) |
|---|---|---|
| Task type | Rules over file kinds, test runs and fail-then-pass pairs; mixed sessions fall to `other` | Classify the session from a scrubbed digest |
| Rejection or not | Counts denials, undos after a contributor prompt, and interrupts; cannot tell why | Judge whether an interrupt or undo was a rejection, a redirect, or the model's own iteration |
| Finished | No denial, undo or interrupt at the end, plus at least one edit | Judge whether the session's task was done |
| Codex denials | Unknown: these transcripts record none | Still unknown. No model can observe a denial that was never recorded |

## Principles

1. **The local counts stay the floor.** They are always computed, always
   local, and never zero for unknown. Without the opt-in, Insights behaves
   exactly as the model-answers design says.
2. **A judgement sits beside a count, never in place of it.** Every judged
   field carries its source (`heuristic`, `private_ai`) and the judge's
   version. An answer states which basis it used, and an answer never mixes
   bases within one comparison.
3. **This is the first Insights data to leave the machine.** It is a consent
   moment of its own, off by default, with its own wording. Turning it off
   deletes every stored judgement.
4. **Send the least that decides the question.** That is counts plus scrubbed
   excerpts, never a whole transcript.

## Architecture

```
contributor daemon --(attested TLS, digest only)--> judgement service (CVM)
                                                    |  pinned prompt + model
                                                    +--> NEAR AI private inference
                                                    |    (decision model; per-request receipt)
contributor daemon <--(labels + receipt evidence)-- +
```

### The judgement service

- Runs in a confidential VM, deployed, measured and attested the way the
  redaction witness is (`2026-09-02-redaction-witness-service-design.md`). The
  daemon verifies the service's measurement against a pin before it sends
  anything; this is the existing witness-client pattern.
- **Stateless.** It holds no per-user history: request in, NEAR AI call,
  labels out. The only state is the rate limiter (below).
- **Pinned prompt and model version.** A label's meaning is fixed by
  `(prompt_version, model_id)`, which travels with the label, so labels stay
  comparable across contributors and over time. A change of either is a new
  judge version (principle 2).
- **Verifies NEAR AI's per-request receipt** with
  `trace-commons-attestation::receipt::verify_receipt` (permissive crate,
  already used by the attestation drill). It returns the receipt evidence with
  the labels, so the daemon can show that the judgement ran in an attested
  enclave. A hop from one TEE to another is an accepted boundary, provided each
  hop is attested.
- **Funded by the operator** through the NEAR AI reseller account. That is the
  reason a service exists at all: a contributor's own credential could call
  NEAR AI directly, but then there is no shared prompt or model pin and no
  operator budget.

### What is sent

A **judgement digest**, built locally after the privacy filter runs:

- the counts the model-answers design already stores (tool kinds, edit
  counts, failing and passing commands, denial and interrupt markers);
- per turn, at most N scrubbed excerpts: the contributor prompt, the first
  line of the model's final message, and each edited file's extension class
  (never its path);
- no file contents, diffs, paths, repository names or identifiers.

The digest has a byte cap (default proposed: 8 KiB) and a hash. The daemon
logs only the hash (CLAUDE.md, hash-only logging). The digest builder is pure
and lives in the contributor-insights crate. A test over the serialized bytes
pins that no path, file name or raw text beyond the scrubbed excerpts appears.

### Per-user rate limiting without linking calls

The account identity would make every judgement call linkable to a person, so
this addendum does not use it. The proposal:

- At enrollment, and again per period, the daemon receives a batch of
  **anonymous single-use tokens** (blind-signed, Privacy Pass style) from the
  ingest server, which already knows the account.
- Each judgement call spends one token. The service verifies the issuer's
  signature and rejects reused tokens. It learns that a valid user spent a
  token, never which user.
- The batch size per period is the per-user limit. The operator's NEAR AI
  budget is the sum.
- It is the same goal as `witness_pacing`, which throttles background witness
  work "without carrying identity". Its refusal shapes (capacity refusal is
  retryable; policy and timeout refusals are distinct) are reused here.

Open: blind-signature scheme and library. Any new dependency goes through the
dependency policy (surface it for explicit approval, check license
compatibility).

### What comes back, and where it goes

- **Labels:**
  - `task: {bug_fix, refactor, tests, docs, other}`;
  - per marked event, `kind: {rejection, redirect, iteration, unclear}`;
  - `finished: {yes, no, unclear}`.
  - Each label carries a confidence bucket, the judge version and the receipt
    evidence.
- **Stored** on the counter row beside the local facts, under the counter key,
  with the same retention (13 weeks) and the same clearing on unenroll or
  opt-out. Never on the wire to other parties.
- **Plugged into the existing seams:**
  - task type goes through the reserved `TaskClassifier` interface, as a second
    implementation;
  - rejection and finished judgements are new optional fields on the session
    facts.
  - `model_answers` gains a basis switch: it uses judged fields only when every
    session in a comparison has them under the same judge version, and
    otherwise falls back to the local basis and says so.

## Copy

New consent wording, following `docs/ui-copy-guide.md`, needs owner review.
It must say plainly:

- what leaves the machine (a short scrubbed summary per session);
- where it goes (an attested service, then NEAR AI's attested inference);
- who pays (Trace Commons, not you);
- what is kept (labels only, on this machine);
- how to turn it off.

Answers that use judged fields say "judged privately" in their (i).

## Not decided (owner)

1. Whether to build this before the decision model is available, using a
   stand-in model behind the same interface, or wait.
2. Digest contents and cap: excerpts per turn (N) and the byte cap.
3. Rate limit per user per period, and the operator's monthly budget.
4. Anonymous tokens (recommended) or account-bound calls (simpler, linkable).
5. Whether the service may also compute **cross-user aggregates**, such as
   "your rejection rate vs. others". That is a much larger design, with its own
   threat model (k-anonymity, minimum cohort sizes) and its own consent. This
   addendum excludes it.

## Unknowns

The decision model's name, API shape, whether it carries NEAR AI's
per-request attestation, latency and pricing are unknown as of 2026-10-20. The
service's model call is a single adapter behind the pinned
`(prompt_version, model_id)`. Nothing above depends on the model's specifics
beyond "a short classification call with an attested receipt". If the model
ships without per-request attestation, the receipt evidence is absent and the
(i) must say so; it never claims attestation it does not have.

## Delivery (after PR 0 and PR 1)

1. The judgement digest builder and its privacy tests (contributor-insights,
   pure). It can land before the service exists.
2. The judged fields on the session facts, the `TaskClassifier` second
   implementation, and the basis switch in `model_answers`, all behind the
   opt-in and off by default.
3. The judgement service: CVM image, measurement pin, receipt verification,
   rate limiter. Its own spec, following the witness service spec's structure.
4. The anonymous token issuer on ingest. Its own spec, with a dependency
   proposal.
5. Consent copy and the opt-in UI (macOS first).
