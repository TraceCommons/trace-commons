// These source modules expose helpers for their own integration targets; this
// target intentionally uses only corpus loading and discrimination AUC.
#[allow(dead_code)]
#[path = "../src/bin/gate_calibrate/bakeoff_corpus.rs"]
mod bakeoff_corpus;
#[allow(dead_code)]
#[path = "../src/bin/gate_calibrate/bakeoff_metrics.rs"]
mod bakeoff_metrics;
#[path = "../src/bin/gate_calibrate/bakeoff_report.rs"]
mod bakeoff_report;

use bakeoff_report::{
    BASELINE_DOMINANCE_MARGIN, BaselineResults, CandidateResult, DETERMINISM_GATE, License,
    NoModelBaseline, pick_winner, weighted_score,
};

fn baselines(strongest_auc: f64) -> BaselineResults {
    BaselineResults {
        measures: vec![NoModelBaseline {
            name: "test_baseline".into(),
            discrimination_auc: strongest_auc,
        }],
        strongest_name: Some("test_baseline".into()),
        strongest_auc,
        required_discrimination_auc: strongest_auc + BASELINE_DOMINANCE_MARGIN,
    }
}

fn result(id: &str, auc: f64, para: f64, tail: f64, throughput: f64, det: f64) -> CandidateResult {
    CandidateResult {
        id: id.into(),
        discrimination_auc: auc,
        paraphrase_delta: para,
        tail_fraction_range: tail,
        determinism_stddev: det,
        throughput_tps: throughput,
        peak_vram_mib: 0,
        license: License::Apache2,
        params_b: 8,
        passed_determinism_gate: det < DETERMINISM_GATE,
        passed_baseline_dominance: false,
        dropped_novel_rows: 0,
        dropped_duplicate_rows: 0,
        release_date_unix: 0,
        load_or_eval_error: None,
        metrics: None,
        per_trace_scores: None,
    }
}

#[test]
fn weighted_score_matches_spec_formula() {
    let r = result("x", 0.9, 0.1, 0.5, 100.0, 1e-7);
    let s = weighted_score(&r, 1.0);
    // 0.6*0.9 + 0.3*(1-0.1) + 0.1*0.5 = 0.54 + 0.27 + 0.05 = 0.86
    assert!((s - 0.86).abs() < 1e-9, "score={s}");
}

#[test]
fn determinism_failure_disqualifies() {
    // Flaky candidate has stellar metrics but fails the determinism gate.
    let flaky = result("flaky", 0.99, 0.01, 1.0, 1000.0, 1e-3);
    // Stable candidate is slower and weaker but passes the gate.
    let stable = result("stable", 0.7, 0.2, 0.4, 600.0, 1e-7);
    let cands = [flaky, stable];
    let winner = pick_winner(&cands, &baselines(0.6)).expect("a winner");
    assert_eq!(winner.id, "stable");
}

#[test]
fn throughput_penalty_applied() {
    // Slow candidate runs at 40 % of fastest → dropped, even with the best score.
    let fast = result("fast", 0.7, 0.2, 0.3, 1000.0, 1e-7);
    let slow_but_strong = result("slow", 0.95, 0.05, 0.5, 400.0, 1e-7);
    let cands = [fast, slow_but_strong];
    let winner = pick_winner(&cands, &baselines(0.6)).expect("a winner");
    assert_eq!(winner.id, "fast");
}

#[test]
fn ties_broken_by_license_then_size() {
    let mut a = result("a", 0.8, 0.1, 0.5, 1000.0, 1e-7);
    let mut b = result("b", 0.8, 0.1, 0.5, 1000.0, 1e-7);
    a.license = License::LlamaCommunity;
    b.license = License::Apache2;
    let cands = [a, b];
    let winner = pick_winner(&cands, &baselines(0.6)).expect("a winner");
    assert_eq!(winner.id, "b");
}

#[test]
fn no_winner_if_all_fail_determinism() {
    let a = result("a", 0.9, 0.1, 0.5, 1000.0, 1e-3);
    let b = result("b", 0.95, 0.1, 0.5, 1000.0, 1e-3);
    let cands = [a, b];
    assert!(pick_winner(&cands, &baselines(0.6)).is_none());
}

#[test]
fn tolerance_band_lets_better_license_win_over_marginal_score_lead() {
    // A leads B by 0.001 in score (well within 2 % tolerance) but B has a
    // strictly better license. Tolerance band should hand the win to B.
    let mut a = result("a", 0.801, 0.1, 0.5, 1000.0, 1e-7);
    let mut b = result("b", 0.8, 0.1, 0.5, 1000.0, 1e-7);
    a.license = License::LlamaCommunity;
    b.license = License::Apache2;
    let cands = [a, b];
    let winner = pick_winner(&cands, &baselines(0.6)).expect("a winner");
    assert_eq!(winner.id, "b");
}

