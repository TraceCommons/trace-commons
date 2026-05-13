# Bake-off Arch Dispatch + Gemma 4 Implementation Plan (A2.2 Retrofit)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add architecture-dispatched model loading to `CandlePerplexityScorer`, fix the Qwen3 QK-Norm silent bug discovered in the A2.1 bake-off, add Gemma 4 31B support, and prep for an A2.2 re-run.

**Architecture:** A new `ScorerBackend` enum inside `crates/tracedao-gate-enclave/src/perplexity_candle.rs` carries one of {`LlamaBackend`, `Qwen3Backend`, `Gemma3Backend`, `Gemma4Backend`}. Each holds the correctly-typed candle model + config + KV cache. A small `flatten_text_config` helper pre-processes multimodal `config.json` files. The public `CandlePerplexityScorer::score` interface is unchanged.

A second local enum `BackendArch { Llama, Qwen3, Gemma3, Gemma4 }` lives in `tracedao-gate-enclave` and is the `try_new` parameter — **not** `CandidateArch`. The dep direction is enclave ← server (the manifest in `tracedao-server` calls into the scorer in `tracedao-gate-enclave`), so the scorer must not depend on the manifest's enum. Server-side callers translate `CandidateArch → BackendArch` at the call site via a small inline `match`.

**Tech Stack:** Rust, `candle-core` / `candle-nn` / `candle-transformers` git-pinned against a `main` commit that includes `gemma4.rs` (the 0.10.2 tagged release pre-dates Gemma 4 support).

**Spec:** `docs/superpowers/specs/2026-05-13-bakeoff-arch-dispatch-design.md`

---

## File Map

**Modified files**

| Path | What changes |
|------|--------------|
| `crates/tracedao-gate-enclave/Cargo.toml` | Bump candle deps from `0.10.2` to a git-pinned commit on `main`. Pin in `Cargo.lock`. |
| `crates/tracedao-gate-enclave/src/perplexity_candle.rs` | Refactor: hardcoded Llama loader → `ScorerBackend` enum with arch dispatch. Add `BackendArch` enum + `flatten_text_config` helper + inline tests. ~250 lines of touched code. |
| `crates/tracedao-server/src/bin/gate_calibrate/bakeoff_manifest.rs` | Add `Qwen3` and `Gemma4` variants to `CandidateArch`; keep `Qwen2` as deprecated alias for `Qwen3` with a `tracing::warn!` on parse. |
| `crates/tracedao-server/src/bin/gate_calibrate/run_candidate_eval.rs` | Update `ctx_for` to cover new variants; add `CandidateArch → BackendArch` translation at scorer construction. |
| `crates/tracedao-server/src/bin/tracedao-gate-calibrate.rs` | Update **two** call sites of `CandlePerplexityScorer::try_new`: `Cmd::BakeOff` (passes per-candidate arch) and `Cmd::Calibrate` (reads `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` env, default `"llama"`). |
| `crates/tracedao-server/src/bin/tracedao-ingest.rs:~4274` | Production gate-service scorer wiring. Update the `try_new` call to pass a `BackendArch` derived from `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` (same env var as calibrate). |
| `crates/tracedao-server/tests/bakeoff_manifest.rs` | New tests: `parses_qwen3_arch`, `parses_gemma4_arch`, `qwen2_alias_warns_and_resolves_to_qwen3`. |
| In-crate test at `crates/tracedao-gate-enclave/src/perplexity_candle.rs:~577` | Existing `try_new` invocation in the test must pass a `BackendArch` (use `BackendArch::Llama` to keep test semantics). |
| `docs/operator/calibration.md` | Update Phase 0 candidate-manifest example to use `arch = "qwen3"` and add `arch = "gemma3"` / `"gemma4"` rows. Pin Gemma 4 base-vs-instruct guidance to a config-shape check (operator runs `python3 -c "import json; print(json.load(open('config.json'))['architectures'])"` and verifies the result is `["Gemma4ForCausalLM"]` not the instruct variant). |
| `docs/operator/env-reference.md` | Add `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` entry (default `llama`, accepted: `llama`/`qwen3`/`gemma3`/`gemma4`/`qwen2` deprecated). |
| `docs/trace-commons-roadmap.md` | One-line addition under Phase A: A2.2 retrofit (status pending → done after re-run). |

