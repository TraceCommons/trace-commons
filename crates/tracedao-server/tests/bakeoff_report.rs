#[path = "../src/bin/gate_calibrate/bakeoff_report.rs"]
mod bakeoff_report;

use bakeoff_report::{pick_winner, weighted_score, CandidateResult, License, DETERMINISM_GATE};

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

