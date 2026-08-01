//! Integration test for the `bake-off` subcommand of
//! `trace-commons-gate-calibrate`. Builds a synthetic corpus tarball + a
//! 2-candidate manifest, invokes the binary, and validates the produced
//! JSON + markdown reports.
//!
//! No `assert_cmd` dependency — `std::process::Command` plus the
//! `CARGO_BIN_EXE_*` env var (provided by Cargo for binary integration
//! tests) is sufficient and avoids a new dev-dep.

use std::fs;
use std::path::PathBuf;
use std::process::Command;

use sha2::{Digest, Sha256};

fn hex_lower(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        use std::fmt::Write as _;
        write!(&mut s, "{b:02x}").unwrap();
    }
    s
}

/// Build a synthetic corpus tarball matching the contract that
/// bakeoff_corpus::load_corpus accepts. Copy of the helper in
/// tests/bakeoff_corpus.rs, intentionally duplicated to avoid a
/// shared-helper module that all the bake-off test files would need to
/// re-import.
fn build_synthetic_corpus(
    dir: &tempfile::TempDir,
    novel_n: usize,
    dup_n: usize,
    para_n: usize,
) -> PathBuf {
    let novel = (0..novel_n)
        .map(|i| format!("novel-{i}-body"))
        .collect::<Vec<_>>();
    let duplicate = (0..dup_n)
        .map(|i| format!("dup-{i}-body"))
        .collect::<Vec<_>>();
    build_synthetic_corpus_from_rows(dir, &novel, &duplicate, para_n)
}

fn build_synthetic_corpus_from_rows(
    dir: &tempfile::TempDir,
    novel_rows: &[String],
    duplicate_rows: &[String],
    para_n: usize,
) -> PathBuf {
    let staging = dir.path().join("staging");
    fs::create_dir_all(staging.join("novel")).unwrap();
    fs::create_dir_all(staging.join("duplicate")).unwrap();
    fs::create_dir_all(staging.join("paraphrase")).unwrap();

    let mut novel_files: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, body) in novel_rows.iter().enumerate() {
        let name = format!("novel-{i:04}.txt");
        let body = body.as_bytes().to_vec();
        fs::write(staging.join("novel").join(&name), &body).unwrap();
        novel_files.push((name, body));
    }
    let mut dup_files: Vec<(String, Vec<u8>)> = Vec::new();
    for (i, body) in duplicate_rows.iter().enumerate() {
        let name = format!("dup-{i:04}.txt");
        let body = body.as_bytes().to_vec();
        fs::write(staging.join("duplicate").join(&name), &body).unwrap();
        dup_files.push((name, body));
    }

    let mut jsonl = String::new();
    for i in 0..para_n {
        jsonl.push_str(&format!(
            "{{\"original\":\"orig-{i}\",\"paraphrase\":\"para-{i}\"}}\n"
        ));
    }
    fs::write(
        staging.join("paraphrase").join("paraphrase.jsonl"),
        jsonl.as_bytes(),
    )
    .unwrap();

    fn sha_of_concat(files: &[(String, Vec<u8>)]) -> String {
        let mut sorted: Vec<&(String, Vec<u8>)> = files.iter().collect();
        sorted.sort_by(|a, b| a.0.cmp(&b.0));
        let mut h = Sha256::new();
        for (_, body) in sorted {
            h.update(body);
        }
        format!("sha256:{}", hex_lower(h.finalize()))
    }
    let novel_sha = sha_of_concat(&novel_files);
    let dup_sha = sha_of_concat(&dup_files);
    let para_sha = {
        let mut h = Sha256::new();
        h.update(jsonl.as_bytes());
        format!("sha256:{}", hex_lower(h.finalize()))
    };

    let manifest = serde_json::json!({
        "version": 1,
        "novel_sha256": novel_sha,
        "duplicate_sha256": dup_sha,
        "paraphrase_sha256": para_sha,
    });
    fs::write(
        staging.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();

    let tarball = dir.path().join("corpus.tar.zst");
    let out = fs::File::create(&tarball).unwrap();
    let zenc = zstd::Encoder::new(out, 0).unwrap().auto_finish();
    let mut tar = tar::Builder::new(zenc);
    tar.append_dir_all(".", &staging).unwrap();
    tar.finish().unwrap();
    drop(tar);

    tarball
}

