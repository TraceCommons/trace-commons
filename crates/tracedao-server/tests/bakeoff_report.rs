#[path = "../src/bin/gate_calibrate/bakeoff_report.rs"]
mod bakeoff_report;

use bakeoff_report::{CandidateResult, DETERMINISM_GATE, License, pick_winner, weighted_score};

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
        release_date_unix: 0,
        load_or_eval_error: None,
        metrics: None,
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
    let winner = pick_winner(&cands).expect("a winner");
    assert_eq!(winner.id, "stable");
}

#[test]
fn throughput_penalty_applied() {
    // Slow candidate runs at 40 % of fastest → dropped, even with the best score.
    let fast = result("fast", 0.7, 0.2, 0.3, 1000.0, 1e-7);
    let slow_but_strong = result("slow", 0.95, 0.05, 0.5, 400.0, 1e-7);
    let cands = [fast, slow_but_strong];
    let winner = pick_winner(&cands).expect("a winner");
    assert_eq!(winner.id, "fast");
}

#[test]
fn ties_broken_by_license_then_size() {
    let mut a = result("a", 0.8, 0.1, 0.5, 1000.0, 1e-7);
    let mut b = result("b", 0.8, 0.1, 0.5, 1000.0, 1e-7);
    a.license = License::LlamaCommunity;
    b.license = License::Apache2;
    let cands = [a, b];
    let winner = pick_winner(&cands).expect("a winner");
    assert_eq!(winner.id, "b");
}

#[test]
fn no_winner_if_all_fail_determinism() {
    let a = result("a", 0.9, 0.1, 0.5, 1000.0, 1e-3);
    let b = result("b", 0.95, 0.1, 0.5, 1000.0, 1e-3);
    let cands = [a, b];
    assert!(pick_winner(&cands).is_none());
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
    let winner = pick_winner(&cands).expect("a winner");
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
    let winner = pick_winner(&cands).expect("a winner");
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
    let winner = pick_winner(&cands).expect("a winner");
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
        partial: false,
    }
}

#[test]
fn report_json_round_trips() {
    let r = fixture_report();
    let json = serde_json::to_string(&r).expect("serialize");
    let back: bakeoff_report::Report = serde_json::from_str(&json).expect("parse");
    assert_eq!(back.winner_id.as_deref(), Some("x"));
    assert_eq!(back.ctx_max_tokens, 4096);
}

#[test]
fn report_markdown_includes_winner_and_table() {
    let md = bakeoff_report::render_markdown(&fixture_report());
    assert!(md.contains("Winner: x"), "missing winner line: {md}");
    assert!(
        md.contains("| candidate | auc |"),
        "missing table header: {md}"
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
    assert!(pick_winner(&[failed.clone()]).is_none());

    // With a healthy candidate alongside, the healthy one wins.
    let healthy = result("healthy", 0.8, 0.1, 0.5, 1000.0, 1e-7);
    let cands = [failed, healthy];
    let winner = pick_winner(&cands).expect("a winner");
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
