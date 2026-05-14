# Embedder — Design (Phase A3)

Date: 2026-05-13
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets lane
Predecessor: `2026-05-11-private-vector-system-design.md` (rephased; this spec
implements its `Embedder` slot for Phase A)
Sibling: `2026-05-12-perplexity-scorer-design.md` (A2 — shares the
`local-gpu-models` feature flag and runs on the same H100)

## Goal

Replace `MockEmbedder` in `tracedao-gate-enclave` with a real
implementation that loads a configured embedding model, runs inference
on a candidate trace plaintext, and returns a unit-normalized
embedding vector. The result feeds the novelty half of the gate
decision via `EnclaveGateOrchestrator`'s vector-index query.

Phase A target: regular GPU. Phase B (dstack): same impl, different
host.

## Decisions baked in

| Decision | Value |
|----------|-------|
| Library | **`fastembed-rs`** (Apache-2.0, healthy releases, Anush008-maintained, ONNX-backed via `ort`) |
| Default model | **`BGE-large-en-v1.5`** (335M params, well-studied, matryoshka-compatible) |
| Output dim | **1024 (full BGE-large)** with truncation to 256 for nearest-neighbor coarse search if matryoshka is enabled |
| Numeric format | f32 cosine similarity over unit-normalized vectors |
| Cargo feature | **`local-gpu-models`** (shared with A2 — single feature for the whole local-inference path) |

## Update (2026-05-14)

This spec remains current. The A2.3 mistralrs migration
(`2026-05-13-mistralrs-migration-design.md`) and the A2.5 gate-floor
recalibration (`2026-05-14-gate-floor-recalibration-design.md`) only
touched the perplexity-scorer half of the local-inference path; the
fastembed + BGE-large-en-v1.5 embedder is unchanged. Because A2.5
ships the perplexity floor disabled at pilot launch, the embedder
feeds the **primary active gate** (novelty), making this path
load-bearing rather than a secondary signal.
| Hardware | Single NVIDIA H100 80GB (shared with A2) |

**Why these:**

- `fastembed-rs` is purpose-built for sentence-embedding inference.
  Smaller and simpler than running an LLM stack for embedding (BGE
  through a candle Llama runtime would be technically possible but
  ugly).
- It uses `ort` (ONNX Runtime) for inference — the standard format
  for sentence embedders. ONNX conversion is a one-time prep step,
  not a runtime concern.
- BGE-large gives strong English performance and has matryoshka
  variants (Snowflake / BGE-M3 style) for nested-dim ranking.
  Acceptable fallback: `BGE-base-en-v1.5` (110M params) if memory
  pressure becomes real.

## Non-goals

- Multi-lingual embedding. v1 is English-only; non-English traces
  will get unreliable novelty scores. Acceptable for the pilot since
  Trace Commons traces are AI-agent conversations which are
  English-dominant.
- Multi-model selection at runtime. One configured embedder per
  deployment; swap means redeploy.
- Custom fine-tuned embedders. Use off-the-shelf BGE; tuning is a
  future spec when there's calibration data.
- Streaming / chunked embedding. Single forward pass over the full
  trace plaintext.
- ColBERT-style multi-vector embeddings. Single vector per trace.

## Architecture

```
                  EnclaveGateOrchestrator::evaluate()
                                |
              +-----------------+-----------------+
              |                                   |
              v                                   v
    PerplexityScorer (A2 / candle)       Embedder (A3 / fastembed)
              |                                   |
              v                                   v
    PerplexityResult                      Vec<f32> (unit-normalized)
                                                  |
                                                  v
                                       VectorIndex (A4 / usearch)
                                                  |
                                                  v
                                       NoveltyResult (top-k cosine)
```

The embedder is invoked once per `evaluate_trace` call. It loads its
model once at startup and holds it for the lifetime of the process.

## `FastEmbedTextEmbedder`

New impl of the existing `Embedder` trait in
`crates/tracedao-gate-enclave/src/embedder.rs`, gated by
`#[cfg(feature = "local-gpu-models")]`. The mock impl stays available
for tests.

### Type sketch

```rust
#[cfg(feature = "local-gpu-models")]
pub struct FastEmbedTextEmbedder {
    model: fastembed::TextEmbedding,  // owned; loaded once
    model_id: String,                  // for safe_status reporting
    output_dim: usize,                 // 1024 for BGE-large
    matryoshka_truncate_dim: Option<usize>,  // e.g. Some(256) for coarse-then-fine
    max_tokens: usize,                 // truncation cap, default 512
}

#[cfg(feature = "local-gpu-models")]
impl Embedder for FastEmbedTextEmbedder {
    fn embed(&self, plaintext: &[u8]) -> Vec<f32> { ... }
}
```

