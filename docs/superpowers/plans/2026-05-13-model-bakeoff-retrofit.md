# Perplexity Scorer Model Bake-off Implementation Plan (A2.1 Retrofit)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend the existing `trace-commons-gate-calibrate` binary with a `--bake-off` mode that runs a held-out perplexity-discrimination evaluation across multiple candidate models, then emits a JSON + markdown report applying a pre-committed decision rule. Produce a corpus-builder script alongside.

**Architecture:** Four self-contained additions, not a refactor. (1) A `bakeoff_corpus` module that loads and validates a tarball of stratified eval slices. (2) A `bakeoff_metrics` module that computes the headline metrics (discrimination AUC, paraphrase stability, tail-fraction range, determinism, throughput, VRAM) from raw per-token logprobs. (3) A `bakeoff_report` module that applies the decision rule and emits report artifacts. (4) A new `bake-off` CLI subcommand in `trace-commons-gate-calibrate` that wires them together, sequencing one candidate at a time so models don't fight for VRAM. The corpus-builder is a shell script under `scripts/operator/`, hand-curated rather than crate code, because the slices need human eyes during construction.

**Tech Stack:** Rust (existing crate), candle (existing dep), `roc-auc` formula computed inline (no new dep), bash for the corpus builder.

**New Cargo deps required (gated by Pre-flight dependency approval below):** `toml`, `tar`, `zstd`. `tempfile` and `sha2` are already in `crates/trace-commons-server/Cargo.toml` and need no approval. Per `~/.claude/CLAUDE.md` and the repo's "Be extremely conservative" stance, **do not start Slice 1 or Slice 2 until the dependency-approval pre-flight passes.**

**Spec:** `docs/superpowers/specs/2026-05-13-model-bakeoff-retrofit-design.md`

---

## File Map

**New files**

| Path | Responsibility |
|------|----------------|
| `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_corpus.rs` | Tarball loader + slice validation |
| `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_metrics.rs` | AUC, paraphrase delta, tail-fraction range, determinism, throughput, VRAM accounting |
| `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_report.rs` | Decision-rule application, JSON + markdown emission |
| `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_manifest.rs` | `candidates.toml` parser |
| `crates/trace-commons-server/tests/bakeoff_metrics.rs` | Metric unit tests with synthetic logprob fixtures |
| `crates/trace-commons-server/tests/bakeoff_report.rs` | Decision-rule unit tests |
| `crates/trace-commons-server/tests/bakeoff_manifest.rs` | Manifest parser tests |
| `scripts/operator/build-bakeoff-corpus.sh` | Corpus assembler (OASST2 + GAIA + duplicate + paraphrase) |
| `scripts/operator/.bakeoff-corpus-checksums` | Pinned SHA256 of the generated corpus tarball |

**Modified files**

| Path | What changes |
|------|--------------|
| `crates/trace-commons-server/src/bin/trace-commons-gate-calibrate.rs` | Add `bake-off` subcommand, plumb to the four modules above; existing `calibrate` mode untouched |
| `docs/operator/calibration.md` | New "Model bake-off (A2.1)" section pointing at the spec + binary + corpus builder |
| `docs/operator/env-reference.md` | Document the (post-decision) flipped default for `TRACE_COMMONS_PERPLEXITY_MODEL_ID` — placeholder note that the value is decided by the bake-off |

**Out of scope (do not touch)**

- Floor recalibration. That's a separate pass after the winner is chosen (see spec rollout A2.1d).
- The `calibrate` mode in `trace-commons-gate-calibrate` — only `bake-off` is added.
- Embedder, vector index, gate orchestrator. All unchanged.
- Production env-var default flips. Those land in a separate post-decision PR, not in this plan.

---

## Pre-flight

- [ ] **Confirm green baseline.**

```bash
cargo check -p trace-commons-server --bins --features local-gpu-models
cargo test -p trace-commons-server --test trace_corpus_storage_contract
```

Expected: clean. If anything fails, stop and fix before starting.

- [ ] **Read the spec.**

```bash
$EDITOR docs/superpowers/specs/2026-05-13-model-bakeoff-retrofit-design.md
```

You need to internalize the decision rule (`0.6·AUC + 0.3·(1-paraphrase_delta) + 0.1·tail_range`) and the candidate set before writing any code. The metric module is wrong if it doesn't match the rule the spec commits to.

- [ ] **Get dependency approval before touching Cargo.toml.**

This plan requires three new direct dependencies that are not currently
in the tree. Per `~/.claude/CLAUDE.md`, **stop and surface to the
operator for explicit approval** before adding any of them:

| Crate | Version | Purpose | Why this one | Approval blocking |
|-------|---------|---------|--------------|-------------------|
| `toml` | latest 0.8.x | Parse `candidates.toml` manifest | Standard, well-maintained, ~3 transitive deps, MIT/Apache-2.0 | Slice 1 |
| `tar` | latest 0.4.x | Read corpus `.tar.zst` | `tar-rs`, single-maintainer but ubiquitous, MIT/Apache-2.0, no unsafe in public API | Slice 2 |
| `zstd` | latest 0.13.x | Decompress corpus `.tar.zst` | Standard Rust zstd binding via `zstd-safe`, MIT, ~2 transitive deps | Slice 2 |

When surfacing for approval, also report: latest publish date, open
issues count, and any RUSTSEC advisories. Do not silently add. Append
to `~/.claude/approved-dependencies.md` after approval.

If approval is denied for `tar` or `zstd`, fall back to: corpus on disk
as an uncompressed directory (no tarball), validated by a per-file
sha256 manifest. The plan stays applicable; only Task 2 changes
implementation.

If `toml` approval is denied: use the manifest as JSON (`serde_json` is
already in tree). Task 1 fixture changes accordingly.

---

## Slice 1 — Candidate manifest parser

### Task 1: Manifest types + parser

**Files:**
- Create: `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_manifest.rs`
- Create: `crates/trace-commons-server/tests/bakeoff_manifest.rs`

- [ ] **Step 1: Write the failing test (`bakeoff_manifest.rs` test crate)**

