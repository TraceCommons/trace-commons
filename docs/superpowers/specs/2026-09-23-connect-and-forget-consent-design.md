# Connect-and-Forget Contribution Consent — Design

Date: 2026-09-23
Status: draft for review
Scope: `trace-commons-contributor` (`daemon/policy.rs`, `daemon/uploader.rs`,
`daemon/eligibility.rs`, `consent_copy.rs`), the onboarding surface in each
shell, and one server-side visibility change
(`TraceCorpusStatus::AwaitingPiiBackstop`). No production code in this PR.
Motivated by `docs/contributor-ux-review.md`.

## What this is

Today a contributor approves sessions. The product asks them to be the last
check before anything leaves the machine, once per session or once per project
after they have earned the right to stop being asked.

The proposal is the flow the product brief calls connect-and-forget:

> Connect → consent to automatically contribute scrubbed sessions → sessions
> are scrubbed → contributed → earn

The UX review states the case plainly: *"I do not really want to become the
privacy filter myself. I want to understand and consent to the privacy model
once, then trust the app to apply it."*

This spec is about what has to be true for that sentence to be honest.

## The thing worth saying first

**Connect-and-forget does not remove a consent gate. It relocates one.** The
gate moves from per-session approval to a per-policy grant, and the safety
burden moves from a human backstop onto two machine backstops. Both of those
backstops already exist. The design's whole job is to make that shift honest
rather than silent.

It is also worth being clear about how much of this is already built, because
the answer is most of it.

| Capability | Where it lives | State |
|---|---|---|
| Per-project unattended upload | `daemon/policy.rs`, `ProjectMode::AutoUpload` | built |
| "Session is finished" inference | `daemon/eligibility.rs` | built |
| Unattended upload pipeline with re-hash and input-fingerprint guards | `daemon/uploader.rs` | built |
| Batched digest instead of per-session prompts | `daemon/settings.rs`, `digest_interval_secs` (4h default) | built |
| Independent server-side PII backstop, held out of distribution | `trace_corpus_storage.rs`, `TraceCorpusStatus::AwaitingPiiBackstop` | built |
| Withdrawal after the fact | `withdraw.rs`, `POST /v1/account/traces/{id}/withdraw` | built |
| Publishable list of what the scrubber looks for | `secret_leak_pattern_names()` | built |

What is missing is not a subsystem. It is the onboarding path that grants
`AutoUpload` at connect time, the copy that makes that grant truthful, and the
fallback rules that decide which sessions are *not* eligible for it.

## Four places where the current design says the opposite

Each of these is a deliberate decision with a written rationale. None should be
reversed by accident.

### 1. The arming threshold inverts

`policy.rs` offers to arm a project only after five successful contributions:

> Five, because the offer has to be backed by evidence the contributor actually
> has. Arming asks someone to stop reading previews from a project; the only
> honest basis for that question is that they have read several already and kept
> approving. One or two is a coincidence.

Connect-and-forget grants the same autonomy at zero. That is not a small
parameter change — it removes the stated basis for the grant.

**Proposal.** The basis changes rather than disappearing. Today it is
*contributor evidence*: you have seen five of these and they were fine. Under
connect-and-forget it becomes *disclosed mechanism*: you were told precisely
what the scrubber removes, what it cannot promise, and what you can do
afterwards. That is a weaker basis and the spec should say so out loud rather
than claim equivalence. It is defensible only if the disclosure is specific,
which is finding 3.

`ARMING_SUGGESTION_THRESHOLD` should not be deleted. It still governs the
review-everything mode, which the UX review keeps as an option.

### 2. Per-project specificity is load-bearing, and connect time has no projects

`policy.rs` opens:

> Autonomy is per-project and opt-in. An unknown project is `NotifyOnly`, so a
> freshly installed daemon uploads nothing until the contributor has
> deliberately said otherwise **about a specific project**.

At connect time there is no list of specific projects to say it about. A blanket
"everything, including things I have not opened yet" grant is a different act
from the one the current model was built around.

**Proposal.** The grant is prospective but not silent. `AutoUpload` becomes the
default mode for newly discovered projects, and every newly discovered project
produces a first-contribution notice naming it — not a prompt, a notice, in the
digest the UX review already asks for ("12 conversations contributed"). The
contributor learns a new repository started contributing on the first digest
after it did, not never.

**Non-negotiable:** `UNKNOWN_PROJECT_KEY` stays permanently `NotifyOnly`.
`policy.rs` locks it because the daemon cannot attribute those sessions to a
project, and therefore cannot honour any opt-in or any later exclusion for them.
Subagent transcripts and normalized trajectory files land there. A blanket grant
must not reach that bucket, or "exclude this project" becomes unenforceable for
the sessions most likely to need it.