### Loading the model

```rust
pub async fn try_new(
    model_id: impl Into<String>,
    cache_dir: impl Into<PathBuf>,
    matryoshka_truncate_dim: Option<usize>,
    max_tokens: usize,
) -> anyhow::Result<Self>
```

`fastembed` downloads model files to `cache_dir` on first use (via
`hf-hub`). Subsequent runs load from disk. Construction is async to
match `CandlePerplexityScorer::try_new`'s shape so both can be built
during the gate-service binary's async startup.

Default model_id: `"BAAI/bge-large-en-v1.5"`. The fastembed crate
exposes a `EmbeddingModel::BGELargeENV15` enum variant that maps to
this; configuration can take either the enum variant or the raw
model_id string.

### `embed(plaintext)` algorithm

1. **Decode** plaintext bytes as UTF-8 (lossy; the trace is already
   redacted and tokenizer-friendly).
2. **Truncate** to `max_tokens` (default 512 — BGE's nominal context).
   Tokenizer-aware truncation, not byte-aware.
3. **Run** `model.embed([text])` and take the first row.
4. **L2-normalize** the result so cosine similarity == dot product.
5. **If `matryoshka_truncate_dim`** is set, slice to the first N dims
   and re-normalize. This is a workaround if the configured model
   doesn't expose matryoshka directly; for BGE-large it just works.
6. Return the `Vec<f32>`.

`fastembed` handles batching internally; we always send batches of
size 1 because we evaluate one trace at a time. For throughput later,
we could batch across multiple submissions in a single worker call.
Out of scope for v1.

### Failure modes

| Situation | Behavior |
|-----------|----------|
| `fastembed::TextEmbedding::try_new` fails (download / load) | Constructor returns `anyhow::Error` with class `EmbedderInit`. Gate service fails to start. |
| Plaintext is empty or zero tokens after truncation | Return a zero vector. Caller (orchestrator) will get `novelty_score = 0` from the index. Acceptable behavior — empty traces shouldn't get novelty credit. |
| GPU/CPU runtime error during embed | `anyhow::bail!("EmbedderInferenceFailed: <model_id_hash>")`. Caller fails the gate evaluation. |
| Resulting vector has wrong dim (mismatch with index config) | `anyhow::bail!("EmbedderDimMismatch: expected <N>, got <M>")`. Caller fails. |

### Hash-only logging

The model_id is operator-configured and safe to log. The plaintext and
the embedding vector never log, never serialize, never leave process
memory beyond the orchestrator → vector index handoff. The vector
index stores the embedding by `vector_entry_id`; the embedding bytes
themselves are not part of the audit row (only the
`embedding_evidence_hash`, which is `sha256(model_id || policy_version
|| quantized_embedding_bytes)`, per the original private-vector spec).

## Configuration

New env vars consumed at gate-service startup:

| Env | Default | Notes |
|-----|---------|-------|
| `TRACE_COMMONS_EMBEDDER_MODEL_ID` | `BAAI/bge-large-en-v1.5` | HuggingFace repo id |
| `TRACE_COMMONS_EMBEDDER_CACHE_DIR` | `/var/cache/tracedao-embedder` | Model file cache |
| `TRACE_COMMONS_EMBEDDER_MAX_TOKENS` | `512` | Truncation cap (must match index config) |
| `TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM` | unset | If set, truncate vectors to N dims (e.g. 256) for coarse-then-fine ranking |

The embedder's `model_id` participates in the `gate_version_hash`
canonical input — changing the embedder model produces a new gate
version, and existing credit under the old version stays per the
grandfather-settled policy.

## Build feature

Shared with A2 — both extensions are part of the same "local GPU
inference" deployment story. Adding the embedder to the existing
`local-gpu-models` feature in `crates/tracedao-gate-enclave/Cargo.toml`:

```toml
[features]
local-gpu-models = [
    # ...A2 candle deps...
    "dep:fastembed",
]

[dependencies]
fastembed = { version = "5", optional = true }
```

Pin a specific version at implementation time. The crate is at major
version 5 (5.13.4 as of late April 2026) and the release cadence has
been healthy.

**Hard dependency-policy gate:** `fastembed` is not on
`~/.claude/approved-dependencies.md`. Before implementation lands,
disclose:
- Version 5.x (latest 5.13.4, 2026-04-27)
- Apache-2.0 license
- Anush008 maintainer (well-known in Rust ML)
- ONNX-backed via `ort` (already on the approved list? Verify)
- Transitive includes `tokenizers` (HuggingFace) and `hf-hub`

Get explicit approval before `cargo add`. The dep is a sibling to the
candle deps from A2; if A2's deps are approved, this one should be
trivial.

## Memory budget

| Component (Phase A) | Footprint |
|---------------------|-----------|
| BGE-large ONNX weights | ~1.3 GB |
| Tokenizer + config | ~10 MB |
| Per-call activation memory | <100 MB |
| **Embedder subtotal** | ~1.5 GB |

Comfortably fits alongside A2's 8B perplexity model. Even with
BGE-large + Llama-3.1-8B both resident, total GPU usage is ~30 GB,
leaving plenty of H100 80GB headroom.

If a deployment swaps in `BGE-base-en-v1.5` (110M params, ~440 MB)
or `BGE-small` (~140 MB), the embedder savings are negligible at our
scale.

## Latency budget

BGE-large via ONNX on H100: ~10-30 ms per embed for 512-token input.
Trivial compared to the perplexity prefill (~1-2 s). Vector-index
query is single-digit milliseconds on a per-tenant index. Total gate
latency stays dominated by A2.

## Testing

### Unit tests (no GPU)

The mock `MockEmbedder` shipped in PR #10 stays the trait double for
orchestrator tests. No new unit tests required for the trait surface.

Add a unit test for the L2-normalize helper if we extract one: given
a fixture vector, assert `norm == 1.0` after normalization.

### Integration tests (require GPU or CPU + model)

Behind `#[ignore]` and an env-gate
(`TRACEDAO_EMBEDDER_INTEGRATION=1`). fastembed supports CPU inference
adequately for BGE-large (~500 ms per embed on a modern CPU), so the
integration test can run on a CI runner without GPU — but it's slow
enough to keep gated.

Tests:
1. **Smoke** — load the configured model, embed a fixed string, assert
   the vector is unit-normalized and has the expected dimension.
2. **Determinism** — embed the same string twice, assert exact
   equality.
3. **Similarity ordering** — embed three strings (two near-paraphrases
   and one unrelated), assert the paraphrase pair has higher cosine
   similarity than either paraphrase vs the unrelated string. This
   is the only test that actually validates the embedder's
   semantic quality.

## Migration to Phase B

Same code path moves inside the dstack-attested enclave. Model file
must be available to the enclave's filesystem (either pre-staged or
downloaded inside the enclave with HF credentials). Phase B's
attestation chain covers the embedder binary the same way it covers
the perplexity scorer.

