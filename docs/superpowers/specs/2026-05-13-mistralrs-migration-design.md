# Mistralrs Migration — Design (Phase A2.3 Retrofit)

Date: 2026-05-13
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets lane
Predecessor: `2026-05-13-bakeoff-arch-dispatch-design.md` (A2.2);
`2026-05-13-model-bakeoff-retrofit-design.md` (A2.1);
`2026-05-12-perplexity-scorer-design.md` (A2)

## Motivation

A2.2 just landed: candle's per-arch loaders are wired correctly, the
Qwen3 QK-Norm silent bug is fixed, and Gemma 4 31B is now a supported
candidate. That covers three of the four candidates the bake-off was
spec'd to evaluate. The fourth — **Qwen 3.6 27B Dense** — remains
unrunnable because candle has no `qwen3_5` loader on main (verified
2026-05-13 at SHA `5447a87`), and Gated DeltaNet hybrid attention
isn't in candle's repertoire.

The bake-off question this blocks: **"is the most-recent
Apache-2.0-licensed dense model (Qwen 3.6 27B) the right production
pick, or does the smaller / older Qwen3-8B-Base win?"** Today we have
no way to measure it. The A2.1 result (Qwen3-8B-Base wins by +82% over
Llama-3.1-Instruct) leaves this question open at the high end of the
candidate set.

Two paths to unblock:

| Path | Effort | Tradeoffs |
|------|--------|-----------|
| Hand-write `qwen3_5.rs` (Gated DeltaNet) in our tree | 5-10 days | Maintenance burden, untested code for a credit-gating critical path |
| Migrate to `mistralrs` as the scorer backend | 2-3 days | Bigger dep surface; single-maintainer crate; but explicit Qwen 3.5/3.6 + Gemma 4 + future-arch support |

This retrofit picks the mistralrs migration. The A2 design originally
rejected mistralrs because its 0.8.0/0.8.1 releases failed docs.rs CI
builds — but docs.rs has strict resource constraints (~3 GB RAM, time
limits) that don't apply to our build hosts. Git-pinning to master
sidesteps the docs.rs publishing issue entirely (verified 2026-05-13:
master HEAD `2d4ba4f`, latest commit 2026-04-15, **30+ commits in the
last 30 days**, active maintenance). This is the same pattern we used
for candle in A2.2.

mistralrs explicitly supports Qwen 3.5 (the family identifier under
which 3.6 ships), Gemma 4 (full multimodal), and Qwen3 dense, and
uses candle as its underlying tensor runtime — so we're not switching
off candle, we're moving up one layer of abstraction.

## Goal

Replace `CandlePerplexityScorer` with `LocalPerplexityScorer` (mistralrs-backed) in
both the bake-off binary and the production gate-service. Enable the
full four-candidate bake-off the original A2.1 spec called for.
Generate an A2.3 result report that supersedes A2.2's (which in turn
superseded A2.1's).

## Non-goals

- **Keeping candle as a fallback backend.** No dual-backend support.
  mistralrs replaces `CandlePerplexityScorer` entirely. The `candle-*`
  deps stay in tree because they're transitive through mistralrs,
  not because we use them directly.
- **Multimodal evaluation.** Gemma 4 and Qwen 3.6 ship with vision/
  audio capabilities. Bake-off scores text only.
- **Switching the production gate's *trust model*.** The gate still
  runs in-process inside the gate-service binary; mistralrs is loaded
  as a library, not a subprocess or HTTP service.
- **A2.2 rollback.** A2.2's work (`ScorerBackend` enum, `BackendArch`,
  manifest schema additions, observability fixes) stays merged.
  Some of it gets superseded — the `ScorerBackend` enum and `BackendArch`
  largely fold into mistralrs's internal dispatch — but the work
  unblocked Gemma 4 in time and ground-truthed candle's surface.

## Decisions baked in

| Decision | Value |
|----------|-------|
| Dep source | Git-pin `mistralrs` (and `mistralrs-core` if needed) to master at a specific SHA. Move to a tagged release when one builds cleanly on docs.rs. |
| Scope of swap | Bake-off binary + production gate-service. No transitional dual-backend phase — adds complexity for no gain. |
| API path | Use mistralrs's raw-logits API (`ForwardInputsResult::RawLogits` / `return_raw_logits=true`) to compute per-token logprobs. Match the existing aggregate-perplexity computation in `aggregate_perplexity_metrics`. |
| Arch dispatch | Delegated entirely to mistralrs. Our `BackendArch` enum becomes a thin wrapper for env-var parsing (or is removed). |
| `CandidateArch` schema | Add `Qwen3_5` variant. Keep all A2.2 variants. mistralrs auto-detects from `config.json` so the manifest's `arch` field becomes informational / used only for `ctx_for` lookup. |
| Feature flag | Rename `local-gpu-models[-cuda]` → `local-mistralrs[-cuda]`? Or keep the same names? **Keep the same** — feature-flag rename is bikeshed; the names describe intent (local GPU inference) not implementation. |

## Architecture