#[test]
fn tolerance_band_does_not_engulf_clearly_better_score() {
    // A leads B by ~5 % — outside the 2 % tolerance — so the license
    // tiebreaker should NOT fire and A wins on raw score.
    let mut a = result("a", 0.9, 0.05, 0.5, 1000.0, 1e-7);
    let mut b = result("b", 0.8, 0.1, 0.5, 1000.0, 1e-7);
    a.license = License::LlamaCommunity;
    b.license = License::Apache2;
    let cands = [a, b];
    let winner = pick_winner(&cands, &baselines(0.6)).expect("a winner");
    assert_eq!(winner.id, "a");
}

#[test]
fn recency_breaks_tie_after_license_and_size_tied() {
    let mut a = result("a", 0.8, 0.1, 0.5, 1000.0, 1e-7);
    let mut b = result("b", 0.8, 0.1, 0.5, 1000.0, 1e-7);
    // Both Apache-2.0, both 8B; only release date differs.
    a.release_date_unix = 1_700_000_000;
    b.release_date_unix = 1_800_000_000;
    let cands = [a, b];
    let winner = pick_winner(&cands, &baselines(0.6)).expect("a winner");
    assert_eq!(winner.id, "b");
}

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
        baselines: baselines(0.8),
        partial: false,
    }
}

#[test]
fn report_json_round_trips() {
    let mut r = fixture_report();
    r.candidates[0].passed_baseline_dominance = true;
    let json = serde_json::to_string(&r).expect("serialize");
    let back: bakeoff_report::Report = serde_json::from_str(&json).expect("parse");
    assert_eq!(back.winner_id.as_deref(), Some("x"));
    assert_eq!(back.ctx_max_tokens, 4096);
    assert_eq!(
        back.baselines.strongest_name.as_deref(),
        Some("test_baseline")
    );
    assert!(back.candidates[0].passed_baseline_dominance);
}

#[test]
fn report_markdown_includes_winner_and_table() {
    let md = bakeoff_report::render_markdown(&fixture_report());
    assert!(md.contains("Winner: x"), "missing winner line: {md}");
    assert!(
        md.contains("| candidate | auc |"),
        "missing table header: {md}"
    );
    assert!(
        md.contains("Strongest baseline: test_baseline (0.800000)"),
        "missing strongest baseline: {md}"
    );
    assert!(
        md.contains("Required discrimination AUC: 0.850000"),
        "missing required floor: {md}"
    );
    assert!(
        md.contains("passed_baseline_dominance"),
        "missing candidate baseline result: {md}"
    );
}

#[test]
fn dropped_baseline_rows_persist_in_json_and_markdown() {
    let mut report = fixture_report();
    report.candidates[0].dropped_novel_rows = 3;
    report.candidates[0].dropped_duplicate_rows = 1;

    let json = serde_json::to_string(&report).expect("serialize report");
    let back: bakeoff_report::Report = serde_json::from_str(&json).expect("parse report");
    assert_eq!(back.candidates[0].dropped_novel_rows, 3);
    assert_eq!(back.candidates[0].dropped_duplicate_rows, 1);

    let md = bakeoff_report::render_markdown(&report);
    assert!(
        md.contains("dropped_novel_rows"),
        "missing novel count: {md}"
    );
    assert!(
        md.contains("dropped_duplicate_rows"),
        "missing duplicate count: {md}"
    );
    assert!(
        md.contains(
            "| x | 0.900000 | 0.100000 | 0.500000 | 1000.000 | 1.000e-7 | Apache2 | 8 | true | 3 | 1 | false |"
        ),
        "missing candidate counts: {md}"
    );
}

#[test]
fn committed_a26_fixture_matches_preregistered_structural_baselines() {
    let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/operator/fixtures/corpus-a26.tar.zst");
    let corpus = bakeoff_corpus::load_corpus(&fixture).expect("load committed A2.6 corpus");
    let baselines = BaselineResults::from_corpus(&corpus.novel, &corpus.duplicate);
    let expected = [
        ("utf8_byte_count", 0.991117),
        ("whitespace_word_count", 0.985883),
        ("line_count", 0.998244),
        ("paragraph_count", 1.0),
    ];

    assert_eq!(baselines.measures.len(), expected.len());
    for (name, expected_auc) in expected {
        let actual = baselines
            .measures
            .iter()
            .find(|baseline| baseline.name == name)
            .unwrap_or_else(|| panic!("missing baseline {name}"))
            .discrimination_auc;
        assert!(
            (actual - expected_auc).abs() < 0.0000005,
            "{name}: expected {expected_auc:.6}, got {actual:.9}"
        );
    }
    assert_eq!(baselines.strongest_name.as_deref(), Some("paragraph_count"));
    assert_eq!(baselines.strongest_auc, 1.0);
    assert_eq!(baselines.required_discrimination_auc, 1.05);
}

