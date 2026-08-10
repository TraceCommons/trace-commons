# Note: logits from IronWire, and what Trace Commons can actually take

Written 2026-08-10 for whoever is evaluating whether IronWire should capture
logits, so that effort and this repository do not converge on shipping
something neither wants.

This is advisory. It is not a decision, and it is not a rejection of the idea
-- the idea is good. It is three constraints that are cheaper to know now than
to discover after the capture format is built, and a recommendation that
preserves most of the value while satisfying all three.

## What is being proposed, as I understand it

IronWire (`github.com/nearai/ironwire`) is a local inference proxy: tools point
at `127.0.0.1:8463` instead of a provider, and it routes across subscriptions
and API accounts while keeping a local ledger of traffic. The proposal is to
have it also capture logits, so that traces reaching Trace Commons carry
per-token model confidence rather than text alone.

If I have that wrong, the three constraints below still apply to any design
that puts raw per-token distributions into a contributed trace.

## Constraint 1: generation logprobs are not the perplexity gate, and cannot replace it

This is the one most likely to cause wasted work, because the substitution
looks obviously correct and is not.

Trace Commons scores novelty by re-running a candidate trace through a
**reference judge** -- currently Qwen3.6-27B, via NEAR AI, using `echo` plus
logprobs. That measures *how surprising this text is to a fixed external
model*. Logits captured at generation time measure *how confident the
producing model was in its own sample*. Those are different questions, and the
second is self-referential: a model is generally confident in text it just
produced, so its own logprobs are a poor novelty signal by construction.

There is direct evidence in this repository that the judge, not the text, is
what makes the signal work. The A2.6 bake-off found perplexity-as-novelty
discriminated only at 27B scale; every 8B-class candidate flunked it across
every corpus. That is a property of the scorer. Swapping in whatever model
happened to generate the trace -- which will vary per contributor, per
session, and per routing decision inside IronWire -- does not inherit that
property, and would make the gate's behaviour depend on the contributor's
subscription mix.

**So:** captured logits are a new signal, not a cheaper version of an existing
one. If part of the motivation is retiring the gate's re-scoring cost, that
part does not hold, and the design should not be sized around it.

## Constraint 2: raw per-token distributions do not fit, by an order of magnitude

`MAX_INGEST_BODY_BYTES` is 2 MiB. That is the whole envelope: transcript,
metadata, receipts, everything.

Taking a real pilot trace of 169 KB of text (roughly 43,000 tokens, the one
the chunker splits into 15 chunks), and costing a `(token, logprob)` pair at
about 26 bytes of JSON:

| top-k | payload | vs the 2 MiB budget |
|---|---|---|
| 1 | 1.1 MB | 0.5x |
| 5 | 5.6 MB | 2.7x |
| 10 | 11.2 MB | 5.4x |
| 20 | 22.5 MB | 10.7x |

Even **top-1** consumes half the entire budget before a single character of
the actual transcript is included. Top-5 is nearly three times over on its
own.

This is not a limit that can be raised casually: it bounds request size on a
public ingest endpoint, and it is one of the things keeping that endpoint
cheap to defend. A design that requires raising it should say so explicitly
and argue for it, rather than discovering it at integration time.

## Constraint 3: publishing raw logprobs is a model-extraction surface and a likely terms problem

Stated carefully, because the strong version of this claim is not true and
overstating it would be its own failure.

There is published work -- Carlini et al., *Stealing Part of a Production
Language Model* (2024) -- recovering the final embedding projection layer and
hidden dimension of production models from logit-bearing APIs. The mechanism
depends on **adaptive querying**: choosing inputs, using logit bias, and
iterating. A static corpus of logprobs harvested from organic traffic is a
materially weaker attack surface than an interactive API, and I am not
claiming a corpus is equivalent.

What is nonetheless true:

- It is a **novel** exposure this project does not currently have anywhere.
  Today no part of the envelope carries logprobs; a grep for `logprob` or
  `logit` across `trace-commons-protocol` returns nothing.