mistralrs has a higher-level API than candle. Where candle gave us
`Llama::forward(&self, input_ids, position, &mut Cache) -> Tensor`,
mistralrs gives us something closer to:

```rust
// (Approximate — exact signatures must be verified during implementation.)
let model = TextModelBuilder::new(model_path)
    .with_dtype(DType::BF16)
    .with_device(device)
    .build()?;

let request = Request {
    messages: ...,  // or raw token ids
    return_raw_logits: true,
    sampling_params: SamplingParams::greedy(),
    ...
};

let response: ForwardInputsResult = model.forward(request).await?;
match response {
    ForwardInputsResult::RawLogits { logits } => {
        // logits: [batch=1, seq_len, vocab]
        // Per-token logprobs via log_softmax + gather of actual next tokens
    }
    _ => bail!("expected RawLogits"),
}
```

The exact API surface needs verification against mistralrs's actual
public types during implementation — this is the spec, not the
reference manual. The implementer's first task is to write a tiny
2-page proof-of-concept against the pinned mistralrs version, confirm
the API shape, then update the rest of the slice plan if the sketch
above is wrong.

### Where `MistralrsPerplexityScorer` lives

Same crate, same file location as today: `crates/tracedao-gate-enclave/
src/perplexity_candle.rs`. **Rename the file to `perplexity_local.rs`**
since "candle" no longer describes the contents. Update internal
module paths accordingly.

The struct name changes too: `CandlePerplexityScorer` →
`LocalPerplexityScorer`. (Backend-agnostic; if we ever swap backends
again, no rename needed.)

`PerplexityScorer` trait stays unchanged.

### What happens to `BackendArch` and `ScorerBackend`?

- **`BackendArch`** — removed. mistralrs auto-detects architecture
  from `config.json`. The `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` env
  var also goes away (deprecated; emit a warn-log if seen).
- **`ScorerBackend` enum** — removed. mistralrs handles per-arch
  dispatch internally.
- **`flatten_text_config` helper** — keep. mistralrs may need its
  own config-flattening, or it may handle multimodal configs natively;
  TBD during the proof-of-concept. Either way the helper is harmless.

The `CandidateArch` manifest field becomes informational (used by
`ctx_for` for context-window sizing) but is no longer required for
loading.

## Candidate set for the A2.3 bake-off

| Candidate | License | Notes |
|-----------|---------|-------|
| Llama-3.1-8B-Instruct | Llama Community | Incumbent baseline (same as A2.1/A2.2) |
| Qwen3-8B-Base | Apache-2.0 | A2.1's winner; corrected via proper backend |
| Qwen 3.6 27B Dense | Apache-2.0 | **NEW** — the question this retrofit unlocks |
| Gemma 4 31B Base | Apache-2.0 | Bigger-dense-model data point |

Full 4-way. Same SHA-pinned corpus as A2.1 (`sha256:8acb0be...`) so
all three reports (A2.1 / A2.2 / A2.3) are mutually comparable.

## Open questions

1. **mistralrs feature-set choices.** mistralrs has many optional
   features (audio, vision, paged-attn, etc.). For our use case, pick
   the *minimum* feature set that gives us text-only Qwen3 / Qwen3.5 /
   Gemma 4 / Llama. **Recommendation:** start with `["cuda"]` only;
   add features as the proof-of-concept reveals missing capabilities.

2. **Production gate behavior parity.** The Llama-3.1-8B AUC measured
   in A2.1 (0.105) was computed via candle's Llama path. Re-running
   under mistralrs may produce a slightly different number because of
   minor differences in attention computation (mistralrs may use
   flash-attention; candle didn't). The qualitative result (Llama-Instruct
   inversely calibrated) should hold; the exact number will shift.
   **Recommendation:** report both, treat the mistralrs number as the
   authoritative going forward.

3. **Build time.** mistralrs has 14 workspace members and a large
   transitive dep count. Build time on Lambda is going to be longer
   than the current candle build (~1m 04s). Estimate: 3-8 minutes for
   a release build with CUDA features.
   **Recommendation:** measure during implementation; if > 10 minutes
   on H100, consider pre-warming a cargo cache via a `cargo build`
   in a Docker layer or similar.

4. **Async/await surface change.** mistralrs is async-first; our
   current `score()` is sync. The `PerplexityScorer` trait method
   signature is sync and must stay sync. **`tokio::task::
   block_in_place` is NOT viable** because it panics in
   current-thread runtimes (the in-crate test uses
   `Runtime::new_current_thread`, and the bake-off binary's main can
   too).

   **Resolution:** `LocalPerplexityScorer` owns its own
   `tokio::runtime::Runtime` (single-thread, dedicated to mistralrs
   I/O) and calls `rt.block_on(self.async_score(...))` from the sync
   trait method. This decouples the scorer's async-ness from the
   caller's runtime flavor entirely. Memory cost: one extra Tokio
   runtime per `LocalPerplexityScorer` instance — acceptable since
   the scorer is process-singleton in both the bake-off (one per
   candidate, sequential) and the production gate (one per process).