```rust
use std::path::PathBuf;

#[path = "../src/bin/gate_calibrate/bakeoff_manifest.rs"]
mod bakeoff_manifest;
use bakeoff_manifest::{parse_manifest_str, CandidateLicense};

#[test]
fn parses_minimal_two_candidate_manifest() {
    let raw = r#"
[[candidate]]
id = "llama-3.1-8b-instruct"
path = "/srv/models/llama-3.1-8b-instruct"
arch = "llama"
license = "llama-community"

[[candidate]]
id = "qwen3-8b-base"
path = "/srv/models/qwen3-8b-base"
arch = "qwen2"
license = "apache-2.0"
"#;
    let manifest = parse_manifest_str(raw).expect("parses");
    assert_eq!(manifest.candidates.len(), 2);
    assert_eq!(manifest.candidates[0].id, "llama-3.1-8b-instruct");
    assert_eq!(manifest.candidates[0].path, PathBuf::from("/srv/models/llama-3.1-8b-instruct"));
    assert_eq!(manifest.candidates[1].license, CandidateLicense::Apache2);
}

#[test]
fn rejects_duplicate_candidate_id() {
    let raw = r#"
[[candidate]]
id = "x"
path = "/a"
arch = "llama"
license = "apache-2.0"

[[candidate]]
id = "x"
path = "/b"
arch = "llama"
license = "apache-2.0"
"#;
    let err = parse_manifest_str(raw).unwrap_err();
    assert!(err.to_string().contains("duplicate candidate id"));
}

#[test]
fn rejects_empty_manifest() {
    let err = parse_manifest_str("").unwrap_err();
    assert!(err.to_string().contains("manifest must contain at least one candidate"));
}

#[test]
fn warns_on_non_apache_non_mit_license_for_non_incumbent() {
    // Spec restricts new picks to Apache-2.0 or MIT; LlamaCommunity is
    // grandfathered ONLY for the incumbent (Llama-3.1-8B-Instruct).
    // A new LlamaCommunity candidate that isn't the incumbent must
    // emit a warning the operator can grep for in the log.
    let raw = r#"
[[candidate]]
id = "some-other-llama-derivative"
path = "/srv/models/x"
arch = "llama"
license = "llama-community"
"#;
    let manifest = parse_manifest_str(raw).expect("parses but warns");
    let warnings = manifest.warnings();
    assert!(warnings.iter().any(|w| w.contains("license")), "warnings: {warnings:?}");
}

#[test]
fn no_warning_for_incumbent_llama_community() {
    let raw = r#"
[[candidate]]
id = "llama-3.1-8b-instruct"
path = "/srv/models/llama-3.1-8b-instruct"
arch = "llama"
license = "llama-community"
"#;
    let manifest = parse_manifest_str(raw).expect("parses clean");
    assert!(manifest.warnings().is_empty());
}
```

`ValidatedManifest` gains a `warnings: Vec<String>` field populated
during parse. The set of "incumbent ids" is a small const slice in
`bakeoff_manifest.rs`, currently `["llama-3.1-8b-instruct"]`. Update
that list (with a code comment pointing to the spec) when the
incumbent changes.

- [ ] **Step 2: Run test, confirm it fails**

```bash
cargo test -p trace-commons-server --test bakeoff_manifest 2>&1 | tail -10
```

Expected: compilation failure ("file not found" or similar).

- [ ] **Step 3: Implement `bakeoff_manifest.rs`**

```rust
use std::path::PathBuf;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateLicense {
    Apache2,
    Mit,
    LlamaCommunity,
    GemmaCustom,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CandidateArch {
    Llama,
    Qwen2,
    Gemma3,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Candidate {
    pub id: String,
    pub path: PathBuf,
    pub arch: CandidateArch,
    pub license: CandidateLicense,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub candidate: Vec<Candidate>,
}

#[derive(Debug, Clone)]
pub struct ValidatedManifest {
    pub candidates: Vec<Candidate>,
}

pub fn parse_manifest_str(raw: &str) -> anyhow::Result<ValidatedManifest> {
    let manifest: Manifest = toml::from_str(raw)
        .map_err(|e| anyhow::anyhow!("manifest parse error: {e}"))?;
    if manifest.candidate.is_empty() {
        anyhow::bail!("manifest must contain at least one candidate");
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in &manifest.candidate {
        if !seen.insert(c.id.clone()) {
            anyhow::bail!("duplicate candidate id: {}", c.id);
        }
    }
    Ok(ValidatedManifest { candidates: manifest.candidate })
}

pub fn parse_manifest_file(path: &std::path::Path) -> anyhow::Result<ValidatedManifest> {
    let raw = std::fs::read_to_string(path)?;
    parse_manifest_str(&raw)
}
```

- [ ] **Step 4: Run test, confirm pass**

```bash
cargo test -p trace-commons-server --test bakeoff_manifest 2>&1 | tail -10
```

Expected: 3 passed.

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_manifest.rs \
        crates/trace-commons-server/tests/bakeoff_manifest.rs
git commit -m "Add bake-off candidate manifest parser"
```

---

## Slice 2 — Bake-off corpus loader

### Task 2: Corpus tarball loader + slice typing

**Files:**
- Create: `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_corpus.rs`
- Test: append to `crates/trace-commons-server/tests/bakeoff_manifest.rs` (or new `bakeoff_corpus.rs`)

The corpus tarball lays out as:

```
bakeoff-corpus.tar.zst
├── manifest.json          # version, slice sizes, sha256 of each slice
├── novel/                 # 500 .txt files, known-novel reasoning traces
├── duplicate/             # 500 .txt files, known-duplicate boilerplate
└── paraphrase/            # 500 .jsonl entries: {"original": "...", "paraphrase": "..."}
```

- [ ] **Step 1: Write failing test for slice loading**

Use a minimal synthetic in-test tarball with 2 entries per slice. Verify
counts, content, and that mismatched manifest sha256 is rejected.

```rust
// crates/trace-commons-server/tests/bakeoff_corpus.rs (new file)
#[path = "../src/bin/gate_calibrate/bakeoff_corpus.rs"]
mod bakeoff_corpus;

#[test]
fn loads_synthetic_corpus_three_slices() {
    let dir = tempfile::tempdir().unwrap();
    let tarball = build_synthetic_corpus(&dir, 2, 2, 2); // 2 of each slice
    let corpus = bakeoff_corpus::load_corpus(&tarball).expect("loads");
    assert_eq!(corpus.novel.len(), 2);
    assert_eq!(corpus.duplicate.len(), 2);
    assert_eq!(corpus.paraphrase.len(), 2);
    assert_eq!(corpus.paraphrase[0].original, "orig-0");
    assert_eq!(corpus.paraphrase[0].paraphrase, "para-0");
}

#[test]
fn rejects_corpus_with_mismatched_slice_sha256() {
    // Build a corpus, tamper with one byte in one slice, verify load fails.
    // ...
}

fn build_synthetic_corpus(dir: &tempfile::TempDir, novel_n: usize, dup_n: usize, para_n: usize) -> std::path::PathBuf {
    // Use tar + zstd crates (both already transitively available via fastembed?
    // If not, add to Cargo.toml dev-deps with explicit approval).
    todo!("write a helper that emits a real .tar.zst the loader can read")
}
```

- [ ] **Step 2: Run test, confirm it fails**

- [ ] **Step 3: Implement `bakeoff_corpus.rs`**

```rust
use std::path::{Path, PathBuf};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Debug, Deserialize)]
struct TarballManifest {
    version: u32,
    novel_sha256: String,
    duplicate_sha256: String,
    paraphrase_sha256: String,
}

#[derive(Debug)]
pub struct ParaphrasePair {
    pub original: String,
    pub paraphrase: String,
}

#[derive(Debug)]
pub struct LoadedCorpus {
    pub novel: Vec<String>,
    pub duplicate: Vec<String>,
    pub paraphrase: Vec<ParaphrasePair>,
}

pub fn load_corpus(tarball: &Path) -> anyhow::Result<LoadedCorpus> {
    // 1. Decompress + untar into a tempdir
    // 2. Read manifest.json
    // 3. Read each slice; sha256-check against manifest
    // 4. Return LoadedCorpus
    // Use existing deps where possible. zstd is already pulled via tonic/fastembed.
    // tar crate is small; gate behind explicit approval if not already present.
    todo!()
}
```

Implementation note: check existing Cargo.toml for `tar` and `zstd` —
if absent, raise approval before adding. They're both tiny and well-
established but the project rules require explicit approval.

- [ ] **Step 4: Run test, confirm pass**

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_corpus.rs \
        crates/trace-commons-server/tests/bakeoff_corpus.rs
git commit -m "Add bake-off corpus tarball loader"
```

