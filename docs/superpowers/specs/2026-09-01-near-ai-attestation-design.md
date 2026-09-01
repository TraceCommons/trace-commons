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
> for anything. But it is a public document that identifies a *machine*, not a
> contributor, so it cannot carry half 2. The per-request receipt that half 2
> needs remains unconfirmed.
>
> The design therefore splits into Object A (available, buildable now) and
> Object B (unconfirmed, blocking). Sections below are marked accordingly. Half
> 1 is partly reachable; **half 2 is blocked**, and no configuration should be
> able to reach it until Object B exists.

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

**Object B — a per-request signed receipt. Unverified; may not exist.** This is
what the rest of this spec needs, and what remains to be settled with NEAR AI.
Nothing establishes that a completion response is signed by `signing_address`.
Probes of `/v1/signature`, `/v1/signature/test` and similar return 401, but so
does every unknown path on that host, so 401 distinguishes nothing either way.

The fields below describe **Object B**. Trust there anchors in NEAR AI's signing
key — genuinely a weaker claim than Object A's hardware quote, and the honest
one to make about a receipt.

For a single object to serve both halves, the receipt must carry:

| Field | Why |
|---|---|
| account subject | Binds the turn to a contributor **and** is the identity onboarding uses. The load-bearing field. |
| request hash, response hash | Binds the receipt to specific content, so it cannot be moved to a trace it did not cover. |
| model | Provenance, and lets a coverage claim name what served it. |
| timestamp | Freshness, and ordering against the transcript. |
| response id or nonce | Replay defence, and the join key onto our events. |
| key id | Rotation, resolved against a published keyset. |

Signed with a key published at a rotating keyset URL, the pattern the
upload-claim issuer already uses.

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

Object B: the receipt format above, **with the account subject the priority
field**, plus a published rotating keyset. Object A is no longer part of the ask
— it ships today.

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

> **Object B.** Moot until a per-request receipt exists — there is nothing to
> preserve yet. Retained because the transit problem is real the moment one does.

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

> **Object B.** Coverage counts attested turns, and nothing can be attested at
> turn level today. The argument for counts-not-a-boolean stands regardless.

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

> **Object B**, and additionally gated on PR #513 landing.

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

## Verification at ingest

> **Object B.** Object A's verification is a different path: a periodic check of
> the endpoint we call, not a per-submission check of contributor-supplied data.

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

> **Object B, and blocked.** Object A carries no identity, so it cannot admit
> anyone. Nothing in this section is reachable until a receipt with an account
> subject exists.

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
building anyway.** It depends on Object B, which is unconfirmed. Object A cannot
substitute: it carries no identity, so nothing can be admitted by it.

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

**Still blocked:**

4. **Object B — a per-request receipt**, above all its account-subject field.
   Everything about coverage, per-trace binding, and attestation-gated admission
   waits on this. It is the whole security argument: without it, cost scales
   with accounts rather than traces.
5. **A published, rotating keyset.** Confirmed absent (404). Object A
   self-publishes its key inside the report, which is adequate for verifying
   that report and not a substitute for a keyset that survives rotation.
6. **The third IronWire ask** — receipt on the exchange row — for the
   proxy-mediated path. Moot until Object B exists; there is no receipt to
   record.
7. **The enrichment spec's `routing_metadata` presence category.** Now more
   concrete than when written: it lives on PR #513, which is open with CI green
   and unreviewed. This design cannot land before it does.

## Open items

- ~~Whether NEAR AI's receipt includes the account subject.~~ **Settled, and
  against us.** The only confirmed artifact carries no identity at all, so
  onboarding needs a separate identity attestation and this spec has split. What
  remains open is what that identity attestation is.
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