**No new top-level files.** `flatten_text_config` tests live inline in `perplexity_candle.rs` under `#[cfg(test)] mod tests` rather than a separate `tests/config_flatten.rs` — the helper is `pub(crate)` and inline tests keep the surface tight.

**Out of scope (do not touch)**

- The bake-off binary's CLI surface or `run_bakeoff` orchestration (PR #34 already covers observability + load-failure resilience).
- The bake-off metric module (`bakeoff_metrics.rs`) — pure math, unaffected.
- Qwen 3.6 / `qwen3_5` support — explicit non-goal in the spec.
- Multimodal evaluation (vision/audio). Text only.

---

## Pre-flight

- [ ] **Confirm green baseline.**

```bash
cargo check -p tracedao-server --bins
cargo check -p tracedao-server --bins --features local-gpu-models
cargo test -p tracedao-server --test bakeoff_manifest --test bakeoff_subcommand
```

Expected: clean. If anything fails, stop and fix before starting.

- [ ] **Read the spec.**

```bash
$EDITOR docs/superpowers/specs/2026-05-13-bakeoff-arch-dispatch-design.md
```

You need to internalize the QK-Norm finding, the arch-dispatch design, and the candidate set before writing code. The decisive thing is that **each arch needs its own candle module** — `Llama`, `Qwen3`, `Gemma3`, `Gemma4` are not interchangeable.

- [ ] **Pick the candle git-pin.**

**Verify first that `gemma4.rs` is actually present on candle's main
branch** at the commit you intend to pin. This was webfetch'd as
present on 2026-05-13 during the design phase, but verify directly
before committing:

```bash
git ls-remote https://github.com/huggingface/candle.git main | head -1
# Clone shallow, check the file exists at that SHA:
git clone --depth 1 https://github.com/huggingface/candle.git /tmp/candle-check
ls /tmp/candle-check/candle-transformers/src/models/gemma4.rs
# Also sanity-check qwen3.rs is there (we use it as the corrected Qwen3 path):
ls /tmp/candle-check/candle-transformers/src/models/qwen3.rs
rm -rf /tmp/candle-check
```

If either file is missing on main, **stop and surface to the operator.**
Options: pick an older commit, wait for upstream, or scope the
retrofit to whichever arch is present.

If both are present, record the SHA + date — they go in the Cargo.toml
edits in Slice 2 and in the resulting commit message.

The pin lands in Cargo.toml as:

```toml
candle-core = { git = "https://github.com/huggingface/candle.git", rev = "<sha>", optional = true }
candle-nn   = { git = "https://github.com/huggingface/candle.git", rev = "<sha>", optional = true }
candle-transformers = { git = "https://github.com/huggingface/candle.git", rev = "<sha>", optional = true }
```

---

## Slice 1 — Config flatten helper

The cheapest piece. Land first so subsequent slices can rely on it.

### Task 1: `flatten_text_config` helper + inline unit tests

**Files:**
- Modify: `crates/tracedao-gate-enclave/src/perplexity_candle.rs` (add helper at top of `candle_impl` mod, and inline `#[cfg(test)] mod tests`)

The helper is `pub(crate)` and tests live inline because the
`candle_impl` module is feature-gated and not visible across the crate
boundary. No new test files needed.

In `crates/tracedao-gate-enclave/src/perplexity_candle.rs`, near the
top of the `candle_impl` mod:

```rust
fn flatten_text_config(raw: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut value: serde_json::Value = serde_json::from_slice(raw)
        .context("config.json must be valid JSON")?;
    let text = value.get("text_config").cloned();
    if let Some(serde_json::Value::Object(text_map)) = text {
        let map = value.as_object_mut()
            .context("config.json must be an object")?;
        for (k, v) in text_map {
            map.entry(k).or_insert(v);
        }
    }
    Ok(serde_json::to_vec(&value)?)
}

#[cfg(test)]
mod tests {
    use super::flatten_text_config;
    use serde_json::json;

    #[test]
    fn flat_config_passes_through_unchanged() {
        let raw = json!({"model_type": "llama", "hidden_size": 4096}).to_string();
        let out = flatten_text_config(raw.as_bytes()).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["hidden_size"], 4096);
        assert!(parsed.get("text_config").is_none());
    }

    #[test]
    fn nested_text_config_is_flattened_to_top_level() {
        let raw = json!({
            "model_type": "gemma4",
            "text_config": {"hidden_size": 5120, "num_attention_heads": 40},
            "vision_config": {"hidden_size": 1024}
        }).to_string();
        let out = flatten_text_config(raw.as_bytes()).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["hidden_size"], 5120, "text hidden_size should be lifted");
        assert_eq!(parsed["num_attention_heads"], 40);
        // vision_config should remain — we just ignore it later.
        assert!(parsed.get("vision_config").is_some());
        // text_config also remains; we merged keys but didn't strip the source.
        assert!(parsed.get("text_config").is_some());
    }

    #[test]
    fn top_level_wins_over_text_config_collision() {
        let raw = json!({
            "model_type": "gemma4",
            "hidden_size": 9999,
            "text_config": {"hidden_size": 5120}
        }).to_string();
        let out = flatten_text_config(raw.as_bytes()).unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&out).unwrap();
        assert_eq!(parsed["hidden_size"], 9999, "explicit top-level wins");
    }

    #[test]
    fn invalid_json_errors_clearly() {
        let err = flatten_text_config(b"{not json").unwrap_err();
        assert!(err.to_string().contains("valid JSON"));
    }
}
```

- [ ] **Step 2: Run tests, confirm they fail (function doesn't exist yet).**

```bash
cargo test -p tracedao-gate-enclave --features local-gpu-models 2>&1 | tail -10
```

- [ ] **Step 3: Implement `flatten_text_config` (per the snippet above).**

- [ ] **Step 4: Run tests, confirm pass (4/4).**

- [ ] **Step 5: Commit.**

```bash
git add crates/tracedao-gate-enclave/src/perplexity_candle.rs
git commit -m "Add flatten_text_config helper for multimodal candle configs"
```

---

## Slice 2 — Candle git-pin

Land the dep bump separately so its diff is reviewable on its own.
The arch dispatch in Slice 3 will rely on the bumped `gemma4` module.

### Task 2: Bump candle to a `main` commit

**Files:**
- Modify: `crates/tracedao-gate-enclave/Cargo.toml`
- Modify: `Cargo.lock` (regenerated by `cargo update`)

- [ ] **Step 1: Pick a candle commit.**

The commit must (a) include `candle-transformers/src/models/gemma4.rs`
and (b) build clean. Cross-reference candle's GitHub for the commit
that introduced gemma4 support; typically the merge commit for the
gemma4 PR. Record the SHA in your notes — it goes in the commit
message.

- [ ] **Step 2: Update `crates/tracedao-gate-enclave/Cargo.toml`.**

Replace the three lines:

```toml
candle-core = { version = "0.10.2", optional = true }
candle-nn = { version = "0.10.2", optional = true }
candle-transformers = { version = "0.10.2", optional = true }
```

with:

```toml
candle-core = { git = "https://github.com/huggingface/candle.git", rev = "<sha>", optional = true }
candle-nn = { git = "https://github.com/huggingface/candle.git", rev = "<sha>", optional = true }
candle-transformers = { git = "https://github.com/huggingface/candle.git", rev = "<sha>", optional = true }
```

If `tracedao-server`'s Cargo.toml also pins specific candle versions
(propagated through the `local-gpu-models` feature), update those too.

- [ ] **Step 3: `cargo update -p candle-core -p candle-nn -p candle-transformers`** to regenerate Cargo.lock.

- [ ] **Step 4: Confirm default + featured builds compile.**

```bash
cargo check -p tracedao-server --bins
cargo check -p tracedao-server --bins --features local-gpu-models
```

The featured build must still compile against the bumped candle even
though we haven't added arch dispatch yet — the existing Llama-only
backend should still work post-bump.

- [ ] **Step 5: Commit.**

```bash
git add crates/tracedao-gate-enclave/Cargo.toml Cargo.lock
git commit -m "Bump candle to <sha> on main (includes gemma4 support)"
```

The commit message body must record:
- The chosen SHA and its date
- A one-line "what's new" (gemma4 module + any other notable model
  additions)
- The reason for git-pinning (no 0.11.x release yet)

---

## Slice 2.5 — Audit candle's per-arch public symbols

Before refactoring, confirm the actual exported types in each candle
module. The plan's sketches below use placeholder names (`Cache`,
`Config`, `Model`); the real exports may be different (e.g.,
`Gemma3ForCausalLM`, `Gemma4ForCausalLM`, no `Cache` type for stateless
forwards). Get this right before writing the refactor.

