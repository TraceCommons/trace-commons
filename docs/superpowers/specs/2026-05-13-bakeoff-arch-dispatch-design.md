# Bake-off Arch Dispatch + Gemma 4 — Design (Phase A2.2 Retrofit)

Date: 2026-05-13
Status: Draft (pre-implementation)
Owner: Trace Commons / Datasets lane
Predecessor: `2026-05-13-model-bakeoff-retrofit-design.md` (A2.1); `2026-05-12-perplexity-scorer-design.md` (A2)
Driver: `2026-05-13-model-bakeoff-result-notes.md` (A2.1 run)

## Motivation

The A2.1 bake-off run on 2026-05-13 surfaced two issues that this
retrofit addresses, plus one opportunity:

1. **`CandlePerplexityScorer` is hardcoded to candle's Llama loader.**
   `crates/trace-commons-gate-enclave/src/perplexity_candle.rs:142` imports
   only `candle_transformers::models::llama::{Cache, Config, Llama,
   LlamaConfig}` and uses them for every candidate regardless of the
   `CandidateArch` value in the manifest. `CandidateArch` exists in the
   manifest schema but is only consumed by `ctx_for(arch) -> 4096`.
2. **Qwen3 dense was loaded via the Llama path in the A2.1 run.** Qwen3
   is mostly Llama-compatible (RoPE, SwiGLU, RMSNorm, GQA, no
   QKV-bias), but adds **QK-Norm** in attention which the Llama loader
   silently ignores. Qwen3-8B-Base's A2.1 AUC of 0.720 was computed
   with mathematically incomplete attention. The decisive
   base-vs-instruct conclusion is robust (the +0.615 AUC gap dwarfs
   QK-Norm noise) but the absolute number has an asterisk.
3. **Candle main branch now has `gemma4.rs`** (verified 2026-05-13;
   not yet in a tagged release). Bumping the dep gets us Gemma 4 31B
   support — the candidate that A2.1 couldn't load. This is a
   bigger-dense-model data point that A2.1 lacked.

## Goal

Add architecture-dispatched model loading to `CandlePerplexityScorer`,
fix the Qwen3 QK-Norm silent bug, add Gemma 4 31B to the supported
candidate set, and re-run the bake-off as a 3-way (Llama-3.1-8B-
Instruct vs Qwen3-8B-Base **with the proper Qwen3 loader** vs
Gemma 4 31B). Generate a corrected result that supersedes the A2.1
report on the points that depended on the Qwen3 attention bug.

## Non-goals

- **Qwen 3.6 27B Dense support.** Candle still has no `qwen3_5.rs`
  module on either main or any release. Hand-writing Gated DeltaNet is
  multi-week work for a credit-gating critical path and isn't
  justified by current information — Qwen3-8B-Base already won A2.1
  by +82%, and the question of "does the 27B model beat the 8B" can
  wait until pilot data tells us calibration needs the extra capacity.
  Parked for A2.3+ (or for mistralrs once its build-health recovers).
- **Switching off candle.** `mistralrs` is still single-maintainer
  with build failures on 0.8.0+; the A2 design rejected it on those
  grounds and the situation hasn't changed.
- **Python sidecar.** Doubles ops surface for the bake-off; not worth
  it for one missing architecture.
- **Multimodal evaluation.** Gemma 4 and Qwen 3.6 ship with vision /
  audio capabilities. The bake-off only scores text traces, so
  multimodal inputs are out of scope. We just need to load the *text*
  half of the model — which means flattening the `text_config: {...}`
  nesting in the config.json before deserialization.

## Decisions baked in

