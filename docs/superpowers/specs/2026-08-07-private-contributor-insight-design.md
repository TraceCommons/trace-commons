# Private contributor insight, on infrastructure we already have

**Date:** 2026-08-07
**Status:** Proposed — decisions required before implementation
**Scope:** A contributor-facing analysis product built on the existing NEAR AI
TEE scoring path, and its boundary with the public register.

## Why

A contributor can today enrol, submit, be accepted, and see nothing. No
credit, no leaderboard row, no feedback about their own work. That is not a
bug in any one component — it is the shape of the system: every score computed
here feeds the corpus, and nothing feeds back to the person who produced the
trace. It leaves the pilot with a cold-start problem where the first
contributor's incentive depends on the hundredth arriving.

Comparable products solve this by making the analysis of *your own* sessions
the product. That works from the first run and needs nobody else to
participate.

They pay a privacy cost to do it. The category norm is to send transcript
excerpts — prompts and model responses — to a frontier provider for
summarisation. Code stays local; the conversation about the code does not.
That is the one place their story is structurally weak, because reading a
transcript requires a model, and a model means a provider.

## What we already have

Nearly all of it, which is the reason to write this down.

- **TEE-hosted inference.** `crates/trace-commons-gate-enclave/src/perplexity_near_ai.rs`
  posts to a NEAR AI Cloud model endpoint backed by vLLM inside Intel TDX plus
  an NVIDIA GPU TEE, smoke-validated 2026-05-17. The operator does not see
  what the model sees.
- **A privacy filter path** — `near-ai-privacy-filter` in the protocol crate,
  `--pii-filter near-ai` on the contributor CLI.
- **Local deterministic scrubbing** that runs before anything leaves, covering
  message text, tool calls, tool results and structured payloads, and never
  sends raw content out to be scrubbed elsewhere.
- **A gate-service notion of production-grade enclaves**
  (`TRACE_COMMONS_GATE_SERVICE=enclave_local_gpu`,
  `TRACE_COMMONS_NOVELTY_UTILITY_REQUIRE_PRODUCTION_GATE`).
- **A contributor CLI** that already discovers, scrubs, previews and submits.
- **A score-attestation endpoint** (`/v1/contributors/me/score-attestation`),
  currently returning 503 `attestation_signing_key_unconfigured`.

What is missing is not infrastructure. It is an output addressed to the
contributor.

## The claim, stated precisely

The temptation is to say "your data never leaves your machine". **We must not
say that**, because it would not be true: analysis needs a model, and the
model is remote.

What is true, and is still stronger than the category norm:

> Sessions are scrubbed on your machine first. What remains is analysed inside
> an attested enclave — not by us, and not by NEAR. You can verify the
> attestation before anything is sent.

Two properties, in this order, and the order matters:

1. **Scrub locally, then send.** The enclave is defence in depth, not the
   first line. If the attestation story ever failed, what the operator could
   have seen is still only scrubbed content.
2. **Verify, do not assert.** NEAR AI publishes attestation evidence. The
   client should check it and refuse to send if it does not verify, and should
   say which enclave measurement it accepted. A privacy property that the
   client takes on trust is a privacy adjective.

This project has twice shipped copy that overstated a guarantee — keyed jitter
described as Laplace noise, and a withdrawal promise the software could not
keep. The claim above is the most that can be said without a third.

## Shape

```
local session files
      |
      v
local deterministic scrub            (existing; nothing raw leaves)
      |
      v
attestation check                    (NEW: verify before sending)
      |
      v
NEAR AI TEE analysis                 (existing transport, new prompt/task)
      |
      v
private profile  ->  stays with the contributor
      |
      +-- optional, explicit, separate ------> submit to the register
```

The private profile is computed for the contributor and belongs to them. It is
**not** derived from, and does not derive, the corpus scorecard. Contribution
stays a separate act with its own consent scopes, as it is today.

### Why not reuse the corpus scorecard

`compute_value_scorecard` answers "is this trace worth adding to a shared
register" — novelty against everything already filed, substance, duplicate
penalty, privacy risk. Those are properties of a trace relative to a corpus.

A contributor wants to know something else: how they work. Those dimensions
have to be recognisable to the person, not to the register. Reusing the corpus
scorer would produce a number that is meaningless as self-knowledge and would
couple two products that should be able to change independently.

There is also a cautionary lesson in the corpus scorer's own history: until
today it applied residual privacy risk twice, and every medium-risk submission
scored exactly zero — ten of ten — without anyone noticing, because nobody was
reading the number for themselves. A score with no audience does not get
checked. That argues for building the personal profile as its own thing, with
its own calibration and its own reader.

### On archetypes

The category uses archetype labels and they are effective engagement design.
They are also a different product from a register of evidence held under
contributor terms. Adding scoring-as-entertainment would compete with the
thing that makes this credible. The recommendation is dimensions and concrete
observations drawn from the contributor's own sessions, and no labels.

## What must not happen

- **The private profile must not become a corpus input.** If it did, "private
  analysis" would be a funnel rather than a product, and the consent story
  would be false.
- **The enclave must not become a place raw content is sent.** Local scrub
  first, always.
- **Insight must not require contribution.** The moment the tool withholds
  analysis until you submit, it is not a privacy product.
- **No new personal data at rest server-side.** The profile is returned, not
  stored. If it is stored anywhere, that is a separate design with its own
  retention and withdrawal story — and note the pilot currently has *no*
  operator path to withdraw a contributor's public attribution, which is
  already an outstanding gap.

## Decisions required

1. **Is the private tool the same product as the register, or a sibling?**
   Shared pipeline is clearly right. A shared funnel — download the analyser,
   get asked to contribute — may not be. This decides packaging, gating and
   whether the analyser is invite-only.
2. **What dimensions?** This is the substance of the product and cannot be
   derived from what exists. It wants a developer's judgement about what is
   worth knowing, not an engineer's about what is computable.
3. **Attestation policy.** Which measurements are accepted, who maintains that
   list, and what the client does when verification fails — refuse, or warn
   and proceed with consent.
4. **Cost.** TEE inference over full sessions is materially more expensive than
   the current per-trace scoring pass. Free-to-contributor means someone pays.

## Prerequisites already known

- `/v1/contributors/me/score-attestation` is unusable: the signing key is
  unconfigured, so `attest` returns 503. Any attestation-shaped promise needs
  that path working first.
- The contributor CLI has no local-only mode; every command assumes an
  enrolled device with an instance. A private analyser that is useful before
  joining implies enrolment becomes optional for analysis.

## Recommendation

Build it, and build it as a sibling rather than a funnel. The infrastructure
that makes the strong claim possible is already here and is unusual; the piece
that is missing is small and is a product decision rather than an engineering
one. The cold-start problem it fixes is the pilot's most acute practical
constraint — more than differential privacy, more than the scoring
calibration, and more than anything else found this week.

Start with decision 2. Everything else follows from knowing what the profile
is supposed to tell someone.
