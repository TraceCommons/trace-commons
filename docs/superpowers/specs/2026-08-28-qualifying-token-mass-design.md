# Qualifying token mass: a composition-sensitive admission statistic for large traces

Status: design, approved 2026-08-28. Ships in shadow mode; gates nothing until
calibrated.

Related: #478 (chunk cap governs most traces), #444 (closed in favour of #478),
#205 (the calibrated perplexity floor admits near-duplicates), #446
(recalibrate against the real-session distribution).

## Problem

The gate admits on `representative_perplexity_micros`, which
`aggregate_chunked_perplexity` computes as

    representative = exp( sum_c sum_nll_c / sum_c n_c )

a token-weighted geometric mean over the scored chunks
(`crates/trace-commons-gate-enclave/src/chunk_aggregate.rs:44`). The floor test
is `representative >= perplexity_floor_micros`
(`orchestrator.rs:137`). Two properties of that statistic fail on the traces
the pilot now receives.

**It regresses toward the global mean as traces grow.** A short trace is
homogeneous and its mean can be extreme. A 176-chunk session mixes reading,
editing, tool output and prose, so its mean lands near the global average of
agent text. A floor calibrated when traces were about 9 chunks therefore
separates short traces and passes essentially every long one. Measured in #478
on 2026-08-28: perplexity passes 98% of uncapped and 93% of capped decisions.
The floor is not discriminating; it is merely precise.

**It cannot see composition.** A trace that is 90% boilerplate with 10%
substantive work produces the same mean as one that is uniformly mediocre.
"How much of this is worth having" is the admission question, and a mean cannot
express it.

Note what is NOT the problem. Strided chunk selection
(`CHUNK_SELECTION_ALGORITHM = "stride_endpoint_inclusive.v1"`) is an unbiased
sampler of a mean, so for the quantity currently gated the chunk cap costs
variance, not bias. #478's question 2 hypothesises that striding is wrong for a
max-like quantity; that hypothesis applies to `peak_novelty_micros`, which is
computed and persisted but never gates. It does not explain the perplexity
pass rates.

## Statistic

    qualifying_token_fraction
        = sum{ n_c : exp(sum_nll_c / n_c) >= chunk_floor } / sum_c n_c

The token-weighted share of the scored trace that sits in chunks clearing a
per-chunk perplexity bar. Reported in micros, matching every other value on
the decision.

Why this shape:

- It inherits calibration from a threshold we know lands in a populated region
  of the distribution, which is precisely what `tail_fraction` did not do
  (below).
- It does not regress with length. A proportion is a property of the session,
  not of how long the session ran.
- It is interpretable to a contributor: "62% of this session was substantive
  content" says something "perplexity 7.1" does not.
- A strided sample is an unbiased estimator of a proportion, so the chunk cap
  admits a computable confidence statement rather than an unexamined one. Not
  built here; noted because it is the reason this shape survives capping.
- It is graded, so the credit pipeline can consume it later without a second
  design.

### What `tail_fraction` teaches

`tail_fraction_micros` is already a mass-shaped statistic and it is degenerate:
81% of pilot traces score exactly 0 and the floor is disabled
(`TAIL_FRACTION_FLOOR_MICROS=0`). The cause is threshold placement, not shape.
`tail_tokens` counts individual TOKENS with `logprob < tail_logprob_cutoff`,
default -8.0, i.e. p < 3.4e-4 (`perplexity_near_ai.rs:471`). Against a 27B
model on agent text almost no token is that surprising, so the numerator is
empty by construction.

Qualifying mass differs in unit and in threshold provenance: the unit is a
chunk of roughly 2048 tokens, and the bar is a perplexity floor whose live
region is already known. That is an argument, not a proof, which is why the
calibration gate below is mandatory rather than advisory.

## Components

### 1. `chunk_aggregate.rs` — the computation

`aggregate_chunked_perplexity` already holds every `ChunkPerplexity` and is
pure. The statistic is computed there and nowhere else, adding
`qualifying_token_fraction_micros` to `ChunkedPerplexityAggregate` beside the
existing representative, peak, tail and `tokens_scored` fields. No new pass
over the data and no additional inference: it reads per-chunk values that have
already been computed.

Signature grows one parameter, `qualifying_chunk_floor_micros: u64`, matching
how `min_chunk_tokens` is already threaded for the peak.

Two deliberate omissions:

- **No `min_chunk_tokens` guard.** Peak needs one because a max over tiny
  chunks is noise. A token-weighted proportion already weights a tiny chunk to
  near-nothing; a second knob would only add a way to be wrong.
