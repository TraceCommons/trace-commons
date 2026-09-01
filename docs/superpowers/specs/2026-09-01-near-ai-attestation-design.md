# NEAR AI inference attestation, and attestation-gated onboarding

Two things, which were meant to be joined by one object and are not.

1. A trace envelope can carry evidence that the agent work it records was
   inferenced on NEAR AI.
2. That same evidence admits a contributor who holds no invite — and then keeps
   admitting every trace they submit.

The second is what makes the first worth building. An attestation checked once
at onboarding is a toll paid once; an attestation required on every submission
is a cost that scales with the number of traces rather than the number of
accounts, which is the ratio that decides whether a credit ledger can be farmed.

> **Revised 2026-09-01, after probing the live service.** The original draft
> assumed no hardware attestation was available and that one negotiated receipt
> would serve both halves. Both assumptions were wrong, in opposite directions.
>
> A fresh, nonce-bound Intel TDX + NVIDIA Hopper attestation **is served today,
> unauthenticated** — better than assumed, and verifiable without asking NEAR AI
> for anything. And a **per-request signed receipt also exists** (Object B),
> confirmed from NEAR AI's own reference verifier rather than from prose. The
> "nothing is buildable until we negotiate a receipt" premise was simply wrong.
>
> What is missing is one field, and smaller than the original spec's framing of
> it: **the receipt carries nothing that distinguishes one payer from another.**
> Not an identity — a *pseudonym* would do. See "What Object B does not carry".
>
> So the design splits along a different line than expected. **Half 1 — an
> envelope carrying verifiable evidence that specific turns ran in a NEAR AI
> enclave — is buildable now**, end to end, with no ask on NEAR AI. **Half 2 —
> admitting uninvited contributors — is blocked**, and no configuration should
> be able to reach it until that field exists.
>
> Two consequences worth stating up front. The cost floor and the
> per-contributor cap were treated as one security argument and are not: the
> floor survives without a pseudonym, the cap does not. And the cap only
> *matters* if a trace earns more credit than the inference that produced it
> costs — which is a pricing relationship, not a cryptographic one. See "Sybil
> economics".

This spec depends on
[`2026-09-01-ironwire-ledger-enrichment-design.md`](./2026-09-01-ironwire-ledger-enrichment-design.md)
and says where.

## What exists today

- **NEAR AI publishes a fresh, nonce-bound hardware attestation today.** Probed
  2026-09-01: `GET https://qwen3-6-35b.completions.near.ai/v1/attestation/report`
  returns HTTP 200, ~390 KB, **unauthenticated** — no credential, no negotiation.
  It carries an `intel_quote` (TDX), a ~98 KB `nvidia_payload` (Hopper
  `evidence_list`), `signing_address` + `signing_public_key` (secp256k1),
  `ohttp_key_config` + `ohttp_attestation` (ed25519), and pinnable measurements
  under `info`: `compose_hash`, `os_image_hash`, `mr_aggregated`, `app_id`,
  `instance_id`, plus `tcb_info.{mrtd,rtmr0..3}`.

  **It binds a caller nonce in hardware.** `?nonce=<exactly 64 hex chars>` is
  echoed as `request_nonce` *and embedded in the TDX quote* — verified by
  observing the quote bytes change with the nonce. A 32-hex nonce is rejected
  and the field returns empty. So the report is fresh and replay-resistant, not
  a static document that can be copied from another verifier.
- **We verify none of it.** `perplexity_near_ai.rs:373-374` reads
  `resp.status()` then `resp.text()`; no response header is ever inspected, and
  deserialization (`:533-548`) keeps only `choices[].logprobs.token_logprobs`,
  so serde silently drops everything else. Grep for
  `signing_address|intel_quote|attestation/report` across the repo: zero hits.
  The TEE property is currently trusted by assertion even though it is checkable.
- **No JWKS.** `.well-known/jwks.json` returns 404 on the gateway. The signing
  key is self-published per instance inside the report, not served from a
  rotating keyset like our upload-claim issuer.