- The **terms** question is sharper than the technical one. These would be
  logprobs from Anthropic, OpenAI and other providers' models, harvested via a
  proxy and republished in an open research commons. Whether that is permitted
  by those providers' terms is a legal question, and the answer is plausibly
  no for at least one of them. That needs a read before the format is built,
  not after contributors have submitted a corpus that cannot be distributed.
- The commons is **public and irrevocable in practice**. Trace Commons already
  supports withdrawal, but a distributed corpus cannot be recalled from
  everyone who has it. Exposure decisions here are effectively permanent.

## What Trace Commons already has, and it is the right shape

The envelope has carried this field since before the question came up:

```rust
pub struct TrainingDynamicsSignals {
    pub mean_confidence: Option<f32>,
    pub variability: Option<f32>,
    pub correctness: Option<f32>,
    pub cartography_bucket: Option<CartographyBucket>,  // Easy | Ambiguous | Hard | Unknown
}
```

It is dataset-cartography shaped, and it is entirely unpopulated today --
every field `None`, because nothing in the pipeline can compute them. Captured
logits are exactly what would populate it.

That granularity satisfies all three constraints at once:

- It is a **generation-time confidence** signal, sitting alongside the
  reference-judge novelty score rather than pretending to replace it
  (Constraint 1).
- It is **four numbers per trace** instead of tens of megabytes
  (Constraint 2).
- Aggregate statistics over a distribution give an attacker essentially
  nothing to invert, and are far easier to defend as fair use of provider
  output than verbatim distributions (Constraint 3).

## Recommendation

**Have IronWire compute the aggregates locally and emit those.** It is the
component that holds the raw distributions at generation time, so it is the
only place the reduction can happen without the raw data ever leaving the
contributor's machine. The raw logits never enter a trace, never cross the
ingest boundary, and never enter the commons.

Concretely, IronWire would emit per-session: mean token confidence, its
variability across the session, and enough to place the session in an
Easy/Ambiguous/Hard bucket. Trace Commons would carry them in the field that
already exists.

This also composes with the other open question about IronWire. If IronWire
becomes a `TraceSource` for this project -- reading its local ledger post-hoc,
the way the Claude Code and Codex adapters read theirs -- then the aggregates
travel the same path as everything else: redacted through the same pipeline,
previewed before submission, and covered by the same consent scopes. No new
trust surface, and specifically no proxy sitting in the contributor's traffic
path holding provider credentials, which is a much larger ask than the current
"we read transcripts you already have on disk."

## What would have to be true for raw logprobs to be worth revisiting

Not never. The conditions are:

1. A named consumer that genuinely needs per-token distributions and cannot
   use aggregates. Distillation and some RL objectives might qualify;
   "richer data" does not.
2. A legal read confirming the providers permit republication of logprobs
   derived from their models.
3. A transport that does not route them through the 2 MiB ingest path -- a
   separate sidecar artifact with its own retention and its own consent scope,
   not the envelope.
4. A consent scope of its own. A contributor consenting to
   `model_training` has not thereby consented to publishing their model's
   full output distributions, and treating one as implying the other would be
   the kind of quiet scope widening this project's consent model exists to
   prevent.

Until those hold, aggregates get most of the value at a small fraction of the
risk.

## Open questions for the IronWire side

1. Does the ledger record request/response **content**, or only metadata and
   routing decisions? If it is counts and costs without prompts and
   completions, it is telemetry rather than a trace source, and the
   `TraceSource` idea does not work at all.
2. Is the on-disk ledger format **stable and documented**? An adapter against
   an internal schema that moves freely is a maintenance treadmill. The
   README's own status note is candid that parts are unproven, which is fair
   for its age but relevant to depending on it.
3. Do the providers IronWire proxies expose logprobs at all on the routes that
   matter? Several do not, or do so only on some endpoints -- which would make
   coverage patchy and the resulting signal biased toward whichever providers
   happen to support it. Biased coverage is worth knowing before it is
   discovered in the corpus.