- **No admission threshold.** In shadow mode the statistic gates nothing, so no
  cutoff exists in code. Naming one now would be a constant nobody has
  measured.

Degenerate input (zero total tokens) collapses to 0, matching the module's
stated fail-closed convention.

### 2. `EnclaveGateOrchestratorConfig` — one new knob

`qualifying_chunk_floor_micros: u64`, defaulting to `perplexity_floor_micros`.

It is a separate field rather than a reuse of the whole-trace floor. Coupling
them means recalibrating the whole-trace floor silently moves the composition
statistic. That is the failure mode #478 describes for the chunk cap, which
changed character without its constant changing, and it is cheap to avoid here.

### 3. Decision plumbing

`OrchestrationDecision` and `GateDecision` carry
`qualifying_token_fraction_micros` through `trace_gate_service.rs` into the
audit row, following the path `total_chunk_count` (V47) and the composite score
(V53, #491) already take.

### 4. `V54__trace_gate_decision_qualifying_mass.sql`

One nullable `BIGINT` on `trace_gate_decisions`. V53 is the highest migration
on `main` as of this writing; confirm before creating the file.

Nullable is load-bearing. A pre-V54 row genuinely has no value, and
`ScoreAttestationCoverage` set the precedent of signing an honest unknown
rather than an estimate for pre-V47 rows.

`run_migrations` is hand-rolled in this repo. V54 must be wired into
`crates/trace-commons-server/src/db/postgres.rs` in the same three places V47
appears, or it will not run.

### 5. `trace-commons-gate-calibrate qualifying-mass`

Calibration is a subcommand beside the existing `tail-floor --sidecar`, not an
admin re-score route. The admin re-score pattern works because perplexity can
be recomputed from stored plaintext; here that would mean paying full inference
again across the corpus.

## The backfill constraint

Per-chunk logprobs are not persisted, only the aggregates derived from them.
The statistic is therefore computable **only at scoring time**, and cannot be
reconstructed for any decision already taken. Every existing pilot decision is
permanently unknown for this field.

This is a genuine cost of the design and the main argument for landing the
shadow write before anything consumes it: calibration data accrues only
forward from deploy, so the clock starts at the shadow deploy, not at the
decision to use the statistic.

## Calibration gate

The statistic may not become a floor until a `qualifying-mass` run over a real
corpus shows non-degenerate spread. Concretely: the statistic must not pin at 0
or 1 for more than an agreed share of traces, with the share fixed before the
run rather than after seeing it.

If it pins, A has failed exactly as `tail_fraction` failed, and the fallback is
the length-aware floor (B below) rather than shipping a second dead statistic.

Rejected alternatives, recorded so they are not re-derived:

- **B, length-aware floor.** Keep the mean, make the floor a function of chunk
  count. Cheapest possible change and no new column, but it only recenters: it
  restores a sensible reject rate without restoring any ability to see
  composition, and needs recalibrating whenever trace length drifts again. Held
  as the fallback if A fails calibration.
- **C, lower-quantile per-chunk perplexity.** Gate on p25 of the per-chunk
  values. Comparable composition-sensitivity at comparable cost, but less
  interpretable, and a quantile from a 16-point sample is noisier than a
  proportion from the same sample.

## Testing

Tests first, in `chunk_aggregate.rs`:

- all chunks clear the floor -> 1.0
- no chunk clears the floor -> 0
- mixed input is token-weighted, not chunk-counted: one large qualifying chunk
  against many small failing ones must score high, which separates the two
  readings
- single-chunk trace -> the chunk's own indicator, matching the existing
  "representative equals peak for one chunk" convention
- zero-token and degenerate input -> 0
- ordering invariance

And the test the design rests on:

- **Composition sensitivity.** Hold a substantive trace fixed and pad it with
  increasing boilerplate. Qualifying mass must fall roughly in proportion to
  the padding while the representative mean moves sub-proportionally. If this
  cannot be made to pass, the premise that the mean cannot see composition is
  wrong and the design should stop here.

## Out of scope

- Any change to novelty, including the mean-vs-peak mismatch
  (`novelty_passed` tests the token-weighted representative while
  `peak_novelty_micros` is persisted and unused). Worth its own issue.
- Any change to the chunk cap, sampling strategy, or inference spend: #478
  questions 1, 2 and 4.
- Contributor-facing rendering. A number that gates nothing should not be shown
  as though it does.
- Confidence intervals over the strided sample.
