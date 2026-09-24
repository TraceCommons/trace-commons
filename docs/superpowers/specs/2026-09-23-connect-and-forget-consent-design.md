# Connect-and-Forget Contribution Consent — Design

Date: 2026-09-23
Status: revised after review (rev 2)
Extends: [`2026-08-31-contributor-trust-by-default-design.md`](2026-08-31-contributor-trust-by-default-design.md) (#507)
Source: [`../../contributor-ux-review.md`](../../contributor-ux-review.md)
Scope: `trace-commons-contributor` (`daemon/policy.rs`, `daemon/watcher.rs`,
`daemon/queue.rs`, `daemon/uploader.rs`, `consent_copy.rs`), the onboarding
surface in each shell. No production code in this PR.

> **Rev 2.** The first revision proposed flipping the default to automatic
> contribution and derived a routing rule from `residual_risk`. Review showed
> the rule does not hold for most contributors, and that #507 had already
> decided this question and named the prerequisite. This revision keeps the
> product goal, drops the blanket default, and reorganises around two
> first-class paths. The corrections are recorded in "What review established"
> rather than quietly absorbed.

## What this is

The product brief's Flow 1: *connect → consent → contribute automatically →
earn*. The UX review's ask is narrower and better stated: *"exclude
selectively, rather than approve continuously."*

## The prior decision this has to answer

#507 was written from the same UX review and reached a conclusion this design
initially contradicted without citing it. Its Pile 3 is the binding part:

> A *global* automatic default should wait on a local content pass, not just a
> key-name pass. Until then the product promise for auto-armed projects is
> "you decided this project is safe", not "we guarantee it is."

And its Pile 2 kept onboarding ask-first, quoting
`OnboardingProjectsView.swift:15`:

> arming automation before the contributor has seen a single preview asks for
> trust they have no basis to give yet.

**That prerequisite is still unmet, and this spec's own research confirms it
independently.** Rev 1 read the redactor and found two mechanisms of different
reliability — nine deterministic credential patterns plus email and path
passes, and an optional LLM prose filter. Review then established the part rev
1 missed: **the prose filter is not configured on any production path.** Every
config writer sets `pii_filter: None` (`commands.rs:66`, `:182`,
`account_onboarding.rs:913`, `nearai_onboarding.rs:413`), and
`envelope.rs:105-106` returns a deterministic-only redactor for `None`.

So #507's "not just a key-name pass" is still the shipped state for
contributors without a witness. **This spec therefore does not propose a
blanket automatic default.** It proposes the two-path model below, in which the
automatic path is available only where its disclosure is true.

## What review established

Recorded rather than absorbed, because several are reversals.

**The routing rule in rev 1 was wrong.** `residual_risk != High` cannot
distinguish "no prose model ran" from "a model ran and found nothing".
`coverage_incomplete` is set only for *a configured* backend
(`trace_contribution.rs:3901-3903`), so with `pii_filter: None` the prose step
returns silently, `message_text_included` yields Medium, and the session
auto-uploads having had no model pass at all — while `AUTO_SCRUB_SCOPE` claims
a model removed names and employers. Additionally `key_finding_detected` is set
only in `classify_structured_payload_node`, which only the server backstop
reaches (`:5247`, `:5422`), so client-side High mostly means "a field over
32,000 bytes" (`:5089-5091`).

**Rev 1 contradicted itself.** Its body held sessions that hit a secret-leak
pattern; its addendum's rule mapped `blocked_secret_detected` to Medium and
uploaded them, while presenting that as clarification. **The body's rule wins**:
all nine patterns are High/Critical (`:4318-4323`), the flag can be true over a
*partially* removed secret (`:3582-3587`), and `README.md:343-350` requires
review before submitting a surviving bearer-token shape.

**`AwaitingPiiBackstop` is not a standing check.** It is an operator opt-in that
ships disabled (`TRACE_COMMONS_PII_BACKSTOP_ENABLED`), applies only when
`risk_status == Accepted && carries_raw_content`, and is skipped under a witness
bypass. Rev 1 leaned on it as a compensating control. It cannot be one, and
client copy must not promise a server-side second check the client cannot
observe.

**The witness path was missing entirely.** For NEAR AI-login and wallet
enrollees, sessions leave the machine **raw and unredacted** for the redaction
witness (`submit.rs:1256-1300`; `witness/mod.rs:5-6` calls it "the largest
disclosure in this system"). Today that is disclosed per session and refused
without `raw_session_confirmed` (`ipc.rs:3581-3587`). Automatic contribution
removes that step and nothing replaced it.

**The grant would not have been prospective.** At connect every project is newly
discovered and arming has no cool-off, so the entire settled backlog is approved
within about two polls and uploads before any digest names a project
(`eligibility.rs:80-85`, `watcher.rs:592-594`).

**Several mechanisms rev 1 relied on do not reach far enough.** `Ignore` refuses
only Pending entries and `drain_approved` never re-reads the mode
(`queue.rs:780-784`, `mod.rs:557-566`), so exclusion does not stop an in-flight
backlog. Policy lookup falls back to `NotifyOnly` on a key miss
(`policy.rs:314-319`) — a fallback that becomes silent arming if the default is
implemented by changing it. Withdrawal is not durable: a withdrawn session that
is resumed and grows re-uploads under a new `submission_id`
(`eligibility.rs:141-157`). The digest cannot deliver a first-contribution
notice — it names three projects alphabetically, records no first contribution,
and has no replay (`notify.rs:113-118`, `mod.rs:1286-1303`).

**Smaller corrections.** Subagent transcripts merge into the parent session; the
guard that keeps staged trajectory imports from auto-uploading is the watcher's
`from_staging` check, not the unknown-project bucket. `input_fingerprint` does
not include the session hash and does cover consent scopes, witness and
endpoints (`preview.rs:295-331`); "pattern-set version" names nothing in the
code, and neither `REDACTION_RULESET_VERSION` nor the crate version tracks
`secret_leak_patterns` — #544 added `cursor_api_key` without moving either, so a
real pattern-set digest is needed. #989 was closed by #990, this branch's own
base commit.

## The proposal: two paths, both first-class

Adopted from review. Connect asks one question, and neither answer is an
advanced setting.

**Automatic** — contribute without per-session review, for contributors whose
work is safe to share by default.

**Choose myself** — per-folder and per-session control, for contributors doing
client, employer or otherwise sensitive work. This is not "review-everything
mode" hidden in settings; it is half the product.

Whichever is chosen becomes **the default mode for newly discovered folders**,
which is the only new policy concept required: today `ProjectMode` is per
project with a hardcoded `NotifyOnly` fallback, and this adds a
contributor-owned default that the fallback reads.

Required properties, each of which review showed is currently absent:

1. **Per-folder selection at connect and after.** Onboarding screen 5 already
   lists discovered projects; it gains Automatic / Ask me / Never. `Never` must
   take effect on entries already Approved but not yet uploaded, which means
   `drain_approved` re-reading policy.
2. **Per-session control in ask-first folders**, with three invariants: bulk
   approval never sends a session held for review; a decline is permanent across
   logout and re-grant; a withdrawn session is never re-offered or re-sent.
3. **A per-entry held-for-review state.** Today a risk verdict exists only
   inside `submit_loaded`, and returning an entry to Pending makes the watcher
   re-approve it next poll — a rebuild/classify/revoke loop that the digest never
   counts. The hold must be a queue state the watcher respects and bulk approval
   excludes.
4. **A revocation path.** Switching from automatic to choose-myself, globally or
   per folder, taking effect on everything not yet uploaded. Today no such path
   exists: per-project changes touch only Pending, there is no global default
   switch in `DaemonSettings`, and the only global stops are pause, source-off
   and CLI logout — which wipes the record of declines.
5. **Migration.** Existing contributors keep their per-project modes and stay
   ask-first for new folders unless they opt in. `ProjectMode::more_restrictive`
   exists because *"a merge may only ever ask more permission than before, never
   less."*
6. **A rule for the pre-existing backlog.** Only sessions first seen after the
   grant, or a mandatory preview of the backlog. Not the current behaviour,
   which uploads a contributor's whole history in about a minute.

## When is the automatic path's disclosure true?

The automatic path may run only where the sentence shown at connect is
actually true of the session. That is a narrower condition than rev 1 proposed
and it is the honest one:

- **A prose pass actually ran.** `pii_filter` configured, or a witness present.
  With neither, there is no model and the disclosure's second clause is false.
  Treat "no filter configured" as disqualifying, not as Low risk.
- **No secret-leak pattern hit.** Per the body's rule above, not the addendum's.
- **`residual_risk` is not High**, as a floor rather than the whole test.

This is deliberately restrictive. It means that today, for most contributors,
**the automatic path is not available at all** — which is the same conclusion
#507 reached, arrived at from the disclosure rather than from the threat model.
Widening it is the local-content-pass slice #507 named, and that slice is the
prerequisite for connect-and-forget being the default experience rather than an
option for contributors who have configured a filter.

## The disclosure

Four constants in `consent_copy.rs`. Note that `harness_copy_is_central.rs`
scans six harness files for `HARNESS_*` fields only; macOS and Windows receive
consent copy through the three-field `ConsentCopy` payload, so new constants
reach GTK alone unless that payload is extended.

```rust
/// What runs on every session, split by how much it can be trusted.
pub const AUTO_SCRUB_SCOPE: &str = "Fixed patterns remove API keys and tokens in the formats we know, your email addresses, and file paths that name you or your machine. Everything else -- names, employers, a password typed into a sentence -- is removed only if a model recognises it.";

/// The limit, and the descendant of GATE_STATEMENT's second clause.
pub const AUTO_SCRUB_LIMIT: &str = "The patterns are reliable for the formats they cover and blind to everything else. The model is not reliable. Nothing here checks whether either of them was right.";

/// The new fact. The old gate never had to state it.
pub const AUTO_NO_REVIEW: &str = "No one looks at a session before it is sent, including you.";

/// Reversal, stated at what it actually is.
pub const AUTO_REVERSAL: &str = "You can withdraw any session at any time, from History. Withdrawing before it has been shared deletes it. After that it stops further use, and anything already shared stays shared.";
```

`AUTO_NO_REVIEW` is the sentence the product will most want to soften and the
one that must not be. Three constraints on the set:

- **`AUTO_SCRUB_SCOPE` is only true where a prose pass ran**, which is why the
  eligibility rule above gates on it rather than showing this sentence
  regardless.
- **A witness enrollee needs a fifth sentence.** Their sessions leave raw. The
  per-session `witness_copy.rs:685` disclosure is being removed by this flow and
  has no replacement.
- **No constant may promise a server-side check.** The client cannot see whether
  the backstop is enabled.

`AUTO_SCRUB_SCOPE` also currently over- and under-states the deterministic tier:
path redaction is a case-sensitive exact match plus a POSIX-only regex, so a
Windows drive letter or doubled-backslash JSON ships the username; and "blind to
everything else" ignores the cue-gated contextual-entropy pass that removes
`password: <high-entropy value>` with no model. Both need the wording revised
against `trace_contribution.rs:4528-4543` and `:3423-3424` before this is final.

## What is not a control

`ARMED_SETTLE_SECS` is 24h and armed sessions do sit Pending for it, which looks
like a settle window that could serve as the reversal period. Its own doc
comment forbids that reading:

> it is not a control, and nothing should be built on it as though it were

because both inputs are contributor-manipulable (file mtime and wall clock). It
protects someone from sending an unfinished session, not from a mistaken grant.
Any real holdback would have to be server-side, and the earlier holdback
proposal is withdrawn on those grounds rather than on the grounds rev 1 gave.

## Open

- **Measure, on the pilot corpus, how many sessions would qualify** under the
  eligibility rule above. If the answer isnear zero without a configured filter,
  that is the argument for scheduling the local content pass, and the number to
  put in front of it. Note the earlier plan to read `trace_submissions.privacy_risk`
  measures the server's re-scored value over sessions that survived client
  refusals — a different population. `residual_risk_basis` (#474, V52) already
  provides the cause breakdown.
- **Does the automatic path ship before the local content pass exists?** On this
  spec's own rule it would be available to almost nobody. Shipping the two-path
  UI anyway has value — it makes the choice legible and is the revocation
  surface — but the "forget" half stays mostly theoretical until the pass lands.
- **Legal posture**, unchanged and now sharper: a one-time grant is a different
  consent basis, and review established that withdrawing it is currently much
  harder than giving it. `docs/legal-counsel-review-checklist.md` should be run
  against this.

---

# Addendum 2: what is consented to on first install, and in what order

Epic 1's two flows are the two paths above: **Flow 1 (Connect-and-Forget)** is
the automatic path, **Flow 2 (Customize-and-Tailor)** is choose-myself. This
addendum answers the sequencing question they raise — what a contributor must
agree to before anything can be sent, and what can wait.

## The gates that already exist

Onboarding today asks for everything up front, but the daemon does not depend
on that: it refuses a send on its own when a precondition is missing, and each
refusal is a named, fail-closed label. Before any session leaves:

| Gate | Where | Applies to |
|---|---|---|
| Enrollment is live (config + device key) | `uploader.rs:380`, `enrollment_is_live` | everyone |
| NEAR AI notice acknowledged | `uploader.rs:389`, `near-ai-notice-not-acknowledged` | NEAR AI configured |
| Token-distribution review | `uploader.rs:490`, `token-distribution-review-required` | opted in |
| Raw-session confirmation | `ipc.rs:3581`, `raw_session_confirmed` | witness enrollees |

The NEAR AI gate's comment is the model for all of them: the notice is
delivered interactively *because* a daemon that consumed the marker
non-interactively "would send the contributor's text to a third party with the
notice never actually delivered."

## The rule

**A step belongs in first-install onboarding only if a send cannot proceed
without it. Everything else moves to the moment it is needed, where the daemon
already refuses and the shell shows a recovery prompt.**

This is not speculative. #1001 has just done exactly that for the NEAR AI
notice: acknowledgement "was wired only into onboarding", so a contributor who
reached the condition outside that flow saw a generic "Daemon needs attention".
It now recovers in the queue with the full notice and fails closed until the
text has loaded. That is the pattern this rule generalises, and it is what lets
onboarding shrink without any promise being dropped.

Applying it to the eleven steps the prototype currently runs:

**Mandatory at first install**

1. **Connect** — invite or NEAR AI login or wallet. Nothing else is meaningful
   without an identity, and `enrollment_is_live` gates every send on it.
2. **The one question** — Flow 1 or Flow 2. This sets the default mode for
   newly discovered folders and is the only genuinely new policy concept.
3. **The disclosure matching the answer.** For Flow 2 this is today's consent
   screen. For Flow 1 it is the four constants above, and it must be complete,
   because Flow 1's whole claim is that nothing further will be asked.

**Deferred to the point of need, with a recovery path**

- NEAR AI notice, per #1001's pattern.
- Token-distribution review.
- Source roots beyond the defaults, and per-project modes. Flow 2 shows the
  discovered-projects screen at connect because choosing is the point of that
  flow; Flow 1 does not, because it has nothing to ask.
- The optional third-party scan, which is already optional.

## The one thing that cannot be deferred

**For a witness enrollee on Flow 1, the raw-upload disclosure has to move
forward into the grant.**

Their sessions leave the machine unredacted for the enclave — `witness/mod.rs`
calls it "the largest disclosure in this system." Today `ipc.rs:3581` refuses
without `raw_session_confirmed` and `witness_copy.rs:685` says it per session.
Under Flow 1 there is no per-session moment, so a deferred prompt would either
block every automatic send forever — making Flow 1 non-functional for exactly
the enrollees most likely to choose it — or be acknowledged once somewhere that
is not the consent screen, which is the "notice never actually delivered"
failure the NEAR AI comment warns about.

There is a second reason, which the shipped disclosure states and rev 2's
`AUTO_REVERSAL` does not survive: *"Cancelling afterwards cannot recall a
session already sent to the witness"* (`witness_copy.rs:685`). The witness
upload is irreversible the moment it happens, before any contribution decision
exists. So for a witness enrollee, "you can withdraw any session at any time"
is not the whole reversal story — withdrawal governs the commons, and nothing
governs the enclave. That gap is invisible today because a person confirms each
one; under Flow 1 nobody does.

So for these contributors Flow 1's disclosure carries a fifth sentence, and the
grant is not obtainable without it. A contributor who declines it is on Flow 2.
That is the honest outcome: their traces cannot be sent unattended under a
disclosure that omits the largest thing that happens to them.

The general form of the rule: **anything that will happen without a further
prompt must be disclosed before the grant, not at the moment it happens** —
because under Flow 1 that moment has no one in front of it.

## What this leaves open

- **Flow 2 is under-specified here.** Epic 1 gives it four steps — tool
  selection, session and repo permissions, scrub model, contribute — and only
  the middle two map onto anything that exists. The "choose scrub model
  (auto/manual)" step in particular has no counterpart in the code: there is no
  contributor-facing scrub-model choice, and `pii_filter` is unset on every
  production path.
- **Where the one question renders**, given the copy reaches GTK alone unless
  `ConsentCopy` is extended past its three fields.
- **Whether declining the witness disclosure should be reversible** into Flow 1
  later, and what re-grants.

---

# Addendum 3: witness enrollees are Flow 1's population, not its exception

Addendum 2 treated the witness path as an obstacle to Flow 1 — a disclosure
that has to move into the grant, failing which the contributor is on Flow 2.
That is the right mechanism and the wrong framing, and inverting it changes who
Flow 1 is for.

## The enclave is the model pass

Rev 2's eligibility rule requires that a prose pass actually ran, and observed
that no production path configures one: `pii_filter: None` everywhere,
`envelope.rs:105-106` returning a deterministic-only redactor.

For a witness enrollee that is not the path taken. The raw session goes to a
verified enclave which performs the redaction, calling NEAR AI from inside it.
A real classifier runs over the whole session, which is exactly what
`AUTO_SCRUB_SCOPE` claims and exactly what the deterministic-only path cannot
deliver.

So the population splits the opposite way from how Addendum 2 read it:

| Enrollment | Prose pass | Is `AUTO_SCRUB_SCOPE` true? | Flow 1 available |
|---|---|---|---|
| NEAR AI login or wallet (witness set) | in the enclave | **yes** | **yes, today** |
| Invite, no witness, no `pii_filter` | none | no | no, until the local content pass |

**Flow 1 is buildable now for precisely the contributors Addendum 2 was
pushing towards Flow 2.** They are not the exception; they are the only
population whose disclosure is currently honest.

## What has to change for them to reach it

One thing: the per-session raw-upload confirmation becomes a standing one.

`ipc.rs:3581` refuses a witness send without `raw_session_confirmed`, and
`witness_copy.rs:685` says it per session. Flow 1 has no per-session moment, so
the confirmation has to be given once, at grant time, as part of the
disclosure — a fifth sentence covering the raw upload to the enclave and its
irreversibility (*"Cancelling afterwards cannot recall a session already sent
to the witness"*), plus a standing `raw_session_confirmed` the uploader honours
for auto-contributed sessions.

## Why this does not weaken the property that makes raw upload acceptable

It is worth being explicit, because "remove the confirmation click from the
largest disclosure in the system" reads as a weakening and is not one.

The property is stated in `witness/mod.rs`: the raw send is acceptable **only
because the enclave's measurement was verified first**, and that ordering is
enforced by types rather than by review — `VerifiedWitness` has private fields
and one constructor which *is* the verification, and the only function that
transmits raw bytes takes a `&VerifiedWitness`. There is no path from having a
witness URL to sending raw bytes that skips it.

That check is structural and runs on every send. It does not consult
`raw_session_confirmed` and is unaffected by how the contributor consented. So
a standing confirmation replaces the **human click**, not the **attestation**.
What the contributor gives up is being asked each time; what protects them —
that the bytes only ever reach a measured enclave — is untouched, and cannot be
turned off by this or any other consent change.

The honest statement of the trade is therefore narrow: under Flow 1 a witness
enrollee is not re-asked before each raw upload, and an upload that has
happened cannot be recalled. Both belong in the grant-time sentence.

## Consequences

- **The standing confirmation must void with the witness.** The grant records
  the witness identity it was given under, and a change to the witness URL or
  its expected measurement voids it, returning affected projects to ask-first
  until re-consented. This is the same rule rev 2 applies to the filter
  configuration, and it matters more here: the enclave's identity is the whole
  basis on which raw upload was acceptable.
- **`AUTO_REVERSAL` still needs its witness clause.** Withdrawal governs the
  commons; nothing governs the enclave.
- **Flow 1's availability is now a sequencing question with an answer.** Ship
  it for witness enrollees, where the disclosure is true today, and let the
  local content pass (#507) bring the rest in later. That is a smaller first
  slice than "Flow 1 for everyone" and a larger one than "Flow 1 for nobody",
  which is what rev 2 amounted to in practice.