- **`trace_score_attestation.rs` is not this.** It is the server signing its own
  scores (`trace_commons.score_attestation.v2`), evidence about how *we*
  evaluated an envelope. It says nothing about where the contributor's inference
  ran.
- **The onboarding half already exists in skeleton.**
  `TraceInstanceEnrollAttestation` (`crates/trace-commons-protocol/src/onboarding.rs:43-51`)
  carries `device_key_id`, `aud`, `instance_id`, `user_subject`, `nonce`, `exp`,
  with canonical length-prefixed signing bytes
  (`instance_enroll_attestation_signing_bytes`, `:113`), a replay cache and
  per-instance token bucket (`crates/trace-commons-server/src/instance_enroll_guard.rs`),
  rate limiting, an `EnrollCapExceeded` code, and derived-tenant idempotency.
  That is already "a signed attestation grants a tenant without a per-user
  invite". It trusts an Ironclaw instance signer; this spec adds a second
  trusted issuer to it.

## The attestation object

There are **two** objects, and conflating them is the mistake this section
originally made.

**Object A — the instance attestation. Exists today, verifiable today.** The
nonce-bound TDX + GPU report described above. It answers: *is the endpoint I am
talking to a genuine Intel TDX enclave with an NVIDIA Hopper GPU, running this
specific image?* Anchored in Intel and NVIDIA roots, not in NEAR AI's word.

An earlier draft of this spec said trust "anchors in NEAR AI's signing key, not
in hardware — a weaker claim than a verified TDX quote ... so no surface may
describe it as hardware-attested." **That is wrong for Object A** and is
corrected here: a verified TDX quote with a nonce we chose is exactly what is
available, and calling it hardware-attested is accurate.

**Object A carries no identity.** This is the load-bearing limitation. The
report is a public, unauthenticated document — anyone can fetch it, and it
contains nothing caller-specific except the nonce the caller supplies. It
therefore cannot attest to *who* obtained it, *who* ran an inference, or *that
any particular trace* passed through the enclave. It attests to a machine, not
to a relationship with a contributor.

**Object B — a per-request signed receipt. It exists.** Confirmed from NEAR AI's
own verifier, `github.com/nearai/nearai-cloud-verifier` (`py/chat_verifier.py`,
`ts/chat_verifier.ts`), not from documentation prose. The earlier 401s on
`/v1/signature/...` were an authenticated endpoint refusing an unauthenticated
probe, not an absent one.

**How it works, from the reference implementation:**

1. A completion response carries an `id` — the `chat_id`. Our client currently
   discards it: serde keeps only `token_logprobs`.
2. `GET {base}/v1/signature/{chat_id}?model={model}&signing_algo=ecdsa`, with
   `Authorization: Bearer {api_key}`, returns
   `{text, signature, signing_address}`.
3. `text` is colon-separated, two or three parts. With three, the leading part
   is discarded and the hashes are `parts[1]`, `parts[2]`; with two they are
   `parts[0]`, `parts[1]`. **Both are lowercase SHA-256 hex** — of the request
   body as sent, and of the response text.
4. `signature` is an Ethereum `personal_sign` (`encode_defunct`) over `text`.
   Recover the address and compare to `signing_address`.
5. Bind that key to hardware:
   `GET /v1/attestation/report?model={model}&nonce={nonce}&signing_algo=ecdsa&signing_address={addr}`
   — **404 if the address does not match**, which makes the check crisp. In the
   quote, `report_data[0..32] = SHA256(signing_address || spki_hash)` and
   `report_data[32..64] = nonce`, so the signing key *and* the TLS SPKI are
   bound to the measured enclave.

Two canonicalization details an implementer will otherwise get wrong. The
request hash is over **the exact bytes sent**, so the envelope must retain them
rather than a re-serialization. For a streaming response the hashed text is
every SSE line joined with a trailing `"\n"` on each, including the terminal
one — reconstructing it from parsed deltas will not reproduce the hash.

