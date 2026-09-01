# The redaction witness

A service running in an attested TEE that sees both the unredacted and the
redacted form of a trace, proves they correspond, and issues a certificate over
the *redacted* form that the server can verify without ever holding the raw
bytes.

It exists to resolve one impasse, stated in
[`2026-09-01-near-ai-attestation-design.md`](./2026-09-01-near-ai-attestation-design.md):
a NEAR AI inference receipt binds `SHA256` of the bytes NEAR AI received, while
redaction happens client-side before upload, so the server holds different bytes
and can never reproduce that hash. Every resolution that keeps the raw bytes
away from an enclave fails — trusting a client verdict is authorization by
self-report, dropping content binding collapses the cost floor to a trivial call
per trace, and redacting before inference degrades the agent's work and inverts
the point of confidential inference.

The witness resolves it by moving "sees both" into a box whose code the
contributor can verify before trusting it. That is the same bargain NEAR AI
offers us, which is what makes it coherent rather than hypocritical.

## What exists today

Verified against the tree on 2026-09-01. Three facts shape this design more than
anything else.

- **Nothing here runs in a TEE.** `trace-commons-gate-enclave` is aspirational
  naming: `docs/operator/architecture.md:78-80` describes components that "move
  out-of-process into the enclave" as future work. The witness would be **the
  project's first real trusted-execution deployment**, and the operational cost
  of that — provisioning, measurement management, attestation serving, upgrade
  discipline — belongs to this design and not to a later slice.
- **The redactor is not fully deterministic.** `DeterministicTraceRedactor`
  (`trace_contribution.rs:3501`) holds
  `privacy_filter: Option<Arc<dyn PrivacyFilterAdapter>>`, the model-based
  prose-PII classifier. Its pattern half reproduces; a model call does not. Any
  witness that recomputes the redaction and compares **fails on honest
  submissions**. This is the single most important constraint here and it
  invalidates the obvious implementation.
- **Redaction spans are counted, not retained.** `SafePrivacyFilterSummary`
  (`:429-440`) keeps `span_count`, `by_label` and a policy label — hash-only, by
  the repo's convention. The client does not emit the spans themselves today, so
  emitting them *to the witness only* is new work and a new boundary to hold.

Also relevant: `MAX_TRACE_ENVELOPE_BYTES = 16_000_000` (`:83`). Raw is larger
than redacted, so the witness handles payloads above that.

## The correspondence check

The contributor sends the witness three things:

1. the raw request and response bytes, as sent to NEAR AI,
2. the redacted artifact it intends to upload,
3. the **redaction span list** — offsets and replacements.

The witness applies the spans to raw and requires **byte equality** with the
submitted redacted artifact.

This is exact rather than reproductive, and that distinction is the whole
design. It never replays the classifier, so nondeterminism cannot make an honest
submission fail. It admits no fuzzy alignment, so there is no loose matcher for
an attacker to smuggle content through. And it needs no agreement between
witness and client about *policy* — only about the mechanical application of
spans that the client already computed.

Offsets are codepoint-indexed and must be **enforced, not trusted** — the
existing privacy-filter adapter already learned this the hard way. A span list
that does not apply cleanly is a refusal, never a fallback to accepting the
submitted artifact.

### What it proves, and what it does not

It proves **faithfulness**: the redacted artifact derives from the raw one by
redaction alone, and was not fabricated, padded, or swapped for the output of a
different session.

It does **not** prove **sufficiency**: whether the redaction removed enough PII
remains the redaction policy's job and the server-side backstop's. Conflating
these would be easy and would make the witness appear to guarantee something it
does not. No surface may describe a witnessed trace as "verified clean".

## The certificate

On success the witness signs:

```
H(redacted_artifact)
chat_id
prompt_tokens, completion_tokens
model
timestamp
redaction_policy_version
witness_enclave_measurement
```

The server verifies the signature against the witness's own attestation, then
checks `H(redacted_artifact)` against the bytes it holds. Raw never reaches the
server, and the certificate is useless on any other artifact.

`redaction_policy_version` is load-bearing for compatibility: a policy change
means old certificates cannot be re-derived, only re-verified against what they
recorded. Treat it as a schema version, not a label.

## What the witness verifies before certifying

1. The receipt's signature over `request_hash:response_hash`, recovered per
   EIP-191 and matched to `signing_address`.
2. That `signing_address` belongs to a genuine enclave — by fetching
   `/v1/attestation/report?signing_address=…&nonce=…` with a **witness-chosen**
   nonce and verifying the TDX quote. The nonce must be the witness's own; a
   contributor-supplied attestation is a replay.
3. That the receipt's hashes match the raw bytes it was given.
4. That the span list applied to raw equals the submitted redacted artifact.

Steps 1 and 2 are the same verification the
[attestation verification plan](../plans/2026-09-01-near-ai-attestation-verification.md)
builds for our own inference path. **The witness should reuse that module rather
than reimplement it** — a second verifier drifts, and this one guards admission.

