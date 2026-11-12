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
2. **Digest contents: not decided.** What leaves the machine is the privacy
   decision of this feature, so it is the owner's (open decision O1, under
   "Not decided"). An earlier draft delegated it to the implementer. That was
   withdrawn after #1501 review point 8.
3. **Budget: $5 to start.** Recorded as a cap of $5 of NEAR AI spend per
   contributor per calendar month, enforced by the service's limiter. If the
   owner meant $5 in total for the first experiment, the limiter's per-user
   cap becomes the total. The operator-wide budget above it is $100 a month
   (service spec, owner decision 4).
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
4. **Send the least that decides the question,** never a whole transcript.
   What that is is open decision O1 ("What is sent").

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

### What is sent (open decision O1)

**Not decided.** The owner decides what leaves the machine. Until O1 is
decided, **no code may send a digest**:

- the digest builder (delivery step 1, #1525) may be reviewed but does not
  merge;
- the daemon client (service slice 7) does not merge, and the release
  daemon's production pin set stays empty in any case until service slice 9.

The options:

| Option | What leaves the machine | Judgements it can make well | Cost to privacy |
|---|---|---|---|
| A. Counts only | The counts below. No contributor text. | `task` from tool and file-class mix; `finished` weakly. Cannot tell a rejection from a redirect: that difference is in what the contributor wrote. | Lowest. No excerpt can carry a name, a path or a secret. No scrub to get right. |
| B. Counts plus scrubbed excerpts | The counts, plus up to 12 prompt excerpts and the final line, scrubbed (below), for sessions from Automatic folders only (section "Folder rules") | All three, including `event_kind`, which is the main gap the local rules leave. | An excerpt that the scrub misses leaves the machine. Bounded by the scrub, the residual-risk gate and the byte budgets below. |
| C. Counts, plus excerpts only with per-session approval | Counts for every eligible session. Excerpts only for a session the contributor approves, one by one, after seeing the exact excerpts. | All three, for approved sessions only. The basis switch then rarely has a whole comparison judged. | Lowest for text. Each send is a consent moment. More prompts to answer. |

**Recommendation: B, with its gates.** Excerpts only from Automatic folders,
only for sessions whose residual-risk class is `low` (others send counts only,
as A), and only after the scrub below. The `event_kind` judgement is the
reason the service exists, and it needs the contributor's words. A cannot
make it, and C would leave most comparisons on the local basis. If the owner
wants no contributor text without an explicit per-session yes, choose C. A is
the fallback if the scrub cannot pass its byte tests.

The shape below is option B's. Under A, `excerpts` and `final_line` are always
empty. Under C they are empty unless that session was approved.

A **judgement digest**, built locally. Every text field passes the scrub
(below) whole, before it is cut:

- **Counts:** the ones the model-answers design already stores (tool kinds,
  edit counts, failing and passing commands, denial and interrupt markers),
  plus per-turn edit and command outcomes.
- **Contributor prompts:** at most 12 per session, each scrubbed whole and
  then cut to its budget ("Size", below). With more than 12, take the first 4, the last 4, and the 4
  just before each denial, interrupt or undo (the 4 up to and including the
  prompt whose turn carried it; earlier events win when slots run out).
  Slots the events leave free go to the middle prompts not yet chosen, at
  evenly spaced positions, so a session with more than 12 prompts always
  sends 12. Never more than 12.
- **Final message:** the first line of the model's final message, scrubbed
  whole and then cut to its budget.
- **Edited files:** each file's extension class, never its path.
- **Never:** file contents, diffs, paths, repository names or identifiers.

The digest has a hash, and the daemon logs only the hash (CLAUDE.md,
hash-only logging). The digest builder is pure and lives in the
contributor-insights crate.

#### When the digest is built, and where it sits (#1501 review point 4)

The counter row holds no prompt text, and the counter pass reads each file
once. So excerpts would have to come from either a second read or text kept on
disk until the send, and sends can be refused for days. The design takes the
second read and keeps **no queue on disk**:

- **Built at send time, in memory.** The digest is built from one more read of
  the session's transcript, immediately before the send. Never during the
  counter pass. It lives only for that send (at most the request's 240 s
  deadline), is never serialized to the daemon's store or any file, and is
  dropped whatever the outcome.
- **Preconditions, checked before the file is opened:** private judgements on;
  the folder rule allows this session, read now ("Folder rules"); a verified
  service is in hand; no account or operator `reset_at` lies in the future;
  and the session is finished: its file is the same size and modification
  time the counter row recorded, and has not been written for 30 minutes.
- **A refused or failed send keeps no text.** The counter row records only a
  fixed state (`judgement: pending | refused:<label> | judged`) and, for a
  capacity refusal, the retry-not-before time. The next attempt rebuilds the
  digest from the file. If the file has gone, changed, or left the 13-week
  window, the session is never judged.
- **The parent design's constraints are amended** to say this
  (`2026-10-20-insights-model-answers-design.md`, "Constraints"): local only
  except this opt-in path, and one send-time read beside the counter pass.
- **Test:** after one judged session and one refused session (built from
  fixtures with distinctive strings), a byte scan of the daemon's whole data
  directory finds none of the fixture strings. After opt-out, unenroll, a move
  to Never, and the 13-week expiry, no judgement field or state remains on any
  affected row.

#### The scrub (#1501 review point 3)

"After the privacy filter runs" named no filter. The scrub is defined here,
and the digest builder runs it itself, so its order cannot be got wrong by a
caller. contributor-insights already depends on `trace-commons-protocol`, so
it can.

1. **The redactor:**
   `DeterministicTraceRedactor::deterministic_only(known_path_prefixes)`
   (`trace-commons-protocol`). It is deterministic and ignores the
   environment by construction. The builder never calls `build_redactor` or
   `build_redactor_with` (`contributor/src/envelope.rs`). Those can attach a
   NEAR AI privacy filter through `pii_filter = "near-ai"` or
   `TRACE_PRIVACY_FILTER_BACKEND`, which is a network call before consent.
   `known_path_prefixes` are the home directory and the session's recorded
   working directory, as `build_deterministic_preview_redactor` uses.
2. **The whole text first.** The redactor runs on the whole prompt and the
   whole final line, before any cut. A secret that straddles the cut is
   therefore masked whole and cannot leave half visible.
3. **The excerpt masker**, a pure function in `judgement_digest.rs`. It runs
   after the redactor, because the redactor masks only secrets and known
   absolute path prefixes:
   - email addresses become `<email>`;
   - URLs (`scheme://...` and `www.` forms) become `<url>`;
   - any token containing `/` or `\` with text on both sides, and any token
     ending in a file extension the file-class table knows, becomes
     `<path:class>` (for example `<path:source>`). This covers relative paths,
     and backticked names such as the "Updated `src/...`" final line;
   - the repository and folder name, case-insensitive, becomes `<repo>`. That
     is the last component of the session's working directory, and its parts
     split on `-`, `_` and `.` that are at least 4 characters long;
   - runs of two or more capitalised words that do not start a sentence
     become `<name>`. This over-masks code identifiers in prose, which is the
     safe direction;
   - digit runs of 6 or more become `<number>`.
4. **The residual gate.** The redactor's secret detector runs again over each
   masked excerpt. An excerpt with any surviving label is dropped and counted
   in `excerpts_dropped`. A session whose `ResidualPiiRisk`, computed by the
   existing `residual_risk` rules over its redaction report, is not `Low` sends
   no excerpts and no final line: counts only, as option A.
5. **Then the cut** to the byte budget ("Size").

**What leaves anyway, stated plainly:** words that none of these rules
recognise. A client name written in lower case, or a product name, can pass.
That residual is what O1 weighs, and it is why option C exists.

**The test covers the excerpts.** A test over the serialized digest bytes
(excerpts included, not "beyond the excerpts") asserts that none of a
fixture's sensitive substrings appears anywhere. The fixtures:

- "fix the refund bug in acme-billing/src/invoices/stripe_refund.rs for Jane
  Doe <jane@acme.com>", with the working directory `.../acme-billing`: no
  `acme`, `billing`, `invoices`, `stripe_refund`, `Jane`, `Doe` or
  `jane@acme.com`;
- a final line "Updated `src/payments/refund.rs`": no `src/`, `payments` or
  `refund.rs`;
- a secret (an API-key pattern the deterministic redactor knows) that starts
  at character 230 of a 400-character prompt: neither half appears;
- a URL with a query string, and an absolute path under the home directory;
- the same input built with `TRACE_PRIVACY_FILTER_BACKEND=near-ai` and
  `pii_filter = "near-ai"` set: byte-identical output. A structural test also
  asserts that the builder's module constructs a redactor only through
  `deterministic_only`.

#### Size (#1501 review point 9)

An earlier draft capped excerpts by characters and dropped any digest over
8 KiB whole. A dropped digest leaves its session unjudged, and the basis switch
needs every session in a comparison judged. So one long or non-Latin session
sent its whole comparison back to the local basis, every time. The fix is to
budget each field in serialized bytes, so the digest **cannot** pass the cap.

**Measured** on 2026-10-21 against #1525's digest shape, serialized compactly
(Python `json.dumps(..., ensure_ascii=False)`, which escapes `"`, `\` and
control characters as serde_json does), with every count at `u32::MAX`, 12
excerpts, 64 turn outcomes, 64 event markers and a 160-character final line:

