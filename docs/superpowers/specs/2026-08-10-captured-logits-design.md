# Captured logits: what they would buy, and what they would cost

**Date:** 2026-08-10
**Status:** Exploratory — recommends an experiment, not an implementation
**Scope:** The novelty and substance gates, the value scorecard, and the
envelope's privacy guarantee.

## The distinction this turns on

We already read logprobs. `perplexity_near_ai.rs` posts `/v1/completions` with
`echo=true` and `logprobs=N`, reads `token_logprobs` across the prompt, and
takes `top_logprobs` for token rarity — one round trip serving both
`PerplexityScorer` and `TokenRarityScorer`.

Those are *our scorer's* logprobs, computed after the fact, over redacted text,
under a model that did not produce the trace. They answer "how surprising is
this text to qwen3-30b".

Captured logits would be the **generating** model's distributions at the moment
of generation. They answer "where was the model that did this work actually
uncertain". Those are different questions, and only the second is about the
trace.

## What they would buy

**Novelty stops being a proxy for informativeness.** Today it is exact
canonical-summary hash, then deterministic redacted-summary similarity, then
private vector nearest-neighbour. All three measure *difference*. The trace
worth having is not the one unlike everything filed; it is the one where the
model was uncertain and a human's judgement resolved it. Entropy at a decision
point localises exactly that, and nothing we currently compute approximates it.

**Substance becomes measurable.** The substance gate asks whether a trace is
real work or template-shaped filler. Filler has flat, low surprisal throughout.
Problem-solving spikes. That is a direct measurement of the thing the gate is
already trying to judge.

**The scorecard could be calibrated rather than asserted.** It currently uses
`quality = clamp(event_count / 8, 0.15, 1.0)`, a binary `replayability`, and
weights chosen by hand. It also applied residual privacy risk twice for months
— every medium-risk submission scored exactly zero, ten of ten — and nobody
noticed, because the number had no audience. Entropy, surprisal of the token
actually taken, and divergence between the model's distribution and the human's
continuation are quantities that can be calibrated against outcomes instead of
argued about.

**It changes what the corpus is.** Post-training consumes preference data with
margins. A trace where a human rejected the top-1 continuation in favour of one
with a measurable logprob gap *is* a preference pair. The public copy already
promises that a sealed system "returns one number: whether the work taught a
machine something new". We cannot currently measure that. This is how it would
be measured.

## What they would cost

### The privacy inversion

**Logits are conditioned on the whole context, including whatever the local
scrubber removed.** Exporting logprobs computed over unredacted text exports a
channel that encodes the redacted content. Scrub-then-attach is incoherent: the
numbers were produced before the scrub.

That inverts the guarantee the architecture rests on — that nothing but the
scrubbed envelope leaves the contributor's machine, and that raw content is
never sent out even to be scrubbed. Three ways out:

1. **Recompute logits over redacted text.** Coherent, and exactly what we do
   today, so it buys nothing.
2. **Treat logits as precisely as sensitive as the raw prompt.** They never
   leave in the clear. Analysis goes to the data; only derived scores come
   back.
3. **Redact before generation, not after.** If the substitution happens on the
   request path, the model generates conditioned on placeholders, and the
   logprobs describe the redacted text because there never was an unredacted
   generation. See "The IronWire path" — this option is not hypothetical, and
   it did not exist when the paragraph above was first written.

(2) is the general answer, and it is the same shape as the private contributor
insight design (2026-08-07): computation inside an attested enclave, with the
sensitive artifact never in the operator's hands.

(3) is narrower but structurally stronger where it applies, because it removes
the contradiction rather than containing it. Its limit is the quality of the
pre-generation filter, which is a measurable-in-principle, unmeasured-in-fact
false-negative rate — not a proof. It reduces the problem; it does not close
it.

Anything else in this document is contingent on accepting (2) or (3).

### Forgeability

A logprob vector is numbers. A fabricated transcript at least has to look like
plausible work; fabricated logits asserting "the model was uncertain here" are a
direct gaming vector against credit. Captured logits are only worth credit if
their provenance is attested, which is condition 3 below.

### Size

Full-vocabulary distributions per token are impractical. Top-k (5–20) is
tractable, is what the OpenAI-shaped API returns, and is what our own scorer
already consumes. Top-k also bounds the leakage in the previous section without
eliminating it.

## What the world would have to look like

Four conditions. One is ours.

**1. Providers exposing per-token distributions.** OpenAI-shaped APIs return
`top_logprobs`; Anthropic's does not expose logprobs at all. Our dominant
source is Claude Code, so most of the corpus structurally cannot carry them
today. This is also not an oversight: per-token distributions materially
assist distillation and model extraction, so restricting them is a commercial
position, not a gap. **The world where logits flow freely is one of open
weights.** That is a coherent place for a commons to stand and an
uncomfortable one for a business premised on frontier-lab traces. It should be
decided deliberately rather than discovered.

