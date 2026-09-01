# NEAR AI inference attestation, and attestation-gated onboarding

Two things, joined by one object.

1. A trace envelope can carry evidence that the agent work it records was
   inferenced on NEAR AI.
2. That same evidence admits a contributor who holds no invite — and then keeps
   admitting every trace they submit.

The second is what makes the first worth building. An attestation checked once
at onboarding is a toll paid once; an attestation required on every submission
is a cost that scales with the number of traces rather than the number of
accounts, which is the ratio that decides whether a credit ledger can be farmed.

This spec depends on
[`2026-09-01-ironwire-ledger-enrichment-design.md`](./2026-09-01-ironwire-ledger-enrichment-design.md)
and says where.

## What exists today

- **No TEE attestation verification anywhere.** The only reference to NEAR AI's
  TEE is a comment: `crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs:15`
  notes "(Intel TDX + NVIDIA GPU TEE)". Nothing verifies a quote. The property is
  trusted by assertion.
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

A **per-request signed receipt** issued by NEAR AI. Trust anchors in NEAR AI's
signing key, not in hardware — a weaker claim than a verified TDX quote, and the
honest one to make, so no surface may describe it as hardware-attested.

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

**If NEAR AI's receipt omits the account subject, this design splits in two** —
part 1 and part 2 then need different objects, with a separate identity
attestation for onboarding. That field is the one to insist on when specifying
the receipt with NEAR AI.

### The ask on NEAR AI

The receipt format above, plus a published, rotating keyset. Whether any of it
exists today is unverified; treat the table as the requirement to negotiate, not
as a description of a shipped API.

## The receipt is destroyed in transit

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

1. **NEAR AI receipt format**, above all the account-subject field. Nothing here
   is buildable until the receipt's contents are settled with NEAR AI.
2. **A published, rotating keyset** for the signing key.
3. **The third IronWire ask** — receipt recorded on the exchange row — for the
   proxy-mediated path. The direct-API path is unblocked.
4. **The enrichment spec's `routing_metadata` presence category**, shared with
   this design.

## Open items

- Whether NEAR AI's receipt includes the account subject. If not, onboarding
  needs a separate identity attestation and this spec splits.
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