| Case | One 240-char excerpt | Whole digest | Over 8,192? |
|---|---|---|---|
| ASCII | 240 B | 10,979 B | yes |
| CJK (3-byte) | 720 B | 17,059 B | yes |
| Emoji (4-byte) | 960 B | 20,099 B | yes |
| `"` and `\` | 480 B | 14,019 B | yes |
| Control characters (`\u0001`) | 1,440 B | 26,179 B | yes |

The fixed part alone (counts, no excerpts, outcomes or events) is 1,142 B. 64
turn outcomes add 3,647 B and 64 event markers 2,815 B. So even an ASCII
session goes over, through the per-turn outcomes alone.

**The budgets**, each measured on the serialized form after escaping:

| Field | Budget |
|---|---|
| Each prompt excerpt | At most 240 characters **and** at most 320 bytes serialized, cut at a character boundary. |
| Final line | At most 160 characters and at most 240 bytes serialized. |
| Turn outcomes | At most 64 rows. The ordinal saturates at 65,535 and every other count at 255 (255 means "255 or more"). Over 64 turns: the first 16, the last 16, and the 32 nearest the marked events, with `turn_outcomes_truncated`. |
| Event markers | At most 16, the earliest first, with `events_truncated`. The service judges 4. `prompt_ix` saturates at 65,535. |

Under these budgets, the same worst cases (a 500-turn session, 16 events, every
count at `u32::MAX`) measure:

| Case | Characters kept per excerpt | Whole digest | Headroom under 8,192 |
|---|---|---|---|
| ASCII | 240 | 6,613 B | 1,579 B |
| CJK | 106 | 7,629 B | 563 B |
| Emoji | 80 | 7,653 B | 539 B |
| `"` | 160 | 7,653 B | 539 B |
| Control characters | 53 | 7,629 B | 563 B |
| CJK mixed with control characters | 71 | 7,626 B | 566 B |