NEAR AI's own note, worth repeating because it bounds the claim: the verifier
supplies a fresh nonce at step 5, so the attestation it fetches is not the one
that signed. It proves the signing key is bound to valid hardware — **not** that
a specific attestation was used for this completion.

### What Object B does not carry

**Nothing that distinguishes one caller from another.** The signed text is hashes
and an optional prefix.

The API key gates *fetching* a signature, which is a real but weak binding: it
proves whoever fetched it held a key at fetch time, and the artifact is a bearer
token afterwards. It does not tell us which account produced the inference, and
two people can present the same receipt.

**What this costs is narrower than "the account subject is load-bearing"
suggests**, and the original spec overstated it. Identity is *not* needed to
authenticate a submission — our own auth does that, and the contributor already
holds a Trace Commons account. It is *not* needed to accrue credit, for the same
reason. And it is *not* needed to prove the inference was paid for: the request
and response hashes establish that with no idea who ran it.

It is needed for exactly one thing: **binding sybils to a single cap.** The
per-contributor cap accumulates per `(auth_principal_ref, epoch_index)`
(`trace-commons-ingest.rs:51356`), and enrollment derives a tenant through
`derive_user_tenant_id(instance_id, user_subject)`
(`onboarding.rs:133`). That chain is what makes one person resolve to one
principal however many times they enroll. With nothing to derive from, every
enrollment mints a fresh principal and one attacker holds N caps instead of one.

**And a stable pseudonym would do that job as well as an identity would.** What
the cap needs is a value that is *the same across requests from the same payer
and different across payers*. It does not need to be an account name, an email,
or anything NEAR AI could resolve to a person. A salted opaque identifier is
sufficient — and is how we already store it: `user_subject_hash`
(`db/postgres.rs:3152`), never the subject in the clear.

That reframes the ask, and makes it one a confidential-computing provider can
plausibly grant. "Include a stable per-account pseudonym in the signed text" is
a much smaller request than "tell us who your users are", which sits badly
inside a product whose entire proposition is that they cannot see your prompts.

So the original table below is now a comparison between what was wanted and what
exists, not a specification to build:

| Field | Why it was wanted | Present? |
|---|---|---|
| stable per-account pseudonym | Binds sybils to one cap. Was framed as "account subject"; a salted opaque value is sufficient and is all we store anyway. | **No** |
| request hash, response hash | Binds the receipt to specific content, so it cannot be moved to a trace it did not cover. | **Yes**, SHA-256 of both |
| model | Provenance, and lets a coverage claim name what served it. | Query parameter, not signed |
| timestamp | Freshness, and ordering against the transcript. | **No** |
| response id or nonce | Replay defence, and the join key onto our events. | `chat_id` addresses the receipt; not inside the signed text |
| key id | Rotation, resolved against a published keyset. | `signing_address`, resolved against the attestation rather than a keyset |

**The split this spec warned about has happened.** The original text said: "If
NEAR AI's receipt omits the account subject, this design splits in two — part 1
and part 2 then need different objects, with a separate identity attestation for
onboarding." The only artifact confirmed to exist (Object A) carries no account
subject and no identity of any kind, so the contingency is now the situation.

The consequence is concrete and worth stating without hedging: **onboarding
cannot be gated on Object A.** Admitting an uninvited contributor requires
binding *that contributor* to something, and a document any anonymous party can
fetch binds nobody. An admission rule built on it would admit everyone, which is
not a weaker gate but an absent one.

### The ask on NEAR AI

**One field.** A stable per-account pseudonym inside the signed text — the same
value for every request from one payer, different across payers, and opaque.
Explicitly *not* an account name, email, or anything resolvable to a person: we
hash it on arrival regardless (`user_subject_hash`), so a pre-hashed or salted
value costs us nothing and costs them much less to agree to.

Object A is no longer part of the ask; nor is the receipt format, which ships
today. A rotating keyset remains desirable but is not blocking, since
`?signing_address=` 404s on mismatch and gives a working per-signature check.