### Task 2.5: Verify candle symbols

**Files:** none (investigation only)

- [ ] **Step 1: For each of `llama`, `qwen3`, `gemma3`, `gemma4`,
  inspect the module's public surface** at the candle commit you
  pinned in Slice 2.

```sh
# Easiest: cargo doc --open -p candle-transformers and look at each
# module's index. Or read the source directly:
find $CARGO_HOME/git/checkouts/candle-* -name 'llama.rs' -path '*models*' | head
```

  Record for each arch:
  - The model type name (e.g., `Llama`, `Qwen3ForCausalLM`,
    `Gemma3Model`)
  - The config type name (e.g., `LlamaConfig`, `Config`)
  - Whether there's a per-arch `Cache` type or whether forward takes
    `&mut KvCache` from `candle_nn` or similar
  - The `forward` signature — argument order, return type

  These names go into the Slice 3 `ScorerBackend` enum directly.

- [ ] **Step 2: Document findings in the Slice 3 commit message** so
  future maintainers know which candle types are pinned.

No commit for this task — it's a reading pass.

---

## Slice 3 — Arch-dispatched `ScorerBackend`

The load-bearing slice. Refactor the backend, wire Qwen3 + Gemma4
properly, keep Llama working unchanged.

### Task 3: Introduce the `BackendArch` + `ScorerBackend` enums + arch dispatch

**Files:**
- Modify: `crates/tracedao-gate-enclave/src/perplexity_candle.rs`

**Structure (the goal shape — verify exact type names per Slice 2.5):**