**2. Harnesses that persist them.** Even where an API returns logprobs, the
client must request them and write them into the session file. Claude Code and
Codex rollouts do not. Ironclaw is ours; the others are not — and in Ironclaw
the request-side work turns out to be small. See "The Ironclaw path" below.

**3. Attestation at generation.** For logits to be creditable they must be
attributable: the provider signs `(model, context hash, token, top-k)`, or
generation happens inside an attested enclave that signs. **We already have
the second half of this** — `perplexity_near_ai.rs` runs inference inside
Intel TDX plus an NVIDIA GPU TEE via NEAR AI. What is missing is signing at
*generation* rather than at *scoring*. This is the one lever we hold, and it is
a real one.

**4. An ecosystem agreement that logits are prompt-sensitive.** Otherwise they
ship as metadata and every local-redaction claim in the category quietly
becomes false — including ours.

## The Ironclaw path

Ironclaw's NEAR AI provider is the one place where all four conditions can be
satisfied at once, because it is the only path where we control the harness,
the model is open-weight, and inference already runs inside a TEE.

The current state, verified in `crates/ironclaw_llm/src/nearai_chat.rs`:

- `ChatCompletionRequest` (line 1122) carries `model`, `messages`,
  `temperature`, `max_tokens`, `stop`, `tools`, `tool_choice`, `stream` and
  `stream_options`. There is no `logprobs` and no `top_logprobs`.
- `ChatCompletionChoice` (1481) deserialises only `message` and
  `finish_reason`. `ChatCompletionStreamChoice` (1556) only `delta` and
  `finish_reason`. Any `logprobs` the backend returns is discarded silently by
  serde.
- The single occurrence of the string `logprobs` anywhere under `crates/` is
  `"logprobs": null` in a test fixture at line 3853, asserting that unknown
  response fields are ignored.

So the request side is two optional fields, and the response side is a field on
each of two structs. The backend is vLLM/SGLang, which implements the
OpenAI-shaped `top_logprobs`. Ironclaw streams by default, so the distributions
arrive per-chunk in the SSE deltas and accumulate alongside the content that is
already being accumulated there.

**The wiring is not the work.** The work is everything downstream of it:

1. **Where do they go?** Ironclaw's trace/event store has no place for
   per-token distributions today. Top-k over a long agent session is a
   substantial volume next to the text, and it is the most sensitive thing in
   the record — see the privacy inversion above. It must not simply land in the
   event stream alongside content.
2. **What reads them?** Adding capture with no consumer repeats the failure
   that hid the double penalty for months: a number nobody reads does not get
   checked. Capture should land with the analysis that justifies it, or
   deliberately as a local-only experiment with an end date.
3. **Cost.** `top_logprobs` inflates response size materially. On a streaming
   agent loop that is bandwidth and storage on every turn, not once per trace.

**Recommended shape:** put capture behind an off-by-default config flag,
persist to a **separate local sidecar** rather than the trace event store, and
scope it to the NEAR AI provider only. That gives the experiment below real
inputs without committing the trace schema, without touching the submission
path, and without any decision about what leaves the machine — because in this
shape nothing does.

## The IronWire path

`nearai/ironwire` is a loopback proxy that sits at the inference boundary:
agents point at `127.0.0.1` and it routes each conversation to whichever
backend is available. That makes it a strictly better vehicle than patching one
harness, and it changes two of the four conditions above.

**It collapses condition 2.** The proxy sees every request from every harness —
Claude Code, Codex, Aider, Cline — without any of them cooperating. Its own
`docs/PRIVACY.md` puts it exactly right: it is "the one place in a coding
agent's life where every byte destined for a model passes through code the user
controls."

**It is already meant to be a contributor.** `docs/DESIGN.md` §8 describes a
trace ledger at `$IRONWIRE_HOME/ledger.sqlite`, local capture on by default and
upload off, handing records to `ironclaw_trace_commons`. There is a pipeline to
attach distributions to rather than one to invent. Note it is *designed, not
built*: no `Cargo.toml` mentions the `contribute` feature, and ROADMAP M6 still
lists wiring it as pending.

### The lane split decides the cost

`crates/ironwire_translate/src/request.rs:52`, `anthropic_to_chat_completions`,
builds the outbound body **from scratch** — a fresh `Map` with `model`,
`messages`, `stream`, `stream_options`, `max_tokens`, `stop`, `temperature`,
`tools`, `tool_choice` inserted key by key. On that cross-family path,
requesting logprobs is two inserts into a map that is already being
constructed, and it breaks no guarantee.

