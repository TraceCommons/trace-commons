// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The redacted-summary similarity `main`'s submit-time duplicate precheck
//! scores (`build_derived_precheck` in `trace-commons-ingest.rs`), shared
//! with the versioned pipeline, which applies `main`'s skip-duplicate rule
//! to a compatibility run's gate decision row from the same score.
//!
//! A canonical summary is `canonical_summary_for_embedding` of an envelope.
//! Two summaries with the same hash score 1.0; otherwise the score is the
//! larger of a signed feature-hash embedding's cosine and the token sets'
//! Jaccard index. A candidate below [`TRACE_SIMILARITY_NEIGHBOR_THRESHOLD`]
//! is not a neighbour, so the duplicate score is 0.0 when none reaches it.

use std::collections::BTreeSet;

use sha2::{Digest, Sha256};

/// The score at or above which a candidate is a neighbour.
pub const TRACE_SIMILARITY_NEIGHBOR_THRESHOLD: f32 = 0.25;

/// The redacted-summary embedding's dimension.
pub const TRACE_LOCAL_REDACTED_SUMMARY_EMBEDDING_DIMENSION: usize = 64;

/// A summary's tokens and redacted-summary embedding, computed once so a
/// pass that compares one summary against many does not re-tokenize and
/// re-hash both sides of every comparison. Scoring from features is
/// bit-identical to scoring from the strings.
#[derive(Debug, Clone)]
pub struct TraceSummaryFeatures {
    tokens: BTreeSet<String>,
    embedding: Vec<f32>,
}

impl TraceSummaryFeatures {
    pub fn new(summary: &str) -> Self {
        let tokens = trace_similarity_tokens(summary);
        let embedding = trace_redacted_summary_embedding_from_tokens(&tokens);
        Self { tokens, embedding }
    }
}

/// `main`'s duplicate score of `target_summary` against `candidates`
/// (summary, summary hash): the best neighbour's score, or 0.0 with no
/// neighbour. `build_derived_precheck` stores this as the derived record's
/// `duplicate_score`.
pub fn trace_summary_duplicate_score<'a>(
    target_summary: &str,
    target_hash: &str,
    candidates: impl IntoIterator<Item = (Option<&'a str>, Option<&'a str>)>,
) -> f32 {
    let target = TraceSummaryFeatures::new(target_summary);
    candidates
        .into_iter()
        .map(|(summary, hash)| {
            let candidate = match hash {
                Some(hash) if hash == target_hash => None,
                _ => summary.map(TraceSummaryFeatures::new),
            };
            trace_summary_features_similarity_score(&target, target_hash, candidate.as_ref(), hash)
        })
        .filter(|score| *score >= TRACE_SIMILARITY_NEIGHBOR_THRESHOLD)
        .fold(0.0f32, f32::max)
}

pub fn trace_summary_similarity_score(
    target_summary: &str,
    target_hash: &str,
    candidate_summary: Option<&str>,
    candidate_hash: Option<&str>,
) -> f32 {
    if candidate_hash.is_some_and(|hash| hash == target_hash) {
        return 1.0;
    }
    let Some(candidate_summary) = candidate_summary else {
        return 0.0;
    };
    trace_summary_features_similarity_score(
        &TraceSummaryFeatures::new(target_summary),
        target_hash,
        Some(&TraceSummaryFeatures::new(candidate_summary)),
        candidate_hash,
    )
}

/// [`trace_summary_similarity_score`] over precomputed features: 1.0 on a
/// matching hash, 0.0 with no candidate summary, otherwise the larger of the
/// embeddings' cosine and the token sets' Jaccard index.
pub fn trace_summary_features_similarity_score(
    target: &TraceSummaryFeatures,
    target_hash: &str,
    candidate: Option<&TraceSummaryFeatures>,
    candidate_hash: Option<&str>,
) -> f32 {
    if candidate_hash.is_some_and(|hash| hash == target_hash) {
        return 1.0;
    }
    let Some(candidate) = candidate else {
        return 0.0;
    };
    trace_summary_embedding_similarity(&target.embedding, &candidate.embedding).max(
        trace_summary_token_similarity(&target.tokens, &candidate.tokens),
    )
}

fn trace_summary_token_similarity(left: &BTreeSet<String>, right: &BTreeSet<String>) -> f32 {
    if left.is_empty() || right.is_empty() {
        return 0.0;
    }
    let intersection = left.intersection(right).count();
    let union = left.len() + right.len() - intersection;
    if union == 0 {
        0.0
    } else {
        (intersection as f32 / union as f32).clamp(0.0, 1.0)
    }
}

pub fn trace_redacted_summary_embedding(input: &str) -> Vec<f32> {
    trace_redacted_summary_embedding_from_tokens(&trace_similarity_tokens(input))
}

fn trace_redacted_summary_embedding_from_tokens(tokens: &BTreeSet<String>) -> Vec<f32> {
    let mut values = vec![0.0f32; TRACE_LOCAL_REDACTED_SUMMARY_EMBEDDING_DIMENSION];
    for token in tokens {
        let digest = Sha256::digest(token.as_bytes());
        let mut index_bytes = [0u8; 8];
        index_bytes.copy_from_slice(&digest[..8]);
        let index = (u64::from_le_bytes(index_bytes) as usize)
            % TRACE_LOCAL_REDACTED_SUMMARY_EMBEDDING_DIMENSION;
        let sign = if digest[8] & 1 == 0 { 1.0 } else { -1.0 };
        values[index] += sign;
    }
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if norm > 0.0 {
        for value in &mut values {
            *value /= norm;
        }
    }
    values
}

