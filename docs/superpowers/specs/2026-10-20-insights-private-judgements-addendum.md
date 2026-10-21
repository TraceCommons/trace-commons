# Insights private judgements -- addendum to the model-answers design

Date: 2026-10-20. Owner: Zaki. Status: draft. The owner answered the open
decisions on 2026-10-20 (see "Owner decisions"). Nothing here is built, and
nothing here blocks the model-answers PRs (PR 0, PR 1), which ship the local
design first.

## Owner decisions (2026-10-20)

1. **Build now. Jev is a development placeholder only; production waits for
   the TEE-hosted decision model,** which the owner expects on NEAR AI the
   week of 2026-10-20. The correction recorded below was checked on
   2026-10-20 against NEAR AI's public `/v1/model/list`. `typesafe/jev-1.13`
   reports `verifiable: false`, `attestationSupported: false`, and
   `providerType: external` with `base_url` `https://openrouter.ai/api/v1`.
   So "NEAR AI-routed Jev" is a pass-through to OpenRouter and TypeSafe,
   outside NEAR AI's TEE guarantees. The service enforces the rule rather
   than relying on convention:
   - A **production allowlist of models.** The service refuses to start, and
     refuses any call, with a model whose catalog entry is not both
     `verifiable` and `attestationSupported`, and whose per-request receipt
     does not verify.
   - Jev (or any unverifiable model) runs only in a **development mode**.
     That mode is a separate build flag plus measurement, never the pinned
     production measurement, and is fed synthetic or developer-owned data
     only. A daemon in a release build accepts only the production
     measurement, so it can never send contributor data to a development
     service.
2. **Digest contents: delegated to the implementer.** The values are below,
   under "What is sent".
3. **Budget: $5 to start.** Recorded as a cap of $5 of NEAR AI spend per
   contributor per calendar month, enforced by the service's limiter. If the
   owner meant $5 in total for the first experiment, the limiter's per-user
   cap becomes the total.
4. **Account-bound calls**, not anonymous tokens. The service learns which
   account made a call; mitigations are below.
5. **Comparisons across users inside the TEE are in scope** as a later
   phase. Outline below; it gets its own spec.

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
contributor daemon --(body sealed to the CVM's per-boot key)--> judgement service (CVM)
                                                                |  pinned prompt + model
                                                                +--> NEAR AI private inference
                                                                |    (decision model; per-request receipt)
contributor daemon <--(sealed, signed labels + receipt evidence)-- +
```

dstack-gateway terminates the outer TLS, so the transport is not what
protects the data. The daemon seals the request to a key that is generated
inside the service's process at boot and bound in its quote. Only the attested
process can open it: not the gateway, and not another image the app's KMS
admits. The response comes back sealed and signed by a second per-boot key in
the same quote. (Service spec, section 3; #1501 review point 1.)

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

- **Counts:** the ones the model-answers design already stores (tool kinds,
  edit counts, failing and passing commands, denial and interrupt markers),
  plus per-turn edit and command outcomes.
- **Contributor prompts:** at most 12 per session, each scrubbed and cut to
  240 characters. With more than 12, take the first 4, the last 4, and the 4
  just before each denial, interrupt or undo (the 4 up to and including the
  prompt whose turn carried it; earlier events win when slots run out).
  Slots the events leave free go to the middle prompts not yet chosen, at
  evenly spaced positions, so a session with more than 12 prompts always
  sends 12. Never more than 12.
- **Final message:** the first line of the model's final message, scrubbed and
  cut to 160 characters.
- **Edited files:** each file's extension class, never its path.
- **Never:** file contents, diffs, paths, repository names or identifiers.

The digest has a byte cap of 8 KiB, and is dropped whole rather than truncated
mid-field. It also has a hash. The daemon
logs only the hash (CLAUDE.md, hash-only logging). The digest builder is pure
and lives in the contributor-insights crate. A test over the serialized bytes
pins that no path, file name or raw text beyond the scrubbed excerpts appears.

### Per-user rate limiting (account-bound, owner decision 4)

- A call carries the contributor's existing account credential. The service
  checks it, then keys the limiter on a **keyed digest of the account ID**,
  under a key derived from the dstack guest agent (bound to the service's app
  id and released only to measurements its KMS admits; see the judgement
  service spec, section 6), never the raw ID.
- **Cap:** the limiter meters spend, not calls. Each call's cost is taken from
  the NEAR AI usage the response reports. The cap is $5 per account per
  calendar month (decision 3). A call that would pass the cap is refused with
  a retryable capacity refusal that carries the reset time.
- **Mitigations for linkability:**
  - The service logs only digest hashes and fixed labels (CLAUDE.md).
  - It stores no digests or labels, only the limiter's spend per keyed account.
  - Digests and claims reach only the attested process: the bodies are
    sealed to its per-boot key (above), so neither the gateway nor another
    admitted image can read them.
  - The consent copy says calls are tied to the account.
- Refusal shapes follow `witness_pacing`: capacity refusal is retryable;
  policy and timeout refusals are distinct.
- An operator-wide monthly budget sits above the per-account cap. Spending
  past it refuses every call with the same capacity refusal.

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

1. (Answered as decision 1: build now with the NEAR AI-routed Jev stand-in.)
2. (Answered as decision 2: delegated; values are under "What is sent".)
3. (Answered as decision 3: $5 per account per month to start. The
   operator-wide budget is still open.)
4. (Answered as decision 4: account-bound.)
5. (Answered as decision 5: in scope as phase 2, outlined below.)

## Phase 2: comparisons across users inside the TEE (outline)

The owner put this in scope. With it, the service is no longer stateless:

- **Opt-in to contribute:** a contributor who opts in sends their per-model
  figures to the service. These are counts and rates per (task type, model),
  never digests or labels. Opting in to contribute is separate from opting in
  to judgements.
- **Sealed store:** the figures are held only inside the enclave, encrypted
  under a key derived from the dstack guest agent (bound to the app id, released
  only to measurements its KMS admits), keyed by the keyed account digest so a
  contributor can replace or delete theirs. An in-place upgrade the KMS admits
  keeps the store; a new app id starts it over.
- **Answers:** the service answers only aggregates. An example is "your Opus
  rejection rate in bug-fix sessions vs. the median of N contributors".
  - Minimum cohort: at least 20 contributors per cell, with no aggregate
    answered below that.
  - Answers are rounded, so that one contributor joining or leaving a cell
    cannot be inferred from two queries.
- **Attestation:** the daemon verifies the measurement before sending
  figures, as for judgements. The (i) on a cross-user answer names the cohort
  size and says the comparison was computed inside an attested enclave.
- **Own spec:** the threat model (query auditing against differencing,
  measurement upgrades, sealed-key migration, deletion) needs its own spec
  before any code.

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
4. The account-bound limiter (keyed account digest, spend metering, $5 cap)
   inside the service.
5. Consent copy and the opt-in UI (macOS first).
6. Phase 2: comparisons across users. Its own spec first.