The **native** lane is the opposite. `docs/PROTOCOL.md` §2 enumerates exactly
five permitted mutations and ends "Nothing else. The body is otherwise the
bytes the client sent", pinned by `tests/passthrough.rs` — and even the `model`
rewrite is "a targeted JSON edit of that one key — never a full re-serialize."
Injecting sampling parameters there is a body mutation and would need the
treatment their privacy filter got: off by default, permanently visible in
`ironwire status`, marked per exchange in the ledger. Their reasoning transfers
without modification — "an exchange that was filtered is not comparable to one
that was not, and the log must not imply otherwise" is equally true of an
exchange whose sampling parameters we changed.

**So: cross-family lane only, and leave native passthrough alone.**

### Why this resolves the privacy inversion

IronWire's privacy filter substitutes sensitive values **on the way out**,
before the request reaches the provider, and restores them on the way back. The
upstream therefore generates conditioned on placeholders, and `docs/PRIVACY.md`
§8 already records post-substitution bodies in the ledger.

That is option (3) above, and it is the first configuration in which logits and
redaction are consistent rather than contradictory — not because the leak is
bounded after the fact, but because the unredacted generation never happened.
It is worth being precise about the limit: the filter's tiers 1 and 2 are
deterministic and reviewable, tier 3 is not started, and their own document
insists the false-negative rate "cannot be measured on the user's actual data".
This makes the problem tractable. It does not make it solved, and no interface
built on it may say otherwise.

### Two things to know before building

- **Capture belongs on the observation tee, not the translated response.** The
  Anthropic Messages shape has nowhere to put logprobs, so they are dropped
  translating back to the client — which is fine, since the agent does not need
  them. But `PROTOCOL.md` §2 says the tee "drops observations under pressure
  rather than blocking bytes", so capture there is **lossy by design**.
  Acceptable for an experiment; not acceptable for anything credit-bearing.
- **The ceiling is unchanged.** Anthropic exposes no logprobs, and the native
  Anthropic lane is Claude Code — still the dominant source. IronWire widens
  coverage to NEAR AI, Ollama, local and other OpenAI-shaped backends. It does
  not widen it to Claude. The `BackendKind::Local` path is the most interesting
  of those: open weights, local inference, nobody to ask.

## Recommendation: measure before re-architecting

Do not build any of this on the strength of the argument above. The argument
predicts that entropy at decision points separates valuable traces from
unremarkable ones. That prediction is testable now, cheaply, on the one path
where the inputs already exist.

**Experiment.** Capture top-k on the Ironclaw NEAR AI path, in the sidecar
shape above, for a small set of sessions. Keep them entirely local. Ask one
question: does an entropy-derived score rank traces in the order a human would?
Compare against the corpus we have — 352 submissions, with known
accept/quarantine outcomes and known scorecard values.

Ironclaw over NEAR AI is the right vehicle rather than the Codex path, even
though Codex's provider also returns `top_logprobs`. We control both ends,
the model is open-weight, and the TEE that condition 3 needs is already under
the inference. Nothing about the experiment depends on that yet, but the
follow-on work does, and the Codex path can never get there.

IronWire is the wider version of the same experiment and should follow rather
than replace it: it reaches Claude Code and Codex sessions too, provided they
are routed to an OpenAI-shaped backend. Do Ironclaw first because it is the
narrower change against a path we already understand; do IronWire second
because it is where the volume is, and because its pre-generation substitution
is the only mechanism here that makes capture and redaction consistent.

**It succeeds if** entropy-derived ranking disagrees with the current scorecard
in ways a reader judges to be improvements. **It fails if** it merely correlates
with length, or with what the existing novelty vector already finds.

Nothing leaves the machine during the experiment. No envelope schema changes.
No gate changes. If it fails, we have learned that the interesting version of
this needs the generating model's distributions rather than any proxy, and we
have spent days rather than a quarter.

## Decisions required

1. **Open weights or not.** Condition 1 is not an engineering problem and does
   not resolve itself. If the answer is that the corpus should be able to carry
   logits, the corpus is largely open-model traces, and that reshapes the
   pilot's recruitment.
2. **Do we accept logits as prompt-sensitive?** Everything else is contingent
   on this. If the answer is no, close this document.
3. **Is generation-time attestation worth pursuing with NEAR?** It is the piece
   only we are positioned to build, and it is useful independently of logits —
   a signed record of what a model produced is valuable to the register whether
   or not distributions come with it.

## Prerequisites already known

- The envelope carries `token_counts` and no distributional data. Any capture
  is a schema addition, with the redaction consequences above.
- `/v1/contributors/me/score-attestation` is unusable — signing key
  unconfigured. Attestation-shaped claims need that path working first.
- The scorecard was demonstrably miscalibrated until 2026-08-07 and remains
  heuristic. Adding a richer signal to a scorer nobody reads would repeat the
  failure that hid the double penalty; whatever consumes logits needs an
  audience.
