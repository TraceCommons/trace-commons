# Perplexity Scorer — Design (Phase A2)

Date: 2026-05-12 (initial), 2026-05-13 (library recommendation flipped)
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets lane
Predecessor: `2026-05-11-private-vector-system-design.md` (rephased; this spec
implements its `PerplexityScorer` slot for Phase A)

> **Update 2026-05-13 — library flipped to `candle`.** The initial draft
> recommended `mistralrs` as the primary inference library with `candle`
> as fallback. A dependency-disclosure check found that `mistralrs`
> 0.8.0 and 0.8.1 (released 2026-04-02) both failed to build on
> docs.rs; the last successful build was 0.7.0 from 2026-01-28. The
> crate has a single maintainer (EricLBuehler). For a production gate
> that decides credit, betting on a single-maintainer crate whose
> latest two releases don't build is the wrong shape. Flipped the
> recommendation: **`candle` (HuggingFace, 20k stars, 0.10.2 released
> 2026-04-01) is now the primary** inference library, with `mistralrs`
> kept as a future alternative if its build-health recovers. Sections
> below are updated to reflect this.

## Update (2026-05-14)

This spec is preserved as historical record for the A2 slice; subsequent
retrofits have superseded several of its decisions. Read alongside the
successor specs:

- **Inference backend.** A2.3
  (`2026-05-13-mistralrs-migration-design.md`) reversed the 2026-05-13
  library flip above and migrated the perplexity scorer from a
  candle-direct Llama loader to a mistralrs-backed loader. The
  architecture is now auto-detected from each model's `config.json`;
  the candle-direct path and its per-arch dispatch table (A2.2,
  `2026-05-13-bakeoff-arch-dispatch-design.md`) are obsolete. The
  QK-Norm silent-fallback bug A2.2 fixed for candle Qwen3 is moot
  under A2.3 because mistralrs owns the implementation.
- **Default model.** A2.5 (`2026-05-14-gate-floor-recalibration-design.md`)
  recommends **Qwen3-8B-Base** as the operator default; the
  Llama-3.1-8B-Instruct recommendation in the table below is the
  pre-bake-off incumbent and is retained as a permitted choice but no
  longer the default.
- **Gate floors.** A2.5 ships the perplexity floor at `0` (disabled)
  for pilot launch because A2.3c + A2.4 measured aggregate-perplexity
  AUC < 0.5 across all candidates. The "set perplexity floor based on
  Phase 1 calibration" guidance below is deferred until Phase A.5
  lands a replacement metric.

## Goal

Replace `MockPerplexityScorer` in `tracedao-gate-enclave` with a real
implementation that loads a configured base language model, runs prefill
on a candidate trace, computes per-token logprobs, and returns an
aggregate perplexity + tail metric. The result drives the perplexity
half of the gate decision shipped in PR #10.

This is the **Phase A** implementation — runs on regular GPU hardware, no
enclave assumption. Phase B will move the same binary inside an attested
dstack enclave with no API change.

## Decisions baked in (confirmed 2026-05-12, library updated 2026-05-13)

| Decision | Value |
|----------|-------|
| Inference library | **`candle`** (HuggingFace-maintained, 20k stars, healthy release cadence) |
| Numeric format | **bf16, full precision** — no quantization in the gate path |
| Default model | **Llama-3.1-8B-Instruct** (well-studied, open weights, fits comfortably on H100) |
| Embedder library | **`fastembed-rs`** (separate concern; A3) |
| Cargo feature | **`local-gpu-models`** (off by default; default build stays hermetic) |
| Phase A target hardware | Single NVIDIA H100 80GB |

**Why these:**

- `candle` over `mistralrs` because mistralrs's last two releases
  (0.8.0, 0.8.1) failed to build on docs.rs as of 2026-04-02. Single-
  maintainer crate, last successful build was 0.7.0 in late January.
  Betting a production gate on a crate whose latest releases don't
  build is the wrong shape. Candle is HuggingFace-maintained with
  20k+ stars and active monthly releases — far more robust foundation.
  Revisit mistralrs only if its build health recovers and it offers a
  material logprob-API advantage.
- `candle` over `llama.cpp` because quantized logprobs are biased.
  For a perplexity *gate* that decides credit, the bias is the wrong
  shape — Q4/Q5 models systematically over-confident vs full precision.
- `candle` over a Python sidecar because that doubles the ops surface
  (Rust gate + Python inference) and makes the Phase B dstack migration
  harder.

## Non-goals

- Quantized inference. Document the choice; do not ship Q4/Q5/Q8 paths in v1.
- Learned classifier for "is this teachable." Single perplexity floor +
  tail-fraction metric only. Door open to swap in a learned gate later
  if calibration data accumulates and proves the simple gate is leaving
  signal on the table.