## Trust model

The contributor trusts the witness with raw bytes. That trust is grounded the
same way NEAR AI grounds ours: the witness publishes a nonce-bound attestation
of its own, and the contributor's client verifies the measurement **before**
sending anything. A client that cannot verify the measurement must refuse to
send, not warn and proceed.

The server trusts the witness's signature, and transitively its measurement. The
server never sees raw bytes, which is what keeps the existing "raw never reaches
the hosted service" property intact.

Nobody trusts the client. That is the point: today's alternative is a
client-computed verdict, which is authorization by self-report.

**Residual exposure, stated plainly.** A compromised witness sees every raw
trace passing through it. That is a larger blast radius than any component
currently in this system, and it is the price of the property. Mitigations —
short retention, no persistence of raw, memory-only processing, measurement
pinning by clients — reduce it but do not remove it.

## The pseudonym question, corrected

An earlier note claimed the witness could derive a stable per-payer pseudonym as
`H(salt, api_key)` because it would observe the key when fetching the signature.
**That was wrong, and it matters.**

Fetching `/v1/signature/{chat_id}` requires the contributor's NEAR AI API key.
For the witness to fetch it, the contributor must hand over that key — custody
of a live credential for a paid service. That is a serious ask and probably an
unacceptable one, and a design should not quietly assume it.

If the contributor fetches the receipt themselves and supplies it, the witness
never sees the key and cannot derive a pseudonym. So:

- **Content binding**: fully resolved by the witness, with no upstream change.
- **Sybil binding**: *not* resolved. It returns to being an ask on NEAR AI — a
  stable per-account pseudonym in the signed text.

The consequence for admission is the one already recorded in the attestation
spec: a per-trace cost floor with no per-attacker ceiling, which is a pricing
judgement rather than an open door, and should be made deliberately.

## Availability and failure

The witness sits on the submission path for attestation-gated contributors. If
it is down, they cannot submit — fail-closed, because a witness bypass is an
admission bypass.

This is a real operational commitment and an argument for keeping the witness's
job small: verify, sign, forget. It holds no database, no queue, and no state
beyond its signing key, so it can be replicated behind a load balancer and
restarted freely. Every stateful concern — replay of `chat_id`, the
per-contributor cap, dedup — stays on the server, where it already lives.

**Do not put the replay cache in the witness.** A distributed one-use check
across replicas is a hard problem the server has already solved for other
credentials.

## Deployment

Recommended: **we operate it**, on a TDX-capable host, publishing an attestation
report the way NEAR AI does.

Alternatives considered:

- **Contributor-side.** Strongest privacy — raw never leaves the machine — but
  consumer hardware largely lacks a usable general-compute TEE, and Apple
  Silicon has no TDX/SGX equivalent for this. Not viable for the population we
  are trying to admit.
- **NEAR AI hosts it.** They already run attested TDX+GPU and already hold the
  raw bytes, so it adds no exposure. Materially the best answer on privacy
  grounds, and a much larger upstream ask than the pseudonym. Worth raising in
  the same conversation.

The pilot is `c3-standard-4`, an Intel Sapphire Rapids generation. **Verify TDX
availability and the confidential-VM path on that machine type before committing
to it** — this design's feasibility rests on it and nothing here has confirmed
it.

## Open items

- **Does the contributor hand over an API key, or supply the receipt?** Decides
  whether the pseudonym is recoverable without NEAR AI. Assume "supplies the
  receipt" until decided; that is the safe default.
- **Whole-trace or per-turn witnessing.** Per-turn keeps payloads small and
  bounds exposure per call; whole-trace makes the correspondence check
  single-shot. Payloads above 16 MB argue for per-turn, the byte-equality check
  argues for whole-trace.
- **Retention of nothing, provably.** "Memory only, no persistence" is easy to
  claim and hard to demonstrate. What does the measurement actually pin, and
  what would an auditor check?
- **Upgrade discipline.** A witness image upgrade changes its measurement, so
  every pinning client breaks until it re-pins. That is the correct failure
  direction, and it needs a rollout story that does not tempt anyone into
  disabling verification.
- **Does the span list leak?** It carries offsets and replacement labels of
  redacted material. It goes to the witness only and must never reach the
  server, but the shape of a span list is itself information about what was
  found. Reason this through before shipping.
- **Whether the witness is worth it at all**, versus accepting a per-trace cost
  floor with no content binding. This is a substantial system — the first TEE
  deployment in the project — to close a gap whose practical cost depends on a
  pricing relationship that is currently favourable. Build the cheap verification
  first and measure before committing.

## Sequencing

1. The attestation verification slice (already planned) builds and proves the
   receipt and quote verification this design reuses.
2. Settle the API-key custody question, since it decides the pseudonym.
3. Confirm TDX availability on the intended host.
4. Only then plan the witness.

Nothing here blocks the current slice, and the current slice is a prerequisite
for this one.