- The cap stays `JUDGEMENT_DIGEST_MAX_BYTES = 8192`. The budgets keep every
  digest under it **by construction**.
- If a future field ever pushed a digest over, the builder shortens the middle
  excerpts first, then drops middle excerpts, and records how many. It never
  drops the whole digest. A test asserts that the whole-digest refusal is
  unreachable from every fixture below.
- The tests, in `judgement_digest_tests.rs`: 12 maximal CJK prompts, 12
  maximal emoji prompts, 12 escape-heavy prompts (`"`, `\`, control
  characters), a 500-turn session with every count saturated, and 64 event
  markers. Each must build, be at most 8,192 bytes, and keep every excerpt
  slot it was given.

### Folder rules (O2: the design's default, for PM sign-off)

The counter pass skips only Never folders (`daemon/insights_week.rs`, module
docs). It reads Ask folders. Judgements must not inherit that, because a
contributor who sets a client repository to Ask expects nothing from it to
leave without review (#1501 review point 5). The judgement send has its own
rule, over the three folder modes (`ProjectMode` in `daemon/policy.rs`:
`AutoUpload` is Automatic, `NotifyOnly` is Ask me, `Ignore` is Never):

| Folder mode or state | Judged? |
|---|---|
| Automatic | Yes, once the person has turned on private judgements. |
| Ask me | Never automatically. Only a session the person approves, one at a time, after being shown exactly what would be sent. |
| Never, by its own rule or the global Never override | Never. Its transcript is not opened for a digest. |
| Contribution paused, the counter pass off (`insights_counter_pass`), or private judgements off | Nothing is sent from any folder. |
| Folder rules unreadable | Nothing is sent. Every folder is treated as Never, as `insights_week.rs` does for the counter pass. |

- The mode is read **at send time**, not when the session was counted. A
  folder moved to Ask or Never after counting sends nothing.
- When two policy entries collapse into one key, the more restrictive mode
  wins (`ProjectMode::more_restrictive`).
- Tests: a session in an Ask folder produces no request (a recording double
  sees no call); an approved Ask session produces exactly one; a Never folder
  and the global Never override produce none and never open the transcript;
  pausing contribution between counting and sending stops the send.

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
  - **What these do not cover** (#1501 review point 10). The operator can
    learn which accounts use judgements and when, in two ways. One is the
    issuer on ingest, which authenticates each claim request. The other is the
    app's KMS admission, since an admitted debug image can open the spend
    ledger and match its keyed digests against tenant ids the operator holds.
    The service spec, section 6, says what reduces this and what does not.
  - The consent copy says calls are tied to the account, and never says "the
    operator cannot see".
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
  with the same retention (13 weeks). Labels and the judgement state are
  cleared on opt-out from judgements, on unenroll, when the session's folder
  becomes Never (its own rule or the global override), and at the 13-week
  expiry. No digest or excerpt is ever stored, so there is nothing else to
  clear. Never on the wire to other parties.
- **Plugged into the existing seams:**
  - task type goes through the reserved `TaskClassifier` interface, as a second
    implementation;
  - rejection and finished judgements are new optional fields on the session
    facts.
  - `model_answers` gains a basis switch: it uses judged fields only when every
    session in a comparison has them under the same judge version, and
    otherwise falls back to the local basis and says so.

## Copy

New consent wording, following `docs/ui-copy-guide.md`, is **not decided**
(O3 under "Not decided"). It is written after O1, because it has to say what
O1 chose. It must say plainly:

- what leaves the machine (a short scrubbed summary per session);
- where it goes (an attested service, then NEAR AI's attested inference);
- who pays (Trace Commons, not you);
- what is kept, and where (#1501 review point 11):
  - **on this machine:** a label set and a fixed judgement state per judged
    session, on the counter row, for 13 weeks. No digest and no excerpt is
    stored ("When the digest is built");
  - **at the judgement service:** one spend entry per account (a keyed
    account digest, the month, and the amount spent), kept until the month
    ends. No digests and no labels;
  - **at NEAR AI and its gateway:** their own per-call records, under NEAR AI's
    retention terms. The copy quotes those terms once they are confirmed from
    NEAR AI's published policy. This spec has not verified them;
- how to turn it off, and what turning it off does: it deletes the labels and
  states on this machine, and it stops all sends. It cannot recall a digest
  already sent. The service's spend entry lapses at the month's end, and NEAR
  AI's records follow NEAR AI's terms.

Answers that use judged fields say "judged privately" in their (i).

## Not decided (owner and PM sign-off)

These are open. Nothing that sends a digest merges until O1 to O4 are signed
off.

- **O1. What leaves the machine:** option A, B or C under "What is sent".
  Recommendation: B with its gates.
- **O2. The folder rule:** the rule under "Folder rules" is the design's
  default, written so the code has one to test. It needs PM sign-off, because
  it decides which of a contributor's folders can send anything.
- **O3. The consent wording** (section "Copy"), after O1.
- **O4. How existing Insights users are told.** Turning the feature on is
  always opt-in. The open question is whether people who already use Insights
  see a one-time notice that it exists, and where.

Answered, for reference: decisions 1, 3, 4 and 5 above, and the service
spec's owner decisions 1 to 6.

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
   pure), #1525. It can be reviewed before the service exists. It does not
   merge until O1 is decided, and it changes to match O1.
2. The judged fields on the session facts, the `TaskClassifier` second
   implementation, and the basis switch in `model_answers`, all behind the
   opt-in and off by default.
3. The judgement service: CVM image, measurement pin, receipt verification,
   rate limiter. Its own spec, following the witness service spec's structure.
4. The account-bound limiter (keyed account digest, spend metering, $5 cap)
   inside the service.
5. Consent copy and the opt-in UI (macOS first), after O1 and O3.
6. Phase 2: comparisons across users. Its own spec first.