| Decision | Value |
|----------|-------|
| Candle dep | Bump from `0.10.2` to git-pin against a `main` commit that includes `gemma4.rs`. Move back to a tagged release (0.11.x) when one lands. |
| Arch dispatch | New `ScorerBackend` enum inside `perplexity_candle.rs`: `LlamaBackend`, `Qwen3Backend`, `Gemma3Backend`, `Gemma4Backend`. Each owns its own `Cache`, `Config`, and `Model` types from the relevant `candle_transformers::models::*` module. |
| Crate boundary | Add a small `BackendArch { Llama, Qwen3, Gemma3, Gemma4 }` enum **inside `trace-commons-gate-enclave`** (the scorer's home crate). `CandidateArch` lives in `trace-commons-server`'s bin module and **must not** become a dep of `trace-commons-gate-enclave` — the dep direction is enclave ← server, not the reverse. Callers in `trace-commons-server` convert their `CandidateArch` to `BackendArch` at the `try_new` call site. |
| `CandidateArch` schema | Add `Qwen3` and `Gemma4` variants. Keep `Qwen2` as an alias for `Qwen3` (deprecated; emit a warning). Keep `Llama` and `Gemma3`. |
| Config flattening | Pre-process pass on the raw `config.json` bytes: if `text_config` exists at top level, merge its keys up. Lives in a single helper, applied uniformly across arches. |
| Bake-off re-run | Operator activity. Same corpus as A2.1 (already SHA-pinned); new manifest with 3 candidates (Llama-3.1-8B-Instruct, Qwen3-8B-Base, Gemma-4-31B-Base). |

## Architecture

The candle backend gains a per-arch dispatch:

```text
                +-----------------------+
                | CandlePerplexityScorer|
                +-----------+-----------+
                            |
                            v
            +---------------+---------------+
            |   ScorerBackend (per arch)   |
            +---+--------+--------+--------+
                v        v        v        v
              Llama    Qwen3   Gemma3   Gemma4
              backend  backend backend  backend
```

Each `ScorerBackend` variant carries:
- The loaded model object (correctly-typed for that architecture)
- The arch-specific `Config`
- An arch-specific `Cache` (KV cache structure differs between
  Llama-style and Gated DeltaNet-style; for our supported set they
  are KV-cache-only and similar in shape, but typed to the model)
- A common `forward(input_ids, cache) -> Tensor` interface that
  produces the same per-position logits the existing aggregation
  helper expects

The public `CandlePerplexityScorer` interface stays unchanged —
`score(&self, plaintext: &[u8]) -> Result<PerplexityResult>` is the
only externally-visible method. The dispatch is private.

### Why an enum and not a trait?

Candle's `Model` types don't share a trait. They each expose their own
`forward(input_ids, &mut Cache)` (or similar) method on different
concrete types. Wrapping them in a trait would either require
boilerplate `Box<dyn>` indirection or generic monomorphization that
makes the constructor signature awkward. The enum dispatch matches the
shape of candle's API and keeps the dispatch logic in one place.

## Config-flattening

`config.json` schemas to handle:

| Candidate | Top-level layout |
|-----------|------------------|
| Llama-3.1-8B-Instruct | flat: `{ "model_type": "llama", "hidden_size": 4096, ... }` |
| Qwen3-8B-Base | flat: `{ "model_type": "qwen3", "hidden_size": 4096, "head_dim": 128, ... }` |
| Qwen 3.6 27B Dense (n/a in this retrofit) | nested: `{ "model_type": "qwen3_5", "text_config": { "hidden_size": 5120, ... }, "vision_config": { ... } }` |
| Gemma 4 31B Base | nested: `{ "model_type": "gemma4", "text_config": { "hidden_size": ..., ... }, "vision_config": { ... }, "audio_config": { ... } }` |

The flattener:

```rust
fn flatten_text_config(raw: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut value: serde_json::Value = serde_json::from_slice(raw)?;
    if let Some(text) = value.get("text_config").cloned() {
        let map = value.as_object_mut().context("config.json must be an object")?;
        if let serde_json::Value::Object(text_map) = text {
            for (k, v) in text_map { map.entry(k).or_insert(v); }
        }
    }
    Ok(serde_json::to_vec(&value)?)
}
```

Preserves any top-level field present in both (top-level wins),
flattens fields only in `text_config` up to top-level. The vision/
audio sibling configs are left in place and ignored (the candle
loader doesn't read them).

## QK-Norm fix for Qwen3-8B-Base

The current bug is in our backend, not in candle. Candle's
`models::qwen3::Qwen3` includes QK-Norm in its forward path; we
weren't using it. Switching the Qwen3 candidate to `Qwen3Backend` (vs
`LlamaBackend`) auto-fixes this.

Side effect: the corrected Qwen3 AUC may shift relative to A2.1's
0.720. The direction is unpredictable — proper QK-Norm could improve
calibration (higher AUC) or hurt it slightly if the wrong-attention
path happened to be lucky. The bake-off re-run will measure.

## Candidate set for the re-run

3 candidates fit on H100 80GB:

| Candidate | Backend | License | Notes |
|-----------|---------|---------|-------|
| Llama-3.1-8B-Instruct | `LlamaBackend` | Llama Community | Incumbent baseline (same as A2.1) |
| Qwen3-8B-Base | `Qwen3Backend` | Apache-2.0 | Properly loaded this time |
| Gemma 4 31B Base | `Gemma4Backend` | Apache-2.0 | New entrant; bigger-dense-model data point |

Notable: we should use the **base** Gemma 4, not the instruct-tuned
variant, consistent with A2.1's finding that instruct-tuning distorts
perplexity calibration. The HF model id is `google/gemma-4-31b`
(unmarked = base) or whichever exact id ships with `architectures:
["Gemma4ForCausalLM"]`.

## Open questions

1. **Candle git-pin vs wait-for-0.11.** Pinning to a `main` commit is
   awkward (Cargo.toml carries a `rev` rather than a version) and we
   inherit any breakage that lands between pin and a stable release.
   Waiting for 0.11.x to land could be days or months — no signal
   either way.
   **Recommended:** git-pin now. The commit that adds `gemma4.rs`
   is small and we control the upgrade window. Move to a tagged 0.11
   release when one lands.

2. **Should we re-run with the same corpus?** The A2.1 corpus is
   SHA-pinned (`sha256:8acb0be339b2da278986c389700884b23a92dafc85e41c
   6549d963b550938660`) so the new run is directly comparable.
   **Recommended:** yes; same corpus. The point of the retrofit is to
   correctly-load three candidates against an apples-to-apples
   distribution, not to also vary the corpus.

3. **Backwards compatibility on `CandidateArch::Qwen2`.** A few people
   may have already written manifests with `arch = "qwen2"`. Renaming
   the variant cleanly is preferred; supporting it as a soft alias
   with a warning is the migration-friendly option.
   **Recommended:** keep `Qwen2` as an alias that resolves to
   `Qwen3Backend`, emit `tracing::warn!(deprecated_arch = "qwen2",
   "use qwen3")`. Drop the alias in 6 months.

4. **Does `Gemma 4 31B Base` actually exist or only the instruct
   variant?** Checking HF: `google/gemma-4-31b` exists (confirmed
   downloaded in A2.1) and its config.json has `model_type: gemma4`.
   Need to verify it's the base model not the instruct one.
   **Action:** verify before the re-run; if only the instruct
   variant is published, use it but note in the report that Gemma 4
   could not be evaluated as a base model.

5. **What if the corrected Qwen3 AUC drops below 0.5?** Possible if
   the no-QK-Norm path was structurally biased toward Qwen3.
   **Recommended:** that's the new ground truth. Report it honestly.
   The base-vs-instruct hypothesis would weaken but A2.1's Llama-3.1
   AUC of 0.105 is so far below 0.5 that the comparison still
   favors Qwen3 / base models, just less decisively.

## Deliverables

1. **A2.2a — code.** PR landing the arch dispatch + config flattening
   + Qwen3 backend wiring + Gemma 4 backend wiring. Includes tests for
   config-flatten and arch-dispatch (the per-backend forward path is
   exercised manually by the operator on a GPU host).
2. **A2.2b — bake-off re-run** (operator activity). 3-candidate run
   on Lambda H100 against the A2.1-pinned corpus. ~3 hr GPU, ~$15.
3. **A2.2c — report.** Comparable JSON + markdown report under
   `docs/superpowers/reports/`. If Qwen3 still wins, note the
   corrected AUC alongside A2.1's. If Gemma 4 wins, that's the new
   recommendation.
4. **A2.2d — env-var flip** (one-line PR). Update
   `TRACE_COMMONS_PERPLEXITY_MODEL_ID` default to whichever wins.
5. **A2.2e — Phase 1 floor recalibration** against the winning
   model. ~3-4 hr GPU, ~$15.
6. **A2.2f — final smoke** on the chosen model.

This retrofit supersedes A2.1's pending rollout (A2.1c/d/e). A2.1's
report stays committed as history but is no longer the basis for the
env-var defaults.

## Trade-offs accepted

- The Qwen 3.6 27B candidate stays unrunnable until candle gets
  `qwen3_5.rs` or we switch backends. Document and move on.
- Pinning candle to a git commit (not a tagged release) introduces a
  small reproducibility cost — `Cargo.lock` records the commit hash
  but the upstream branch can be rewritten or deleted. Mitigation:
  vendor the candle source under `vendor/` if upstream churn becomes
  a problem.
- The retrofit invalidates A2.1's Qwen3-8B AUC of 0.720 as a precise
  measurement. The 2026-05-13 result notes will need a one-line
  superseded-by reference. The qualitative conclusion (base beats
  instruct; Qwen3-8B is the better incumbent candidate) survives.

## Estimated effort

- A2.2a: ~1-2 days code (one engineer, fresh on candle's per-arch
  loader API)
- A2.2b: ~3 hr GPU + 1 hr operator setup
- A2.2c–f: each ~1-2 hours

Total elapsed: ~3 days. Total cost: ~$30 (Lambda) + engineer time.
