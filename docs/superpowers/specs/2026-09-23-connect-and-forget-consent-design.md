# Connect-and-Forget Contribution Consent — Design

Date: 2026-09-23 (rev 3, 2026-09-24)
Status: draft for review
Extends: [`2026-08-31-contributor-trust-by-default-design.md`](2026-08-31-contributor-trust-by-default-design.md) (#507)
Source: [`../../contributor-ux-review.md`](../../contributor-ux-review.md)
Scope: `trace-commons-contributor` (`daemon/policy.rs`, `daemon/watcher.rs`,
`daemon/queue.rs`, `daemon/uploader.rs`, `consent_copy.rs`), the onboarding
surface in the Tauri client (#1003). No production code in this PR.

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

Rev 3 does not overturn that. It agrees with it and adds the reasons review
surfaced.

## What automatic contribution requires

Each requirement names the evidence that satisfies it and the code fact that
currently prevents it. **None is satisfied by configuration presence**, which
is the error rev 2 made twice.

### R1. A complete redaction pipeline actually ran, per session

Not "a witness is configured". The witness has a first-class
`deterministic-only` mode, and the client does not check the certificate's
`redaction_policy_version`.

- **Evidence:** the certified `redaction_policy_version` says `full-pipeline`,
  or the envelope's pipeline version and privacy-filter summary say so.
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
  unpinned witness branch**, covering every `AutoUpload` route.

That last point replaces rev 2's standing `raw_session_confirmed`.
`raw_session_confirmed` is read only by the interactive witness-preview
request (`ipc.rs:3701`); it is not a gate on the automatic path, so a standing
version of it would wrap a check that path never makes.

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
| NEAR AI login / wallet | possible, unverified per session | **blocked by #706** | #1005 | **no** |
| Invite | no prose pass by default | n/a | not deliverable as rev 2 proposed | **no** |

**No enrollment type qualifies today.** Rev 2's Addenda 3 and 4 claimed
witness enrollees qualified "today" and that invite enrollees could be brought
in; both are withdrawn. The work that changes this is #706's answer for
automatic sends, #1005, and #507's local content pass.

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

## Deferral does not work under Flow 1

Rev 2 deferred the NEAR AI notice and token-distribution review to the point of
need. Neither recovers:

- **NEAR AI notice:** an entry refused before acknowledgement stays refused.
  Acknowledging later does not revive it and the watcher will not re-offer an
  unchanged session, so every session drained before acknowledgement is lost.
- **Token-distribution review:** it clears only through a per-entry witness
  review, so a one-time prompt cannot satisfy it, and armed entries cycle
  between revoke and re-approve with no health label.

Both contradict Flow 1's claim that nothing further will be asked. **These
gates are settled before the grant, or the recovery re-offers affected
entries.**

Source roots also belong in the mandatory list: the daemon needs them before it
will start.

## The disclosure

Four constants in `consent_copy.rs`, plus what review established must join
them. `harness_copy_is_central.rs` scans six harness files for `HARNESS_*`
fields; macOS and Windows receive consent copy through the three-field
`ConsentCopy` payload, and the Tauri client — named the main client in #1003 —
is TypeScript the scanner cannot read at all. **Extending `ConsentCopy` and
giving the scanner a `.tsx` needle are prerequisites, not follow-ups.**

```rust
pub const AUTO_SCRUB_SCOPE: &str = "Fixed patterns remove API keys and tokens in the formats we know, your email addresses, and file paths that name you or your machine. Everything else -- names, employers, a password typed into a sentence -- is removed by a model.";

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

- **#706 under Flow 1.** The blocking question. Either admission evidence gets
  a non-per-session form, or Flow 1 is unavailable to the tenants it was
  designed for.
- **Whether R1–R7 gate the mode or the flow.** This spec says the mode; it is
  the single decision that most changes the implementation.
- **Measurement.** How many sessions would satisfy R1 with a `full-pipeline`
  certificate. Rev 2's plan read `trace_submissions.privacy_risk`, the server's
  re-scored value over sessions that survived client refusals — the wrong
  population. `residual_risk_basis` (#474, V52) gives the cause breakdown.
- **Shipped copy that this contradicts.** The arming copy's "Every future
  session" and "time to change your mind", and the Linux attached-daemon quit
  dialog's "Nothing will be sent while nobody's approving", which would steer a
  contributor to Quit while the daemon keeps uploading.
- **Legal posture**, unchanged: a one-time grant is a different consent basis,
  and review established that withdrawing it is currently much harder than
  giving it.