---

## Slice 3 — Metrics module

### Task 3: Discrimination AUC + paraphrase stability + tail-fraction range

**Files:**
- Create: `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_metrics.rs`
- Create: `crates/trace-commons-server/tests/bakeoff_metrics.rs`

- [ ] **Step 1: Write failing tests with synthetic distributions**

```rust
#[path = "../src/bin/gate_calibrate/bakeoff_metrics.rs"]
mod bakeoff_metrics;
use bakeoff_metrics::{discrimination_auc, paraphrase_delta, tail_fraction_range};

#[test]
fn perfect_separation_gives_auc_one() {
    // novel = high perplexity, duplicate = low perplexity, no overlap
    let novel = vec![100.0, 110.0, 120.0];
    let duplicate = vec![1.0, 2.0, 3.0];
    let auc = discrimination_auc(&novel, &duplicate);
    assert!((auc - 1.0).abs() < 1e-9, "auc={auc}");
}

#[test]
fn complete_overlap_gives_auc_half() {
    let novel = vec![50.0, 50.0, 50.0];
    let duplicate = vec![50.0, 50.0, 50.0];
    let auc = discrimination_auc(&novel, &duplicate);
    assert!((auc - 0.5).abs() < 1e-9, "auc={auc}");
}

#[test]
fn auc_handles_ties_correctly() {
    let novel = vec![5.0, 10.0];
    let duplicate = vec![5.0, 1.0];
    // Pairs: (5,5)=tie, (5,1)=novel>=dup win, (10,5)=win, (10,1)=win
    // AUC = (1*0.5 + 1 + 1 + 1) / 4 = 0.875
    let auc = discrimination_auc(&novel, &duplicate);
    assert!((auc - 0.875).abs() < 1e-9, "auc={auc}");
}

#[test]
fn paraphrase_delta_zero_when_identical() {
    let pairs = vec![(10.0, 10.0), (20.0, 20.0)];
    assert_eq!(paraphrase_delta(&pairs), 0.0);
}

#[test]
fn paraphrase_delta_is_median_absolute_relative() {
    // Deltas: |10-12|/10 = 0.2, |20-22|/20 = 0.1, |30-39|/30 = 0.3
    // Median = 0.2
    let pairs = vec![(10.0, 12.0), (20.0, 22.0), (30.0, 39.0)];
    assert!((paraphrase_delta(&pairs) - 0.2).abs() < 1e-9);
}

#[test]
fn tail_fraction_range_measures_spread() {
    // duplicate tail-fraction high (lots below cutoff), novel low
    let novel_frac = vec![0.10, 0.12, 0.08];   // median 0.10
    let duplicate_frac = vec![0.70, 0.72, 0.68]; // median 0.70
    let range = tail_fraction_range(&novel_frac, &duplicate_frac);
    assert!((range - 0.60).abs() < 1e-9, "range={range}");
}
```

- [ ] **Step 2: Run tests, confirm they fail**

- [ ] **Step 3: Implement `bakeoff_metrics.rs`**

```rust
/// Probability that a randomly drawn novel sample has higher perplexity
/// than a randomly drawn duplicate sample. Ties contribute 0.5.
pub fn discrimination_auc(novel: &[f64], duplicate: &[f64]) -> f64 {
    if novel.is_empty() || duplicate.is_empty() {
        return 0.5;
    }
    let mut wins = 0.0_f64;
    for &n in novel {
        for &d in duplicate {
            wins += match n.partial_cmp(&d).unwrap_or(std::cmp::Ordering::Equal) {
                std::cmp::Ordering::Greater => 1.0,
                std::cmp::Ordering::Equal => 0.5,
                std::cmp::Ordering::Less => 0.0,
            };
        }
    }
    wins / (novel.len() as f64 * duplicate.len() as f64)
}

pub fn paraphrase_delta(pairs: &[(f64, f64)]) -> f64 {
    if pairs.is_empty() {
        return 0.0;
    }
    let mut deltas: Vec<f64> = pairs.iter().map(|(orig, para)| {
        if *orig == 0.0 { 0.0 } else { (orig - para).abs() / orig.abs() }
    }).collect();
    deltas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let mid = deltas.len() / 2;
    if deltas.len() % 2 == 1 { deltas[mid] }
    else { (deltas[mid - 1] + deltas[mid]) / 2.0 }
}

pub fn tail_fraction_range(novel: &[f64], duplicate: &[f64]) -> f64 {
    let med = |v: &[f64]| -> f64 {
        if v.is_empty() { return 0.0; }
        let mut s: Vec<f64> = v.to_vec();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = s.len() / 2;
        if s.len() % 2 == 1 { s[mid] } else { (s[mid-1] + s[mid]) / 2.0 }
    };
    (med(duplicate) - med(novel)).abs()
}
```

- [ ] **Step 4: Run tests, confirm pass**

- [ ] **Step 5: Commit**

```bash
git add crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_metrics.rs \
        crates/trace-commons-server/tests/bakeoff_metrics.rs
git commit -m "Add bake-off scoring metrics (AUC, paraphrase, tail range)"
```

### Task 4: Determinism + throughput + VRAM accounting

**Files:**
- Modify: `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_metrics.rs`
- Modify: `crates/trace-commons-server/tests/bakeoff_metrics.rs`

- [ ] **Step 1: Write failing test for `determinism_stddev`**

```rust
#[test]
fn determinism_zero_for_identical_runs() {
    let runs = vec![vec![10.0, 20.0], vec![10.0, 20.0], vec![10.0, 20.0]];
    assert!(bakeoff_metrics::determinism_stddev(&runs) < 1e-12);
}

#[test]
fn determinism_nonzero_when_runs_drift() {
    let runs = vec![vec![10.0, 20.0], vec![10.000001, 20.0], vec![10.0, 20.0]];
    assert!(bakeoff_metrics::determinism_stddev(&runs) > 0.0);
}
```

- [ ] **Step 2: Implement `determinism_stddev`**

```rust
/// Mean per-trace stddev across N repeat runs of the same input.
pub fn determinism_stddev(runs: &[Vec<f64>]) -> f64 {
    if runs.is_empty() || runs[0].is_empty() { return 0.0; }
    let n_traces = runs[0].len();
    let mut total = 0.0;
    for i in 0..n_traces {
        let samples: Vec<f64> = runs.iter().map(|r| r[i]).collect();
        let mean = samples.iter().sum::<f64>() / samples.len() as f64;
        let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / samples.len() as f64;
        total += var.sqrt();
    }
    total / n_traces as f64
}
```