- Multi-model selection at runtime. One configured model per deployment;
  swap means redeploy.
- Real-time streaming gates. The aggregate metric needs the full token
  sequence; always batch.
- Calibration tooling. Operators tune floors per deployment; we ship
  reasonable defaults.
- Inference-server mode. The scorer runs in-process inside the gate
  service binary. No HTTP / gRPC surface for the model.

## Architecture

```
                    +----------------- gate worker route -------------------+
                    | POST /v1/workers/gate/evaluate                        |
                    +-----------------------+-------------------------------+
                                            v
                    +-----------------------+-------------------------------+
                    | TraceGateService::evaluate_trace                      |
                    | (EnclaveGateService adapter shipped in PR #10)        |
                    +-----------------------+-------------------------------+
                                            v
                    +-----------------------+-------------------------------+
                    | EnclaveGateOrchestrator                               |
                    | composes (perplexity, embedder, vector_index)         |
                    +-------+-------------------+-----------------+---------+
                            v                   v                 v
                   +--------+-------+   +-------+-------+  +------+-------+
                   | PerplexityScorer|   | Embedder      |  | VectorIndex  |
                   | (this spec)     |   | (A3 spec)     |  | (A4 spec)    |
                   +-----------------+   +---------------+  +--------------+
                            |
                            v
                   +-----------------+
                   | mistralrs model |  Llama-3.1-8B-Instruct, bf16
                   | (in-process)    |  loaded once at process start
                   +-----------------+
```

## `CandlePerplexityScorer`

New impl of the existing `PerplexityScorer` trait in
`crates/tracedao-gate-enclave/src/perplexity.rs`, gated by
`#[cfg(feature = "local-gpu-models")]`. The mock impl stays available
for tests; the new impl is selected at orchestrator construction time.

### Type sketch

```rust
#[cfg(feature = "local-gpu-models")]
pub struct CandlePerplexityScorer {
    model: candle_transformers::models::llama::Llama,  // owned; loaded once
    tokenizer: tokenizers::Tokenizer,
    device: candle_core::Device,
    tail_logprob_cutoff: f32,        // default -8.0
    model_id: String,                // for safe_status reporting
    max_tokens: usize,               // truncation cap
}

#[cfg(feature = "local-gpu-models")]
impl PerplexityScorer for CandlePerplexityScorer {
    fn score(&self, plaintext: &[u8]) -> PerplexityResult { ... }
}
```

Exact model type depends on the architecture (Llama, Mistral, Qwen all
ship under `candle_transformers::models::*`). Use the Llama variant for
the default Llama-3.1-8B-Instruct target.

### Loading the model

A separate async constructor:

```rust
pub async fn try_new(
    model_id: impl Into<String>,
    model_path: impl Into<PathBuf>,
    device: CandleDeviceKind,        // Cuda / Metal / Cpu
    tail_logprob_cutoff: f32,
    max_tokens: usize,
) -> anyhow::Result<Self>
```

Model loading is async because file I/O + GPU copy benefits from
non-blocking. Called once at gate-service startup. The model is then
held by reference in the orchestrator for the lifetime of the process.

`model_path` accepts:

- Hugging Face repo id (e.g. `meta-llama/Llama-3.1-8B-Instruct`) for
  download via `hf_hub`
- Local directory path for an already-downloaded model (safetensors +
  tokenizer.json + config.json)

`device` defaults to CUDA. Fall back to CPU only for development;
document that CPU inference at this size is impractical for production.

The Llama-3 example at `candle-examples/examples/llama` is the canonical
reference for model loading + forward-pass plumbing — use it as the
scaffold.

### `score(plaintext)` algorithm

1. **Tokenize** the plaintext with the model's tokenizer (mistralrs
   exposes this on the `Model` handle).
2. **Reject overlong inputs** above a configured cap (default 16K tokens
   — most traces fit; longer ones get truncated with a warning event).
   The cap is configurable via env at startup; not a per-call
   parameter because all callers should get the same gate behavior.
3. **Run a single forward pass** over the token sequence. Candle's
   model forward returns logits over the vocabulary for each position.
   Apply softmax to get per-token probability distributions, then for
   each position `i > 0`, look up the probability that the model
   assigned to the actual token at position `i` given positions
   `[0..i)` as context. Take `log` of that probability. We never
   sample; we never call `generate`. This is strictly cheaper than
   generation because there's no KV cache reuse across steps —
   one forward pass for the whole sequence.
4. **Compute the aggregate**:
   - For each token after the first, `nll_i = -logprob_i`.
   - `mean_nll = sum(nll_i) / (token_count - 1)`.
   - `perplexity = exp(mean_nll)`.
   - `perplexity_micros = (perplexity * 1_000_000) as u64` (saturate
     at `u64::MAX` for absurd values).
