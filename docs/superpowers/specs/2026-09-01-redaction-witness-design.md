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
  out-of-process into the enclave" as future work. That was originally the
  heaviest cost in this design — a witness we hosted would have been the
  project's first trusted-execution deployment, with all the provisioning,
  measurement management, attestation serving and upgrade discipline that
  implies. **The decision to have NEAR AI host it removes that cost entirely**,
  which is most of why it is the right call. The fact remains true and is worth
  keeping: it is why any future proposal to run our own enclave should be
  costed carefully.
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
account_pseudonym          <- stable per-account, opaque; see Deployment
prompt_tokens, completion_tokens
model
timestamp
redaction_policy_version
witness_enclave_measurement
```

`account_pseudonym` is the field that makes the per-contributor cap bind. It is
available only because NEAR AI hosts the witness and therefore already knows the
account; it is opaque and must never be a name, an email, or anything resolvable
to a person. We hash it on arrival regardless.

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

The contributor sends raw bytes to **the party that already has them**. Under
the hosted design this is the design's strongest property: NEAR AI served the
inference, so it has already seen every one of those bytes. The witness adds no
reader.

Trust is still grounded the way NEAR AI grounds it today — a nonce-bound
attestation the client verifies before sending, and a client that cannot verify
the measurement must refuse rather than warn and proceed. The verification path
is the one already built for the inference endpoint, not a second trust model.

The server trusts the witness's signature, and transitively its measurement. The
server never sees raw bytes, which is what keeps the existing "raw never reaches
the hosted service" property intact.

Nobody trusts the client. That is the point: today's alternative is a
client-computed verdict, which is authorization by self-report.

**Residual exposure, stated plainly — and smaller than it first appears.** A
compromised witness sees every raw trace passing through it. But under the
hosted design that is **not a new reader**: NEAR AI served the inference, so it
already saw those exact bytes. The witness widens what one already-trusted party
does with data it already had, rather than adding a party.

What it does add is *duration* and *aggregation* — bytes that were transient in
an inference request now arrive again, deliberately, for a second purpose.
Short retention, no persistence of raw, memory-only processing and
client-side measurement pinning all still matter, and are worth asking them to
attest to.

Had we hosted it ourselves, this paragraph would read very differently: that
version genuinely added a reader, and would have been the largest blast radius
in the system.

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

## Deployment: NEAR AI hosts it

**Decided 2026-09-01 (Zaki). This is no longer something we build.**

NEAR AI already runs attested TDX + GPU, and already holds the raw bytes for
every inference it serves. A witness there adds **no exposure that does not
already exist** — the contributor has already sent that provider those exact
bytes. Every other placement adds a party.

Alternatives, and why they lose:

- **We operate it on GCP.** TDX is available as a confidential-compute type, so
  the hardware is not the obstacle. Two things are. It would be the project's
  first TEE deployment, carrying provisioning, measurement management,
  attestation serving and upgrade discipline. And **GCP's attestation model
  differs**: Confidential Space issues tokens from an attestation service rather
  than serving a nonce-bound quote the way dstack does, so the contributor-side
  verification would not resemble the NEAR AI verification we just built. We
  would be adding a party *and* a second trust model.
- **Contributor-side.** Strongest privacy in principle, but consumer hardware
  largely lacks a usable general-compute TEE and Apple Silicon has no equivalent.
  Not viable for the population this exists to admit.

### What this changes about the work

The witness stops being an implementation project and becomes a **specified
ask**. Our side shrinks to two pieces:

1. The client sends raw, redacted and the span list to NEAR AI's witness.
2. The server verifies the returned certificate against the redacted artifact
   it already holds.

Both are small. The enclave, the correspondence check and the signing key are
theirs.

### And it subsumes the pseudonym ask

This is the part that makes hosting worth asking for rather than merely
convenient.

The contributor authenticates to NEAR AI's witness with their **NEAR AI API
key** — the same credential they already use for inference. So the witness
*already knows the account*. It can put a stable per-account pseudonym in the
certificate without anyone adding a field to the receipt format, and without us
ever holding a credential.

So the two asks collapse into one. Previously: "add a pseudonym to the signed
receipt" **and** "consider hosting a witness". Now: **host a witness whose
certificate carries a stable per-account pseudonym.** One conversation, one
deliverable on their side, and it resolves content binding and sybil binding
together.

Note the granularity improves too. A pseudonym derived from the key would be
per-key, so one payer with several keys gets several caps. A witness that knows
the *account* can emit a per-account value, which is what the cap actually needs.

## Key custody: the contributor supplies the receipt

**Decided 2026-09-01 (Zaki).** The witness does not hold a contributor
credential on our behalf.

Under the hosted design this is not a limitation, because the contributor is
authenticating to NEAR AI directly — the same party that issued the key and
already accepts it for inference. No credential is surrendered to a third party,
because there is no third party.

The earlier concern in "The pseudonym question, corrected" — that deriving a
pseudonym would require custody of a live paid credential — applied to a witness
**we** hosted. It does not apply here, and that section should be read as
history rather than as a live constraint.

## Open items

Two of the original items are now settled and struck; the rest changed shape
because the witness is theirs to build, not ours.

- ~~Does the contributor hand over an API key?~~ **Settled: no.** See "Key
  custody".
- ~~Confirm TDX availability on the intended host.~~ **Moot.** NEAR AI's
  enclave is the host, and it is already attested and already verified by the
  code we shipped.
- **Whole-trace or per-turn witnessing.** Per-turn keeps payloads small and
  bounds exposure per call; whole-trace makes the correspondence check
  single-shot. Envelopes reach 16 MB and raw exceeds redacted, which argues for
  per-turn; the byte-equality check argues for whole-trace. **This is now a
  question for the ask**, since it shapes their API.
- **Retention of nothing, provably.** "Memory only, no persistence" is easy to
  claim and hard to demonstrate. Under the hosted design this becomes something
  to *ask them to attest to* rather than something we implement — what does the
  measurement pin, and what would an auditor check?
- **Upgrade discipline.** A witness image upgrade changes its measurement, so
  every pinning client breaks until it re-pins. Correct failure direction, but
  it needs a rollout story that does not tempt anyone into disabling
  verification. Also theirs, and worth agreeing up front.
- **Does the span list leak?** It carries offsets and replacement labels of
  redacted material. It goes to the witness only and must never reach the
  server, but the shape of a span list is itself information about what was
  found. Reason this through before shipping.
- **Whether this is worth asking for at all.** Still the real question, but the
  calculus has changed: it is no longer "is it worth building the project's
  first TEE deployment", it is "is it worth one upstream conversation". That is
  a much lower bar, and the same conversation now carries the pseudonym.

## The ask on NEAR AI, consolidated

Everything this design needs from them, in one place. Each item is something
they can already do or already know; none asks them to reveal content or
identity.

1. **Host a redaction witness** in the enclave that already serves inference.
   It receives raw bytes, the redacted artifact, and a span list; applies the
   spans to raw; requires byte equality; and verifies the inference receipt
   against the raw bytes it was given.
2. **Return a certificate over the redacted artifact**, with the fields in "The
   certificate" above.
3. **Include a stable per-account pseudonym** in that certificate. Opaque,
   salted, not resolvable to a person. This is the field that makes the
   per-contributor cap bind, and hosting is what makes it available.

Notice what is *not* on this list any more: a change to the receipt format. That
was the previous ask, and hosting subsumes it.

## Sequencing

1. ~~The attestation verification slice.~~ **Done** — receipt and quote
   verification are on `main`, and the live capture confirmed the receipt binds
   the model as well as both hashes.
2. ~~Settle API-key custody.~~ **Done.**
3. ~~Confirm TDX availability.~~ **Moot** under the hosted design.
4. **Run the drill once against the live service.** It has never executed end to
   end; #527 fixed the bug that would have stopped it. This validates the
   verification the witness certificate would reuse, and costs one completion.
5. **Put the consolidated ask to NEAR AI.** Their answer determines whether
   there is anything to plan.
6. Only then plan our two pieces: the client's send, and the server's
   certificate check.

Nothing here is blocked on us. The next move is a conversation, not a commit.