Ask them first whether a completion response is already signed by
`signing_address`, since the key exists and is already bound to the enclave
measurement. If it is, most of Object B may already be there and the negotiation
is about exposing it, not building it.

Worth raising in the same conversation: `ohttp_key_config` and
`ohttp_attestation` are published alongside the quote, which means requests can
be encapsulated to a key bound to the attested measurement. A contributor
routing through OHTTP knows their own request reached the enclave. Whether that
knowledge can be turned into something *we* can verify has not been reasoned
through and should not be assumed — but it is the most promising path that does
not depend on NEAR AI adding a new field.

## The receipt is destroyed in transit

> **Object B — now live.** The receipt exists, so this problem is no longer
> hypothetical: `chat_id` and the exact request/response bytes must survive to
> reach us, and the current path preserves none of them.

NEAR AI returns the receipt in its response. IronWire forwards that response to
Claude Code, which writes **its own** transcript format and drops fields it does
not model. By the time our scraper reads the session file, the receipt is gone.
Nothing in the current path preserves it.

So the proxy-mediated path needs **a third IronWire upstream ask**: record the
receipt on the exchange row. It rides the same mechanism as Ask 1 in the
enrichment spec — same table, same `LedgerContext::write` insertion point
(`ironwire_proxy/src/pipeline.rs:872-903`), one more column. But a receipt is
larger and less obviously content-free than a bare response id, so **file it
after the first PR lands rather than bundling it.** Bundling makes the small ask
carry the larger argument.

One path is not blocked: a contributor calling NEAR AI directly, with no proxy,
sees the receipt themselves and can bind it at capture time. Both collection
paths must be supported, and the direct path is the one that can ship first.

## Coverage, not a boolean

> **Object B — buildable.** Turn-level attestation exists, so coverage is
> computable now. The counts-not-a-boolean argument stands and now has data
> behind it.

"This trace was inferenced on NEAR AI" is almost never wholly true.

IronWire's design is a fidelity ladder that moves a conversation between
backends under sustained pressure — rungs 0 through 3, descending after twenty
seconds and climbing back after five continuous minutes. A real session that
ever meets a rate limit is part Anthropic, part NEAR AI. A boolean
"attested on NEAR AI" flag would be false on most such sessions while reading as
true.

The envelope therefore expresses **coverage**: attested turns, inference-bearing
turns, and the key id that signed them.

**Coverage is computed by the server from verified receipts.** It is never read
from a contributor-supplied field. Self-reported coverage from
contributor-controlled software is precisely what the attribution-only rule in
the enrichment spec says not to trust, and coverage gates admission here, which
makes it authorization rather than attribution.

## Where receipts ride

> **Object B — buildable**, gated only on PR #513 landing. The `chat_id` is the
> natural per-turn join, alongside that PR's session-level one.

Receipts attach to the `RoutingDecision` events introduced by the enrichment
spec — the events that already record `backend` — reached by the same
`client_session_id` join that spec's Ask 1 adds. The two designs converge on one
join rather than each inventing one.

That join is session-level, not turn-level: it identifies which exchanges belong
to this trace, and the receipt's own response id and content hashes then bind it
to a specific turn within that session. So the receipt carries its own
turn-level binding and does not need the ledger to supply one.

The trace-level coverage summary is a small envelope block: attested count,
inference-bearing count, key id, and the issuer. Not a fraction: store the two
counts and let readers divide, so a reader can tell "3 of 3" from "300 of 300".

Consequence for the presence-flag problem in the enrichment spec: a receipt is
structured data on an event, so it lands under the same
`EnvelopeContentPresence::routing_metadata` category that spec introduces, and
must not flip `tool_payloads`. If the routing_metadata category is dropped from
that plan, this spec inherits the same silent quarantine regression.

## Redaction severs the receipt's binding to the trace

**This is the hardest problem in the design and it has no cheap answer.**