- [ ] **Step 3: Add `ThroughputRecord` and `VramRecord` types** — these are just data wrappers; no algorithm. Test that they round-trip through serde.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThroughputRecord {
    pub tokens_per_second: f64,
    pub total_tokens: u64,
    pub elapsed_seconds: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VramRecord {
    pub peak_mib: u64,
    pub model_mib: u64,
}
```

- [ ] **Step 4: Run tests, confirm pass; commit**

```bash
git commit -m "Add determinism + throughput + VRAM accounting for bake-off"
```

---

## Slice 4 — Report module (decision rule)

### Task 5: Per-candidate result aggregation + weighted score

**Files:**
- Create: `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_report.rs`
- Create: `crates/trace-commons-server/tests/bakeoff_report.rs`

- [ ] **Step 1: Write failing tests for the decision rule**

```rust
#[path = "../src/bin/gate_calibrate/bakeoff_report.rs"]
mod bakeoff_report;
use bakeoff_report::{CandidateResult, pick_winner, weighted_score};

fn result(id: &str, auc: f64, para: f64, tail: f64, throughput: f64, det: f64) -> CandidateResult {
    CandidateResult {
        id: id.into(),
        discrimination_auc: auc,
        paraphrase_delta: para,
        tail_fraction_range: tail,
        determinism_stddev: det,
        throughput_tps: throughput,
        peak_vram_mib: 0,
        license: bakeoff_report::License::Apache2,
        params_b: 8,
        passed_determinism_gate: det < bakeoff_report::DETERMINISM_GATE,
        release_date_unix: 0,
    }
}

#[test]
fn weighted_score_matches_spec_formula() {
    // auc=0.9, para_delta=0.1, tail_range=0.5 → 0.6*0.9 + 0.3*0.9 + 0.1*0.5 = 0.54 + 0.27 + 0.05 = 0.86
    // tail_range needs to be normalized; for this test we feed a normalized tail.
    let r = result("x", 0.9, 0.1, 0.5, 100.0, 1e-7);
    let score = weighted_score(&r, /*tail_norm_max*/ 1.0);
    assert!((score - 0.86).abs() < 1e-9, "score={score}");
}

#[test]
fn determinism_failure_disqualifies() {
    let results = vec![
        result("flaky", 0.99, 0.01, 0.9, 100.0, 1e-3),     // best metrics but fails det gate
        result("solid", 0.80, 0.10, 0.6, 100.0, 1e-7),
    ];
    let winner = pick_winner(&results).expect("winner");
    assert_eq!(winner.id, "solid");
}

#[test]
fn throughput_penalty_applied() {
    // candidate A: fast (1000 tps), score 0.86
    // candidate B: slow (400 tps, > 50% penalty from fastest), score 0.90 — disqualified
    let results = vec![
        result("fast",  0.86, 0.10, 0.6, 1000.0, 1e-7),
        result("slow",  0.90, 0.10, 0.6,  400.0, 1e-7),
    ];
    let winner = pick_winner(&results).expect("winner");
    assert_eq!(winner.id, "fast");
}

#[test]
fn ties_broken_by_license_then_size() {
    use bakeoff_report::License;
    let mut a = result("a", 0.85, 0.10, 0.6, 1000.0, 1e-7);
    a.license = License::LlamaCommunity;
    a.params_b = 8;
    let mut b = result("b", 0.85, 0.10, 0.6, 1000.0, 1e-7);
    b.license = License::Apache2;
    b.params_b = 14;
    // a vs b: scores equal, b's license wins.
    let winner = pick_winner(&vec![a, b]).expect("winner");
    assert_eq!(winner.id, "b");
}

#[test]
fn no_winner_if_all_fail_determinism() {
    let results = vec![
        result("a", 0.9, 0.1, 0.6, 100.0, 1e-3),
        result("b", 0.9, 0.1, 0.6, 100.0, 1e-3),
    ];
    assert!(pick_winner(&results).is_none());
}

#[test]
fn tolerance_band_lets_better_license_win_over_marginal_score_lead() {
    use bakeoff_report::License;
    // a leads by 0.001 (well within TIE_TOLERANCE=0.02) with worse license;
    // b should win on the license tiebreaker.
    let mut a = result("a", 0.851, 0.10, 0.6, 1000.0, 1e-7);
    a.license = License::LlamaCommunity;
    let mut b = result("b", 0.850, 0.10, 0.6, 1000.0, 1e-7);
    b.license = License::Apache2;
    let winner = pick_winner(&vec![a, b]).expect("winner");
    assert_eq!(winner.id, "b");
}

#[test]
fn tolerance_band_does_not_engulf_clearly_better_score() {
    use bakeoff_report::License;
    // a leads by 5% (well outside TIE_TOLERANCE) with worse license;
    // a should win on score — the band shouldn't reach b.
    let mut a = result("a", 0.95, 0.10, 0.6, 1000.0, 1e-7);
    a.license = License::LlamaCommunity;
    let mut b = result("b", 0.90, 0.10, 0.6, 1000.0, 1e-7);
    b.license = License::Apache2;
    let winner = pick_winner(&vec![a, b]).expect("winner");
    assert_eq!(winner.id, "a");
}

#[test]
fn recency_breaks_tie_after_license_and_size_tied() {
    use bakeoff_report::License;
    let mut a = result("a", 0.85, 0.10, 0.6, 1000.0, 1e-7);
    a.license = License::Apache2; a.params_b = 8; a.release_date_unix = 1_700_000_000; // older
    let mut b = result("b", 0.85, 0.10, 0.6, 1000.0, 1e-7);
    b.license = License::Apache2; b.params_b = 8; b.release_date_unix = 1_800_000_000; // newer
    let winner = pick_winner(&vec![a, b]).expect("winner");
    assert_eq!(winner.id, "b");
}
```

- [ ] **Step 2: Run tests, confirm they fail**

- [ ] **Step 3: Implement `bakeoff_report.rs`**

```rust
use serde::{Deserialize, Serialize};

/// Maximum acceptable stddev across repeat runs of the same input.
/// Locked to 1e-5 by the spec; bumped only with a decision_rule_version
/// bump and an updated report schema.
pub const DETERMINISM_GATE: f64 = 1e-5;

/// Throughput floor as a fraction of the fastest candidate's tps. Slower
/// candidates are excluded from contention.
pub const THROUGHPUT_FLOOR_RATIO: f64 = 0.5;

/// Tolerance band around the top weighted score. Candidates within
/// this fraction of the leader are considered tied and proceed to
/// the license/size/recency tiebreakers. Implements the spec's
/// "within 2%" rule.
pub const TIE_TOLERANCE: f64 = 0.02;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum License { Apache2, Mit, LlamaCommunity, GemmaCustom }

impl License {
    fn permissiveness(&self) -> u8 {
        match self {
            License::Apache2 => 4,
            License::Mit => 3,
            License::GemmaCustom => 2,
            License::LlamaCommunity => 1,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CandidateResult {
    pub id: String,
    pub discrimination_auc: f64,
    pub paraphrase_delta: f64,
    pub tail_fraction_range: f64,
    pub determinism_stddev: f64,
    pub throughput_tps: f64,
    pub peak_vram_mib: u64,
    pub license: License,
    pub params_b: u32,
    pub passed_determinism_gate: bool,
    /// Release date of the underlying model weights, unix seconds.
    /// Sourced from the manifest. Used as the third tiebreaker (newer wins).
    pub release_date_unix: i64,
}

pub fn weighted_score(r: &CandidateResult, tail_norm_max: f64) -> f64 {
    let stab = 1.0 - r.paraphrase_delta.min(1.0).max(0.0);
    let tail = if tail_norm_max == 0.0 { 0.0 } else { r.tail_fraction_range / tail_norm_max };
    0.6 * r.discrimination_auc + 0.3 * stab + 0.1 * tail
}

pub fn pick_winner(results: &[CandidateResult]) -> Option<&CandidateResult> {
    let eligible: Vec<&CandidateResult> = results.iter()
        .filter(|r| r.passed_determinism_gate)
        .collect();
    if eligible.is_empty() { return None; }
    let fastest = eligible.iter().map(|r| r.throughput_tps).fold(0.0_f64, f64::max);
    let throughput_floor = THROUGHPUT_FLOOR_RATIO * fastest;
    let in_budget: Vec<&CandidateResult> = eligible.into_iter()
        .filter(|r| r.throughput_tps >= throughput_floor)
        .collect();
    if in_budget.is_empty() { return None; }
    let tail_norm_max = in_budget.iter().map(|r| r.tail_fraction_range).fold(0.0_f64, f64::max);
    let scored: Vec<(&CandidateResult, f64)> = in_budget.iter()
        .map(|r| (*r, weighted_score(r, tail_norm_max.max(1e-12))))
        .collect();
    // Tolerance band: anyone within TIE_TOLERANCE of the leading score is a tie-candidate.
    let top_score = scored.iter().map(|(_, s)| *s).fold(0.0_f64, f64::max);
    let band_floor = top_score * (1.0 - TIE_TOLERANCE);
    let mut contenders: Vec<&(&CandidateResult, f64)> = scored.iter()
        .filter(|(_, s)| *s >= band_floor)
        .collect();
    // Sort the tied set by tiebreakers: license (more permissive wins), then
    // params (smaller wins — leaves headroom for KV cache), then recency
    // (more recent release wins, larger release_date_unix is later).
    contenders.sort_by(|(a, _), (b, _)| {
        b.license.permissiveness().cmp(&a.license.permissiveness())
            .then_with(|| a.params_b.cmp(&b.params_b))
            .then_with(|| b.release_date_unix.cmp(&a.release_date_unix))
    });
    contenders.first().map(|(r, _)| *r)
}
```

Note that `release_date_unix: i64` is added to `CandidateResult` to
implement the spec's third tiebreaker. Source: read from the manifest
(`release_date = "2025-04-29"` per candidate). The corpus builder /
operator populates this when assembling the manifest.

- [ ] **Step 4: Run tests, confirm pass; commit**

```bash
git commit -m "Add bake-off decision rule + winner picking"
```

### Task 6: Report emission (JSON + markdown)

- [ ] **Step 1: Define the `Report` struct in `bakeoff_report.rs`.**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Report {
    pub generated_at: String,           // RFC3339
    pub corpus_sha256: String,          // "sha256:..."
    pub manifest_sha256: String,
    pub candidates: Vec<CandidateResult>,
    pub winner_id: Option<String>,
    pub decision_rule_version: u32,     // bumped if the formula changes
    pub mock_scorer: bool,              // true if --mock-scorer was used; report not valid for decisions
    pub ctx_max_tokens: u32,            // bakeoff_metrics::ctx_for value used
    pub determinism_gate_value: f64,    // DETERMINISM_GATE constant; pinned in the report so future readers see the threshold of the day
}
```

- [ ] **Step 2: Write failing tests for JSON round-trip and markdown.**

```rust
fn fixture_report() -> bakeoff_report::Report {
    bakeoff_report::Report {
        generated_at: "2026-05-13T12:00:00Z".into(),
        corpus_sha256: "sha256:abc".into(),
        manifest_sha256: "sha256:def".into(),
        candidates: vec![result("x", 0.9, 0.1, 0.5, 1000.0, 1e-7)],
        winner_id: Some("x".into()),
        decision_rule_version: 1,
        mock_scorer: false,
        ctx_max_tokens: 4096,
        determinism_gate_value: 1e-5,
    }
}

#[test]
fn report_json_round_trips() {
    let report = fixture_report();
    let json = serde_json::to_string(&report).unwrap();
    let parsed: bakeoff_report::Report = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.winner_id, Some("x".into()));
    assert_eq!(parsed.ctx_max_tokens, 4096);
}

#[test]
fn report_markdown_includes_winner_and_table() {
    let report = fixture_report();
    let md = bakeoff_report::render_markdown(&report);
    assert!(md.contains("Winner: x"));
    assert!(md.contains("| candidate | auc |"));
}

#[test]
fn mock_report_renders_warning_banner() {
    let mut report = fixture_report();
    report.mock_scorer = true;
    let md = bakeoff_report::render_markdown(&report);
    assert!(md.contains("[MOCK SCORER"), "mock reports must visibly self-identify");
    assert!(!md.contains("\u{26A0}"), "no emoji in banner (repo convention)");
}
```

- [ ] **Step 3: Implement `render_markdown` and `write_report`.**

- [ ] **Step 4: Run, confirm pass; commit.**

```bash
git commit -m "Add bake-off report emission (JSON + markdown)"
```

---

## Slice 5 — Runner glue inside the binary

> The existing `trace-commons-gate-calibrate.rs` is env-driven with a flat
> `main()` and no clap subcommand surface today. This slice **first**
> restructures the binary so subcommands are possible, **then** adds the
> `bake-off` subcommand. Doing both at once muddies the diff and makes
> it impossible to assert "back-compat preserved" cleanly.

### Task 7a: Refactor existing binary to clap subcommand structure

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-gate-calibrate.rs`

Existing behavior: `main()` reads `TRACE_COMMONS_CALIBRATE_*` env vars
and runs a single calibration pass. No CLI subcommand.

Target behavior: clap dispatch between two subcommands. `calibrate`
(default if no subcommand given, for back-compat) runs the existing
env-driven body. `bake-off` (added in Task 7b) runs the new flow.

- [ ] **Step 1: Add clap + the `Cli` / `Cmd` enum at the top of the binary.**

```rust
use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "trace-commons-gate-calibrate")]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Run the existing env-driven calibration pass.
    Calibrate,
    // BakeOff added in Task 7b.
}
```

clap is already in this workspace (verify with
`grep '^clap' crates/trace-commons-server/Cargo.toml` — used elsewhere).

- [ ] **Step 2: Move existing `main()` body into a `run_calibrate()` function.**

Everything that's currently in `main()` after env-var loading goes into
`run_calibrate() -> anyhow::Result<()>`. `main()` becomes:

```rust
fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    match cli.cmd.unwrap_or(Cmd::Calibrate) {
        Cmd::Calibrate => run_calibrate(),
    }
}
```

`Cmd::Calibrate` as the default preserves back-compat: existing
invocations that pass no subcommand still work.

- [ ] **Step 3: Add a smoke test for back-compat.**

```rust
// at the bottom of the binary file, in a #[cfg(test)] mod
#[test]
fn calibrate_subcommand_is_default_when_omitted() {
    let cli = Cli::parse_from(["trace-commons-gate-calibrate"]);
    assert!(matches!(cli.cmd, None));
}

