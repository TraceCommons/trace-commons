// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Trivial-measure battery (#204, sub-project B of the gate-validity program).
//!
//! The battery exists because the A2.6 corpus was separable by a single
//! integer and nothing in the repository would have said so. These tests do
//! two jobs: pin the six measure definitions, and pin the battery's verdict on
//! the known-bad `corpus-a26.tar.zst` fixture. A battery that cannot reproduce
//! a known-bad corpus's failure is not evidence about a good one, so the
//! fixture case is a permanent regression test rather than a one-off audit.

// The `#[path]` modules below are compiled whole into this test target, so
// every helper they carry for the gate-calibrate binary reads as dead code
// here. CI builds with `-D warnings`, so silence it at the target rather than
// scattering per-item allows through shared modules.
#![allow(dead_code)]

#[path = "../src/bin/gate_calibrate/bakeoff_corpus.rs"]
mod bakeoff_corpus;
#[path = "../src/bin/gate_calibrate/bakeoff_metrics.rs"]
mod bakeoff_metrics;
#[path = "../src/bin/gate_calibrate/trivial_measures.rs"]
mod trivial_measures;

use std::path::PathBuf;

use trivial_measures::{
    DEFAULT_CEILING, TRIVIAL_MEASURES, distinct_word_count, line_count, mean_word_length,
    paragraph_count, run_battery, utf8_byte_count, whitespace_word_count,
};

fn a26_fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../scripts/operator/fixtures/corpus-a26.tar.zst")
}

// --- measure definitions --------------------------------------------------

#[test]
fn paragraph_count_splits_on_blank_lines_and_ignores_empty_blocks() {
    assert_eq!(paragraph_count("one block only"), 1.0);
    assert_eq!(paragraph_count("a\nb\nc"), 1.0);
    assert_eq!(paragraph_count("a\n\nb"), 2.0);
    assert_eq!(paragraph_count("a\n\n\n\nb"), 2.0);
    assert_eq!(paragraph_count(""), 0.0);
}

#[test]
fn line_count_counts_newline_separated_lines() {
    assert_eq!(line_count(""), 0.0);
    assert_eq!(line_count("a"), 1.0);
    assert_eq!(line_count("a\nb"), 2.0);
    assert_eq!(line_count("a\nb\n"), 2.0);
}

#[test]
fn distinct_word_count_is_case_sensitive_whitespace_split() {
    assert_eq!(distinct_word_count("a b a b c"), 3.0);
    assert_eq!(distinct_word_count("Word word"), 2.0);
    assert_eq!(distinct_word_count("   "), 0.0);
}

#[test]
fn utf8_byte_count_counts_bytes_not_chars() {
    assert_eq!(utf8_byte_count("abc"), 3.0);
    assert_eq!(utf8_byte_count("é"), 2.0);
}

#[test]
fn whitespace_word_count_counts_tokens() {
    assert_eq!(whitespace_word_count("a  b\tc\nd"), 4.0);
    assert_eq!(whitespace_word_count(""), 0.0);
}

#[test]
fn mean_word_length_is_chars_per_token_and_zero_when_empty() {
    assert_eq!(mean_word_length("ab cd"), 2.0);
    assert_eq!(mean_word_length("a bcd"), 2.0);
    assert_eq!(mean_word_length("   "), 0.0);
}

#[test]
fn the_battery_is_exactly_the_six_preregistered_measures() {
    let names: Vec<&str> = TRIVIAL_MEASURES.iter().map(|m| m.name).collect();
    assert_eq!(
        names,
        vec![
            "paragraph_count",
            "line_count",
            "distinct_word_count",
            "utf8_byte_count",
            "whitespace_word_count",
            "mean_word_length",
        ]
    );
}

// --- battery verdicts -----------------------------------------------------

#[test]
fn identical_slices_are_admissible_at_auc_one_half() {
    let a = vec!["one\n\ntwo".to_string(), "three\n\nfour".to_string()];
    let outcome = run_battery("self", &a, &a, DEFAULT_CEILING);
    for m in &outcome.measures {
        assert!(
            (m.auc - 0.5).abs() < 1e-9,
            "{} auc={} expected 0.5",
            m.measure,
            m.auc
        );
    }
    assert!(outcome.admissible, "identical slices must be admissible");
}