```rust
#[cfg(feature = "local-gpu-models")]
mod candle_impl {
    use candle_core::{DType, Device, Tensor};
    use candle_nn::VarBuilder;

    /// Locally-owned arch enum. Distinct from `tracedao-server`'s
    /// `CandidateArch` because gate-enclave must not depend on the
    /// server bin crate. Server-side callers convert at the boundary.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum BackendArch { Llama, Qwen3, Gemma3, Gemma4 }

    // Per-arch loader imports — placeholder names; verify in Slice 2.5
    // against the candle commit pinned in Slice 2.
    use candle_transformers::models::llama::{/* Llama, LlamaConfig, Cache, ... */};
    use candle_transformers::models::qwen3::{/* Qwen3*, ... */};
    use candle_transformers::models::gemma3::{/* ... */};
    use candle_transformers::models::gemma4::{/* ... */};

    enum ScorerBackend {
        Llama   { /* per-arch state: model, config, cache (or KvCache from candle_nn) */ },
        Qwen3   { /* per-arch state */ },
        Gemma3  { /* per-arch state */ },
        Gemma4  { /* per-arch state */ },
    }

    impl ScorerBackend {
        async fn try_new(
            arch: BackendArch,                  // local enum, NOT CandidateArch
            model_path: &Path,
            device: &Device,
            dtype: DType,
        ) -> anyhow::Result<Self> {
            let raw_cfg = std::fs::read(model_path.join("config.json"))
                .context("config.json missing")?;
            let cfg_bytes = flatten_text_config(&raw_cfg)?;
            match arch {
                BackendArch::Llama => {
                    // ... existing Llama load logic, unchanged
                    Ok(Self::Llama { /* ... */ })
                }
                BackendArch::Qwen3 => {
                    // ... candle_transformers::models::qwen3 loader
                    Ok(Self::Qwen3 { /* ... */ })
                }
                BackendArch::Gemma3 => { /* ... */ }
                BackendArch::Gemma4 => { /* ... */ }
            }
        }

        fn forward(&self, input_ids: &Tensor, position: usize) -> anyhow::Result<Tensor> {
            match self {
                Self::Llama  { model, cache, .. } => {
                    let mut c = cache.lock().unwrap();
                    // candle takes &mut Cache; deref the MutexGuard.
                    Ok(model.forward(input_ids, position, &mut *c)?)
                }
                Self::Qwen3  { model, cache, .. } => {
                    let mut c = cache.lock().unwrap();
                    Ok(model.forward(input_ids, position, &mut *c)?)
                }
                Self::Gemma3 { /* ... */ } => { /* ... */ }
                Self::Gemma4 { /* ... */ } => { /* ... */ }
            }
        }

        fn reset_cache(&self) -> anyhow::Result<()> {
            // Per-arch cache rebuild. If some arches have stateless forward
            // (no Cache type), this is a no-op for those variants.
        }
    }

    pub struct CandlePerplexityScorer {
        backend: ScorerBackend,
        tokenizer: Tokenizer,
        device: Device,
        dtype: DType,
        tail_logprob_cutoff: f32,
        model_id: String,
        max_tokens: usize,
    }

    impl CandlePerplexityScorer {
        pub async fn try_new(
            model_id: impl Into<String>,
            model_path: impl AsRef<Path>,
            device: CandleDeviceKind,
            tail_logprob_cutoff: f32,
            max_tokens: usize,
            arch: BackendArch,           // NEW parameter — local enum
        ) -> anyhow::Result<Self> {
            // ... same shape as today; pass `arch` to ScorerBackend::try_new
        }
    }
}
```

**Note on `&mut *c`:** `cache.lock().unwrap()` returns a
`MutexGuard<Cache>`. Candle's `forward` takes `&mut Cache`, so the
call site needs `&mut *c`, not `&mut c`. The plan's example reflects
this; don't accidentally drop the `*` when typing it in.

**Important:** `CandlePerplexityScorer::try_new` gains an
`arch: BackendArch` parameter. **Three** in-tree callers and **one**
in-crate test need updates:

1. `crates/tracedao-server/src/bin/tracedao-gate-calibrate.rs`
   `run_bakeoff` (`Cmd::BakeOff`) — convert per-candidate
   `CandidateArch` to `BackendArch` (small inline `match`).
2. `crates/tracedao-server/src/bin/tracedao-gate-calibrate.rs`
   `run_calibrate` (`Cmd::Calibrate`) — read
   `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` (default `"llama"`),
   parse into `BackendArch`.
3. `crates/tracedao-server/src/bin/tracedao-ingest.rs` (~line 4274)
   — production gate-service scorer wiring. Same env var as #2;
   default `"llama"` for back-compat. **This is the production
   path; treat carefully.**
4. In-crate test at
   `crates/tracedao-gate-enclave/src/perplexity_candle.rs:~577` —
   pass `BackendArch::Llama` to preserve test semantics.

Add a small `impl BackendArch { pub fn parse(s: &str) -> Result<Self> }` for env-var parsing. Accepted strings: `"llama"`, `"qwen3"`, `"gemma3"`, `"gemma4"`. Unknown strings return `Err`.