#[test]
fn calibrate_subcommand_parses_explicitly() {
    let cli = Cli::parse_from(["trace-commons-gate-calibrate", "calibrate"]);
    assert!(matches!(cli.cmd, Some(Cmd::Calibrate)));
}
```

- [ ] **Step 4: Confirm `cargo check -p trace-commons-server --bin trace-commons-gate-calibrate` is clean and existing tests still pass.**

- [ ] **Step 5: Commit.**

```bash
git commit -m "Refactor trace-commons-gate-calibrate to clap subcommand dispatch"
```

### Task 7b: `bake-off` subcommand wiring

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-gate-calibrate.rs`

This is where the four bake-off modules get wired together. The bulk
of the work moves into Task 7c (`run_candidate_eval`); this task only
adds the CLI surface and the top-level orchestration.

- [ ] **Step 1: Add subcommand to clap parser**

```rust
#[derive(Subcommand)]
enum Cmd {
    Calibrate(CalibrateArgs),     // existing
    BakeOff(BakeOffArgs),         // new
}

#[derive(Args)]
struct BakeOffArgs {
    #[arg(long)]
    candidates: PathBuf,
    #[arg(long)]
    corpus: PathBuf,
    #[arg(long, value_enum)]
    hardware: HardwareTier,
    #[arg(long)]
    report_out: PathBuf,
    #[arg(long, default_value_t = 3)]
    determinism_repeat_runs: u32,
    #[arg(long)]
    skip_models: Option<String>, // comma-separated ids, for resuming
    /// Use the mock scorer instead of loading real model weights.
    /// CI / dry-run use only — disqualifies the report for any real
    /// decision. The emitted report's `mock_scorer` field gets set
    /// to `true` so it can't be confused with a real bake-off.
    #[arg(long, default_value_t = false)]
    mock_scorer: bool,
}
```