fn write_two_candidate_manifest(dir: &tempfile::TempDir) -> PathBuf {
    let path = dir.path().join("candidates.toml");
    let body = r#"
[[candidate]]
id = "llama-3.1-8b-instruct"
path = "/tmp/mock-llama-path"
arch = "llama"
license = "llama-community"
params_b = 8
release_date_unix = 1722470400

[[candidate]]
id = "qwen-2.5-7b"
path = "/tmp/mock-qwen-path"
arch = "qwen2"
license = "apache-2.0"
params_b = 7
release_date_unix = 1728604800
"#;
    fs::write(&path, body).unwrap();
    path
}

#[test]
fn bake_off_subcommand_writes_mock_report() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = build_synthetic_corpus(&dir, 6, 4, 4);
    let manifest = write_two_candidate_manifest(&dir);
    let report_json = dir.path().join("report.json");

    let bin = env!("CARGO_BIN_EXE_trace-commons-gate-calibrate");
    let out = Command::new(bin)
        .arg("bake-off")
        .arg("--candidates")
        .arg(&manifest)
        .arg("--corpus")
        .arg(&corpus)
        .arg("--hardware=cpu")
        .arg("--report-out")
        .arg(&report_json)
        .arg("--mock-scorer")
        .arg("--determinism-repeat-runs=2")
        .output()
        .expect("invoke binary");

    assert!(
        out.status.success(),
        "bake-off must succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let json_bytes = fs::read(&report_json).expect("report.json exists");
    let report: serde_json::Value =
        serde_json::from_slice(&json_bytes).expect("report.json parses as JSON");

    assert_eq!(
        report.get("mock_scorer").and_then(|v| v.as_bool()),
        Some(true)
    );
    // Rule version 3 adds the baseline-dominance floor ahead of throughput.
    // Pinned so a rule change cannot land without updating report consumers.
    assert_eq!(
        report.get("decision_rule_version").and_then(|v| v.as_u64()),
        Some(3)
    );
    let cands = report
        .get("candidates")
        .and_then(|v| v.as_array())
        .expect("candidates array");
    assert_eq!(cands.len(), 2);
    let baselines = report.get("baselines").expect("baselines object");
    assert_eq!(
        baselines.get("strongest_name").and_then(|v| v.as_str()),
        Some("utf8_byte_count")
    );
    assert_eq!(
        baselines
            .get("required_discrimination_auc")
            .and_then(|v| v.as_f64()),
        Some(1.05)
    );
    assert_eq!(
        baselines
            .get("measures")
            .and_then(|v| v.as_array())
            .map(Vec::len),
        Some(4)
    );
    assert!(cands.iter().all(|candidate| {
        candidate
            .get("passed_baseline_dominance")
            .and_then(|v| v.as_bool())
            == Some(false)
    }));
    assert!(
        report
            .get("winner_id")
            .is_some_and(serde_json::Value::is_null)
    );

    // Companion markdown must include the loud mock banner.
    let md_bytes = fs::read(report_json.with_extension("md")).expect("markdown companion exists");
    let md = String::from_utf8(md_bytes).expect("markdown utf8");
    assert!(
        md.contains("[MOCK SCORER"),
        "markdown report must carry the mock banner, got: {md}"
    );
    assert!(md.contains("Winner: "));
    assert!(md.contains("Strongest baseline: utf8_byte_count (1.000000)"));
    assert!(md.contains("Required discrimination AUC: 1.050000"));
    assert!(md.contains("passed_baseline_dominance"));
}