Object B binds `SHA256(request_body_as_sent)` and `SHA256(response_text)`. NEAR
AI hashes the bytes it received. But redaction is **client-side and
pre-upload** -- `DeterministicTraceRedactor` in
`crates/trace-commons-contributor/src/envelope.rs`, and the wire field is
literally `redacted_content` (`trace_contribution.rs:276`). The server never
holds the bytes NEAR AI hashed, so it can never reproduce the hash, so it can
never verify a contributor's receipt against their trace.

Not a downstream bug. Structural: the receipt binds the *inference input*, the
trace stores the *publishable artifact*, and they are different objects by
construction.

Four resolutions, and only one keeps the property:

| Option | Consequence |
|---|---|
| Trust a client-computed verdict | Rejected. Coverage gates admission, so it is authorization, and self-reported coverage from contributor-controlled software is what the enrichment spec's attribution-only rule exists to refuse. |
| Drop content binding; require each `chat_id` to be used once | Keeps "one paid inference per submitted trace" but an attacker satisfies it with a one-token completion. The floor collapses from *inference over the whole trace* to *a trivial call per trace*, which is nearly free. |
| Send raw bytes to the server | Rejected. Defeats the redaction design entirely. |
| ~~Redact before inference~~ | Rejected. The agent reasons over scrubbed text, so the work degrades; and scrubbing into a TEE inverts the point of confidential inference. |

**Redacting before inference is also rejected** (Zaki, 2026-09-01). The agent
would reason over scrubbed text, so the work degrades and a trace of degraded
work is worth less -- the corpus exists to record real work. It is also
backwards against a TEE: confidential inference exists so that real data *can*
be sent, and scrubbing on the way in is strictly worse than scrubbing on the way
out.

### The requirement was never content identity

All four options fail, which is the signal that the problem was framed wrongly.

Re-read what the security argument actually asks for: *every submission requires
a real, paid-for NEAR AI inference over the whole trace.* **The requirement is
proportional cost.** Content binding was only ever a means of guaranteeing that
the inference an attacker paid for was as large as the trace they submitted. If
the receipt states *how much* inference was bought, that guarantee holds
directly and the raw bytes stop mattering.

So the resolution is an upstream field, not a client behaviour change. Ask NEAR
AI to include in the **signed** text what the completion response already
returns:

- **token usage** (prompt and completion),
- **model**,
- **timestamp**.

Then the rule becomes: each `chat_id` is accepted once, so one paid inference
per submitted trace; the signed token counts must be commensurate with the
trace's own size; and the signed timestamp must fall within the trace's window.

An attacker must buy inference proportional to what they submit. They may buy
*different* inference of the same size -- but it costs the same, and cost is the
entire property. Content binding was a proxy for it, not the thing itself.

This also keeps the ask acceptable to the counterparty: usage, model and
timestamp reveal neither content nor identity, so nothing here asks a
confidential-computing provider to undermine its own proposition.

**Unresolved:** the receipt's `text` carries an optional third leading part that
NEAR AI's reference verifier discards without naming. It may already carry some
of this. Reading one requires an API key, which lives on `tc-pilot-host`.
Check before asking for anything new.

**Consequence for sequencing.** This must be settled before the contributor
slice is planned, because it decides whether the proxy redacts outbound -- an
IronWire behaviour change, not a server one. It does **not** block verifying our
own inference path, where we hold both the exact request bytes and the response
and nothing is redacted.

## Attestation material is never scored and never scrubbed

Receipts, quotes, signatures and signing addresses must never enter an event's
`redacted_content`, and must never reach the perplexity scorer, the
novelty/dedup path, or the privacy filter. They ride as typed sibling fields.

This is a hard invariant, not a preference, and the reason is mechanical.
`crates/trace-commons-gate-enclave/src/chunker.rs:85-98` iterates **every**
event, reads `event_type` and `redacted_content`, and renders
`"{event_type}: {content}\n"` into the text that both the perplexity scorer and
the dedup signal consume. **There is no filter by event type.** Whatever lands
in that field is scored.

Three things break, and none of them look like a bug:

- **Perplexity.** A TDX quote is kilobytes of high-entropy hex with no
  linguistic structure. It scores as maximally surprising, moving a trace's
  perplexity for a reason unrelated to the contributor's work.
- **Novelty and dedup.** Every trace from one enclave carries near-identical
  attestation bytes — a large block of *identical* text across unrelated
  traces, which is precisely the shape the duplicate penalty exists to punish.
  Contributors would be penalised for carrying the evidence that their work was
  real.
- **The privacy filter.** There is no PII in a quote, so scrubbing it is waste
  in the scarcest place: the classify budget is per-request tokens signalled as
  a generic 502, and multi-kilobyte opaque blobs are the failure mode that
  already wedges the backstop queue.

Note the related trap this repo has already hit once. Routing events reached
scoring not through their text but through **cardinality** —
`compute_value_scorecard` counted `envelope.events.len()`, so adding
contentless events halved contributor quality. Attestation must be checked
against both paths: it must contribute no scored text *and* no scored count.

`policy_examines_event` already returns `false` for `RoutingDecision`
(`trace_contribution.rs:2748`), which is the right direction. But
`derive_envelope_content_presence` maps `RoutingDecision` to
`presence.message_text = true` (`:5300`) — receipts must land under the
enrichment spec's `routing_metadata` category instead, or they will be declared
as message text.

## Verification at ingest

> **Object B — buildable, and this is the security-critical path.** Distinct from
> Object A's verification, which is a periodic check of the endpoint we call
> rather than a per-submission check of contributor-supplied data.

Receipts are verified before gating.

A receipt that fails signature, key-id resolution, or hash binding is **not
coverage zero**. It is a malformed submission, refused with a named error code,
following the existing `TraceOnboardErrorCode` pattern
(`onboarding.rs:63-79`). Treating a forged receipt as merely uncovered would let
an attacker probe the verifier for free.

Per repo convention the verification path is hash-only in what it stores and
logs: receipt hashes and key ids, never the receipt body, the account subject in
the clear, or the request/response hashes' preimages.

## Onboarding

> **Blocked, and now for a precise reason.** Object B exists but carries no
> value distinguishing one payer from another, and Object A is a public document
> identifying a machine. Neither can bind a sybil to a cap. Nothing here is
> reachable until a receipt carries a stable per-account pseudonym.

NEAR AI becomes a second trusted issuer in the existing instance-enroll flow.

- `user_subject` is the NEAR AI account.
- Derived-tenant idempotency maps one account to one tenant, permanently.
- The replay cache, per-issuer token bucket, rate limiting and enrollment cap
  come from the existing guard rather than being reimplemented.
- New error codes extend `TraceOnboardErrorCode` in its established naming.

A dedicated parallel route was considered and rejected: it would duplicate the
replay, rate-limit, cap and idempotency machinery, and that machinery is the
entire security of the mechanism. A second copy drifts.

## The admission rule

`trace_tenant_policies` (`migrations/V1__trace_commons_schema.sql:17-25`) gains
the **admission method** — how the tenant was admitted — rather than a
`require_attestation` boolean.

A boolean can be forgotten when a new tenant-creating path appears. Provenance
cannot, it is auditable, and it fails closed in the right direction: an unknown
or absent admission method requires attestation rather than waiving it.

| Admitted by | May submit |
|---|---|
| invite | with or without attestation |
| instance vouch | with or without attestation |
| NEAR attestation | only traces where **every inference-bearing turn** is attested |
| unknown / absent | attestation required (fail closed) |

**The `NEAR attestation` row is not reachable yet, and the column is worth
building anyway.** It depends on a stable per-account pseudonym, which neither
object carries — Object B omits it, and Object A identifies a machine, not a
payer.

Build the column, the provenance, and the fail-closed default now; leave that
one row unreachable until Object B exists. The column is what makes the gate
survive a new tenant-creating path appearing later, and that value does not
depend on the row being populated. Shipping the enum variant with no way to
reach it is correct, not incomplete — and it must not be reachable by
configuration either, or an operator will find it before the mechanism does.