- [ ] **Step 1: Refactor `CandlePerplexityScorer` to hold `ScorerBackend`.**

  Lift each piece of arch-specific code into the matching enum variant.
  Llama's existing logic moves into `Self::Llama { ... }`; the other
  three variants are new.

- [ ] **Step 2: Implement each variant's `try_new`, `forward`, and
  `reset_cache` arms.**

  Reference candle's example binaries for each arch
  (`candle-examples/examples/{llama,qwen3,gemma3,gemma4}`) to confirm
  the constructor + forward signatures match.

- [ ] **Step 3: Pipe `arch: CandidateArch` through
  `CandlePerplexityScorer::try_new`.**

- [ ] **Step 4: Update all four caller sites.**

  - `tracedao-gate-calibrate.rs` `run_bakeoff`: inline-match
    `c.arch: CandidateArch` → `BackendArch`. The `CandidateArch::Qwen2`
    deprecated alias maps to `BackendArch::Qwen3`.
  - `tracedao-gate-calibrate.rs` `run_calibrate`: read
    `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` env (default `"llama"`),
    parse via `BackendArch::parse`.
  - `tracedao-ingest.rs` (~line 4274): read the same env var with the
    same default and parser; this is the production gate-service
    scorer load. Don't bury the env-var read in a deep helper —
    surface it next to the other startup config so an operator can
    grep for `MODEL_ARCH`.
  - `perplexity_candle.rs:~577` in-crate test: pass `BackendArch::Llama`.

  Confirm via grep:
  ```sh
  grep -rn "CandlePerplexityScorer::try_new\|CandlePerplexityScorer::new" --include="*.rs" .
  ```
  Every match must compile after this slice.

- [ ] **Step 5: Compile + spot-test.**

```bash
cargo check -p tracedao-server --bins --features local-gpu-models
cargo test -p tracedao-server --features local-gpu-models 2>&1 | tail -20
```

  Existing tests must stay green. The arch-dispatch *forward* path is
  hard to unit-test without real model weights; it's exercised by the
  operator at bake-off time. The plan accepts this trade-off — slot it
  into the operator runbook as "if a candidate fails to load, you'll
  see `CandlePerplexityScorerLoadFailed candidate_id=<id>`; PR #34
  catches and reports load failures without killing the run."

- [ ] **Step 6: Commit.**

```bash
git commit -m "Add arch-dispatched ScorerBackend (Llama/Qwen3/Gemma3/Gemma4)"
```

The commit message body must:
- Explicitly call out the Qwen3 QK-Norm bug fix
- Note the `arch:` parameter addition to `try_new` (back-compat-
  breaking for any external callers; the only callers are in-tree)
- List the four arches supported

---

## Slice 4 — Manifest schema + ctx_for + deprecation alias

### Task 4: Add `Qwen3` and `Gemma4` to `CandidateArch`

**Files:**
- Modify: `crates/tracedao-server/src/bin/gate_calibrate/bakeoff_manifest.rs`
- Modify: `crates/tracedao-server/src/bin/gate_calibrate/run_candidate_eval.rs`
- Modify: `crates/tracedao-server/tests/bakeoff_manifest.rs`

- [ ] **Step 1: Update the enum.**

The current enum has `#[serde(rename_all = "kebab-case")]` at the
container level. Drop the container attribute and use per-variant
`#[serde(rename = "...")]` so the variant names map cleanly to the
lowercase manifest tokens (`llama`, `qwen3`, `qwen2`, `gemma3`,
`gemma4`). Without the explicit drop, both attributes would compete
and parsing becomes order-dependent.

```rust
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum CandidateArch {
    #[serde(rename = "llama")]
    Llama,
    #[serde(rename = "qwen3")]
    Qwen3,
    /// Deprecated alias for Qwen3. Manifest parsing emits a warning when
    /// this is seen; future PRs will drop the alias entirely.
    #[serde(rename = "qwen2")]
    Qwen2,
    #[serde(rename = "gemma3")]
    Gemma3,
    #[serde(rename = "gemma4")]
    Gemma4,
}
```