5. **Candle dep conflict.** Our crates currently git-pin candle to
   `5447a87`. mistralrs git-master pins its own candle SHA via its
   Cargo.toml. Two pinned SHAs for the same crate is a real cargo
   resolution problem.

   **Resolution:** **drop our direct candle deps** from
   `tracedao-gate-enclave/Cargo.toml`. Let mistralrs supply candle
   transitively at whichever SHA it pins. Our code stops referencing
   `candle_transformers::models::*` directly (it talks to mistralrs);
   any incidental `candle_core::Tensor` uses can go through the
   re-export mistralrs exposes (or imported from the transitive dep
   without a direct pin).

6. **Production-gate behavior parity test.** A2.2 doesn't have a
   test that exercises `tracedao-ingest`'s scorer-init code path
   end-to-end (only the gated in-crate test in
   `perplexity_local.rs`). A2.3 should add a small unit test that
   calls the env-var-driven init helper used by `tracedao-ingest`
   with a tiny mock model path and asserts construction returns the
   expected error class (`LocalPerplexityScorerLoadFailed`), proving
   the production code path compiles and dispatches correctly.

7. **mistralrs lock-in vs candle escape hatch.** mistralrs is
   single-maintainer. Two ways to mitigate the long-tail risk:
   (a) accept the dep and plan to fork if necessary;
   (b) keep the candle-direct `perplexity_local_candle.rs` module
   alongside the new mistralrs one under a `legacy-candle-scorer`
   feature flag, removable in a future release.

   **Recommendation:** (a) — the carrying cost of two scorer impls
   isn't justified by current information. mistralrs's 30+ commits/
   month cadence makes near-term abandonment unlikely. Revisit if
   the cadence falls below 5 commits/month for two consecutive
   months.

5. **A2.2's `Gemma3` backend.** A2.2 added Gemma 3 support to the
   candle `ScorerBackend` enum, even though no current candidate is
   Gemma 3 (the spec'd candidates jumped Gemma 3 in favor of Gemma 4).
   In A2.3, mistralrs probably supports Gemma 3 too — but we don't
   need it. **Recommendation:** drop it. The candidate-arch enum
   keeps the variant for forward-compat, but no dispatch code is
   needed.

## Deliverables

1. **A2.3a — proof-of-concept** (~1 day). A tiny standalone binary
   under `crates/tracedao-gate-enclave/examples/` that loads a model
   via mistralrs and computes one logprob. Validates the API shape +
   build success on Lambda H100. **Not** a commit on main — operator
   activity that informs A2.3b.
2. **A2.3b — code retrofit** (~2 days). Replace `CandlePerplexityScorer`
   with `LocalPerplexityScorer` backed by mistralrs. PR with 4-5
   commits matching the slice structure in the plan.
3. **A2.3c — 4-way bake-off run** (operator activity). ~5 hours GPU,
   ~$25 Lambda time. Same SHA-pinned corpus as A2.1/A2.2.
4. **A2.3d — result report.** `docs/superpowers/reports/YYYY-MM-DD-
   model-bakeoff-result-a23.{json,md,notes.md}`. Supersedes A2.1's
   and A2.2's.
5. **A2.3e — env-var defaults flip** (one-line PR). Set
   `TRACE_COMMONS_PERPLEXITY_MODEL_ID` default to the A2.3 winner.
6. **A2.3f — Phase 1 floor recalibration** against the winner.
   ~3-4 hr GPU.
7. **A2.3g — final smoke** on the chosen model.

A2.3 supersedes A2.2's and A2.1's pending rollout steps. A2.1's and
A2.2's reports stay committed as history.

## Trade-offs explicitly accepted

- **A2.2's `ScorerBackend` enum gets deleted.** Roughly 250 lines of
  the 535-line A2.2 retrofit go away. We knew this when we did A2.2 —
  it was necessary as a ground-truth pass before committing to a
  bigger refactor. Not lost work; just superseded.
- **mistralrs is single-maintainer (EricLBuehler).** Risk of
  abandonment. Mitigation: git-pinning means we control upgrades; if
  the project goes dormant, we can fork. The 30+ commits/month
  cadence suggests this is low-risk near-term.
- **Build time grows.** Acceptable given that bake-off and production
  start are both infrequent operations.
- **Bigger dep surface.** mistralrs's transitive crate count is large
  (~80+ vs candle's ~40). Cargo audits may flag more advisories;
  triage burden goes up modestly.
- **The A2.1 AUC numbers stop being directly comparable** to A2.3
  numbers because of subtle attention-path differences (flash-attn,
  cache layout, etc.). The A2.3 report notes this; downstream
  consumers should rely on A2.3 numbers as authoritative.

## Out of scope (recorded so we don't accidentally re-open it)

- Dual-backend support (candle + mistralrs in parallel)
- Switching off candle entirely (it stays as a transitive dep)
- Multimodal evaluation
- Replacing the gate service's HTTP boundary or trust model
- Phase B / dstack work — orthogonal; A2.3 carries forward
- Hand-writing Qwen 3.5 in candle — explicit non-goal, the whole
  point of A2.3 is to avoid this