### 3. `GATE_STATEMENT` becomes false

The sentence is:

> "Exactly what would be sent" is the exact text that would leave this machine.
> Pattern-based scrubbing may have missed something in it, and nothing here
> checks that you looked.

Under automatic contribution there is no "exactly what would be sent" screen and
nothing for anyone to have looked at. The sentence cannot survive unchanged, and
it must not simply be dropped: its doc comment records that it is what survived
when the acknowledgement checkbox was removed, kept precisely because the
*claim* had to outlive the friction.

**Proposal.** A parallel constant in `consent_copy.rs` for the automatic path,
stating what is actually true of it:

- what the scrubber removes, enumerated from `secret_leak_pattern_names()` —
  the names are already publishable by design, the regexes deliberately are not;
- that it is pattern-based and may miss things, which is unchanged and must stay;
- **that no person reviews a session before it is sent** — the new fact, and the
  one the old sentence never had to state;
- that there is a second, independent check server-side before anything is
  distributed (finding 4), and what withdrawal does afterwards.

This constant must be covered by `harness_copy_is_central.rs`. Note that scanner
currently hardcodes a path list of exactly three shells and has no needle for
`.tsx`, so a fourth shell would not be held to it — see the discussion on #963.

### 4. The approval-binding property loses its anchor

`uploader.rs` describes what it calls the central consent property of the whole
daemon: an approval pins an input fingerprint, and for a previewed entry the
exact bytes shown are written to disk and re-sent verbatim rather than
re-derived.

Under connect-and-forget nothing was previewed, so there are no pinned bytes.
The input-fingerprint guard still functions — it pins the session hash, the
filter selection, the backend and model, the consent scopes, the identity and
endpoints — but the property it protects changes from *"you saw these exact
bytes"* to *"the mechanism you consented to is the mechanism that ran."*

**Proposal.** Make that explicit and enforce it. The policy grant records the
filter configuration it was given under — backend, model, pattern-set version.
A change to any of them reverts affected projects to `NotifyOnly` until the
contributor re-consents. If the consent is to a mechanism rather than to bytes,
then changing the mechanism must void the consent. This is the direct analogue
of the existing re-offer on fingerprint mismatch, lifted from the session to the
policy.

## What compensates for the missing human

Three controls, all on substrate that exists.

**Auto-contribute only the confident path.** A session whose local redaction hits
a secret-leak pattern, or which ran while the LLM privacy filter was unavailable
or degraded, does not auto-upload. It falls back to `NotifyOnly` and appears in
the digest as needing a look. Automatic contribution is for sessions the scrubber
had no trouble with; the human comes back exactly where the machine is unsure.
This is the substantive safety mechanism in the proposal and it is the one most
worth arguing about.

**Make the server-side backstop load-bearing and visible.**
`AwaitingPiiBackstop` is documented as *"Never consumer/export/credit eligible
and never reviewer-eligible."* Today it is a second opinion behind a human first
opinion. Under connect-and-forget it becomes the only independent check between
a bad scrub and distribution. That is a promotion, and it should be stated in
the design and surfaced in the app rather than left as server-side detail.

**Keep exclusion cheap and retroactive.** The UX review's framing — *"exclude
selectively, rather than approve continuously"* — is right, and it only holds if
exclusion works after the fact. Withdrawal exists. Note that #989 reports
withdrawal currently returns `credit_retained: true` while unsettled credit
never settles; connect-and-forget raises the volume flowing through that path
and makes it more load-bearing.

## Open questions

- **Legal posture.** Moving from per-session approval to a one-time grant is
  plausibly a different consent basis, not a UI change.
  `docs/legal-counsel-review-checklist.md` should be run against this before it
  ships.
- **Corpus quality and credit.** Auto-contribution raises volume and probably
  raises duplication. Interaction with the dedup work (#975, #980, #985) and with
  credit quality (#969) is unmodelled here.
- **Does a blanket grant have a scope ceiling?** `ConsentScope` distinguishes
  `benchmark_only` through `model_training` and `public_attribution`. Whether a
  connect-time grant should reach the widest scopes, or cap at something
  narrower until the contributor widens it deliberately, is not settled here.
- **Migration.** What happens to existing contributors with per-project policies
  and earned arming offers. They should not be silently widened; `ProjectMode`'s
  `more_restrictive` merge rule exists because *"a merge may only ever ask more
  permission than before, never less."*

## Not in scope

No production code in this PR. The onboarding flow itself (Flow 1 of Epic 1),
the settings surface for review-everything mode, and the digest redesign are
each their own slice.