5. **Compute the tail metric**:
   - `tail_count = count of i where logprob_i < tail_logprob_cutoff`.
   - `tail_fraction = tail_count / (token_count - 1)`.
   - `tail_fraction_micros = (tail_fraction * 1_000_000) as u64`.
6. **Return** `PerplexityResult { aggregate_perplexity_micros,
   tail_fraction_micros }`.

The first token is excluded from both metrics because it has no
predictive context (BOS-prefixed); its logprob is conditioned on
nothing and is uninformative.

### Why both metrics

`mean_nll` answers "on average, how surprising was this?" `tail_fraction`
answers "did anything specific catch the model out?" A trace that's
uniformly easy with one hard span has low mean perplexity but a
non-zero tail. A trace that's uniformly hard but in a way the model
already partially handles has high mean perplexity but a low tail.

Either alone is a worse gate than both together. The orchestrator
(already shipped in PR #10) applies floors to both independently:
both must clear their respective floors.

### Failure modes

| Situation | Behavior |
|-----------|----------|
| Tokenization yields 0 or 1 tokens | Return `PerplexityResult` with `aggregate_perplexity_micros = 0, tail_fraction_micros = 0`. Orchestrator's perplexity floor will treat this as failing (assuming a configured floor > 0). |
| Plaintext exceeds the token cap | Truncate to the cap; emit a hash-only `tracing::warn!` event with the configured cap as label. Do not silently use the prefix only — record the truncation. |
| GPU OOM during prefill | `anyhow::bail!("PerplexityScorerOom: <model_id_hash>")`. Caller fails the gate evaluation with `PerplexityScorerUnavailable`. |
| Model not loaded (constructor failed) | Caller hits `Arc<dyn PerplexityScorer>` construction error at startup; never reaches the worker route. |
| Tokenizer disagrees with model (chat template / special tokens mismatch) | Surfaces at construction time as a model-load failure. Pin model + tokenizer versions in the deployment config to prevent drift. |

### Hash-only logging

All operational logs and error strings hash-only. The model id can
appear because it's an operator-configured label, not user state. The
tokenized plaintext never logs, never serializes, never leaves process
memory. The per-token logprob vector is consumed inline; only the two
aggregate u64s leave the function.

## Configuration

New env vars, all consumed at gate-service startup:

| Env | Default | Notes |
|-----|---------|-------|
| `TRACE_COMMONS_PERPLEXITY_MODEL_ID` | (required when `local-gpu-models` enabled) | e.g. `meta-llama/Llama-3.1-8B-Instruct` |
| `TRACE_COMMONS_PERPLEXITY_MODEL_PATH` | (required) | Local path to weights, or HF cache dir |
| `TRACE_COMMONS_PERPLEXITY_DEVICE` | `cuda` | `cuda` / `metal` / `cpu` |
| `TRACE_COMMONS_PERPLEXITY_MAX_TOKENS` | `16384` | Truncation cap |
| `TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF` | `-8.0` | Per-token cutoff for the tail metric |

The `gate_version_hash` (from PR #9's schema) is derived at startup from
a canonical hash of `(model_id, max_tokens, tail_logprob_cutoff,
gate_policy_version)`. Any of these changes → new `gate_version_hash`
→ existing credit events under the old version stay (grandfather-settled
policy from the rephased private vector spec). Operators bumping the
model version see this surface in `safe_status()`.

## Build feature

```toml
[features]
default = []
local-gpu-models = [
    "dep:candle-core",
    "dep:candle-nn",
    "dep:candle-transformers",
    "dep:tokenizers",
    "dep:hf-hub",
]

[dependencies]
candle-core = { version = "0.10", optional = true, features = ["cuda"] }
candle-nn = { version = "0.10", optional = true, features = ["cuda"] }
candle-transformers = { version = "0.10", optional = true }
tokenizers = { version = "0.21", optional = true }
hf-hub = { version = "0.4", optional = true, features = ["tokio"] }
```

Pin versions. Even though candle has a healthier release cadence than
mistralrs, pre-1.0 crates can still break on minor bumps. Pick the
exact versions at implementation time based on what's published; the
example above is illustrative.

**Hard dependency-policy gate:** None of these are currently on
`~/.claude/approved-dependencies.md`. Before implementation lands,
disclose:
- `candle-core`, `candle-nn`, `candle-transformers` — HuggingFace, 20k stars
- `tokenizers` — HuggingFace, widely deployed
- `hf-hub` — HuggingFace, for model downloading
- All Apache-2.0 / MIT licensed
- All on the recent-release radar (April 2026 releases)
- Transitive surface is significant (likely 50+ crates) — flag this
  honestly

Get explicit approval before `cargo add`.

## Memory budget

| Component | Footprint |
|-----------|-----------|
| Llama-3.1-8B-Instruct, bf16 weights | ~16 GB |
| Tokenizer + config | ~10 MB |
| KV cache for max prefill (16K tokens × 32 layers × 128 dims × bf16) | ~4 GB peak |
| Embedder (fastembed BGE-large) | ~2 GB |
| Vector index in RAM (Phase A scale) | <1 GB |
| OS + runtime + headroom | ~5 GB |
| **Total** | ~28 GB |

H100 80GB has ~50 GB headroom. A 70B model would need a multi-GPU
deployment; we're not doing that in Phase A.

## Latency budget

Prefill on an 8B model for a 4K-token trace is ~1 second on H100. Worst
case at the 16K cap is ~4 seconds. Embedder + vector query add ~50 ms.
Total worker-route latency at the 16K cap is ~4-5 seconds.

The `POST /v1/workers/gate/evaluate` route is async and worker-driven
(not synchronous-with-ingest), so a multi-second budget is acceptable.
Production deployments should scale gate workers based on submission
volume, not request latency.

## Testing

### Unit tests (no GPU)

- The mock `MockPerplexityScorer` shipped in PR #10 stays available and
  is what the orchestrator tests use. No new unit tests required for
  the trait surface — it's stable.
- Add a unit test for the aggregate-math helper if we extract one:
  given a fixture vector of per-token logprobs, produce expected
  `perplexity_micros` and `tail_fraction_micros`. This isolates the
  math from the mistralrs API surface so we can swap inference libs
  without re-testing the math.

### Integration tests (require GPU + model)

Behind `#[ignore]` plus an env-gate (`TRACEDAO_PERPLEXITY_INTEGRATION=1`
plus `TRACE_COMMONS_PERPLEXITY_MODEL_PATH`). Same idiom as the
fake-gcs-server integration test in PR #8.

Test cases:
1. **Smoke** — load the configured model, score a fixed plaintext,
   assert the result is deterministic across runs (same model + same
   input → same micros).
2. **Surprising vs unsurprising** — score "the quick brown fox jumps
   over the lazy dog" (low perplexity expected) and a randomly
   generated high-entropy byte string (high perplexity expected).
   Assert ordering, not specific values.
3. **Truncation** — feed input above the cap, assert the call still
   returns a result and the truncation warning fires.

These tests run in CI only when a GPU runner is available and the
model is pre-staged. Not blocking for the merge.

## Migration to Phase B

When dstack-GPU hardware is settled, the same `MistralRsPerplexityScorer`
moves inside the attested enclave. Code path stays identical; only:

- The binary's hosting moves from a regular GPU host to a dstack-GPU
  host
- The KEK swaps from `CloudKmsKeyWrapper` to `DstackKekWrapper`
- Attestation token verification gets added at the API boundary

No changes to this scorer's implementation are needed.

## Open questions

1. **Candle's Llama-3 example coverage of bf16.** Verify at
   implementation time that the example's loading path supports
   bf16 weights cleanly. Most recent HuggingFace Llama-3 releases
   ship in bf16 by default; this should work, but the
   `candle-examples/examples/llama/main.rs` source is the
   authoritative reference.

2. **Model id vs gate_version_hash binding.** Should the
   `gate_version_hash` include the model's weight-hash (e.g.,
   SHA-256 of the safetensors files)? Stronger forensic story, but
   ties the gate version to file content which is awkward for HF
   downloads. Decision: include the canonical model_id string only;
   document that operators must pin specific revisions for stability.

3. **Tail-fraction cutoff calibration.** Default `-8.0` is a guess.
   The right value depends on the corpus and model. Recommend
   operators tune this empirically; ship the default and document
   the calibration procedure.

4. **CPU fallback.** Candle supports CPU inference but it's
   impractical for 8B models. Should the constructor refuse CPU
   for production? Or warn-and-allow for dev?

   Recommendation: warn-and-allow. Production deployments will use
   GPU; dev convenience matters; the `is_production_trust_boundary`
   gate is the load-bearing prod check, not this.

## Cost estimate

| Item | Estimate |
|------|----------|
| Disclose `mistralrs` dependency, get approval | 1 day (response time) |
| `MistralRsPerplexityScorer` impl + unit tests | 3-5 days |
| Integration test harness (GPU + model staging) | 2-3 days |
| Operational validation against real traces | 1-2 days |
| Documentation | 1 day |
| **Total** | **~2 weeks of focused work** |

This assumes mistralrs's logprob API is workable. If we have to fall
back to candle or to a lower-level forward pass, add a week.

## What this spec does not commit to

- A specific mistralrs version (pinned at implementation time)
- A specific Llama variant (Llama-3.1-8B-Instruct is the default; the
  configured model_id is operator-controlled)
- Specific perplexity / tail floors (those are calibrated, not designed)
- Multi-GPU deployment patterns (single H100 in Phase A)
- Token cap (default 16K is a guess; operators tune)
