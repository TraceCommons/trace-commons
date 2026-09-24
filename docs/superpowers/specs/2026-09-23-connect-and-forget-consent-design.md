# Connect-and-Forget Contribution Consent — Design

Date: 2026-09-23 (rev 5, 2026-09-24)
Status: draft for review
Extends: [`2026-08-31-contributor-trust-by-default-design.md`](2026-08-31-contributor-trust-by-default-design.md) (#507)
Source: [`../../contributor-ux-review.md`](../../contributor-ux-review.md)
Scope: `trace-commons-contributor` (`daemon/policy.rs`, `daemon/watcher.rs`,
`daemon/queue.rs`, `daemon/uploader.rs`, `consent_copy.rs`), the onboarding
surface in the Tauri client (#1003, merged as #963). No production code in this PR.

> **Rev 5** reverses rev 3's conclusion for invited contributors. R3's
> admission requirement is scoped to the `near-` and `nearai-` tenant
> namespaces, which an invited tenant is never in, so the gate that blocks
> wallet and NEAR AI-login enrollees does not apply to them. What they lack is
> a prose pass, and the witness supplies exactly that. Offering the witness at
> the grant makes invite the first enrollment that can have Flow 1 -- the
> opposite of what rev 3 said. The inference-receipt half of rev 2's Addendum 4
> stays withdrawn, with the reason recorded so it is not rebuilt.
>
> **Rev 4** keeps rev 3's structure, which review found sound, and fixes nine
> findings against it -- three of which were requirements that could not be met
> or checked anything as written: the certified value named did not exist, the
> consent gate sat on the branch that never sends, and logout undid both the
> arming rule and withdrawal durability.
>
> **Rev 3.** Two prior revisions each proposed a safety mechanism that the code
> does not implement: rev 1 derived automatic contribution from
> `residual_risk != High`, which cannot tell "no model ran" from "a model found
> nothing"; rev 2's addenda derived it from a standing `raw_session_confirmed`,
> which the automatic upload path never reads, and from "a witness is
> configured", which is not evidence a classifier ran.
>
> Rev 3 stops proposing mechanisms. It states what automatic contribution
> **requires**, lists the code facts that constrain each requirement, and is
> explicit that **no enrollment type qualifies today**. What changed is the
> method, not the goal.

## What this is

Epic 1's two flows. **Flow 1 (Connect-and-Forget)**: connect, consent once,
contribute automatically, earn. **Flow 2 (Customize-and-Tailor)**: per-folder
and per-session control. The UX review's ask is the narrower and better stated
one: *"exclude selectively, rather than approve continuously."*

The two-path structure survives all three revisions and is the part of this
design that has never been contested.

## The prior decision

#507, written from the same UX review, concluded that a global automatic
default *"should wait on a local content pass, not just a key-name pass"*, and
kept onboarding ask-first because *"arming automation before the contributor
has seen a single preview asks for trust they have no basis to give yet"*
(`OnboardingProjectsView.swift:15`).

**Rev 3 reverses both of #507's conclusions, and should say so rather than
claim agreement.**

- #507 kept onboarding ask-first *because the contributor has not seen a single
  preview*. None of R1-R7 requires one: Flow 1 still takes the grant at
  connect. The basis has moved from earned trust to disclosed mechanism, which
  is a weaker basis, as rev 1 already conceded.
- #507 also concluded that per-project arming was fine as-is. Putting the gates
  on the mode reverses that too.

Both reversals are defensible and neither is accidental, but they are
reversals.

**Folders already armed through today's arming offer do not meet R1-R7.** The
spec must choose: disarm them and re-ask, grandfather them with the gap
recorded, or migrate them as each next session supplies the evidence. This
design takes the third -- an armed folder keeps its mode, and its next session
is held rather than sent until R1 is satisfied for it -- because disarming
silently discards a decision the contributor did make, and grandfathering
carries the gap forever.

## What automatic contribution requires

Each requirement names the evidence that satisfies it and the code fact that
currently prevents it. **None is satisfied by configuration presence**, which
is the error rev 2 made twice.

### R1. A complete redaction pipeline actually ran, per session

Not "a witness is configured". The witness has a first-class
`deterministic-only` mode, and the client does not check the certificate's
`redaction_policy_version`.

`full-pipeline` is the witness's **startup mode name**, not a certified value,
so a check for that literal would reject every real certificate. What is
certified is `redaction_pipeline_version()`: the deterministic identifier
`ironclaw-deterministic-secret-path-v3`, alone when no classifier ran, or with
a `+<suffix>` when one did (`privacy-filter-near-ai-v1`, and the sidecar and
self-hosted equivalents).

- **Evidence:** a certified pipeline version carrying the deterministic
  prefix **and** one of the named classifier suffixes.
- **Not evidence:** the bare deterministic identifier with no suffix -- that is
  the deterministic-only run, and it is the value an implementation guessing a
  mapping is most likely to accept by mistake.
- **Not evidence:** `witness.is_some()`, `pii_filter == Some("near-ai")`.
- Note `pii_filter: None` can still pick up an environment-configured filter
  (`TRACE_PRIVACY_FILTER_BACKEND`), so the config field alone reads the
  situation wrongly in both directions.

### R2. The pipeline is specified, not inferred

The intended shape, which the spec should state rather than reverse-engineer:

- **The witness is the single, complete redaction point** — deterministic plus
  classifier, inside the enclave.
- **The client may run the deterministic pass first**, as defence in depth,
  **except** on the `HttpExchange` request and response bodies a NEAR AI
  receipt covers. The receipt binds `SHA256(request_body_as_sent)` and
  `SHA256(response_body_as_received)` and the witness verifies by hashing those
  exact bytes, so pre-scrubbing them breaks verification.
- **The consent gate for automatic sends is a fail-closed check on the
  _pinned_ witness path, before the send**, covering every `AutoUpload` route.

The branch matters and rev 3 had it wrong. `submit.rs:580` bails with
`witness_expected_measurement` when `!trust.is_pinned()`, before anything
reaches the network, so raw sessions leave **only** on the pinned path. A gate
on the unpinned branch would wrap a path that never sends -- the same shape as
rev 2's standing `raw_session_confirmed`, which is read only by the interactive
witness-preview request (`ipc.rs:3701`) and never by the automatic path.

### R3. Admission evidence exists for the session

**This is what makes Flow 1 unavailable to NEAR AI and wallet enrollees
today, and rev 2's addenda said the opposite.**

Ingest refuses new submissions from `near-` / `nearai-` tenants unless they
carry receipt-bound admission evidence (#706). That evidence comes from a
confirmed, per-session `prepare_admission_session` — exactly the per-session
step Flow 1 removes. Without an answer here, a wallet or NEAR AI-login
enrollee on Flow 1 has every session disclosed to the witness and then refused
with `admission_refused`: the disclosure happens and the contribution does not.

### R4. Provenance, where it is claimed

Rev 2's Addendum 5 proposed a receipt opt-in for invite enrollees. It does not
deliver provenance:

- a receipt exists only for sessions with an attested call, needing IronWire
  body capture and `ironwire_attested_bodies`, both off by default;
- under a not-required policy the witness certifies unattested sessions too,
  and the certificate records nothing about it (#1005);
- a receipt proves the last call, not the session, and a replayed receipt is
  not detectable;
- invite tenants are outside the admission ledger.

Provenance for attested traces comes from **#1005** — a v2 witness certificate
with a signed `inference_provenance` field — not from anything this spec
invents.

**Correction to rev 2:** its cost argument attributed #967–#985 to spam
filtering. Those fixed a scorer swap, a novelty floor and a simhash collision
among genuine traces. The claim that a certified receipt is cheaper than
statistical filtering needs different support, and is not made here.

### R5. A per-entry hold that works where the risk is known

For witness enrollees the secret-hit and `residual_risk` inputs exist only
after the witness has processed the session. So the hold can stop an upload to
the commons but **not** the send to the enclave: it is a post-witness hold, and
the spec must say so rather than implying the session is held before anything
leaves.

Because the unattended path does not keep the certified artifact, opening a
held session runs the witness again. **The certified artifact must be saved
when a session is held.**

### R6. A void rule, with an identity that includes the signer

Rev 1 had one; rev 2 deleted it and then cited it. Neither is acceptable.

The existing mechanism does not return a project to ask-first when inputs
change: an armed entry is revoked to `Pending` and the watcher re-approves it
next poll under the new inputs. So a changed witness — URL, pins, or
`signing_address` alone — gets sessions with no re-consent.

**Required:** a void rule whose identity includes `signing_address`, and which
holds re-approval until re-consent rather than letting the next poll resume.

### R7. The data-use scope is chosen

Flow 1 replaces today's consent screen, and that screen **is** the "How may
your traces be used?" scope picker. Rev 2 dropped it from both the mandatory
and deferred lists. NEAR AI and wallet enrollees start with empty scopes, so
every automatic contribution would ship at the floor scope with nobody asked,
or an implementer pre-selects scopes nobody chose. **Scope needs a named place
in Flow 1 onboarding.**

## Where that leaves availability

| Enrollment | R1 pipeline | R3 admission | R4 provenance | Flow 1 |
|---|---|---|---|---|
| Invite **with the witness offered at the grant** | satisfied by the enclave | **does not apply** | #1005 | **the first viable path** |
| Invite, as shipped | no prose pass | does not apply | — | no |
| NEAR AI login / wallet | satisfied by the enclave | **blocked by #706** | #1005 | no, until #706 |

**The invite path is the one that can work first, and rev 3 had this
backwards.** R3 is scoped to two tenant namespaces and invite tenants are in
neither. `ANCHOR_NAMESPACES` is `["near-", "nearai-"]`, and `admission.rs:76`
is explicit about the rest:

> A tenant in neither namespace is not refused, it is `None`: this is the
> invite-free path, and an invited tenant simply does not use it.

So the per-session admission step that Flow 1 removes, and that blocks wallet
and NEAR AI-login enrollees, is not a gate an invited contributor ever passes
through. Rev 3 marked invite "n/a" on R3 and then did not draw the conclusion.

What invite lacks is R1, and that is exactly what the witness supplies: the
enclave runs the full pipeline, which no local path does. **Offering the
witness at the grant therefore turns invite from the least eligible enrollment
into the only currently eligible one.**

**Rev 5 reverses rev 3's conclusion for invite enrollees.** Rev 2's Addendum 4
proposed offering the witness at the grant and pairing it with an inference
receipt; rev 3 withdrew the whole of it because the receipt half does not
deliver provenance. The witness half was withdrawn with it, and should not have
been -- the receipt was answering R4, and the witness answers R1. Rev 5 keeps
the witness offer and leaves provenance to #1005.

**The receipt half stays withdrawn**, and the reason is worth stating in full
because it is the part most likely to be reconstructed: `AttestedCall`
(`routing/attested.rs:282`) carries a single `request_body`, `response_body`
and `upstream_id`, and the receipt is a signature over one
`<requestHash>:<responseHash>` pair. **One receipt attests one call; a session
holds many.** A genuine call wrapped in an otherwise fabricated transcript
verifies. Separately, `chat_id` is `RoutedExchange::upstream_id`, which "exists
in the local proxy's SQLite ledger and nowhere else", so a receipt exists only
for contributors routing inference through the local proxy -- which most
invited contributors are not. It proves neither that the session is real nor
that it is theirs.

What stands between here and Flow 1 is more than the three external items.
External: #706's answer for automatic sends, #1005, and #507's local content
pass. Internal to this design and not yet built: R5's post-witness hold with a
saved certified artifact, R6's void rule, R7's scope placement, the
discovered-after-the-grant default with its audit row, withdrawal durability in
every mode, and the logout rule above.

## The two paths

Unchanged from rev 2 and not contested.

**Flow 1 — automatic.** Available only where R1–R7 hold.

**Flow 2 — choose myself.** Per-folder `Automatic` / `Ask me` / `Never`, and
per-session approve or decline.

### Flow 2's armed folders need the same gates

`AutoUpload` stays reachable inside Flow 2 — screen 5's "Automatic", the arming
offer, Settings, and CLI `--mode auto` — with none of R1–R7 attached. So rev 2's
claim that Flow 2 "costs the commons nothing, because a human is still reading
each one" is false for an armed Flow 2 folder.

**Decision required:** either R1–R7 apply to any `AutoUpload` project whatever
flow reached it, or the spec states plainly that arming in Flow 2 is automatic
contribution without them. This spec takes the first position: the gates belong
to the *mode*, not to the onboarding path.

### The default must not arm what already exists

A contributor-owned default read by `resolve()`'s `NotifyOnly` fallback is
silent arming. That fallback covers every project never explicitly ruled on,
discovery stores no mode, and only an explicit change writes one — so an
Automatic default would arm already-discovered projects and their queued
backlog, including sessions from before the grant, with no `armed-auto-upload`
audit row.

**Required:** the default applies only to projects discovered *after* the
grant, and writes an explicit policy entry and an audit row when it does.
`UNKNOWN_PROJECT_KEY` stays permanently `NotifyOnly`.

**Logout defeats both this rule and withdrawal durability, and the spec has to
say what happens.** Logout clears local state wholesale -- project modes, the
queue, history and the audit trail. After a logout and a Flow 1 re-grant every
rediscovered project counts as "discovered after the grant" and is armed,
including a folder previously set to `Never` and sessions predating the
original grant; and the record of what was withdrawn is gone, so the
durability requirement has nothing left to check against.

Two ways out: keep mode, withdrawal and discovery state across logout, or rule
that **a re-grant after logout arms nothing already on disk**. This design
takes the second as the floor, because it holds even when the first is
incomplete, and it fails in the safe direction: the contributor is asked again
about folders they had already decided.

## Deferral does not work under Flow 1

Rev 2 deferred the NEAR AI notice and token-distribution review to the point of
need. Neither recovers:

- **NEAR AI notice:** an entry refused before acknowledgement stays refused.
  Acknowledging later does not revive it and the watcher will not re-offer an
  unchanged session, so every session drained before acknowledgement is lost.
- **Token-distribution review:** it clears only through a per-entry witness
  review, so a one-time prompt cannot satisfy it, and armed entries cycle
  between revoke and re-approve with no health label.

Both contradict Flow 1's claim that nothing further will be asked, and the two
need different answers.

- **The NEAR AI notice can be settled before the grant.** #1001 already ships a
  recovery prompt for it, but that records the acknowledgement without reviving
  entries refused before it, so **the recovery must also re-offer those
  entries** or those sessions stay lost.
- **Token-distribution review cannot be settled before the grant**, because it
  clears only through a per-entry witness review. There is no one-time form of
  it. Either Flow 1 is unavailable while it is on, or it becomes a per-entry
  hold that the digest counts -- this design takes the hold, since the
  alternative silently disables the flow.

Source roots are a further gate of the same kind: the daemon needs them before
it will start, so they are settled at connect rather than deferred.

## The disclosure

Three constants in `consent_copy.rs` -- matching the three it carries today --
plus what review established must join them. `harness_copy_is_central.rs` checks `HARNESS_*` harness-state copy and never
consent copy, so it is the wrong test to name. The one that matters is
`tauri_copy_surface_is_central.rs`, which already scans `.ts` and `.tsx` and
already requires the Tauri preview to go through `consent_copy` -- so the
Tauri client is not unreachable by tooling, as rev 3 said.

**The prerequisite is extending that test, and the Swift and C# consent-copy
tests, to the new `AUTO_*` constants**, together with `ConsentCopy` itself,
which carries three fields today.

```rust
pub const AUTO_SCRUB_SCOPE: &str = "Fixed patterns remove API keys and tokens in the formats we know, and many file paths and email addresses that name you. Everything else -- names, employers, a password typed into a sentence -- is removed when a model recognises it.";

pub const AUTO_SCRUB_LIMIT: &str = "The patterns are reliable for the formats they cover and blind to everything else. The model is not reliable. Nothing here checks whether either of them was right.";

pub const AUTO_NO_REVIEW: &str = "No one looks at a session before it is sent, including you.";
```

`AUTO_NO_REVIEW` is the sentence the product will most want to soften and the
one that must not be.

**`AUTO_REVERSAL` is withdrawn rather than reworded.** Rev 2's text gave a
second, weaker erasure model than the canonical withdrawal copy, which the
server honours in every tier. **Reuse the canonical copy**, and make withdrawal
durability a required property for *every* folder mode — rev 2 required it only
in ask-first folders, and nothing stopped a withdrawn session in an Automatic
folder being re-offered once it grew.

Three further sentences are required and not yet written:

1. **The raw send, and both enclaves.** The session leaves unredacted for the
   witness enclave, whose classifier call then goes to NEAR AI's TEE-hosted
   privacy-filter endpoint. Two enclaves, two operators. Both are TEEs, so this
   is not an exposure — but describing one enclave when there are two is
   inaccurate, and the spec should say whether the witness verifies NEAR AI's
   attestation on each classifier call.
2. **How the witness got there.** NEAR AI and wallet enrollees did **not** meet
   it at signup: the witness URL, signer and pins are published by the commons
   and written silently at join, with join copy that says only *"Use the NEAR
   AI account you already sign in with to join a commons."* That is the
   server-pushed enablement this design claimed to rule out, and the grant has
   to disclose it. The witness is also not env-only — every shell's Settings
   can configure it.
3. **What enabling it costs elsewhere.** It is a single config field, so
   enabling it at the grant disables local preview in every "Ask me" folder:
   reviewing there would then send the unredacted session to the enclave first.

Separately, each receipt fetch tells the provider that an exchange is being
contributed, and no sentence covers that.

## Open

- **What stops spam on the invite path.** This is the cost of rev 5 being
  right about availability. Invited tenants sit outside the admission ledger,
  and the receipt mechanism does not substitute for it, so an invited
  contributor on Flow 1 contributes at volume with neither a per-session
  admission step nor attested provenance until #1005. Manual review was
  carrying part of that load and Flow 1 removes it. The honest options are to
  gate the invite Flow 1 grant on #1005 landing, to cap automatic volume for
  unattested tenants, or to accept the exposure and say so.
- **#706 under Flow 1**, still open for wallet and NEAR AI-login enrollees.
  Either admission evidence gets a non-per-session form, or Flow 1 stays
  unavailable to them while being available to invited contributors, which is
  an odd shape and worth deciding deliberately.
- **Whether R1–R7 gate the mode or the flow.** This spec says the mode; it is
  the single decision that most changes the implementation.
- **Measurement.** How many sessions would satisfy R1 -- a certified pipeline
  version carrying the deterministic prefix and a classifier suffix. Rev 2's plan read `trace_submissions.privacy_risk`, the server's
  re-scored value over sessions that survived client refusals — the wrong
  population. `residual_risk_basis` (#474, V52) gives the cause breakdown.
- **Shipped copy that this contradicts.** The arming copy's "Every future
  session" and "time to change your mind"; and the attached-daemon quit copy,
  which since #1000 is the shared `QUIT_ATTACHED_BODY` in `quit_copy.rs` used
  by the Tauri confirmation as well as GTK, with macOS carrying its own. It
  tells the contributor nothing will be sent while nobody is approving, which
  is **already wrong today for any armed Flow 2 folder**, not only under Flow
  1. Scoping the fix to the Linux dialog would leave Tauri and macOS users with
  the same false sentence.
- **Legal posture**, unchanged: a one-time grant is a different consent basis,
  and review established that withdrawing it is currently much harder than
  giving it.
