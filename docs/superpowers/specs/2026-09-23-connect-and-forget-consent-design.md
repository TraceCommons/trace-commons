# Connect-and-Forget Contribution Consent — Design

Date: 2026-09-23 (rev 8, 2026-09-25)
Status: draft for review
Extends: [`2026-08-31-contributor-trust-by-default-design.md`](2026-08-31-contributor-trust-by-default-design.md) (#507)
Source: [`../../contributor-ux-review.md`](../../contributor-ux-review.md)
Scope: `trace-commons-contributor` (`daemon/policy.rs`, `daemon/watcher.rs`,
`daemon/queue.rs`, `daemon/uploader.rs`, `consent_copy.rs`), the onboarding
surface in the Tauri client, named the main client for the MVP in #1003 and
merged in #963. No production code in this PR.

> **Rev 8** adopts the trust model set out in review on #991 (2026-09-25):
> every contributor has a NEAR AI login, an invite is full trust, and trust
> otherwise lives on the account and is enforced on the server. The consent
> decision becomes "arm this folder"; trust decides how much an armed folder
> may send. That changes what several requirements are *for*: R1 now governs
> what the arming disclosure may claim rather than whether a folder may be
> armed, R3 becomes a server-side volume check, and already-armed folders stay
> armed rather than being disarmed. R6 is now specified in full. See "The
> trust model", which takes precedence over earlier sections where they
> disagree; those sections are kept as the record of how the design got here.
>
> **Rev 7** records three answers settled in review. The witness reaches an
> invited contributor through **connected inference**, which is the
> contributor-chosen relationship the no-server-pushed-enablement rule needs;
> an invitee who has not connected inference gets no witness at the grant.
> Connected inference is **one route** to a witness and receipts rather than
> the door to automatic contribution. And there is a **third path**: automatic
> contribution earned incrementally from trust signals gathered during and
> after onboarding, whose mechanism is deliberately left open.
>
> Gate placement and the R1 allowlist were confirmed as rev 6 implemented them.
>
> **Rev 6** fixes the findings against revs 4 and 5 that do not depend on an
> open question, and marks the two that do. The gate moves to the `AutoUpload`
> decision -- misplaced in three consecutive revisions, each time by gating a
> branch rather than the decision. R1 becomes an exact allowlist, because the
> classifier suffix is stamped from configuration and `sidecar` fails open. The
> armed-folder migration was circular and becomes disarm-with-notice.
>
> **Two questions are open and rev 6 does not guess at them**: where an invited
> contributor's witness configuration comes from, and whether connecting
> inference belongs in onboarding. Both are in Open.
>
> **Rev 5** reverses rev 3's conclusion for invited contributors. R3's
> admission requirement is scoped to the `near-` and `nearai-` tenant
> namespaces, which a derived invite tenant is never in, so the gate that blocks
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

The two-path structure survives every revision and is the part of this
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

**Folders already armed through today's arming offer do not meet R1-R7.**

Rev 4 proposed migrating them -- keep the mode, hold the next session until R1
is satisfied. **That is circular and is withdrawn.** R1's evidence is a witness
certificate, which exists only *after* the session has gone to the witness. So
"hold until R1 is satisfied" either sends the raw session to obtain the
evidence, bypassing the gate, or can never be satisfied at all. For an invited
contributor with no witness it is strictly the second: the folder would show
Automatic while holding every session indefinitely, which discards their
decision more thoroughly than disarming would. Rev 4 also checked only R1, so a
migrated folder could auto-contribute without R3, R6 or R7, contradicting the
rule that the gates attach to the mode.

**Rev 6 took disarm-with-notice; rev 8 withdraws it.** Under the trust model
below there is no blanket disarm. An invitee's armed folders stay armed,
because an invite is full trust. A non-invitee's armed folders stay armed and
send within their account's server-side limits. What changes for an
already-armed folder is its disclosure, not its mode: where R1 does not hold,
its arming copy must not claim a model scrubs it (see "The trust model").

**The contributor is told when that happens.** A folder armed under the old
"will be scrubbed" copy whose wording becomes deterministic-only gets the same
notice a newly automatic folder gets, stating what its arming now means.
Changing the words under an armed folder without saying so would break the
"nothing silently" property this design depends on.

## The trust model

Adopted in review on #991 and taking precedence over the sections after it
where they disagree.

1. **Every contributor has a NEAR AI login.** Settlement is in NEAR AI
   inference credits, so everyone has a NEAR account.
2. **A folder whose traces have witnesses can be armed automatically, and
   sends once the contributor consents.** What "have witnesses" means is open;
   see below.
3. **An invite is full trust.** An invited contributor may arm any folder,
   with no volume limit.
4. **Without an invite, a contributor may still arm any folder**, and sends
   within server-side limits set by how far the server trusts their account.
   That trust grows over time; it is Flow 3's mechanism.

So **the consent decision is "arm this folder"**, and **trust decides how much
an armed folder may send**. Trust belongs to the account and is enforced on
the server, not in a per-session step on the client.

### Which requirements full trust relaxes

| | Invitee (full trust) | Non-invitee (trust-limited) |
|---|---|---|
| **R1** pipeline verified per session | **relaxed as a gate.** A folder without a witness may be armed and sends after deterministic redaction only. R1 still governs the **disclosure**: the model-scrub wording may be shown only where a certified full pipeline actually ran. | same as invitee: R1 governs the disclosure, not the arming. Volume is limited on the server instead |
| **R2** pipeline specified, gate at the `AutoUpload` decision | unchanged | unchanged |
| **R3** admission evidence | **applies until #1020's account check is enforced.** Once invites move onto the NEAR account an invitee is in `nearai-`, where #706 still applies; after #1020 is enforced, it does not apply | **becomes the trust-limited volume check** (see below) once #1020 is enforced; until then, #706 as today |
| **R4** provenance where claimed | unchanged: claim it only where #1005 supports it | unchanged, and it feeds account trust |
| **R5** held for review | unchanged | unchanged |
| **R6** void rule | unchanged; see R6 | unchanged |
| **R7** scope chosen | unchanged | unchanged |

The principle behind the table: **trust relaxes what may be sent, never what
may be said.** Full trust lets an invitee's folder send without a verified
model pass, but no contributor is ever told a model scrubbed a session that
none did. The arming copy therefore has two forms, chosen by whether R1
holds: the `AUTO_*` wording where a certified full pipeline ran, and a
deterministic-only wording otherwise, which is still to be written.

### #706 under the trust model

With a NEAR AI login for everyone, contributors without an invite land in the
`nearai-` namespace, where #706 refuses submissions lacking per-session
admission evidence -- evidence that is prepared interactively, per session,
and that an armed folder never prepares. Left as it is, every automatic send
from a non-invitee is refused.

**The design takes the first of the two answers:** admission becomes the
account's trust-limited volume check, enforced on the server, rather than
evidence produced unattended under the folder-level consent. A per-session
artefact produced by nobody attests to nothing. The server-side work titled
"Reserve contribution processing against account trust" (#1020) is the
candidate implementation; this spec depends on its outcome rather than
restating it.

### An invite trusts the account, not a separate tenant

An invite today creates its own `tenant-…` identity from the device key, so
one person can hold two identities, and logging out, which discards the device
key, loses the invite. Under this model **an invite raises the trust of the
contributor's NEAR account**: one person, one identity, and an invite that
survives logout. "Grant invite trust to authenticated NEAR accounts" (#1016)
is the corresponding server work. Consequences here:

- R3's namespace test stops distinguishing invitees from everyone else, since
  everyone is `nearai-`. What distinguishes them is account trust, which the
  client does not decide. The client-side gate's R3 check is withdrawn **once
  #1020's check is enforced on the server, and not before**: withdrawn
  earlier, an armed folder's sessions go to the witness and classifier and
  are then refused at ingest, which is what R3 exists to prevent. It is on the
  switch-on list below.
- The logout rule below (a re-grant arms nothing already on disk) still
  stands for folder modes, which remain local.

### "Traces with witnesses" names two different rules

For automatic arming this has to mean one of two things, and they lead to
different rules:

- **(a) Redaction quality:** the contributor has a witness configured, so
  every session in the folder is certified by the enclave. This is what R1's
  disclosure half turns on.
- **(b) Provenance:** the folder's sessions include NEAR AI-routed inference
  with receipts, attested per #1005. This is what R4 and account trust turn on.

Which one point 2's automatic arming turns on is to be confirmed in review;
it is in Open. It is not a question about arming as such: any contributor may
arm any folder (points 3 and 4).

### The client-side gate under this model

The gate stays where rev 6 put it, at the decision to approve on the
contributor's behalf, before either upload branch (pending in #1012). Its
checks change:

- **R7** (scope chosen): kept.
- **R1**: withdrawn as a gate. It moves to choosing the arming disclosure.
- **R3**: withdrawn once #1020 is enforced, as above; the server then decides
  volume. Kept until then.

**When enforcement is switched on:** `automatic_gate::ENFORCED` ships `false`
and is switched on only when all of these hold, so the dry run cannot become
the permanent state by default:

1. #1020's account admission check is enforced on the server, so withdrawing
   the client's R3 check sends nothing that ingest will refuse;
2. Z2 (#1005) is in place, so R4 is claimed only where it holds;
3. the label-only would-refuse log is live (pending in #1012), together with
   the count of sessions an enforced gate holds and a label-only health
   condition that every shell shows, so enforcement cannot stop an armed
   folder without saying so;
4. the gate's checks are revised to the list above.

R1 is not on this list, because rev 8 withdraws it as a gate. It still decides
which disclosure an armed folder gets, and the model-scrub wording needs the
published pipeline-version allowlist (Z1, #1013, merged) and the client's check
against it (K6); until both exist, no folder qualifies for that wording.

It is switched on together with the arming-disclosure change, not with a
disarm: under this model already-armed folders stay armed.

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

**A prefix match is not sufficient, because the suffix is stamped from
configuration rather than from what ran.** The `sidecar` backend fails open: it
catches the error, marks coverage incomplete, returns the deterministic output,
and the certificate still carries `+privacy-filter-sidecar-v1`. The
fail-closed test covers only the NEAR AI and self-hosted backends. A prefix
match would also accept any future or unknown suffix.

- **Evidence:** the certified pipeline version appears in an **exact
  allowlist**, modelled on the server's
  `WitnessBypassConfig::policy_version_allowed`. `sidecar` stays off that
  allowlist until it fails closed.
- **Not evidence:** the bare deterministic identifier with no suffix -- that is
  the deterministic-only run.
- **Not evidence:** any suffix matched by prefix rather than by membership,
  which would admit both the fail-open sidecar and anything added later.
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
- **The consent gate for automatic sends sits at the `AutoUpload` decision, in
  policy or the uploader, ahead of either branch.**

**Placement has been wrong in three revisions and the reason is the same each
time: gating a branch rather than the decision.** Rev 2 put it on a standing
`raw_session_confirmed`, read only by the interactive witness-preview request
(`ipc.rs:3701`). Rev 3 put it on the unpinned witness branch, which never
sends. Rev 4 put it on the pinned witness path, which misses automatic uploads
entirely when no witness is configured -- the client then redacts locally and
uploads without entering the witness path at all, which is exactly the route an
invited contributor with an armed folder takes.

Since the gates attach to the mode rather than the onboarding path, the only
placement that covers every route is the decision itself, before either branch
is chosen.

For the avoidance of a fourth misplacement: `submit.rs:580` is **not** the
automatic path. It sits inside `prepare_token_review`, the interactive
token-distribution route. The automatic submission path's pin check is the
`match settings.trust()` in the witness-settings arm, around `submit.rs:1263`,
backed by `witness::witness_session`. A gate placed beside the first citation
protects token review and leaves automatic sends open.

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

**Specified in review; implementation pending in #1024.** The identity is structured terms,
not `input_fingerprint`, which hashes the crate version and would void every
grant on every release: the destination (ingest and issuer endpoints, audience,
host allowlist), the identity (tenant, instance, subject, device), the consent
scopes, the privacy filter and its classifier host and model, the receipt
endpoint, the witness URL, `signing_address` and measurements, and attested
bodies. The NEAR AI API key stays out.

A grant is voided -- the project returns to ask-first and the void is audited
-- when **the parties who see content grow or change, or the content that
leaves grows**: any change of destination, identity, witness or classifier;
scopes gaining an entry; a filter added or removed; a receipt endpoint added
or changed; a witness measurement admitted; attested bodies turning on.
Scopes narrowing, attested bodies off, a receipt endpoint removed and a
measurement retired do not void: each is one fewer party or less content.

**Admitting a measurement voids**, even under an unchanged `signing_address`,
because different code then sees the session. It happens before every witness
rollout, so each rollout returns every armed folder to ask-first; that is the
accepted cost.

**The invite-to-account migration re-baselines rather than voids.** Moving an
invitee from their `tenant-…` identity to their NEAR account (#1016) changes
the identity term, and left to the rule above it would disarm every folder
they armed, contradicting "an invitee's armed folders stay armed". The client
performs that migration itself, so as part of it the daemon re-records each
armed folder's terms with the new identity and audits the re-baseline. Only
the identity term is re-baselined: if the migration also changes anything
else -- destination, witness, scopes -- that still voids. A general
same-person exception in the identity term is not taken, because the client
cannot tell "same person" from any other identity change outside this one
step.

### R7. The data-use scope is chosen

Flow 1 replaces today's consent screen, and that screen **is** the "How may
your traces be used?" scope picker. Rev 2 dropped it from both the mandatory
and deferred lists. NEAR AI and wallet enrollees start with empty scopes, so
every automatic contribution would ship at the floor scope with nobody asked,
or an implementer pre-selects scopes nobody chose.

**The placement, named rather than deferred:** the scope picker runs
immediately after connect and before the path question, it blocks any grant,
and it has no default. A contributor who declines to choose does not
get a floor-scope grant -- they get no grant, and land on Flow 2.

## Where that leaves availability

> **Superseded by "The trust model".** Under it, an invitee may arm any folder
> and a non-invitee may arm any folder within server-side limits, so
> availability is a question of disclosure and volume rather than of the
> table below. Kept as the record of how the design got there. Its R3 column
> describes invites in their own `tenant-…` namespace, as shipped; once
> invites move onto the NEAR account, R3 is as the trust-model table says.

| Enrollment | R1 pipeline | R3 admission | R4 provenance | Flow 1 |
|---|---|---|---|---|
| Invite **with a witness** | satisfiable once the client checks the certified version | **does not apply** | #1005 | **the first path R3 does not block** |
| Invite, as shipped | no prose pass | does not apply | — | no |
| NEAR AI login / wallet | satisfiable once the client checks the certified version | **blocked by #706** | #1005 | no, until #706 |

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
enclave runs the full pipeline, which no local path does. **A witness therefore removes
invite's R1 problem, which is what leaves it the only enrollment with no
structural blocker.**

That is narrower than eligible, and rev 5 overstated it. R4 waits on #1005 and
R5-R7 are unbuilt, so Flow 1 cannot be granted to anyone yet.

**Where the witness comes from, settled in review: connected inference.** An
invited contributor receives the witness as part of deliberately connecting
NEAR AI inference, which is the contributor-chosen relationship the
no-server-pushed-enablement rule requires -- the enclave arrives with something
the person asked for rather than as a side effect of joining. **An invited
contributor who has not connected inference does not get a witness from the
commons at the grant**, and stays on the local-redaction path.

Connecting inference is therefore **one route** to a witness and to receipts.
It is not a precondition for contributing, and it is not the only door to
automatic contribution -- see the third path below.

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
External, and no longer uniform across enrollments: #706's answer for automatic
sends, which blocks wallet and NEAR AI-login enrollees and does not apply to
invited ones; #1005 for provenance; and #507's local content pass, which
matters for an invited contributor **without** a witness and is not on the path
for one with a witness, since the enclave supplies R1. Internal to this design and not yet built: R5's post-witness hold with a
saved certified artifact, R6's void rule, R7's scope placement, the
discovered-after-the-grant default with its audit row, withdrawal durability in
every mode, and the logout rule above.

## The three paths

Flow 1 and Flow 2 are unchanged from rev 2 and have not been contested. The
third was settled in review and is new here.

**Flow 1 — automatic from the grant.** Available only where R1–R7 hold. A
single decision at connect, covering everything after it.

**Flow 3 — earned.** Automatic contribution is **granted incrementally**,
from trust signals gathered during and after onboarding, rather than in one
act at connect. It is not a lesser Flow 1 and not a staging area for it: it is
the path for a contributor who will not make a blanket grant at connect and
should not have to approve every session forever.

The requirements do not weaken for it. R1–R7 still bind whatever becomes
automatic, because the gates attach to the `AutoUpload` mode rather than to the
onboarding path that reached it. What differs is *when* and *how much* becomes
automatic, not what automatic means.

**Where the mechanism lives is now settled; its details are not.** Under the
trust model, earned trust is a property of the **account**, enforced on the
**server** as the limit on how much an armed folder may send. It is not a
client-side counter. Which signals count, what they accumulate toward and where
thresholds sit remain deliberately unfixed. Two structural properties are
fixed, because the rest of this spec depends on them:

- **Whatever is earned is expressed as project mode**, so that exclusion,
  retraction, the void rule and the logout rule all reach it unchanged.
- **Nothing is earned silently.** A folder that becomes automatic is announced
  in the same way the first-contribution notice is, since the contributor did
  not make a decision at the moment it changed.

This is also where connected inference sits. It supplies a witness and
receipts, which satisfy R1 and feed R4 -- so it is **one route toward
automatic contribution, not the door to it**. A contributor who never connects
inference can still reach Flow 3; a contributor who connects it on day one
still meets R1–R7 like anyone else.

**Flow 2 — choose myself.** Per-folder `Automatic` / `Ask me` / `Never`, and
per-session approve or decline.

### Flow 2's armed folders need the same gates, and so does anything Flow 3 earns

`AutoUpload` stays reachable inside Flow 2 — screen 5's "Automatic", the arming
offer, Settings, and CLI `--mode auto` — with none of R1–R7 attached. So rev 2's
claim that Flow 2 "costs the commons nothing, because a human is still reading
each one" is false for an armed Flow 2 folder.

**Decision required:** either R1–R7 apply to any `AutoUpload` project whatever
flow reached it, or the spec states plainly that arming in Flow 2 is automatic
contribution without them. This spec takes the first position: the gates belong
to the *mode*, not to the onboarding path.

Under the trust model this still holds -- the requirements attach to the mode,
whichever path reached it -- but what they require changes: R1 chooses the
disclosure rather than gating the arming, and R3 is a server-side volume check.
See "The trust model".

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

**Read per session, not per folder.** A folder previously set to `Never` whose
discovery state was wiped would otherwise have its *new* sessions counted as
discovered after the grant and armed, which is the same defect one level down.

**This rule covers arming and does not rescue withdrawal durability.** Logout
also wipes receipts, history and the audit log, so after a re-grant a session
the contributor withdrew is re-offered in an "Ask me" folder with no record of
the withdrawal and can be approved again. Withdrawal durability therefore
requires state that survives logout; a re-grant rule cannot supply it.

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

**The prerequisite is extending that test, the Swift and C# consent-copy tests,
and GTK's `tests/shell_wording.rs`, to the new `AUTO_*` constants**, together
with `ConsentCopy` itself, which carries three fields today. GTK re-exports
`trace_commons_contributor::consent_copy` through
`crates/trace-commons-contributor-gtk/src/copy.rs`, so leaving it out would let
Linux render stale or hand-written automatic-contribution copy with nothing
failing.

```rust
pub const AUTO_SCRUB_SCOPE: &str = "Fixed patterns remove API keys and tokens in the formats we know, and many file paths and email addresses that name you. Everything else -- names, people, addresses, account numbers, a password typed into a sentence -- is removed when a model recognises it.";

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
   is not an exposure, but describing one enclave when there are two is
   inaccurate. **The answer, rather than the question:** the witness does not
   verify NEAR AI's attestation on each classifier call -- it requires only a
   TLS or loopback endpoint, and the measured witness compose file describes
   that endpoint as outside the enclave, which places the classifier's operator
   inside the transcript's trust boundary. The classifier receives the
   deterministic-pass output, not the unredacted session.
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

Settled since rev 7 and removed from this list: where the earned-trust
mechanism lives (the account, on the server), what stops spam on the invite
path (server-side, keyed on the account), #706 under Flow 1 (the trust-limited
volume check, once #1020 is enforced), the void rule's identity and
measurement admission (R6; implementation pending in #1024), and the arming
and quit copy (pending in #1011 and #1007; on `main` the quit copy still says
"Nothing will be sent while nobody's approving").

- **"Traces with witnesses."** Whether a folder is armed automatically (trust
  model, point 2) because of (a) a configured witness certifying redaction, or
  (b) NEAR AI-routed inference with receipts. Arming by the contributor is not
  in question; any contributor may arm any folder.
- **The deterministic-only arming disclosure.** Under full trust an invitee's
  folder without a witness may be armed, and its copy must not claim a model
  scrubs it. That wording is not yet written, and it needs the same pinning and
  cross-shell treatment as the `AUTO_*` constants (pending in #1008).
- **Earned-trust signals and thresholds.** Where the mechanism lives is
  settled; which signals count, what they accumulate toward and where the
  limits sit are not.
- **Witness capacity and back-pressure.** Automatic sessions, including
  pre-grant backlogs, through a shared witness with bounded slots (#1014).
- **Telling the contributor.** A void and a hold are audited and announced as
  events, but no shell shows the contributor a notice yet, which the "nothing
  silently" property requires. The same notice is owed to an armed folder whose
  disclosure becomes deterministic-only.
- **Legal posture**, unchanged: a one-time grant is a different consent basis,
  and review established that withdrawing it is currently much harder than
  giving it.