- [ ] **Step 2: Wire `run_bakeoff` function**

```rust
async fn run_bakeoff(args: BakeOffArgs) -> anyhow::Result<()> {
    let manifest = bakeoff_manifest::parse_manifest_file(&args.candidates)?;
    let corpus = bakeoff_corpus::load_corpus(&args.corpus)?;
    let manifest_sha = sha256_of_file(&args.candidates)?;
    let corpus_sha = sha256_of_file(&args.corpus)?;

    let mut results: Vec<bakeoff_report::CandidateResult> = Vec::new();
    let skip: std::collections::BTreeSet<String> = args.skip_models
        .as_deref().unwrap_or("").split(',').map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty()).collect();

    for c in &manifest.candidates {
        if skip.contains(&c.id) {
            tracing::info!(candidate_id = %c.id, "bakeoff_skip_candidate");
            continue;
        }
        tracing::info!(candidate_id = %c.id, "bakeoff_load_candidate");
        let scorer = CandlePerplexityScorer::try_new(
            c.id.clone(), c.path.clone(), CandleDeviceKind::Cuda,
            /*tail_logprob_cutoff*/ -8.0, /*max_tokens*/ ctx_for(&c.arch),
        ).await?;

        let result = run_candidate_eval(
            &scorer, c, &corpus, args.determinism_repeat_runs,
        ).await?;
        results.push(result);
        // Drop scorer before next iteration — explicit `drop(scorer)` is fine,
        // but Rust's scope-end suffices; the next loop iteration won't allocate
        // until this one ends.
    }

    let report = bakeoff_report::Report {
        generated_at: chrono::Utc::now().to_rfc3339(),
        corpus_sha256: corpus_sha,
        manifest_sha256: manifest_sha,
        candidates: results.clone(),
        winner_id: bakeoff_report::pick_winner(&results).map(|w| w.id.clone()),
        decision_rule_version: 1,
        mock_scorer: args.mock_scorer,
        ctx_max_tokens: 4096,
        determinism_gate_value: bakeoff_report::DETERMINISM_GATE,
    };

    bakeoff_report::write_report(&report, &args.report_out)?;
    Ok(())
}
```

- [ ] **Step 3: Confirm `cargo check -p trace-commons-server --bins --features local-gpu-models` is clean**

- [ ] **Step 4: Commit**

```bash
git commit -m "Wire bake-off subcommand into trace-commons-gate-calibrate"
```

### Task 7c: `run_candidate_eval` — the load-bearing glue

This task implements the function that takes a loaded scorer and the
corpus and produces a `CandidateResult`. It is the densest single piece
of integration in this plan and gets its own task.

**Files:**
- Modify: `crates/trace-commons-server/src/bin/trace-commons-gate-calibrate.rs`
- Modify: `crates/trace-commons-server/src/bin/gate_calibrate/bakeoff_metrics.rs`
  (add `ctx_for(&CandidateArch) -> usize` helper)

**Signature:**

```rust
async fn run_candidate_eval(
    scorer: &dyn PerplexityScorer,
    candidate: &Candidate,
    corpus: &LoadedCorpus,
    repeat_runs: u32,
) -> anyhow::Result<CandidateResult>;
```

**Concrete contract:**

1. **Per-slice scoring.** For each entry in `corpus.novel`,
   `corpus.duplicate`, and (both halves of) `corpus.paraphrase`, call
   `scorer.score(&entry.as_bytes())` and collect `(perplexity,
   fraction_below_cutoff)`. The mock scorer must implement the same
   trait; the function is generic over the trait.

2. **Throughput.** Wrap the entire scoring loop with
   `std::time::Instant::now()` / `elapsed()`. Total tokens = sum of
   `scorer.score(...)`'s reported `tokens_scored` field (extend
   `PerplexityResult` to include this if it isn't already there;
   pull-request adjusted accordingly). Per-slice throughput is too
   noisy; aggregate is the headline metric.

3. **Determinism.** Pick the first 16 novel entries (or all of them if
   fewer). Run them through the scorer `repeat_runs` times in fresh
   loop iterations. Collect a `Vec<Vec<f64>>` shaped
   `[repeat_runs][n_entries]`. Pass into
   `bakeoff_metrics::determinism_stddev`. Set
   `passed_determinism_gate = stddev < DETERMINISM_GATE` (named
   constant in `bakeoff_report.rs`, value `1e-5`).

4. **VRAM.** On CUDA hardware, query peak GPU memory via shelling
   `nvidia-smi --query-gpu=memory.used --format=csv,noheader,nounits`
   *immediately after model load* (peak), *and* at end-of-eval
   (settled). Both are reported. On non-CUDA hardware (mock scorer,
   CPU dry run), set `peak_vram_mib = 0` and the report renders
   "n/a." Implementation: a small `peak_vram_mib(device:
   &CandleDeviceKind) -> anyhow::Result<u64>` helper that returns 0
   for non-CUDA. Don't introduce an `nvml` crate dep; the shell-out
   is fine for an offline binary.

5. **`ctx_for(&CandidateArch) -> usize`.** Returns the
   `max_tokens` to pass into `CandlePerplexityScorer::try_new`.
   Conservative per-arch values so smaller models don't blow up:

   ```rust
   pub fn ctx_for(arch: &CandidateArch) -> usize {
       match arch {
           CandidateArch::Llama  => 4096,  // Llama-3.1-8B is 128K but the gate caps to 4K for memory predictability
           CandidateArch::Qwen2  => 4096,  // Qwen3-8B / Qwen3.6-27B both >= 32K natively
           CandidateArch::Gemma3 => 4096,  // Gemma-4 is 128K
       }
   }
   ```

   The 4096 cap is intentional — bake-off traces are short, and a
   smaller cap reduces VRAM noise and makes throughput comparable
   across candidates. Document this choice in the report alongside
   the metrics.