(Also bump `Copy` on the derive — `BackendArch` is Copy, and callers
will pass `c.arch` by value at the conversion site.)

- [ ] **Step 2: Add a deprecation warning to the manifest parser.**

In `parse_manifest_str`, after the dedup check:

```rust
for c in &manifest.candidate {
    if matches!(c.arch, CandidateArch::Qwen2) {
        warnings.push(format!(
            "candidate {} uses deprecated arch=qwen2; switch to arch=qwen3 \
             (the loader resolves qwen2 to qwen3 internally)",
            c.id
        ));
    }
}
```

- [ ] **Step 3: Update `ctx_for`.**

```rust
pub fn ctx_for(arch: &CandidateArch) -> usize {
    match arch {
        CandidateArch::Llama
        | CandidateArch::Qwen3
        | CandidateArch::Qwen2
        | CandidateArch::Gemma3
        | CandidateArch::Gemma4 => 4096,
    }
}
```

- [ ] **Step 4: Add three new manifest tests.**

```rust
#[test]
fn parses_qwen3_arch() {
    let raw = r#"
[[candidate]]
id = "qwen3-8b-base"
path = "/srv/q3"
arch = "qwen3"
license = "apache-2.0"
"#;
    let manifest = parse_manifest_str(raw).expect("parses");
    assert!(matches!(manifest.candidates[0].arch, CandidateArch::Qwen3));
    assert!(manifest.warnings().is_empty());
}

#[test]
fn parses_gemma4_arch() {
    let raw = r#"
[[candidate]]
id = "gemma-4-31b"
path = "/srv/g4"
arch = "gemma4"
license = "apache-2.0"
"#;
    let manifest = parse_manifest_str(raw).expect("parses");
    assert!(matches!(manifest.candidates[0].arch, CandidateArch::Gemma4));
    assert!(manifest.warnings().is_empty());
}

#[test]
fn qwen2_alias_warns_and_resolves_to_qwen3() {
    let raw = r#"
[[candidate]]
id = "qwen3-8b-base"
path = "/srv/q3"
arch = "qwen2"
license = "apache-2.0"
"#;
    let manifest = parse_manifest_str(raw).expect("parses");
    assert!(matches!(manifest.candidates[0].arch, CandidateArch::Qwen2));
    let warnings = manifest.warnings();
    assert!(warnings.iter().any(|w| w.contains("deprecated arch=qwen2")));
}
```

- [ ] **Step 5: Run tests, confirm 3 new + 5 existing pass.**

- [ ] **Step 6: Commit.**

```bash
git commit -m "Add Qwen3 + Gemma4 manifest variants with Qwen2 deprecation alias"
```

---

## Slice 5 — Operator docs

### Task 5: Update `calibration.md` Phase 0 candidate example

**Files:**
- Modify: `docs/operator/calibration.md`
- Modify: `docs/trace-commons-roadmap.md` (one-line A2.2 status)

- [ ] **Step 1: Update the candidate-manifest example.**

The current example uses `arch = "qwen2"` for Qwen3-8B-Base. Update
to `arch = "qwen3"`. Add a `Gemma4` example row. Add a brief note
that Qwen 3.6 (`qwen3_5`) and earlier-Gemma `gemma`/`gemma2` are
not in the supported set today.

- [ ] **Step 2: Update the roadmap with A2.2 status.**

In `docs/trace-commons-roadmap.md`, under the Phase A status block,
add a one-line entry:

```
- A2.2: candle arch dispatch + Gemma 4 support + Qwen3 QK-Norm fix — done
```

(Status updates to `pending` first; flip to `done` only after the
re-run report lands.)

- [ ] **Step 3: Commit.**

```bash
git commit -m "Document A2.2 arch-dispatch retrofit in operator runbook"
```

---

## Slice 6 — End-to-end dry run

### Task 6: Re-confirm the bake-off binary still works

**Files:** none (verification only)

- [ ] **Step 1: Build + run the dry-run path.**