#[test]
fn empty_slices_do_not_certify_a_corpus() {
    // `discrimination_auc` returns 0.5 for empty inputs — no information, no
    // preference. That must not read as "admissible": an empty slice is a
    // corpus defect, not a passing corpus.
    let outcome = run_battery("empty", &[], &[], DEFAULT_CEILING);
    assert!(!outcome.admissible, "empty slices must not be admissible");
}

#[test]
fn the_battery_reproduces_the_a26_corpus_failure_reported_in_204() {
    // Issue #204's table, recomputed with the repository's own tie
    // convention. If these move, either the measures changed or the fixture
    // did; both are findings, neither is a test to relax.
    let corpus = bakeoff_corpus::load_corpus(&a26_fixture()).expect("load a26 fixture");
    assert_eq!(corpus.novel.len(), 300);
    assert_eq!(corpus.duplicate.len(), 300);

    let outcome = run_battery(
        "novel_vs_duplicate",
        &corpus.novel,
        &corpus.duplicate,
        DEFAULT_CEILING,
    );

    let expected = [
        ("paragraph_count", 1.000000_f64),
        ("line_count", 0.998244),
        ("distinct_word_count", 0.993128),
        ("utf8_byte_count", 0.991117),
        ("whitespace_word_count", 0.985883),
        ("mean_word_length", 0.983622),
    ];
    for (name, want) in expected {
        let got = outcome
            .measures
            .iter()
            .find(|m| m.measure == name)
            .unwrap_or_else(|| panic!("measure {name} missing from battery"));
        assert!(
            (got.auc - want).abs() < 1e-6,
            "{name}: got {} want {want}",
            got.auc
        );
    }

    // Every one of the 300 duplicate files has exactly one paragraph.
    let paragraphs = outcome
        .measures
        .iter()
        .find(|m| m.measure == "paragraph_count")
        .expect("paragraph_count");
    assert_eq!(paragraphs.duplicate_min, 1.0);
    assert_eq!(paragraphs.duplicate_max, 1.0);
    assert_eq!(paragraphs.novel_min, 7.0);
    assert_eq!(paragraphs.novel_max, 163.0);

    assert!(
        !outcome.admissible,
        "the A2.6 corpus must be reported inadmissible"
    );
    assert_eq!(outcome.worst_measure.as_deref(), Some("paragraph_count"));
}

#[test]
fn the_a26_paraphrase_pair_is_also_inadmissible_on_length_alone() {
    // #204: holding the source constant removes the format confound and
    // leaves a length confound just as strong. 299 of 300 paraphrases are
    // shorter than their original.
    let corpus = bakeoff_corpus::load_corpus(&a26_fixture()).expect("load a26 fixture");
    let originals: Vec<String> = corpus
        .paraphrase
        .iter()
        .map(|p| p.original.clone())
        .collect();
    let paraphrases: Vec<String> = corpus
        .paraphrase
        .iter()
        .map(|p| p.paraphrase.clone())
        .collect();
    let outcome = run_battery(
        "original_vs_paraphrase",
        &originals,
        &paraphrases,
        DEFAULT_CEILING,
    );
    let bytes = outcome
        .measures
        .iter()
        .find(|m| m.measure == "utf8_byte_count")
        .expect("utf8_byte_count");
    assert!(
        (bytes.auc - 0.996106).abs() < 1e-6,
        "byte-count auc={} expected 0.996106",
        bytes.auc
    );
    assert!(!outcome.admissible);
}

#[test]
fn the_length_covariate_is_reported_alongside_every_battery_result() {
    // Methodology guardrail from #199: report a length covariate beside every
    // result, so a score that merely tracks length is visible as such.
    let corpus = bakeoff_corpus::load_corpus(&a26_fixture()).expect("load a26 fixture");
    let outcome = run_battery(
        "novel_vs_duplicate",
        &corpus.novel,
        &corpus.duplicate,
        DEFAULT_CEILING,
    );
    let bytes = outcome
        .measures
        .iter()
        .find(|m| m.measure == "utf8_byte_count")
        .expect("utf8_byte_count");
    assert_eq!(outcome.length_covariate_auc, bytes.auc);
}