6. **Failure handling.** If any single entry fails to score, log the
   failure with `tracing::warn!(candidate_id, entry_index, "score_failed")`
   and skip it (don't abort the candidate). If > 5% of entries fail,
   abort the candidate with an error — the result wouldn't be
   trustworthy. The aborted candidate appears in the report with all
   numeric fields zeroed and `passed_determinism_gate = false`.

7. **`params_b` source.** Add an optional `params_b: Option<u32>` field
   to `Candidate` in the manifest. If absent in the manifest, attempt
   to read the model's `config.json` and infer (Llama / Qwen / Gemma
   configs all have `num_hidden_layers` + `hidden_size` from which
   total params can be approximated). If both manifest and inference
   fail, default to 0 and warn — `params_b` is only a tiebreaker, so
   missing it isn't fatal.

- [ ] **Step 1: Add `tokens_scored` to `PerplexityResult`** (if not
  present) and a `bakeoff_metrics::ctx_for` helper. Test both.

- [ ] **Step 2: Add `peak_vram_mib(&CandleDeviceKind) -> Result<u64>`**
  helper. Returns 0 for non-CUDA. On CUDA, shells out to nvidia-smi.
  Unit-test the non-CUDA branch; the CUDA branch is exercised by the
  real bake-off run.

- [ ] **Step 3: Implement `run_candidate_eval` per the contract above.**
  Unit-tests use the mock scorer; they verify per-slice counts,
  throughput is non-zero, determinism is zero for the mock (since
  the mock is deterministic by construction), and failure-handling
  paths.

- [ ] **Step 4: Add `params_b: Option<u32>` to the manifest schema +
  parser test + scorer-derived fallback.**

- [ ] **Step 5: Confirm `cargo test -p trace-commons-server` is green.**

- [ ] **Step 6: Commit.**

```bash
git commit -m "Implement run_candidate_eval with throughput / determinism / VRAM accounting"
```

---

## Slice 5.5 — Mock scorer (for dry runs + CI)

### Task 7.5: PerplexityScorer trait abstraction + MockPerplexityScorer

This task makes the dry-run path in Task 11 actually function without
real GPU weights. Without it, the "exercise the binary on a laptop"
claim is fiction.

**Files:**
- Create: `crates/trace-commons-server/src/bin/gate_calibrate/mock_scorer.rs`
- Modify: `crates/trace-commons-gate-enclave/src/perplexity.rs` (verify the
  `PerplexityScorer` trait is `pub`; if not, expose it)
- Test: `crates/trace-commons-server/tests/mock_scorer.rs`

**Mock behavior:** deterministic by construction. Hashes the input
bytes with sha256, then maps the hash to a perplexity in `[1.0, 200.0]`
and a tail-fraction in `[0.0, 1.0]`. The same input always produces
the same output (passes the determinism gate trivially). For the
"novel" vs "duplicate" slices to actually exercise the discrimination
AUC, the corpus builder's dry-run mode tags entries by file naming
(`novel-*.txt` vs `dup-*.txt`); the mock scorer peeks at the
filename prefix the corpus loader passes through and biases the
output accordingly. (This is acceptable because the mock scorer is
explicitly marked in the report as "mock" — operators will not
mistake a mock report for a real one.)

- [ ] **Step 1: Confirm `PerplexityScorer` trait is `pub` in
  `trace-commons-gate-enclave`. If not, expose it via an
  intra-workspace re-export.**

- [ ] **Step 2: Write failing tests for `MockPerplexityScorer`.**

```rust
#[test]
fn mock_is_deterministic() {
    let s = MockPerplexityScorer::new();
    let a = s.score(b"hello world");
    let b = s.score(b"hello world");
    assert_eq!(a.perplexity, b.perplexity);
    assert_eq!(a.fraction_below_cutoff, b.fraction_below_cutoff);
}

#[test]
fn mock_biases_by_filename_prefix() {
    let s = MockPerplexityScorer::new();
    let novel = s.score_with_label(b"some text", "novel");
    let dup   = s.score_with_label(b"some text", "duplicate");
    assert!(novel.perplexity > dup.perplexity);
}
```

- [ ] **Step 3: Implement `MockPerplexityScorer`.**

- [ ] **Step 4: Wire `--mock-scorer` into `run_bakeoff` (Task 7b)** so it
  dispatches to `MockPerplexityScorer` instead of
  `CandlePerplexityScorer`.

- [ ] **Step 5: Set `mock_scorer: true` on the emitted `Report`** so
  the report can't be confused with a real one. Add a markdown banner
  at the top of any mock report: `> [MOCK SCORER — NOT VALID FOR
  PRODUCTION DECISIONS]`. The bracketed prefix is intentionally loud;
  no emojis (repo convention).

- [ ] **Step 6: Commit.**

```bash
git commit -m "Add MockPerplexityScorer + --mock-scorer dry-run path"
```

---

## Slice 6 — Corpus builder script

The corpus builder is intentionally split into two tasks: 8a ships the
skeleton + dry-run mode that the binary's CI dry-run depends on (Task
11); 8b implements the real slice download/filter/paraphrase pipeline
the operator runs once on real data. The split is deliberate — 8b is
much more likely to need iteration after first use (HF dataset shapes
change, paraphrase quality needs tuning), and we don't want 8a's CI
dependency held hostage to 8b's churn.

### Task 8a: Corpus builder skeleton + synthetic dry-run

**Files:**
- Create: `scripts/operator/build-bakeoff-corpus.sh` (executable)

Scope: the script's CLI surface, prerequisite checks, output tarball
layout, manifest.json shape, and a fully functional
`BAKEOFF_CORPUS_DRY_RUN=1` path that emits a 6-entry corpus (2 per
slice) using inline fixtures — no HF downloads, no model inference.

- [ ] **Step 1: Write the script skeleton.**

```bash
#!/usr/bin/env bash
set -euo pipefail

OUT="${1:-}"
[ -n "$OUT" ] || { echo "BakeoffCorpusUsage: build-bakeoff-corpus.sh <output.tar.zst>" >&2; exit 1; }

DRY_RUN="${BAKEOFF_CORPUS_DRY_RUN:-0}"

if [ "$DRY_RUN" = "1" ]; then
  build_synthetic_corpus "$OUT"
  exit 0
fi

# Real path — implemented in Task 8b
echo "BakeoffCorpusRealPathNotImplemented: see Task 8b" >&2
exit 1
```

- [ ] **Step 2: Implement `build_synthetic_corpus`.**

Emits a temp directory with novel/ (2 .txt), duplicate/ (2 .txt),
paraphrase/ (1 .jsonl with 2 entries), and manifest.json with per-slice
SHA256. Packs to `.tar.zst`. The fixture text is hard-coded — that's
fine, the dry-run only needs to flow through the corpus loader.

- [ ] **Step 3: Smoke the dry-run path.**

```bash
BAKEOFF_CORPUS_DRY_RUN=1 ./scripts/operator/build-bakeoff-corpus.sh /tmp/dry.tar.zst
file /tmp/dry.tar.zst   # should report zstd archive
```

- [ ] **Step 4: Commit.**

```bash
chmod +x scripts/operator/build-bakeoff-corpus.sh
git add scripts/operator/build-bakeoff-corpus.sh
git commit -m "Add bake-off corpus builder skeleton with synthetic dry-run"
```

### Task 8b: Real corpus assembly (HF datasets + paraphrase)

