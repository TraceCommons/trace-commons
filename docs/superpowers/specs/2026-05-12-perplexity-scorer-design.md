# Perplexity Scorer — Design (Phase A2)

Date: 2026-05-12
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets lane
Predecessor: `2026-05-11-private-vector-system-design.md` (rephased; this spec
implements its `PerplexityScorer` slot for Phase A)

## Goal

Replace `MockPerplexityScorer` in `trace-commons-gate-enclave` with a real
implementation that loads a configured base language model, runs prefill
on a candidate trace, computes per-token logprobs, and returns an
aggregate perplexity + tail metric. The result drives the perplexity
half of the gate decision shipped in PR #10.

This is the **Phase A** implementation — runs on regular GPU hardware, no
enclave assumption. Phase B will move the same binary inside an attested
dstack enclave with no API change.

## Decisions baked in (confirmed 2026-05-12)

| Decision | Value |
|----------|-------|
| Inference library | **`mistralrs`** (pure Rust, LLM-focused, exposes logprob-friendly APIs) |
| Numeric format | **bf16, full precision** — no quantization in the gate path |
| Default model | **Llama-3.1-8B-Instruct** (well-studied, open weights, fits comfortably on H100) |
| Embedder library | **`fastembed-rs`** (separate concern; A3) |
| Cargo feature | **`local-gpu-models`** (off by default; default build stays hermetic) |
| Phase A target hardware | Single NVIDIA H100 80GB |

**Why these:**

- `mistralrs` over `candle` because its evaluation-path APIs are more
  mature (logprobs, prefill-only). Falls back to `candle` if the API
  stalls — same compile target, same Phase B migration story.
- `mistralrs` over `llama.cpp` because quantized logprobs are biased.
  For a perplexity *gate* that decides credit, the bias is the wrong
  shape — Q4/Q5 models systematically over-confident vs full precision.
- `mistralrs` over a Python sidecar because that doubles the ops
  surface (Rust gate + Python inference) and makes the Phase B dstack
  migration harder.

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

## `MistralRsPerplexityScorer`

New impl of the existing `PerplexityScorer` trait in
`crates/trace-commons-gate-enclave/src/perplexity.rs`, gated by
`#[cfg(feature = "local-gpu-models")]`. The mock impl stays available
for tests; the new impl is selected at orchestrator construction time.

### Type sketch

```rust
#[cfg(feature = "local-gpu-models")]
pub struct MistralRsPerplexityScorer {
    model: mistralrs::Model,        // owned; loaded once
    tail_logprob_cutoff: f32,        // configured; default -8.0 (i.e. exp(-8) ~ 0.03%)
    model_id: String,                // for safe_status reporting
}

#[cfg(feature = "local-gpu-models")]
impl PerplexityScorer for MistralRsPerplexityScorer {
    fn score(&self, plaintext: &[u8]) -> PerplexityResult { ... }
}
```

### Loading the model

A separate async constructor:

```rust
pub async fn try_new(
    model_id: impl Into<String>,
    model_path: impl Into<PathBuf>,
    device: MistralRsDevice,        // Cuda / Metal / Cpu
    tail_logprob_cutoff: f32,
) -> anyhow::Result<Self>
```

Model loading is async because mistralrs file I/O + GPU copy is async.
Called once at gate-service startup. The model is then held by reference
in the orchestrator for the lifetime of the process.

`model_path` accepts:

- Hugging Face repo id (e.g. `meta-llama/Llama-3.1-8B-Instruct`) for
  download-on-startup
- Local directory path for an already-downloaded model

`device` defaults to CUDA. Fall back to CPU only for development;
document that CPU inference at this size is impractical for production.

### `score(plaintext)` algorithm

1. **Tokenize** the plaintext with the model's tokenizer (mistralrs
   exposes this on the `Model` handle).
2. **Reject overlong inputs** above a configured cap (default 16K tokens
   — most traces fit; longer ones get truncated with a warning event).
   The cap is configurable via env at startup; not a per-call
   parameter because all callers should get the same gate behavior.
3. **Run prefill** via `model.generate_with_logprobs(...)` (or whichever
   API mistralrs exposes for "return per-token logprobs without
   actually generating new tokens"). Set `max_new_tokens = 0` so
   nothing is generated; we only want the prefill logprobs.
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
local-gpu-models = ["dep:mistralrs"]

[dependencies]
mistralrs = { version = "<pinned>", optional = true }
```

Pin to a specific version. The crate is pre-1.0; pinning prevents
silent breakage on a `cargo update`. Pick the version at implementation
time based on what's published.

**Hard dependency-policy gate:** `mistralrs` is not currently on
`~/.claude/approved-dependencies.md`. Before implementation lands,
disclose:
- Crate version, monthly downloads, last publish date
- Direct + transitive dep count
- License (likely Apache-2.0)
- Maintenance signals
- Whether `candle` (already-on-the-approved-list-eligibility-radar)
  ends up in the transitive tree anyway

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

Behind `#[ignore]` plus an env-gate (`TRACE_COMMONS_PERPLEXITY_INTEGRATION=1`
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

1. **mistralrs prefill-with-logprobs API stability.** Verify at
   implementation time that the crate's logprob-returning path
   actually exposes per-token logprobs for prefill tokens, not just
   for newly generated tokens. If it doesn't, two fallbacks: (a) use
   the lower-level model.forward path and compute logprobs from
   logits ourselves (no library dependency on the high-level API);
   (b) switch to candle.

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

4. **CPU fallback.** `mistralrs` supports CPU inference but it's
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