```bash
cargo build --release --bin tracedao-gate-calibrate
# (default features — exercises arch parsing without GPU)

# Build a 4-candidate synthetic manifest with one entry per arch:
cat > /tmp/dry-candidates.toml <<'TOML'
[[candidate]]
id = "fake-llama" 
path = "/tmp/notreal"
arch = "llama"
license = "apache-2.0"

[[candidate]]
id = "fake-qwen3"
path = "/tmp/notreal"
arch = "qwen3"
license = "apache-2.0"

[[candidate]]
id = "fake-gemma3"
path = "/tmp/notreal"
arch = "gemma3"
license = "apache-2.0"

[[candidate]]
id = "fake-gemma4"
path = "/tmp/notreal"
arch = "gemma4"
license = "apache-2.0"
TOML

BAKEOFF_CORPUS_DRY_RUN=1 ./scripts/operator/build-bakeoff-corpus.sh /tmp/dry.tar.zst

./target/release/tracedao-gate-calibrate bake-off \
  --candidates=/tmp/dry-candidates.toml \
  --corpus=/tmp/dry.tar.zst \
  --hardware=cpu \
  --report-out=/tmp/dry-report.json \
  --mock-scorer
```

Expected: all 4 candidates parse, mock scorer runs against them,
report.json emitted with 4 entries (no real scoring happens — mock
ignores the arch entirely).

- [ ] **Step 2: Validate the report.**

```bash
jq '.candidates[].id' /tmp/dry-report.json
# Should output the 4 ids in order
```

---

## Done criteria

- [ ] `cargo check -p tracedao-server --bins` clean (default features).
- [ ] `cargo check -p tracedao-server --bins --features local-gpu-models` clean.
- [ ] `cargo test -p tracedao-server` green — all existing + 3 new tests.
- [ ] `cargo test -p tracedao-gate-enclave --features local-gpu-models` green — 4 new `flatten_text_config` tests.
- [ ] Five commits on `feat/a22-bakeoff-arch-dispatch`, in this order with these subjects:
  1. `Add flatten_text_config helper for multimodal candle configs`
  2. `Bump candle to <sha> on main (includes gemma4 support)`
  3. `Add arch-dispatched ScorerBackend (Llama/Qwen3/Gemma3/Gemma4)`
  4. `Add Qwen3 + Gemma4 manifest variants with Qwen2 deprecation alias`
  5. `Document A2.2 arch-dispatch retrofit in operator runbook`

  (Slice 2.5 is a reading pass — no commit. Slice 6 is dry-run verification — no commit; findings go into the Slice 3 commit body.)
- [ ] Dry-run end-to-end smoke produces a 4-candidate mock report.
- [ ] All commits carry the Co-Authored-By trailer. No `--no-verify`, no `--amend`.
- [ ] No emojis.
- [ ] PR opened against `main`.

---

## What this plan does NOT do

(Recording to head off scope creep — spec is explicit about deferrals.)

- Does **not** run the real 3-way bake-off. That's operator activity
  (spec rollout A2.2b), a separate Lambda session and a separate PR
  for the result.
- Does **not** add Qwen 3.6 (`qwen3_5`) support. Candle doesn't have
  a loader; this is out of scope.
- Does **not** switch to mistralrs. Build-health still a concern;
  spec explicitly defers.
- Does **not** flip the production env-var defaults. That's a
  one-line PR after the re-run report lands (spec rollout A2.2d).

## Spec open questions parked here

1. **Candle git-pin vs wait-for-0.11.** Plan implements the git-pin
   recommendation. If a 0.11.x release lands during the implementation
   window, switch the dep back to a version pin in a follow-up.
2. **Same corpus for re-run.** Yes — see spec section 3.
3. **Qwen2 deprecation alias.** Implemented per spec recommendation:
   parses, resolves to Qwen3, emits warning.
4. **Gemma 4 31B Base existence.** Operator verifies in A2.2b; if only
   instruct is available, document and proceed.
5. **Corrected Qwen3 AUC dropping below 0.5.** Reported honestly in
   A2.2c.