No code changes needed for migration.

## Open questions

1. **GPU vs CPU inference for the embedder.** BGE-large on H100 GPU is
   ~10-30 ms; on H100 CPU is ~500 ms. Either is acceptable for an
   async worker route. GPU is the obvious pick when the H100 is
   already there for A2. Decision: GPU.

2. **Matryoshka truncation default.** Coarse-then-fine ranking saves
   index storage and accelerates large-corpus queries. At Phase A
   scale (small corpus), it's marginal. Decision: ship with the
   feature available but `matryoshka_truncate_dim` unset by default;
   operators enable when their corpus grows.

3. **ONNX vs candle for embedder.** fastembed uses ONNX Runtime
   (`ort` crate); candle is what A2 uses. Two runtimes in the same
   binary is fine but adds binary size. Alternative: run BGE through
   candle directly. Decision: stay with fastembed for embedder —
   ONNX is the standard format, fastembed's API is clean, swap cost
   is low if we ever consolidate.

## Cost estimate

| Item | Estimate |
|------|----------|
| Disclose `fastembed` dep, get approval | <1 day |
| `FastEmbedTextEmbedder` impl + unit tests | 1-2 days |
| Integration test harness | 1 day |
| Documentation | <1 day |
| **Total** | **~3-5 days of focused work** |

Smaller than A2 because the API surface is narrow and there's no
manual logprob math.

## What this spec does not commit to

- A specific fastembed version (pinned at implementation time)
- A specific BGE variant (operator-configurable; default
  `BAAI/bge-large-en-v1.5`)
- Multi-lingual support (English-only in v1)
- Custom fine-tuned models (use off-the-shelf in v1)