A refusal names the coverage fraction, so a contributor can diagnose it rather
than guess.

### Why full coverage rather than a threshold

Any bar below 100% is a discount an attacker arbitrages by mixing cheap capacity
into an otherwise-free trace, and every threshold value is arbitrary.

Full coverage is achievable for the population it binds: an uninvited
contributor running NEAR AI as their only backend has nowhere to fall back to,
so IronWire's ladder never descends and coverage is naturally complete. The
population that routinely sees partial coverage is the one holding other
capacity — subscriptions and API keys — which is the invited cohort, for whom
attestation is optional anyway.

## Why the per-contributor cap still binds

Full credit eligibility for attested contributors does not mean uncapped credit.

Because `user_subject` is the NEAR account and tenants derive from it
idempotently, one NEAR account resolves to one `auth_principal_ref` however many
times it enrolls. The per-contributor cap is computed cross-tenant per
`auth_principal_ref` (PR #171), so it binds sybils together rather than resetting
per tenant.

This is the property that makes attestation-granted eligibility safe, and it is
easy to lose: any change that mints a fresh principal per enrollment silently
disables the cap. It should be asserted by a test, not by review.

## Sybil economics, stated plainly

The design's security is not that a NEAR AI account is hard to get. It is that
**every submission from an attestation-admitted account requires a real,
paid-for NEAR AI inference over the whole trace.**

An attacker's cost scales with traces produced rather than accounts created.
Credit per trace is already bounded by quality, duplicate penalty and the
per-contributor cap, so the attack has to clear a cost floor per unit of credit
rather than amortising one signup across unlimited submissions.

What this does **not** defend against: an attacker for whom NEAR AI inference is
cheaper than the credit a trace earns. That is a pricing relationship, not a
cryptographic one, and it should be monitored rather than assumed. If credit per
trace ever exceeds the inference cost of producing one, this gate inverts.

### The two properties need different things, and only one is blocked

This section originally treated the cost floor and the per-contributor cap as
one argument. They are not, and separating them is what makes partial delivery
possible.

**The cost floor does not need identity.** Requiring every inference-bearing
turn to carry a valid Object B receipt means producing a trace requires paying
for the inference that produced it — whether or not we know whose account paid.
The receipt's request and response hashes bind it to *this* trace, so it cannot
be reused across submissions. That property is available now.

**The per-contributor cap needs a stable pseudonym.** It binds sybils by
computing across tenants per `auth_principal_ref`, which relies on one payer
resolving to one principal. With nothing payer-distinguishing in the receipt
there is nothing to derive that principal from, so an attacker with many API
keys — or one key and many enrollments — gets a fresh cap each time.

**And that only bites under one condition.** If a trace earns less credit than
the inference producing it costs, an attacker holding a thousand principals
simply loses money a thousand ways, and the missing field buys nothing. The cap
is insurance against the inversion, not against farming as such.

The inversion is unusually easy to check here, because credit is redeemable
against NEAR AI inference: cost and reward are denominated in the same thing.
The question is literally *does a trace earn more inference than it consumes?*
Today it does not. That is a parameter we control only partly, so the cap is
worth restoring — but its absence is a monitored risk, not an open door.

The consequence for admission is precise rather than fatal. Attestation-gated
admission gives a **per-trace cost floor without a per-attacker ceiling**. Which
is to say: an attacker pays full inference cost for every trace, and can do so
unboundedly in parallel. Whether that is acceptable is a pricing judgement — the
same judgement the paragraph above describes — and it should be made explicitly
rather than inherited by assuming the cap still binds. It does not.

## Blockers

The original list said "nothing here is buildable". That was true of the design
as then understood and is no longer true: Object A moved from *assumed absent*
to *verified present*, and the work it unblocks is real.

**Not blocked — buildable now:**

1. **Verifying Object A.** Fetch with a 32-byte nonce, verify the TDX quote to
   Intel roots and the Hopper evidence to NVIDIA roots, confirm our nonce is in
   the quote's report data, and pin `mr_aggregated` / `compose_hash` /
   `os_image_hash` against an expected set. This turns a comment into a check
   and is worth doing on its own: it is the difference between believing the
   pilot's scorer runs in a TEE and knowing it.
2. **The admission-method column** on `trace_tenant_policies`, with the
   fail-closed default and the `NEAR attestation` row left unreachable.
3. **The cap-binding test.** See "Why the per-contributor cap still binds" — the
   spec asks for it to be asserted by test rather than review, and it does not
   depend on any NEAR AI artifact.

4. **Object B verification and per-trace coverage.** The receipt exists and its
   format is known from NEAR AI's reference verifier, so coverage — every
   inference-bearing turn carrying a hash-bound, TEE-signed receipt — is
   buildable. This was the largest blocker and it is gone.
5. **Capturing what verification needs.** `chat_id` from the completion
   response, and the exact request bytes and response text, none of which the
   current client retains. This is a change to what we record, not a
   negotiation.

**Still blocked:**

6. **A stable per-account pseudonym in the signed text.** Confirmed absent from
   Object B. It blocks the per-contributor cap's sybil binding, and through that
   attestation-gated *admission* — not coverage, which needs no such value. This
   is the entire remaining ask on NEAR AI, and deliberately not a request for
   identity: an opaque salted value is sufficient, and is all we store anyway.

   Note the blocking is conditional. Without it, attestation-gated admission
   still imposes a real per-trace cost floor; what it loses is the per-attacker
   ceiling. Shipping without it is a pricing judgement rather than an open
   door — see "Sybil economics" — but it should be made deliberately.
7. **A published, rotating keyset.** Confirmed absent (404). Object A
   self-publishes its key inside the report and
   `?signing_address=` returns 404 on mismatch, which is adequate for verifying
   a given signature and is not a substitute for a keyset that survives
   rotation.
8. **The third IronWire ask** — receipt on the exchange row. Now well-specified
   rather than speculative: the row needs `chat_id`, and the proxy is the only
   component that sees both the request bytes and the response text. Worth
   filing once PR #513 lands.
9. **The enrichment spec's `routing_metadata` presence category.** It lives on
   PR #513, open with CI green and unreviewed. This design cannot land before it
   does.

## Open items

- ~~Whether NEAR AI's receipt includes the account subject.~~ **Settled: it does
  not.** But the follow-on is smaller than the original framing assumed — what
  onboarding needs is a stable pseudonym, not an identity. Open: whether NEAR AI
  will add one, and whether a per-key value or a per-account value is the right
  granularity (per-key is easier for them and weaker for us, since one payer can
  hold many keys).
- Whether the cap's absence is tolerable in the interim, which reduces to
  whether credit per trace stays below inference cost per trace. This wants a
  monitored number, not a one-time judgement.
- Whether a completion response is already signed by `signing_address`. The key
  exists and is bound to the enclave measurement, so this may be a question of
  exposure rather than construction. Ask before specifying anything new.
- Whether OHTTP encapsulation, using the published `ohttp_key_config`, can
  produce something a *third party* can verify — or only something the client
  itself knows. If only the latter, it does not help admission.
- Which measurements to pin, and the operational cost of pinning them. Pinning
  `mr_aggregated` means every NEAR AI image upgrade breaks verification until
  the expected set is updated. That is the correct failure direction, but it is
  a live operational burden, and a deployment that responds by disabling the
  check is worse off than one that never had it.
- Whether "inference-bearing turn" is cleanly derivable from our transcripts,
  particularly for sessions that predate an IronWire install or mix
  pre- and post-install turns. The refusal path must not punish a contributor
  for turns we cannot classify.
- Whether a receipt binds to the contributor strongly enough that two people
  cannot present the same one. Content hashing makes cross-trace replay hard and
  dedup would catch a duplicated trace, but this has not been reasoned through
  against a deliberate sharer.
- Whether attestation-admitted tenants should be visible as such to reviewers.
  The admission method is stored; whether it surfaces is a separate decision.
