// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

#[path = "../src/bin/gate_calibrate/bakeoff_metrics.rs"]
mod bakeoff_metrics;

use bakeoff_metrics::{
    ThroughputRecord, VramRecord, discrimination_auc, paraphrase_delta, tail_fraction_range,
};

#[test]
fn perfect_separation_gives_auc_one() {
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
    // Pairs: (5,5)=tie -> 0.5, (5,1)=win, (10,5)=win, (10,1)=win
    // AUC = (0.5 + 1 + 1 + 1) / 4 = 0.875
    let novel = vec![5.0, 10.0];
    let duplicate = vec![5.0, 1.0];
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
    // Median across {0.1, 0.2, 0.3} = 0.2.
    let pairs = vec![(10.0, 12.0), (20.0, 22.0), (30.0, 39.0)];
    assert!((paraphrase_delta(&pairs) - 0.2).abs() < 1e-9);
}

#[test]
fn tail_fraction_range_measures_spread() {
    // |median(duplicate) - median(novel)| = |0.70 - 0.10| = 0.60
    let novel_frac = vec![0.10, 0.12, 0.08];
    let duplicate_frac = vec![0.70, 0.72, 0.68];
    let range = tail_fraction_range(&novel_frac, &duplicate_frac);
    assert!((range - 0.60).abs() < 1e-9, "range={range}");
}

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

#[test]
fn throughput_record_round_trips_serde() {
    let r = ThroughputRecord {
        tokens_per_second: 1234.5,
        total_tokens: 9876,
        elapsed_seconds: 8.0,
    };
    let j = serde_json::to_string(&r).expect("serialize");
    let back: ThroughputRecord = serde_json::from_str(&j).expect("deserialize");
    assert!((back.tokens_per_second - 1234.5).abs() < 1e-9);
    assert_eq!(back.total_tokens, 9876);
    assert!((back.elapsed_seconds - 8.0).abs() < 1e-9);
}

#[test]
fn vram_record_round_trips_serde() {
    let r = VramRecord {
        peak_mib: 18432,
        model_mib: 14000,
    };
    let j = serde_json::to_string(&r).expect("serialize");
    let back: VramRecord = serde_json::from_str(&j).expect("deserialize");
    assert_eq!(back.peak_mib, 18432);
    assert_eq!(back.model_mib, 14000);
}