**Files:**
- Modify: `scripts/operator/build-bakeoff-corpus.sh` — fill in the
  real path
- Create: `scripts/operator/.bakeoff-corpus-checksums` (initially empty,
  populated after first real run)

Scope: the actual data pipeline.

1. Download OASST2 conversations (Hugging Face, gated — operator
   provides `HF_TOKEN`). Bail with a clear instruction if absent.
2. Filter to "novel-reasoning-shaped" entries: trace length 200-2000
   tokens, contains step-by-step reasoning markers.
3. Sample 500.
4. Download GAIA reasoning traces; sample 500.
5. Curate the duplicate slice from public corpora — pull a list of
   common library docstrings (rust-lang/rust, python/cpython), FAQ
   completions, stock boilerplate. Sample 500.
6. For the paraphrase slice: run Qwen3-4B-Base inference
   back-translation on 500 originals from the novel slice. Operator
   needs Qwen3-4B staged locally — script checks and bails with a
   clear instruction if absent. Don't auto-download (model staging is
   its own concern).
7. Emit `manifest.json` with per-slice SHA256.
8. Pack into `bakeoff-corpus-<timestamp>.tar.zst`.
9. Print SHA256 of the tarball. Operator appends to
   `.bakeoff-corpus-checksums` to pin reproducibility.

This task is **expected to need a real iteration on first use.** HF
dataset filenames drift, paraphrase quality needs eyes, and the
"common boilerplate" set is hand-curated. Land the first version and
re-open after the operator's first real run.

- [ ] **Step 1: Implement the OASST2 + GAIA download + filter path.**
- [ ] **Step 2: Implement the duplicate-slice curation.**
- [ ] **Step 3: Implement the paraphrase back-translation step.**
- [ ] **Step 4: Implement manifest emission + tarball packing.**
- [ ] **Step 5: Exercise on real data (operator activity; not CI).**
  Append the resulting tarball SHA256 to `.bakeoff-corpus-checksums`.
- [ ] **Step 6: Commit.**

```bash
git add scripts/operator/build-bakeoff-corpus.sh scripts/operator/.bakeoff-corpus-checksums
git commit -m "Implement real bake-off corpus assembly pipeline"
```

---

## Slice 7 — Operator docs

### Task 9: `calibration.md` gets a "Model bake-off (A2.1)" section

**Files:**
- Modify: `docs/operator/calibration.md`

- [ ] **Step 1: Read the current calibration.md** to find the right
  insertion point (recommend: a new H2 section near the top, before
  the existing per-floor calibration content).

- [ ] **Step 2: Add the section**

Section should cover:
- Pointer to the spec.
- Pointer to `scripts/operator/build-bakeoff-corpus.sh`.
- Command-line examples for the binary.
- Note that the result is a *one-time* decision; once the winner is
  picked and the production default flipped, this section is reference
  only.
- Pointer to the resulting report (path TBD: `docs/superpowers/reports/
  2026-MM-DD-model-bakeoff-result.md`).

- [ ] **Step 3: Commit**

```bash
git commit -m "Document model bake-off in operator calibration runbook"
```

### Task 10: `env-reference.md` placeholder note

- [ ] **Step 1: Add a one-line note next to
  `TRACE_COMMONS_PERPLEXITY_MODEL_ID` that the default is empirically
  determined by the A2.1 bake-off; cite the report once it lands.**

- [ ] **Step 2: Commit**

```bash
git commit -m "Note that the perplexity model default is bake-off-determined"
```

---

## Slice 8 — End-to-end dry run

### Task 11: Build + dry-run on the synthetic corpus

This task is *not* the real bake-off (that's a separate
operator-driven activity per the spec rollout A2.1b). This is just the
"does the binary actually function end-to-end on synthetic inputs"
check, runnable on a laptop without GPU weights.

- [ ] **Step 1: Build the binary**

```bash
cargo build --release -p trace-commons-server --bin trace-commons-gate-calibrate
```

Expected: clean. No `local-gpu-models` feature for the dry run — use
a Mock scorer via a `--mock-scorer` flag if needed, OR exercise the
metric/report/manifest modules independently via their unit tests.

- [ ] **Step 2: Build a synthetic 6-entry corpus**

```bash
BAKEOFF_CORPUS_DRY_RUN=1 ./scripts/operator/build-bakeoff-corpus.sh /tmp/dry-corpus.tar.zst
```

- [ ] **Step 3: Build a 2-candidate manifest pointing at the existing
  mock scorer paths (no real weights)**

- [ ] **Step 4: Run the binary, get a report**

```bash
./target/release/trace-commons-gate-calibrate bake-off \
  --candidates=/tmp/manifest.toml \
  --corpus=/tmp/dry-corpus.tar.zst \
  --hardware=a10 \
  --report-out=/tmp/report.json \
  --mock-scorer
```

- [ ] **Step 5: Sanity-check the report**

```bash
jq . /tmp/report.json
```

Expected: valid JSON, two candidates listed, decision rule applied,
winner field populated.

- [ ] **Step 6: Commit any test-only adjustments needed**

---

## Done criteria

- [ ] `cargo check -p trace-commons-server --bins --features local-gpu-models` is clean.
- [ ] `cargo test -p trace-commons-server --test bakeoff_manifest --test bakeoff_corpus --test bakeoff_metrics --test bakeoff_report` is green.
- [ ] `trace-commons-gate-calibrate bake-off --help` prints reasonable usage.
- [ ] End-to-end dry run produces a valid `report.json` with a populated `winner_id` (Task 11).
- [ ] Operator docs reference the new flow.
- [ ] No production default flipped yet — that PR is rollout step A2.1c per the spec, separate from this implementation work.

---

## What this plan does NOT do

(Recording to head off scope creep — the spec is explicit about these
being deferred to subsequent rollout PRs.)

- Does **not** run the real bake-off. That's operator activity on
  provisioned GPU hardware (spec rollout A2.1b).
- Does **not** flip the production model default. That's a separate
  one-line PR after the report is reviewed (spec rollout A2.1c).
- Does **not** re-calibrate floors. That's a separate pass against
  the winner's perplexity distribution (spec rollout A2.1d).
- Does **not** re-smoke against the chosen model. That's spec rollout
  A2.1e.

Each of those is a separate PR with its own review and own changelog
entry. This plan ships only the bake-off binary + corpus builder +
docs.

## Spec open questions parked here

The spec has five open questions. This plan handles them as follows:

1. **Corpus storage (LFS vs external).** Parked. Task 8b prints the
   tarball SHA256; the operator decides where to store it (git LFS,
   GCS bucket, internal artifact host). Not blocking for this plan.
2. **Paraphrase model strength.** Parked. Task 8b uses Qwen3-4B-Base
   back-translation as the spec recommends; tuning the prompt /
   model is a real-data iteration in the same task.
3. **"Barely better" threshold.** Implemented via the `TIE_TOLERANCE
   = 0.02` constant in `bakeoff_report.rs` (Task 5) — anyone within
   2% of the leading score goes to the license tiebreaker, which
   keeps the incumbent if the alternative isn't materially better.
4. **H100 budget dependency.** Documented in the spec's rollout
   section, not a code concern.
5. **Recalibration cost after the swap.** Out of scope (spec rollout
   A2.1d).