#[test]
fn committed_a26_v1_report_deserializes_unchanged() {
    let archived =
        include_str!("../../../docs/superpowers/reports/2026-05-14-model-bakeoff-result-a26.json");
    let report: bakeoff_report::Report = serde_json::from_str(archived).expect("parse A2.6 JSON");
    assert_eq!(report.decision_rule_version, 1);
    assert_eq!(report.candidates.len(), 4);
    assert_eq!(report.winner_id.as_deref(), Some("llama-3.1-8b-instruct"));
    assert!(report.baselines.measures.is_empty());
    assert!(
        report
            .candidates
            .iter()
            .all(|candidate| !candidate.passed_baseline_dominance)
    );
}

#[test]
fn failed_candidate_with_load_error_is_excluded_from_winner() {
    // A candidate row produced by the new failure path:
    // load_or_eval_error = Some(_), passed_determinism_gate = false,
    // all numeric fields zero. pick_winner must not pick it even if it's
    // the only candidate in the list.
    let failed = bakeoff_report::CandidateResult::failed(
        "broken".into(),
        License::Apache2,
        8,
        0,
        "LocalPerplexityScorerLoadFailed",
    );
    assert!(failed.load_or_eval_error.is_some());
    assert!(!failed.passed_determinism_gate);
    assert!(pick_winner(std::slice::from_ref(&failed), &baselines(0.6)).is_none());

    // With a healthy candidate alongside, the healthy one wins.
    let healthy = result("healthy", 0.8, 0.1, 0.5, 1000.0, 1e-7);
    let cands = [failed, healthy];
    let winner = pick_winner(&cands, &baselines(0.6)).expect("a winner");
    assert_eq!(winner.id, "healthy");
}

#[test]
fn failed_candidate_renders_in_markdown_failed_section() {
    let mut r = fixture_report();
    r.candidates.push(bakeoff_report::CandidateResult::failed(
        "broken".into(),
        License::Apache2,
        8,
        0,
        "LocalPerplexityScorerLoadFailed",
    ));
    let md = bakeoff_report::render_markdown(&r);
    assert!(
        md.contains("## Failed candidates"),
        "missing failed section: {md}"
    );
    assert!(
        md.contains("LocalPerplexityScorerLoadFailed"),
        "missing class: {md}"
    );
    assert!(md.contains("broken"), "missing id: {md}");
}

#[test]
fn failed_candidate_json_omits_field_when_none() {
    // When load_or_eval_error is None, the field must be skipped from
    // serialized JSON so existing report consumers don't see a new key.
    let r = result("x", 0.9, 0.1, 0.5, 1000.0, 1e-7);
    let json = serde_json::to_string(&r).unwrap();
    assert!(
        !json.contains("load_or_eval_error"),
        "unexpected key: {json}"
    );
}

#[test]
fn mock_report_renders_warning_banner() {
    let mut r = fixture_report();
    r.mock_scorer = true;
    let md = bakeoff_report::render_markdown(&r);
    assert!(md.contains("[MOCK SCORER"), "missing banner: {md}");
    assert!(
        !md.contains('\u{26A0}'),
        "banner contained the warning-sign emoji"
    );
}

#[test]
fn rarity_block_omitted_from_json_when_none() {
    // Default-mode reports (perplexity-only) must serialize without a
    // `metrics` key so existing report consumers don't see a new field.
    let r = result("x", 0.9, 0.1, 0.5, 1000.0, 1e-7);
    let json = serde_json::to_string(&r).unwrap();
    assert!(
        !json.contains("\"metrics\""),
        "unexpected metrics key: {json}"
    );
}