#[test]
fn eligible_candidate_sets_baseline_flag_in_json_and_markdown() {
    let dir = tempfile::tempdir().unwrap();
    let novel = vec!["H".to_string(); 20];
    let duplicate = vec!["D".to_string(); 20];
    let corpus = build_synthetic_corpus_from_rows(&dir, &novel, &duplicate, 20);
    let manifest = write_two_candidate_manifest(&dir);
    let report_json = dir.path().join("eligible-report.json");

    let bin = env!("CARGO_BIN_EXE_trace-commons-gate-calibrate");
    let out = Command::new(bin)
        .arg("bake-off")
        .arg("--candidates")
        .arg(&manifest)
        .arg("--corpus")
        .arg(&corpus)
        .arg("--hardware=cpu")
        .arg("--report-out")
        .arg(&report_json)
        .arg("--mock-scorer")
        .arg("--determinism-repeat-runs=2")
        .output()
        .expect("invoke binary");
    assert!(
        out.status.success(),
        "bake-off must succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_json).unwrap()).unwrap();
    let baselines = report.get("baselines").expect("baselines object");
    assert_eq!(
        baselines.get("strongest_auc").and_then(|v| v.as_f64()),
        Some(0.5)
    );
    assert_eq!(
        baselines
            .get("required_discrimination_auc")
            .and_then(|v| v.as_f64()),
        Some(0.55)
    );
    let candidates = report
        .get("candidates")
        .and_then(|v| v.as_array())
        .expect("candidates array");
    let json_passed = candidates.iter().all(|candidate| {
        candidate
            .get("passed_baseline_dominance")
            .and_then(|v| v.as_bool())
            == Some(true)
            && candidate.get("dropped_novel_rows").and_then(|v| v.as_u64()) == Some(0)
            && candidate
                .get("dropped_duplicate_rows")
                .and_then(|v| v.as_u64())
                == Some(0)
    });
    assert!(report.get("winner_id").is_some_and(|v| !v.is_null()));

    let md = fs::read_to_string(report_json.with_extension("md")).unwrap();
    let markdown_passed = candidates.iter().all(|candidate| {
        let id = candidate.get("id").and_then(|v| v.as_str()).unwrap();
        md.lines()
            .find(|line| line.starts_with(&format!("| {id} |")))
            .is_some_and(|line| line.ends_with("| 0 | 0 | true |"))
    });
    assert!(
        json_passed && markdown_passed,
        "eligible candidates must persist true in JSON and markdown: {md}"
    );
}

// The third real-scorer-path test that would exercise the
// `#[cfg(feature = "local-gpu-models")]` branch requires actual candle
// model weights on a CUDA host; it is out of scope for hermetic CI and
// is exercised manually by the operator on the GPU host during Phase 0.
// The two tests below cover the negative paths (no feature flag; CPU
// hardware) which are reachable in default-feature CI.

#[test]
fn real_scorer_path_without_feature_bails_clearly() {
    // Without `--mock-scorer` and without the `local-gpu-models` feature
    // (the default-features build that CI uses), the binary must refuse
    // with a named error class so operators get a self-explanatory
    // diagnostic instead of a generic load failure deeper in the candle
    // stack. `--hardware=h100` is used so the CPU short-circuit does not
    // fire first.
    let dir = tempfile::tempdir().unwrap();
    let corpus = build_synthetic_corpus(&dir, 4, 4, 4);
    let manifest = write_two_candidate_manifest(&dir);
    let report_json = dir.path().join("report.json");

    let bin = env!("CARGO_BIN_EXE_trace-commons-gate-calibrate");
    let out = Command::new(bin)
        .arg("bake-off")
        .arg("--candidates")
        .arg(&manifest)
        .arg("--corpus")
        .arg(&corpus)
        .arg("--hardware=h100")
        .arg("--report-out")
        .arg(&report_json)
        .arg("--determinism-repeat-runs=2")
        // intentionally NO --mock-scorer
        .output()
        .expect("invoke binary");

    assert!(
        !out.status.success(),
        "bake-off without --mock-scorer and without local-gpu-models must fail"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("BakeoffRealScorerRequiresFeature"),
        "stderr should name BakeoffRealScorerRequiresFeature; got: {stderr}"
    );
}

#[test]
fn cpu_hardware_with_real_scorer_bails_clearly() {
    // `--hardware=cpu` paired with a real scorer is unsupported: the
    // candle Llama loader needs CUDA at any reasonable model size, so
    // we refuse early with a named error class.
    let dir = tempfile::tempdir().unwrap();
    let corpus = build_synthetic_corpus(&dir, 4, 4, 4);
    let manifest = write_two_candidate_manifest(&dir);
    let report_json = dir.path().join("report.json");

    let bin = env!("CARGO_BIN_EXE_trace-commons-gate-calibrate");
    let out = Command::new(bin)
        .arg("bake-off")
        .arg("--candidates")
        .arg(&manifest)
        .arg("--corpus")
        .arg(&corpus)
        .arg("--hardware=cpu")
        .arg("--report-out")
        .arg(&report_json)
        .arg("--determinism-repeat-runs=2")
        // intentionally NO --mock-scorer
        .output()
        .expect("invoke binary");

    assert!(
        !out.status.success(),
        "bake-off with --hardware=cpu and without --mock-scorer must fail"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("BakeoffCpuRequiresMockScorer"),
        "stderr should name BakeoffCpuRequiresMockScorer; got: {stderr}"
    );
}

#[test]
fn bake_off_writes_incremental_report_after_each_candidate() {
    // With --mock-scorer, both candidates succeed; the final report on
    // disk should have partial=false and both candidates present. We
    // cannot easily test the *incremental* mid-run write without racy
    // timing, but this exercises the new write-after-each-candidate code
    // path and confirms the partial flag flips to false on the final
    // write.
    let dir = tempfile::tempdir().unwrap();
    let corpus = build_synthetic_corpus(&dir, 6, 4, 4);
    let manifest = write_two_candidate_manifest(&dir);
    let report_json = dir.path().join("report.json");

    let bin = env!("CARGO_BIN_EXE_trace-commons-gate-calibrate");
    let out = Command::new(bin)
        .arg("bake-off")
        .arg("--candidates")
        .arg(&manifest)
        .arg("--corpus")
        .arg(&corpus)
        .arg("--hardware=cpu")
        .arg("--report-out")
        .arg(&report_json)
        .arg("--mock-scorer")
        .arg("--determinism-repeat-runs=2")
        .output()
        .expect("invoke binary");

    assert!(
        out.status.success(),
        "bake-off must succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );

    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_json).unwrap()).unwrap();
    assert_eq!(
        report.get("partial").and_then(|v| v.as_bool()),
        Some(false),
        "final report must have partial=false"
    );
    let cands = report.get("candidates").and_then(|v| v.as_array()).unwrap();
    assert_eq!(cands.len(), 2);
    // No tmp file should be left behind.
    let tmp_path = report_json.with_extension("json.tmp");
    assert!(!tmp_path.exists(), "stray tmp file: {}", tmp_path.display());
}

