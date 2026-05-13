# Mistralrs Migration Implementation Plan (A2.3 Retrofit)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace `CandlePerplexityScorer` with `LocalPerplexityScorer` backed by `mistralrs`, unlocking Qwen 3.6 27B Dense (and any future architecture mistralrs supports) for both the bake-off and the production gate.

**Architecture:** Git-pin `mistralrs` (uses candle under the hood) and use its raw-logits API (`ForwardInputsResult::RawLogits`) to compute per-token logprobs. Delete the A2.2 `ScorerBackend` enum + `BackendArch` enum — mistralrs handles arch dispatch internally. The public `PerplexityScorer` trait stays unchanged.

**Tech Stack:** Rust, `mistralrs` git-pinned to a master SHA (verify build-health during pre-flight). Candle deps stay in tree as transitive dependencies of mistralrs.

**Spec:** `docs/superpowers/specs/2026-05-13-mistralrs-migration-design.md`

---

## File Map

**Modified files**

| Path | What changes |
|------|--------------|
| `crates/tracedao-gate-enclave/Cargo.toml` | Add `mistralrs` (git pin) behind the `local-gpu-models` feature flag. Keep candle deps (they're transitive). |
| `crates/tracedao-gate-enclave/src/perplexity_candle.rs` → **renamed** `crates/tracedao-gate-enclave/src/perplexity_local.rs` | Replace `CandlePerplexityScorer` with `LocalPerplexityScorer`. Delete `ScorerBackend` enum + `BackendArch` enum + arch dispatch. Keep `flatten_text_config` (may still be useful). Rewrite `try_new` and `score` against mistralrs's API. ~300 lines net change. |
| `crates/tracedao-gate-enclave/src/lib.rs` | Update `mod perplexity_candle` → `mod perplexity_local`. Public re-export name follows. |
| `crates/tracedao-server/src/bin/gate_calibrate/run_candidate_eval.rs` | Replace `CandlePerplexityScorer::try_new(..., arch: BackendArch)` calls with `LocalPerplexityScorer::try_new(...)` — no arch parameter, mistralrs auto-detects. Drop the `BackendArch` translation match. |
| `crates/tracedao-server/src/bin/tracedao-gate-calibrate.rs` | Same call-site update. Drop `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` env-var parsing (deprecated; warn if set). |
| `crates/tracedao-server/src/bin/tracedao-ingest.rs` (~line 4274) | Same. |
| `crates/tracedao-server/src/bin/gate_calibrate/bakeoff_manifest.rs` | Add `Qwen3_5` (Qwen 3.6) variant to `CandidateArch`. Keep all existing variants. The `arch` field is now informational (used for `ctx_for`); mistralrs doesn't read it. |
| `crates/tracedao-server/src/bin/gate_calibrate/run_candidate_eval.rs` | Add `Qwen3_5` case to `ctx_for` (max_tokens 4096, same as others). |
| `crates/tracedao-server/tests/bakeoff_manifest.rs` | New test: `parses_qwen3_5_arch`. |
| In-crate test in the renamed scorer file | Update to use `LocalPerplexityScorer`; pass no `arch:` parameter. |
| `docs/operator/calibration.md` | Update Phase 0 candidate-manifest example to include a `qwen3_5` entry. Update build instructions to reflect mistralrs is the backend. |
| `docs/operator/env-reference.md` | Note that `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` is now deprecated (mistralrs auto-detects from config.json). |
| `docs/trace-commons-roadmap.md` | A2.3 status line under Phase A. |

**New files**

| Path | Responsibility |
|------|----------------|
| `crates/tracedao-gate-enclave/examples/mistralrs_smoke.rs` | Standalone proof-of-concept binary. Loads a tiny model via mistralrs, computes one logprob. Not run in CI; operator-only. |

**Out of scope (do not touch)**

- A2.2's report (it stays committed as history).
- A2.1's report (stays committed).
- The bake-off observability work from PR #34 (unaffected).
- The gate service's HTTP boundary, auth, audit chain, RLS, etc. — backend swap only.

---

## Pre-flight

- [ ] **Confirm green baseline.**

```bash
cargo check -p tracedao-server --bins
cargo check -p tracedao-server --bins --features local-gpu-models
cargo test -p tracedao-server --test bakeoff_manifest --test bakeoff_subcommand --test bakeoff_report
```

Expected: clean. If anything fails, stop and fix before starting.

- [ ] **Read the spec.**

You need to internalize: (a) mistralrs is the new backend, (b) the
`ScorerBackend`/`BackendArch` types we just added in A2.2 get deleted,
(c) the production gate also switches backends, (d) the proof-of-
concept (Slice 0) precedes the refactor.

- [ ] **Pick the mistralrs git-pin.**

```bash
git ls-remote https://github.com/EricLBuehler/mistral.rs.git master | head -1
```

Record the SHA + date. Verify:

```bash
git clone --depth 1 --filter=blob:limit=1k \
  https://github.com/EricLBuehler/mistral.rs.git /tmp/mistralrs-check
ls /tmp/mistralrs-check/mistralrs/Cargo.toml
ls /tmp/mistralrs-check/mistralrs-core/src/pipeline/
grep -l "ForwardInputsResult\|RawLogits" /tmp/mistralrs-check/mistralrs-core/src/pipeline/*.rs
rm -rf /tmp/mistralrs-check
```

Must confirm: `mistralrs` is a workspace member; `mistralrs-core/src/
pipeline/` exists; the `RawLogits` token appears in at least one
pipeline file. If any of these fail, **STOP and report BLOCKED.**

- [ ] **Dependency approval.**

mistralrs and its transitive deps (~80+ crates) constitute a substantial
new direct-dep addition. **Surface to the operator for approval before
adding** — per `~/.claude/CLAUDE.md`, dependency policy is "extremely
conservative." Approval data to surface:

| Crate | Version | Purpose | Maintainers | Cadence |
|-------|---------|---------|-------------|---------|
| `mistralrs` | git master `<sha>` | LLM inference backend, replaces hand-written candle integration | Single maintainer (EricLBuehler) | 30+ commits/30 days |

Record approval in `~/.claude/approved-dependencies.md` after green-light.

If approval is denied, stop. The A2.3 retrofit doesn't work without
mistralrs — there's no graceful fallback in scope.

---

## Slice 0 — Proof-of-concept and API verification

**This is a hard pre-flight gate, not a soft exploration.** The rest
of the plan assumes specific mistralrs API shapes. If the PoC reveals
those shapes wrong, the plan is invalid and needs a redraft.

**Exit criteria (must all hold before Slice 1):**

1. `ForwardInputsResult::RawLogits { logits: Tensor }` (or equivalent
   raw-logits return path) exists as a **public** variant of
   mistralrs's response type. Document the exact import path.
2. The model-builder API supports loading a local HF-format directory
   (config.json + tokenizer.json + safetensors). Document the
   exact builder type and `await`-chain.
3. The PoC binary, run on Lambda H100, produces a finite per-token
   logprob sum for the input `"Hello, world."` on a real model
   (e.g. `Qwen3-8B-Base`).
4. **Build time** recorded (release-mode `cargo build` on Lambda H100
   from scratch). If > 15 minutes, flag in the runbook.
5. **Binary size delta** recorded (size of the produced binary vs
   today's `tracedao-gate-calibrate`). `tracedao-ingest` is already
   large; understand if mistralrs pushes it into different
   distribution-shape territory.
6. **Candle SHA mistralrs pins.** `cat Cargo.lock | grep '^name = "candle-core"' -A 2` after `cargo update -p mistralrs`. Compare to our current candle pin (`5447a87`); the spec's resolution (drop our direct candle deps) handles divergence, but record what we observed.

If any of 1-3 fails, report BLOCKED. Items 4-6 are observations
the operator records but don't block.

### Task 0: PoC binary

**Files:**
- Create: `crates/tracedao-gate-enclave/examples/mistralrs_smoke.rs`

Tiny standalone program:

```rust
//! Standalone proof-of-concept: load a model via mistralrs, compute
//! one perplexity number for a hardcoded input string. Not run in CI.

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    // TODO: write this against the mistralrs version you pinned.
    // Pseudocode:
    //   let model = TextModelBuilder::new(path).with_dtype(BF16).with_device(Cuda(0)).build()?;
    //   let req = build_raw_logits_request("Hello world.");
    //   let res = model.forward(req).await?;
    //   match res { ForwardInputsResult::RawLogits { logits } => compute_perplexity(logits)? }
    //   ...
}
```

The exact mistralrs API may differ from the pseudocode. Use `cargo
doc --open -p mistralrs` after the dep is added, or read `mistralrs/
src/lib.rs` directly to find the right builder + request types.

- [ ] **Step 1: Add mistralrs to Cargo.toml under `[dev-dependencies]`** so it doesn't perturb the production build.

- [ ] **Step 2: Write the PoC.** Target: load Qwen3-8B-Base from a local path, score "Hello, world." and print the per-token logprobs sum.

- [ ] **Step 3: Run it on Lambda H100.** Build, scp, execute. Confirm it produces finite numbers.

- [ ] **Step 4: Document the actual mistralrs API shape in your scratch notes** — function names, async-ness, error types, tensor shapes. This is the reference for Slice 1.

- [ ] **Step 5: Remove mistralrs from `[dev-dependencies]`** (Slice 1 adds it to `[dependencies]` properly).

No commit. If the PoC fails (mistralrs API doesn't expose raw logits the way the spec assumed, or builds fail, or model loading fails), report BLOCKED with the specific issue.

---

## Slice 1 — Add mistralrs dep + rename module

### Task 1: Cargo dep + file rename

**Files:**
- Modify: `crates/tracedao-gate-enclave/Cargo.toml`
- Rename: `crates/tracedao-gate-enclave/src/perplexity_candle.rs` → `crates/tracedao-gate-enclave/src/perplexity_local.rs`
- Modify: `crates/tracedao-gate-enclave/src/lib.rs`

- [ ] **Step 1: Add mistralrs as a feature-gated dependency AND drop our direct candle deps.**

Our crates currently git-pin candle to `5447a87`. mistralrs pins its
own candle SHA. Two simultaneous pins for the same crate is a cargo
resolution problem. Resolution: **drop our direct candle deps; let
mistralrs supply them transitively.**

In `crates/tracedao-gate-enclave/Cargo.toml`:

```toml
# REMOVE these three direct candle pins:
# candle-core = { git = "https://github.com/huggingface/candle.git", rev = "5447a87...", optional = true }
# candle-nn = { ... }
# candle-transformers = { ... }

# ADD mistralrs:
mistralrs = { git = "https://github.com/EricLBuehler/mistral.rs.git", rev = "<sha>", optional = true, default-features = false, features = ["cuda"] }
```

Update `local-gpu-models[-cuda]` feature definitions to pull in `mistralrs` instead of the candle stack. Exact feature wiring depends on what mistralrs's feature flags look like — verify against the pinned `Cargo.toml`.

If our code references `candle_core::Tensor` directly (e.g., for the
`flatten_text_config` helper's return type), find the mistralrs
re-export path (it lives somewhere under `mistralrs::*` since mistralrs
itself uses candle internally), OR add a minimal direct candle-core
dep with `default-features = false` pinned to the SAME SHA mistralrs
uses (from `Cargo.lock` after the swap).

- [ ] **Step 2: Rename the file.**

```bash
git mv crates/tracedao-gate-enclave/src/perplexity_candle.rs \
       crates/tracedao-gate-enclave/src/perplexity_local.rs
```

Update `crates/tracedao-gate-enclave/src/lib.rs`:

```rust
// Before:
mod perplexity_candle;
// After:
mod perplexity_local;
```

If anything outside the crate references `perplexity_candle::*`, update those imports. Grep:

```bash
grep -rn "perplexity_candle" --include="*.rs" .
```

- [ ] **Step 3: Confirm both feature configurations compile** (the rename and dep addition shouldn't break the existing candle-based code yet — Slice 2 does the actual swap).

```bash
cargo check -p tracedao-server --bins
cargo check -p tracedao-server --bins --features local-gpu-models
```

- [ ] **Step 4: Commit.**

```bash
git commit -m "Rename perplexity_candle to perplexity_local; add mistralrs dep"
```

Commit message body must record the mistralrs SHA + date.

---

## Slice 2 — Replace `CandlePerplexityScorer` with `LocalPerplexityScorer`

The load-bearing slice. Rewrite the scorer against mistralrs's API.

### Task 2: Drop candle backend, wire mistralrs

**Files:**
- Modify: `crates/tracedao-gate-enclave/src/perplexity_local.rs`

What gets deleted:
- `enum ScorerBackend` (A2.2 addition)
- `enum BackendArch` + `BackendArch::parse` (A2.2 addition)
- Per-arch `try_new` arms inside the deleted enum
- Candle-specific imports (`candle_transformers::models::*`)

What stays:
- `flatten_text_config` helper (mistralrs may or may not need it;
  keep until proven unnecessary)
- The public `PerplexityScorer` trait impl
- The `aggregate_perplexity_metrics` aggregation helper
- The `Mutex` pattern (mistralrs's model state is mutable across
  forward calls; same need)

What gets rewritten:
- `try_new`: builds a mistralrs `TextModel` (or whatever the
  appropriate builder is) from the local model path
- `score`: tokenize input, call mistralrs's forward with
  `return_raw_logits=true`, compute log_softmax + gather actual-
  next-token logprobs, aggregate via existing helper

**Approximate target shape (verify against actual mistralrs API from Slice 0):**

```rust
#[cfg(feature = "local-gpu-models")]
mod local_impl {
    use mistralrs::{TextModel, TextModelBuilder, ForwardInputsResult, ...};
    use std::sync::Mutex;

    pub struct LocalPerplexityScorer {
        model: Mutex<TextModel>,
        device: Device,
        dtype: DType,
        tail_logprob_cutoff: f32,
        model_id: String,
        max_tokens: usize,
    }

    impl LocalPerplexityScorer {
        pub async fn try_new(
            model_id: impl Into<String>,
            model_path: impl AsRef<Path>,
            device: LocalDeviceKind,
            tail_logprob_cutoff: f32,
            max_tokens: usize,
            // No arch parameter — mistralrs auto-detects from config.json
        ) -> anyhow::Result<Self> {
            let model = TextModelBuilder::new(model_path)
                .with_dtype(DType::BF16)
                .with_device(map_device(device))
                .build()
                .await?;
            Ok(Self {
                model: Mutex::new(model),
                ...
            })
        }
    }

    impl PerplexityScorer for LocalPerplexityScorer {
        fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
            // 1. Tokenize plaintext into token_ids.
            // 2. Build a forward request with return_raw_logits=true.
            // 3. Call model.forward(req).await.
            //    (Need to bridge sync trait method to async API —
            //     use tokio::task::block_in_place or a runtime handle.)
            // 4. Extract RawLogits tensor.
            // 5. Compute log_softmax over vocab dim.
            // 6. Gather actual-next-token logprobs.
            // 7. Aggregate via aggregate_perplexity_metrics.
        }
    }
}
```

**Key design notes:**
- mistralrs is async. The `PerplexityScorer` trait is sync.
  **`tokio::task::block_in_place` does NOT work** — it panics in
  current-thread runtimes (the in-crate test and PoC both use
  current-thread). Instead, `LocalPerplexityScorer` owns its own
  `tokio::runtime::Runtime` (single-thread, dedicated to mistralrs
  I/O) and calls `rt.block_on(self.async_score(...))` from the sync
  trait method. The owned runtime decouples from the caller's runtime
  flavor entirely.
- The `Mutex<TextModel>` wraps the model because mistralrs's forward
  mutates internal state (KV cache); same constraint as our candle path.
- Use of `return_raw_logits=true` is critical — the default mistralrs
  response is a chat-completion message, not raw logits.

**Owned-runtime sketch:**

```rust
pub struct LocalPerplexityScorer {
    model: Mutex<TextModel>,
    runtime: tokio::runtime::Runtime,  // owned; bridges sync trait to async API
    // ... other fields
}

impl LocalPerplexityScorer {
    pub fn try_new(
        model_id: impl Into<String>,
        model_path: impl AsRef<Path>,
        device: LocalDeviceKind,
        tail_logprob_cutoff: f32,
        max_tokens: usize,
    ) -> anyhow::Result<Self> {
        // Build a single-thread runtime owned by this scorer.
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("LocalPerplexityScorer runtime init")?;
        let model = runtime.block_on(async {
            TextModelBuilder::new(model_path).with_dtype(DType::BF16)
                .with_device(map_device(device)).build().await
        })?;
        Ok(Self {
            model: Mutex::new(model),
            runtime,
            // ...
        })
    }
}

impl PerplexityScorer for LocalPerplexityScorer {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
        self.runtime.block_on(self.async_score(plaintext))
    }
}
```

Note: `try_new` is **sync** in this design (not async) because the
caller is also sync. The constructor calls into the owned runtime to
do the async build, then stores both the model and the runtime
together. Existing async-call-site updates in Slice 3 drop their
`.await` on `try_new`.

### Sub-tasks

- [ ] **Step 1: Delete the `ScorerBackend` enum + `BackendArch` enum + arch-dispatched `try_new` arms.**

- [ ] **Step 2: Add mistralrs imports + rewrite `LocalPerplexityScorer::try_new`** (async; see sketch above).

- [ ] **Step 3: Rewrite `PerplexityScorer::score` impl** for the new struct. Sync trait, async impl — use `block_in_place` + the runtime handle.

- [ ] **Step 4: Verify compile.**

```bash
cargo check -p tracedao-gate-enclave --features local-gpu-models 2>&1 | tail -20
cargo check -p tracedao-server --bins --features local-gpu-models 2>&1 | tail -20
```

  Will fail until Slice 3 updates callers (caller signature changed:
  removed `arch:` parameter).

- [ ] **Step 5: Move/update the existing in-crate test** (around the old `:577` line) to use `LocalPerplexityScorer` and the new signature. The test logic — load the mock-like model, score, assert finite — stays the same.

- [ ] **Step 6: Commit (after Slice 3 unblocks compile — this slice ends red).**

  Don't commit until Slice 3 makes the world compile again. Otherwise
  bisection on `main` later sees a broken commit.

  Marker: this is Task 2 of 5 numbered commits, but Slice 2 + Slice 3
  ship together at the end of Slice 3.

---

## Slice 3 — Update callers

Three production call sites + the in-crate test were touched by A2.2.
This slice reverses A2.2's "add arch parameter" change.

### Task 3: Caller updates

**Files:**
- Modify: `crates/tracedao-server/src/bin/gate_calibrate/run_candidate_eval.rs`
- Modify: `crates/tracedao-server/src/bin/tracedao-gate-calibrate.rs`
- Modify: `crates/tracedao-server/src/bin/tracedao-ingest.rs`

- [ ] **Step 1: Update `run_bakeoff` (bake-off subcommand).**

  Drop the `CandidateArch → BackendArch` inline match. Just call:

  ```rust
  LocalPerplexityScorer::try_new(
      c.id.clone(), c.path.clone(),
      LocalDeviceKind::Cuda, -8.0_f32, ctx_for(&c.arch),
  ).await?
  ```

- [ ] **Step 2: Update `run_calibrate` (calibrate subcommand).**

  Drop `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` env-var parsing. If the
  env is set, emit `tracing::warn!(deprecated_env = "TRACE_COMMONS_
  PERPLEXITY_MODEL_ARCH", "mistralrs auto-detects arch from
  config.json")`.

- [ ] **Step 3: Update `tracedao-ingest.rs` (production scorer wiring).**

  Same: drop arch parameter, drop env-var parsing.

  Caller-scope re-verification before editing:

  ```bash
  grep -rn "CandlePerplexityScorer\|BackendArch" --include="*.rs" .
  ```

  Expected output (post-A2.2): production callers in
  `tracedao-gate-calibrate.rs` (twice — bake-off + calibrate),
  `tracedao-ingest.rs`, plus the in-crate test in
  `perplexity_local.rs`. `run_candidate_eval.rs` references the type
  but the actual `try_new` call lives in `tracedao-gate-calibrate.rs`
  (`run_bakeoff` constructs the scorer; `run_candidate_eval` receives
  it as `&dyn PerplexityScorer`). Update all four.

- [ ] **Step 4: Add a production-gate init test.**

  Production gate-service scorer wiring is in `tracedao-ingest.rs`
  but never exercised by any test today. Add a small unit test that
  calls the env-var-driven init helper used by `tracedao-ingest`
  with a tiny mock model path and asserts construction returns the
  expected error class (`LocalPerplexityScorerLoadFailed` or
  similar). This proves the production code path compiles and
  dispatches correctly.

  Test placement: in `tracedao-ingest.rs`'s `#[cfg(test)] mod tests`,
  or a new integration test file
  `crates/tracedao-server/tests/ingest_scorer_init.rs`. Either way,
  do NOT load a real model in CI — assert the failure class.

- [ ] **Step 5: Compile + run tests.**

```bash
cargo check -p tracedao-server --bins
cargo check -p tracedao-server --bins --features local-gpu-models
cargo test -p tracedao-server
```

  Default-features tests pass. Featured tests pass for everything
  not requiring real GPU. The new production-init test (Step 4) must
  pass under default features.

- [ ] **Step 6: Commit Slices 2+3 together.**

```bash
git commit -m "Replace CandlePerplexityScorer with mistralrs-backed LocalPerplexityScorer"
```

  Combined commit message body covers: deleted ~250 lines of A2.2
  `ScorerBackend` enum, deleted ~30 lines of `BackendArch` parsing,
  rewrote `try_new` and `score` against mistralrs's API, updated
  three caller sites + one in-crate test, removed
  `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` env (now deprecated /
  ignored).

---

## Slice 4 — Manifest schema + `Qwen3_5` variant

### Task 4: Add Qwen 3.6 to candidate set

**Files:**
- Modify: `crates/tracedao-server/src/bin/gate_calibrate/bakeoff_manifest.rs`
- Modify: `crates/tracedao-server/src/bin/gate_calibrate/run_candidate_eval.rs`
- Modify: `crates/tracedao-server/tests/bakeoff_manifest.rs`

- [ ] **Step 1: Add `Qwen3_5` variant.**

```rust
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
pub enum CandidateArch {
    #[serde(rename = "llama")]    Llama,
    #[serde(rename = "qwen3")]    Qwen3,
    #[serde(rename = "qwen3_5")]  Qwen3_5,    // NEW
    #[serde(rename = "qwen2")]    Qwen2,
    #[serde(rename = "gemma3")]   Gemma3,
    #[serde(rename = "gemma4")]   Gemma4,
}
```

- [ ] **Step 2: Update `ctx_for`** to cover `Qwen3_5` (returns 4096, same as others — A2.3 doesn't change context sizing).

- [ ] **Step 3: Add a manifest test.**

```rust
#[test]
fn parses_qwen3_5_arch() {
    let raw = r#"
[[candidate]]
id = "qwen3.6-27b-dense"
path = "/srv/q36"
arch = "qwen3_5"
license = "apache-2.0"
"#;
    let m = parse_manifest_str(raw).expect("parses");
    assert!(matches!(m.candidates[0].arch, CandidateArch::Qwen3_5));
    assert!(m.warnings().is_empty());
}
```

- [ ] **Step 4: Run tests, confirm pass.**

- [ ] **Step 5: Commit.**

```bash
git commit -m "Add Qwen3_5 (Qwen 3.6) candidate arch variant"
```

---

## Slice 5 — Operator docs + roadmap

### Task 5: Document the migration

**Files:**
- Modify: `docs/operator/calibration.md`
- Modify: `docs/operator/env-reference.md`
- Modify: `docs/trace-commons-roadmap.md`

- [ ] **Step 1: Update `calibration.md` Phase 0.**

  - Add a Qwen 3.6 entry to the candidate-manifest example:
    ```toml
    [[candidate]]
    id = "qwen3.6-27b-dense"
    path = "/srv/models/qwen3.6-27b"
    arch = "qwen3_5"
    license = "apache-2.0"
    params_b = 27
    release_date_unix = 1776470400
    ```
  - Add a note that the backend is now mistralrs (not raw candle).
    Build instruction stays the same (`--features local-gpu-models-cuda`).
  - Pin the mistralrs SHA in operator-runbook documentation so the
    operator can reproduce the exact build.

- [ ] **Step 2: Update `env-reference.md`.**

  - Mark `TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` as **deprecated**
    (auto-detected by mistralrs from config.json).
  - No new env vars needed.

- [ ] **Step 3: Update the roadmap.**

  Add a one-line status entry under Phase A:
  ```
  - A2.3: mistralrs backend migration + Qwen 3.6 support — done
  ```

  (Marker `pending` until the re-run report lands; flip to `done`
  in the report PR.)

- [ ] **Step 4: Commit.**

```bash
git commit -m "Document A2.3 mistralrs migration in operator runbook"
```

---

## Slice 6 — Dry-run smoke

### Task 6: Confirm the binary works end-to-end

**Files:** none (verification only)

- [ ] **Step 1: Build + dry-run.**

```bash
cargo build --release --bin tracedao-gate-calibrate
# (default features — exercises arch parsing without GPU)

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
id = "fake-qwen3-5"
path = "/tmp/notreal"
arch = "qwen3_5"
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
report.json has 4 entries.

- [ ] **Step 2: Validate the report.**

```bash
jq '.candidates | length' /tmp/dry-report.json   # 4
jq '.candidates[].id' /tmp/dry-report.json
```

No commit. Findings go into the Slice 3 commit body.

---

## Done criteria

- [ ] `cargo check -p tracedao-server --bins` clean (default features).
- [ ] `cargo check -p tracedao-server --bins --features local-gpu-models` clean.
- [ ] `cargo test -p tracedao-server` green — all existing + 1 new manifest test.
- [ ] Four commits on `feat/a23-mistralrs-migration`, in this order with these subjects:
  1. `Rename perplexity_candle to perplexity_local; add mistralrs dep`
  2. `Replace CandlePerplexityScorer with mistralrs-backed LocalPerplexityScorer`
  3. `Add Qwen3_5 (Qwen 3.6) candidate arch variant`
  4. `Document A2.3 mistralrs migration in operator runbook`

  (Slice 0 is operator PoC — no commit. Slice 6 is verification — no commit; findings in Slice 2+3 body.)
- [ ] Dry-run end-to-end smoke produces a 4-candidate mock report.
- [ ] All commits carry the Co-Authored-By trailer. No `--no-verify`, no `--amend`.
- [ ] No emojis.
- [ ] mistralrs SHA recorded in commit 1 body + operator runbook.
- [ ] PR opened against `main`.

---

## What this plan does NOT do

(Recording to head off scope creep — spec is explicit.)

- Does **not** run the real 4-way bake-off. That's operator activity
  (A2.3c), separate Lambda session, separate PR for the result.
- Does **not** keep dual-backend support. Mistralrs replaces candle
  in our integration; the candle crates stay only as transitive deps.
- Does **not** add multimodal evaluation. Text-only.
- Does **not** flip env-var defaults. That's a one-line PR after the
  re-run report lands (A2.3e).
- Does **not** change the gate service's trust model or HTTP boundary.

## Spec open questions parked here

1. **mistralrs feature-set choices.** Plan starts with `["cuda"]`
   only. Add features iteratively as the PoC reveals missing
   capabilities (Slice 0).
2. **Production gate behavior parity.** Implementer verifies via the
   new ingest-scorer-init test (Slice 3 Step 4) + the existing
   in-crate test running under mistralrs; any AUC drift gets noted
   in the Slice 2+3 commit body.
3. **Build time + binary size.** Measured during Slice 0 PoC;
   recorded in the operator runbook. If > 15 minutes, the runbook
   suggests a Docker build cache. If binary size > 1.5x today,
   document the distribution-shape impact.
4. **Async/await bridge.** Spec resolution: owned
   `tokio::runtime::Runtime` inside `LocalPerplexityScorer`,
   `rt.block_on` from the sync trait method. **Not**
   `block_in_place`.
5. **A2.2's `Gemma3` backend.** Dropped. `Gemma3` variant stays in
   the manifest enum for forward-compat but the dispatch code goes
   away with the rest of `ScorerBackend`.
6. **`flatten_text_config` keep-or-delete.** Decide in Slice 0 by
   loading a known-multimodal model (e.g. Gemma 4 31B) via mistralrs
   without the helper and seeing if it succeeds. If mistralrs
   flattens internally, **delete** the helper in Slice 2. If
   mistralrs needs the flattened input, **keep** the helper and
   call it during `try_new` before passing the path to the builder.
   Don't carry dead code forward.
7. **`TRACE_COMMONS_PERPLEXITY_MODEL_ARCH` deprecation timeline.**
   Slice 3 emits a warn-log if set. Add a follow-up TODO: in
   A2.4 (next retrofit), bump this to a hard error so we don't
   carry the deprecated env forever.
8. **Rollback plan.** If A2.3 deploys to production and mistralrs
   misbehaves (memory leak, hang, miscalibration regression), the
   rollback is to revert the merge commit. A2.2's candle-direct path
   is gone from `main` but recoverable via `git revert`. Operator
   runbook should note the revert SHA. If mistralrs problems are
   chronic (multiple incidents in the first month), reconsider open
   question 7 in the spec (keeping a candle escape hatch under a
   feature flag).