#[test]
fn rarity_table_renders_in_markdown_when_metrics_present() {
    // When at least one candidate has a rarity metrics block, the markdown
    // gains a "Per-token rarity" section. The legacy table header still
    // appears unchanged.
    let mut r = fixture_report();
    let mut c = result("rare", 0.7, 0.1, 0.4, 900.0, 1e-7);
    c.metrics = Some(bakeoff_report::CandidateMetrics {
        perplexity: Some(bakeoff_report::MetricBlock {
            discrimination_auc: 0.7,
            novel_scores: vec![1.0, 2.0],
            duplicate_scores: vec![0.5, 0.6],
        }),
        token_rarity: Some(bakeoff_report::TokenRarityMetricBlock {
            discrimination_auc: 0.812345,
            novel_scores: vec![3.0, 4.0],
            duplicate_scores: vec![1.5, 1.6],
            k: 12,
        }),
    });
    r.candidates.push(c);
    let md = bakeoff_report::render_markdown(&r);
    // Legacy header preserved.
    assert!(
        md.contains("| candidate | auc |"),
        "legacy header missing: {md}"
    );
    // New section + the rarity row's value + K column.
    assert!(
        md.contains("## Per-token rarity"),
        "missing rarity header: {md}"
    );
    assert!(md.contains("0.812345"), "missing rarity AUC: {md}");
    assert!(md.contains("| rare |"), "missing rarity row: {md}");
    // K is emitted verbatim.
    assert!(md.contains("| 12 |"), "missing K column: {md}");
}

#[test]
fn per_trace_scores_omitted_from_json_when_none() {
    // Default-mode reports (no per-trace block) must serialize without a
    // `per_trace_scores` key so A2.6 / archived report consumers don't see
    // a new field unless the bake-off explicitly populated it.
    let r = result("x", 0.9, 0.1, 0.5, 1000.0, 1e-7);
    let json = serde_json::to_string(&r).unwrap();
    assert!(
        !json.contains("per_trace_scores"),
        "unexpected per_trace_scores key: {json}"
    );
}

#[test]
fn per_trace_scores_round_trips_through_json_with_nulls() {
    // The per-trace block is the canonical wire format for the A2.7
    // calibration consumer; serde must round-trip every slice plus the
    // `None` markers that flag scorer failures (e.g. Gemma 4 31B's OOMs).
    let mut c = result("x", 0.9, 0.1, 0.5, 1000.0, 1e-7);
    c.per_trace_scores = Some(bakeoff_report::PerTraceScores {
        novel: vec![Some(2.5), None, Some(3.0)],
        duplicate: vec![Some(1.0), Some(1.1)],
        paraphrase_original: vec![Some(2.0), None],
        paraphrase_back_translation: vec![Some(2.1), Some(2.2)],
    });
    let json = serde_json::to_string(&c).unwrap();
    // JSON null is the wire form for a failed entry.
    assert!(
        json.contains("\"novel\":[2.5,null,3.0]"),
        "novel slice should serialize None as JSON null: {json}"
    );
    let back: bakeoff_report::CandidateResult = serde_json::from_str(&json).unwrap();
    let pts = back
        .per_trace_scores
        .as_ref()
        .expect("per_trace_scores must round-trip");
    assert_eq!(pts.novel, vec![Some(2.5), None, Some(3.0)]);
    assert_eq!(pts.duplicate, vec![Some(1.0), Some(1.1)]);
    assert_eq!(pts.paraphrase_original, vec![Some(2.0), None]);
    assert_eq!(pts.paraphrase_back_translation, vec![Some(2.1), Some(2.2)]);
}

#[test]
fn rarity_block_round_trips_through_json() {
    // The `metrics.token_rarity` sub-object is the canonical wire format
    // for the new column; serde must round-trip the field names.
    let c = bakeoff_report::CandidateResult {
        id: "x".into(),
        discrimination_auc: 0.7,
        paraphrase_delta: 0.1,
        tail_fraction_range: 0.5,
        determinism_stddev: 1e-7,
        throughput_tps: 100.0,
        peak_vram_mib: 0,
        license: License::Apache2,
        params_b: 8,
        passed_determinism_gate: true,
        passed_baseline_dominance: true,
        dropped_novel_rows: 0,
        dropped_duplicate_rows: 0,
        release_date_unix: 0,
        load_or_eval_error: None,
        metrics: Some(bakeoff_report::CandidateMetrics {
            perplexity: None,
            token_rarity: Some(bakeoff_report::TokenRarityMetricBlock {
                discrimination_auc: 0.65,
                novel_scores: vec![1.0, 2.0],
                duplicate_scores: vec![0.5],
                k: 10,
            }),
        }),
        per_trace_scores: None,
    };
    let json = serde_json::to_string(&c).unwrap();
    let back: bakeoff_report::CandidateResult = serde_json::from_str(&json).unwrap();
    let rarity = back
        .metrics
        .as_ref()
        .and_then(|m| m.token_rarity.as_ref())
        .expect("rarity sub-object must round-trip");
    assert!((rarity.discrimination_auc - 0.65).abs() < 1e-12);
    assert_eq!(rarity.k, 10);
    assert_eq!(rarity.novel_scores, vec![1.0, 2.0]);
    assert_eq!(rarity.duplicate_scores, vec![0.5]);
}