#[test]
fn bake_off_subcommand_respects_skip_models() {
    let dir = tempfile::tempdir().unwrap();
    let corpus = build_synthetic_corpus(&dir, 4, 4, 4);
    let manifest = write_two_candidate_manifest(&dir);
    let report_json = dir.path().join("report.json");

    let bin = env!("CARGO_BIN_EXE_trace-commons-gate-calibrate");
    let out = Command::new(bin)
        .arg("bake-off")
        .arg("--candidates")
        .arg(&manifest)
        .arg("--corpus")
        .arg(&corpus)
        .arg("--hardware=cpu")
        .arg("--report-out")
        .arg(&report_json)
        .arg("--mock-scorer")
        .arg("--determinism-repeat-runs=2")
        .arg("--skip-models=qwen-2.5-7b")
        .output()
        .expect("invoke binary");

    assert!(
        out.status.success(),
        "skip must succeed; stderr={}",
        String::from_utf8_lossy(&out.stderr)
    );
    let report: serde_json::Value =
        serde_json::from_slice(&fs::read(&report_json).unwrap()).unwrap();
    let cands = report.get("candidates").unwrap().as_array().unwrap();
    assert_eq!(cands.len(), 1);
    assert_eq!(
        cands[0].get("id").unwrap().as_str().unwrap(),
        "llama-3.1-8b-instruct"
    );
}