pub fn trace_summary_embedding_similarity(left: &[f32], right: &[f32]) -> f32 {
    if left.is_empty() || right.is_empty() || left.len() != right.len() {
        return 0.0;
    }
    left.iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum::<f32>()
        .clamp(0.0, 1.0)
}

fn trace_similarity_tokens(input: &str) -> BTreeSet<String> {
    const STOP_WORDS: &[&str] = &[
        "and", "are", "for", "from", "that", "the", "this", "trace", "with",
    ];

    let mut tokens = BTreeSet::new();
    let mut current = String::new();
    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            current.push(ch.to_ascii_lowercase());
        } else {
            push_trace_similarity_token(&mut tokens, &mut current, STOP_WORDS);
        }
    }
    push_trace_similarity_token(&mut tokens, &mut current, STOP_WORDS);
    tokens
}

fn push_trace_similarity_token(
    tokens: &mut BTreeSet<String>,
    current: &mut String,
    stop_words: &[&str],
) {
    if current.len() > 1 && !stop_words.contains(&current.as_str()) {
        tokens.insert(std::mem::take(current));
    } else {
        current.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The string-based scorer as it stood before features were
    /// precomputed (5a85898d1), verbatim, as the reference the features
    /// path must match bit for bit.
    fn reference_score(
        target_summary: &str,
        target_hash: &str,
        candidate_summary: Option<&str>,
        candidate_hash: Option<&str>,
    ) -> f32 {
        if candidate_hash.is_some_and(|hash| hash == target_hash) {
            return 1.0;
        }
        let Some(candidate_summary) = candidate_summary else {
            return 0.0;
        };
        let target_embedding = trace_redacted_summary_embedding(target_summary);
        let candidate_embedding = trace_redacted_summary_embedding(candidate_summary);
        let left_tokens = trace_similarity_tokens(target_summary);
        let right_tokens = trace_similarity_tokens(candidate_summary);
        let token_similarity = if left_tokens.is_empty() || right_tokens.is_empty() {
            0.0
        } else {
            let intersection = left_tokens.intersection(&right_tokens).count() as f32;
            let union = left_tokens.union(&right_tokens).count() as f32;
            if union == 0.0 {
                0.0
            } else {
                (intersection / union).clamp(0.0, 1.0)
            }
        };
        trace_summary_embedding_similarity(&target_embedding, &candidate_embedding)
            .max(token_similarity)
    }

    #[test]
    fn features_score_matches_the_string_scorer_bit_for_bit() {
        let summaries = [
            "",
            "a",
            "the and for",
            "Task type: calendar scheduling timezone retry",
            "Task type: calendar scheduling timezone ambiguity retry",
            "tool call retry calendar auth failure",
            "tool call retry calendar auth",
            "database migration checksum repair",
            "Ünïcode tokens: naïve café_bar 42 x y zz",
            "repeat repeat repeat repeat other",
        ];
        for (i, target) in summaries.iter().enumerate() {
            let target_features = TraceSummaryFeatures::new(target);
            for (j, candidate) in summaries.iter().enumerate() {
                let target_hash = format!("h{i}");
                let candidate_hash = format!("h{j}");
                for (summary, hash) in [
                    (Some(*candidate), Some(candidate_hash.as_str())),
                    (Some(*candidate), None),
                    (None, Some(candidate_hash.as_str())),
                ] {
                    let expected = reference_score(target, &target_hash, summary, hash);
                    let features = summary.map(TraceSummaryFeatures::new);
                    let got = trace_summary_features_similarity_score(
                        &target_features,
                        &target_hash,
                        features.as_ref(),
                        hash,
                    );
                    assert_eq!(
                        got.to_bits(),
                        expected.to_bits(),
                        "{target:?} vs {summary:?}"
                    );
                    assert_eq!(
                        trace_summary_similarity_score(target, &target_hash, summary, hash)
                            .to_bits(),
                        expected.to_bits()
                    );
                }
            }
        }
    }

    #[test]
    fn a_matching_hash_scores_one() {
        assert_eq!(
            trace_summary_duplicate_score("a", "sha256:x", [(None, Some("sha256:x"))]),
            1.0
        );
    }

    #[test]
    fn no_neighbour_scores_zero() {
        assert_eq!(
            trace_summary_duplicate_score("alpha beta", "h1", [(Some("gamma delta"), Some("h2"))]),
            0.0
        );
        assert_eq!(
            trace_summary_duplicate_score("alpha beta", "h1", std::iter::empty()),
            0.0
        );
    }

    #[test]
    fn the_best_neighbour_is_the_score() {
        let score = trace_summary_duplicate_score(
            "alpha beta gamma delta",
            "h1",
            [
                (Some("alpha beta gamma delta epsilon"), Some("h2")),
                (Some("zeta eta"), Some("h3")),
            ],
        );
        assert!((0.8..1.0).contains(&score), "{score}");
    }
}
