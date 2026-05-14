//! Integration test for the `tail-floor` subcommand of
//! `tracedao-gate-calibrate`. Drives the pure-data pipeline
//! (`read_sidecar` → `compute_report` → `emit`) against a 100-submission
//! fixture and verifies the proposed floor matches the expected
//! nearest-rank percentile of the mock decision stream.
//!
//! The DB path (`PgDecisionsFetcher`) is exercised indirectly: the trait
//! `DecisionsFetcher` is satisfied by a `BTreeMap` constructed inline,
//! standing in for the real fetcher's output. This mirrors the existing
//! `bakeoff_subcommand.rs` pattern of pulling in submodules via
//! `#[path = ...]` rather than spinning up a real backend.

#[path = "../src/bin/gate_calibrate/tail_floor.rs"]
mod tail_floor;

use std::collections::BTreeMap;

use tail_floor::{SidecarRow, compute_report, emit, read_sidecar};

/// 100-submission fixture: `tail_fraction_micros = (i + 1) * 100`, split
/// evenly across two domain tags ("code" for even i, "math" for odd i).
fn make_fixture() -> (Vec<SidecarRow>, BTreeMap<String, i64>) {
    let mut sidecar = Vec::with_capacity(100);
    let mut decisions = BTreeMap::new();
    for i in 0..100 {
        let id = format!("submission-{i:03}");
        let domain = if i % 2 == 0 { "code" } else { "math" };
        sidecar.push(SidecarRow {
            submission_id: id.clone(),
            source_domain_tag: domain.to_string(),
        });
        decisions.insert(id, ((i + 1) as i64) * 100);
    }
    (sidecar, decisions)
}

#[test]
fn fixture_at_p5_yields_expected_nearest_rank() {
    let (sidecar, decisions) = make_fixture();
    let report = compute_report(&sidecar, &decisions, 5, 0);
    assert_eq!(report.n_submissions, 100);
    assert_eq!(report.n_with_decisions, 100);
    assert_eq!(report.n_without_decisions, 0);
    // Sorted values are 100, 200, …, 10_000. p5 nearest-rank:
    //   rank = ceil(0.05 * 100) = 5 → index 4 → value 500.
    assert_eq!(report.raw_percentile_value_micros, Some(500));
    assert_eq!(report.proposed_floor_micros, Some(500));
    assert_eq!(
        report.env_var_line.as_deref(),
        Some("export TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS=500")
    );
    // Two domains, each with 50 samples.
    assert_eq!(report.per_domain.len(), 2);
    for d in &report.per_domain {
        assert_eq!(d.n_with_decisions, 50);
        assert!(d.percentile_value_micros.is_some());
    }
}

#[test]
fn fixture_with_partial_decisions_counts_missing_correctly() {
    let (sidecar, mut decisions) = make_fixture();
    // Drop the last 30 decisions to simulate submissions that never
    // reached the tail-fraction stage (rejected earlier in the gate).
    for i in 70..100 {
        decisions.remove(&format!("submission-{i:03}"));
    }
    let report = compute_report(&sidecar, &decisions, 5, 0);
    assert_eq!(report.n_submissions, 100);
    assert_eq!(report.n_with_decisions, 70);
    assert_eq!(report.n_without_decisions, 30);
    // Sorted values are 100..=7000 (70 samples). p5 nearest-rank:
    //   rank = ceil(0.05 * 70) = ceil(3.5) = 4 → index 3 → value 400.
    assert_eq!(report.raw_percentile_value_micros, Some(400));
}

#[test]
fn fixture_with_headroom_subtracts_and_propagates_env_line() {
    let (sidecar, decisions) = make_fixture();
    let report = compute_report(&sidecar, &decisions, 5, 150);
    // Raw percentile is 500; with 150 micros of headroom → 350.
    assert_eq!(report.raw_percentile_value_micros, Some(500));
    assert_eq!(report.proposed_floor_micros, Some(350));
    assert_eq!(
        report.env_var_line.as_deref(),
        Some("export TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS=350")
    );
}

#[test]
fn emit_writes_json_report_when_out_path_given() {
    let (sidecar, decisions) = make_fixture();
    let report = compute_report(&sidecar, &decisions, 5, 0);
    let tmp = tempfile::NamedTempFile::new().expect("tmpfile");
    let path = tmp.path().to_path_buf();
    emit(&report, Some(&path)).expect("emit");
    let body = std::fs::read_to_string(&path).expect("read report");
    let parsed: serde_json::Value = serde_json::from_str(&body).expect("parse json");
    assert_eq!(parsed["percentile"].as_u64(), Some(5));
    assert_eq!(parsed["n_submissions"].as_u64(), Some(100));
    assert_eq!(parsed["n_with_decisions"].as_u64(), Some(100));
    assert_eq!(parsed["proposed_floor_micros"].as_i64(), Some(500));
    assert_eq!(
        parsed["env_var_line"].as_str(),
        Some("export TRACE_COMMONS_GATE_TAIL_FRACTION_FLOOR_MICROS=500")
    );
    let per_domain = parsed["per_domain"].as_array().expect("per_domain array");
    assert_eq!(per_domain.len(), 2);
}

#[test]
fn read_sidecar_round_trips_fixture_jsonl() {
    let (sidecar, _) = make_fixture();
    let tmp = tempfile::NamedTempFile::new().expect("tmpfile");
    let path = tmp.path().to_path_buf();
    // Serialize the fixture as JSONL using only the fields read_sidecar
    // consumes; extra keys would be silently ignored by serde anyway.
    let mut body = String::new();
    for row in &sidecar {
        body.push_str(&format!(
            "{{\"submission_id\":\"{}\",\"source_domain_tag\":\"{}\"}}\n",
            row.submission_id, row.source_domain_tag
        ));
    }
    std::fs::write(&path, body).expect("write");
    let parsed = read_sidecar(&path).expect("parse");
    assert_eq!(parsed.len(), 100);
    assert_eq!(parsed[0].submission_id, "submission-000");
    assert_eq!(parsed[99].source_domain_tag, "math");
}
