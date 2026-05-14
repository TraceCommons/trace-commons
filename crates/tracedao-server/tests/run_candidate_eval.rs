//! Integration tests for the bake-off `run_candidate_eval` glue.
//!
//! Pulls the same `gate_calibrate/` submodules used by the binary in via
//! `#[path = ...]` so the test suite doesn't need a separate library.

#[path = "../src/bin/gate_calibrate/bakeoff_corpus.rs"]
mod bakeoff_corpus;
#[path = "../src/bin/gate_calibrate/bakeoff_manifest.rs"]
mod bakeoff_manifest;
#[path = "../src/bin/gate_calibrate/bakeoff_metrics.rs"]
mod bakeoff_metrics;
#[path = "../src/bin/gate_calibrate/bakeoff_report.rs"]
mod bakeoff_report;
#[path = "../src/bin/gate_calibrate/run_candidate_eval.rs"]
mod run_candidate_eval;

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use tracedao_gate_enclave::perplexity::{MockPerplexityScorer, PerplexityResult, PerplexityScorer};

use bakeoff_corpus::{LoadedCorpus, ParaphrasePair};
use bakeoff_manifest::{Candidate, CandidateArch, CandidateLicense};
use run_candidate_eval::{DeviceKind, run_candidate_eval};

fn synth_corpus() -> LoadedCorpus {
    LoadedCorpus {
        novel: vec![
            "Alpha quark fluctuation lattice notebook eta.".to_string(),
            "Beta solenoid retrograde harmonic vector psi.".to_string(),
            "Gamma capstan delineation projector omega.".to_string(),
            "Delta heliotrope axial buoyancy quotient.".to_string(),
            "Epsilon tessellation polynomial drift sigma.".to_string(),
            "Zeta inertial drift cardiogram lambda.".to_string(),
        ],
        duplicate: vec![
            "The quick brown fox jumps over the lazy dog.".to_string(),
            "The quick brown fox jumps over the lazy dog.".to_string(),
            "Lorem ipsum dolor sit amet consectetur.".to_string(),
            "Lorem ipsum dolor sit amet consectetur.".to_string(),
        ],
        paraphrase: vec![
            ParaphrasePair {
                original: "The cat sat on the mat.".to_string(),
                paraphrase: "A feline rested upon the rug.".to_string(),
            },
            ParaphrasePair {
                original: "Sunlight streams through the window.".to_string(),
                paraphrase: "Daylight pours in via the pane.".to_string(),
            },
            ParaphrasePair {
                original: "She wrote a long letter home.".to_string(),
                paraphrase: "A lengthy missive was sent to her family.".to_string(),
            },
            ParaphrasePair {
                original: "The orchard yielded a bountiful crop.".to_string(),
                paraphrase: "Fruit trees produced a generous harvest.".to_string(),
            },
        ],
    }
}

fn synth_candidate() -> Candidate {
    Candidate {
        id: "mock-test-candidate".to_string(),
        path: PathBuf::from("/tmp/nonexistent-mock-path"),
        arch: CandidateArch::Llama,
        license: CandidateLicense::Apache2,
        params_b: Some(7),
        release_date_unix: Some(1_700_000_000),
    }
}

#[tokio::test]
async fn mock_scorer_produces_finite_results() {
    let scorer = MockPerplexityScorer::new();
    let candidate = synth_candidate();
    let corpus = synth_corpus();

    let result = run_candidate_eval(&scorer, &candidate, &corpus, 3, DeviceKind::NonCuda)
        .await
        .expect("mock eval must succeed");

    assert_eq!(result.id, "mock-test-candidate");
    // The mock's hash-derived outputs differ per input, so the AUC across
    // novel vs duplicate is non-trivial — it might be > or < 0.5 depending
    // on the hash, but it must be finite and in range.
    assert!(result.discrimination_auc.is_finite());
    assert!((0.0..=1.0).contains(&result.discrimination_auc));
    assert!(result.paraphrase_delta.is_finite());
    assert!(result.tail_fraction_range.is_finite());
    // Mock is deterministic — repeated scorings of the same input return
    // bitwise-identical micros, but the population-stddev arithmetic can
    // accumulate tiny float noise (sub-1e-10). The gate is 1e-5, so the
    // candidate must pass cleanly; we just don't require an exact zero.
    assert!(
        result.determinism_stddev < 1e-9,
        "determinism stddev for a deterministic mock should be near-zero, got {}",
        result.determinism_stddev
    );
    assert!(result.passed_determinism_gate);
    // tokens_scored is per-input; with multiple inputs the total is > 0.
    assert!(result.throughput_tps > 0.0);
    // NonCuda short-circuits the nvidia-smi call.
    assert_eq!(result.peak_vram_mib, 0);
    // Manifest-supplied values flow through.
    assert_eq!(result.params_b, 7);
    assert_eq!(result.release_date_unix, 1_700_000_000);
}

/// Decorator scorer that fails every third call. Used to verify that the
/// failure-rate guard fires once cumulative failures exceed 5 %.
struct FlakyScorer {
    inner: MockPerplexityScorer,
    calls: AtomicU64,
}

impl FlakyScorer {
    fn new() -> Self {
        Self {
            inner: MockPerplexityScorer::new(),
            calls: AtomicU64::new(0),
        }
    }
}

impl PerplexityScorer for FlakyScorer {
    fn score(&self, plaintext: &[u8]) -> anyhow::Result<PerplexityResult> {
        let n = self.calls.fetch_add(1, Ordering::SeqCst);
        if n % 3 == 0 {
            anyhow::bail!("FlakyScorerInjectedFailure");
        }
        self.inner.score(plaintext)
    }
}

#[tokio::test]
async fn failure_above_5pct_aborts_candidate() {
    let scorer = FlakyScorer::new();
    let candidate = synth_candidate();
    let corpus = synth_corpus();

    let res = run_candidate_eval(&scorer, &candidate, &corpus, 2, DeviceKind::NonCuda).await;
    let err = res.expect_err("flaky scorer must abort");
    let msg = format!("{err}");
    assert!(
        msg.contains("RunCandidateEval"),
        "error must come from the candidate-eval guard, got: {msg}"
    );
    assert!(
        msg.contains("failure rate"),
        "error must mention failure rate, got: {msg}"
    );
}

#[tokio::test]
async fn missing_params_b_defaults_to_zero_without_panicking() {
    // No manifest params_b, path doesn't exist on disk — inference returns
    // None, run_candidate_eval emits a warn and stores 0.
    let scorer = MockPerplexityScorer::new();
    let candidate = Candidate {
        id: "noparams".to_string(),
        path: PathBuf::from("/tmp/nonexistent-noparams"),
        arch: CandidateArch::Qwen2,
        license: CandidateLicense::Mit,
        params_b: None,
        release_date_unix: None,
    };
    let corpus = synth_corpus();
    let result = run_candidate_eval(&scorer, &candidate, &corpus, 2, DeviceKind::NonCuda)
        .await
        .expect("eval must succeed even when params_b is missing");
    assert_eq!(result.params_b, 0);
    assert_eq!(result.release_date_unix, 0);
}
