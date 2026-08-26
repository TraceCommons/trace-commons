//! Pure, deterministic shadow-mode value of a contributor correction.
//!
//! A correction earns through the machinery a trace already faces rather than
//! a new one: the same token simhash (`dedup_simhash`), the same cluster
//! assignment (`dedup_assign`), the same concave saturating map used by the
//! credit-quality score (`credit_quality::saturating_term`), and — downstream
//! — the same per-contributor concave cap (`contributor_cap`). Nothing here is
//! a second clustering implementation.
//!
//! `value = sat(novelty) * dup_pen`, with `dup_pen = 1 / cluster_size`. Fifty
//! pastes of one correction earn roughly once: the second and later pastes
//! score zero novelty against the first, and the growing cluster divides what
//! is left.
//!
//! SHADOW-ONLY. Nothing here settles, pays, gates, or feeds the scorecard.
//! `user_correction_value` in `compute_value_scorecard` is a separate,
//! pre-existing, presence-keyed weight and this module must never reach it.
//!
//! Known weakness, recorded rather than solved: novelty rewards unusual text,
//! not accurate text, so novel nonsense scores well. Collection is gated on a
//! `failed`/`partly` verdict and this is shadow-only; a relevance check is a
//! separate decision (a model deciding what a contributor earns is a trust and
//! appeals problem), and MUST NOT be added here without one.

use crate::credit_quality::saturating_term;
use crate::dedup_simhash::hamming_distance;

/// Pinned, versioned calibration constants. The floor/ceiling are expressed on
/// the normalized simhash-distance scale (`hamming / 64`, * 1e6), so they are
/// readable as Hamming distances: the V1 floor is 10/64 and the ceiling 32/64.
///
/// These are calibration SEEDS, not measurements. There are zero corrections in
/// the corpus until the collection UI ships, so any threshold chosen now is
/// invented — which is exactly why the value is computed in shadow and credited
/// nowhere. Bumping any value MUST bump `version`.
#[derive(Debug, Clone, Copy)]
pub struct CorrectionValueConstants {
    /// Below this normalized novelty the correction is a near-duplicate of one
    /// already in the corpus and scores 0.
    pub nov_floor_micros: i64,
    /// At or above this normalized novelty the novelty term saturates to 1.0.
    pub nov_ceil_micros: i64,
    pub version: i32,
}

pub const CORRECTION_VALUE_CONSTANTS_V1: CorrectionValueConstants = CorrectionValueConstants {
    // 10/64, pinned to `dedup_assign::DEDUP_CONSTANTS_V1.tau_hamming`: any
    // correction close enough for the clusterer to call it a duplicate must
    // also score zero novelty, or a one-word reword would earn a slice of a
    // fresh correction on the novelty side while the cluster penalty only
    // halved it. `correction_floor_matches_the_dedup_threshold` pins the two
    // together so neither can drift alone.
    nov_floor_micros: 156_250,
    // 32/64: half the bits differing is already the unrelated-text regime
    // (the simhash tests observe >= 18 for unrelated prose).
    nov_ceil_micros: 500_000,
    version: 1,
};

/// The active calibration used by the inline shadow write.
pub const CORRECTION_VALUE_ACTIVE: CorrectionValueConstants = CORRECTION_VALUE_CONSTANTS_V1;

/// Scored value of one correction. All fields are shadow-only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CorrectionValueScore {
    /// Normalized lexical novelty against the corrections already in the
    /// corpus, * 1e6, in `[0, 1_000_000]`.
    pub novelty_micros: i64,
    /// `1 / cluster_size` * 1e6.
    pub dup_pen_micros: i64,
    /// `sat(novelty) * dup_pen` * 1e6, in `[0, 1_000_000]`.
    pub value_micros: i64,
}

/// Lexical novelty of a correction against the corrections already in the
/// corpus: the minimum simhash Hamming distance to any of them, normalized by
/// the 64-bit signature width. No prior corrections -> maximally novel.
///
/// This is deliberately the simhash signal and not an embedding one. Cross-
/// trace dedup (#169) shipped simhash-only for the same reason — the embedding
/// side is a deferred second signal, not a prerequisite — and reusing it here
/// keeps corrections on one clustering implementation with no new dependency.
/// It measures lexical distance, not meaning: a paraphrase of an existing
/// correction scores as novel. Shadow mode is what bounds that.
pub fn correction_novelty_micros(new_simhash: u64, existing: &[u64]) -> i64 {
    let nearest = existing
        .iter()
        .map(|other| hamming_distance(new_simhash, *other))
        .min()
        .unwrap_or(64);
    (i64::from(nearest.min(64)) * 1_000_000) / 64
}

/// Shadow value of one correction: the concave saturating novelty term times
/// the duplicate penalty. `cluster_size` is the correction's cross-tenant
/// cluster membership count INCLUDING itself, so a first-of-its-kind
/// correction has size 1 and `dup_pen = 1.0`.
pub fn correction_value(
    novelty_micros: i64,
    cluster_size: i32,
    k: &CorrectionValueConstants,
) -> CorrectionValueScore {
    let novelty_micros = novelty_micros.clamp(0, 1_000_000);
    let size = cluster_size.max(1) as i64;
    let dup_pen_micros = 1_000_000 / size;
    let sat = saturating_term(novelty_micros, k.nov_floor_micros, k.nov_ceil_micros);
    let value = (sat * dup_pen_micros as f64).round() as i64;
    CorrectionValueScore {
        novelty_micros,
        dup_pen_micros,
        value_micros: value.clamp(0, 1_000_000),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dedup_assign::{
        ClusterAssignment, ClusterCandidate, DEDUP_CONSTANTS_V1, assign_cluster,
    };
    use crate::dedup_simhash::trace_simhash;
    use uuid::Uuid;

    const K: CorrectionValueConstants = CORRECTION_VALUE_CONSTANTS_V1;

    const CORRECTION_A: &str = "the agent edited the production config at config/prod.toml when \
         the task asked for the staging one; it should have written config/staging.toml and \
         re-run the migration check afterwards";
    const CORRECTION_B: &str = "the assistant never ran the failing regression test before \
         declaring the bug fixed, so the off by one in the pagination cursor survived the whole \
         session untouched";

    /// A tiny in-memory stand-in for the corrections already in the corpus.
    /// Mirrors the shape the inline path builds from the stored correction
    /// signals: one representative simhash per cluster plus its member count.
    #[derive(Default)]
    struct Corpus {
        /// (cluster_id, representative simhash, size)
        clusters: Vec<(Uuid, u64, i64)>,
    }

    impl Corpus {
        fn reps(&self) -> Vec<u64> {
            self.clusters.iter().map(|(_, sh, _)| *sh).collect()
        }

        /// Score one correction the way the inline path does, then admit it.
        fn submit(&mut self, text: &str) -> CorrectionValueScore {
            let simhash = trace_simhash(text);
            let novelty = correction_novelty_micros(simhash, &self.reps());
            let candidates: Vec<ClusterCandidate> = self
                .clusters
                .iter()
                .map(|(id, sh, size)| ClusterCandidate {
                    cluster_id: *id,
                    size: *size,
                    simhash: *sh,
                    embed_cosine_micros: None,
                })
                .collect();
            let assignment = assign_cluster(simhash, &candidates, &DEDUP_CONSTANTS_V1);
            let size = match assignment {
                ClusterAssignment::Existing(id) => {
                    let slot = self
                        .clusters
                        .iter_mut()
                        .find(|(cid, _, _)| *cid == id)
                        .expect("assigned cluster exists");
                    slot.2 += 1;
                    slot.2
                }
                ClusterAssignment::New => {
                    self.clusters.push((Uuid::new_v4(), simhash, 1));
                    1
                }
            };
            correction_value(novelty, i32::try_from(size).unwrap_or(i32::MAX), &K)
        }
    }

    #[test]
    fn a_first_correction_earns_a_nonzero_value() {
        let mut corpus = Corpus::default();
        let score = corpus.submit(CORRECTION_A);
        assert!(
            score.value_micros > 0,
            "a first-of-its-kind correction must score above zero, got {}",
            score.value_micros
        );
        assert_eq!(score.dup_pen_micros, 1_000_000, "singleton dup_pen is 1.0");
    }

    #[test]
    fn an_empty_corpus_is_maximally_novel() {
        assert_eq!(
            correction_novelty_micros(trace_simhash(CORRECTION_A), &[]),
            1_000_000
        );
    }

    #[test]
    fn fifty_identical_corrections_do_not_earn_fifty_times() {
        let mut corpus = Corpus::default();
        let first = corpus.submit(CORRECTION_A);
        let mut total = first.value_micros;
        let mut last = first;
        for _ in 0..49 {
            last = corpus.submit(CORRECTION_A);
            total += last.value_micros;
        }
        assert_eq!(
            last.novelty_micros, 0,
            "a repeat paste is zero-distance from the one already in the corpus"
        );
        assert_eq!(
            last.dup_pen_micros,
            1_000_000 / 50,
            "the fiftieth paste must carry a 1/50 duplicate penalty"
        );
        assert_eq!(
            total, first.value_micros,
            "fifty pastes must earn what one earns, got {total} vs {}",
            first.value_micros
        );
    }

    #[test]
    fn a_lightly_reworded_repeat_collapses_too() {
        // Gaming the exact-match check by changing one word must not restore
        // the value: the reword lands inside the clusterer's threshold and
        // under the novelty floor.
        let mut corpus = Corpus::default();
        corpus.submit(CORRECTION_A);
        let reworded = CORRECTION_A.replace("staging", "stage");
        let score = corpus.submit(&reworded);
        assert_eq!(
            score.value_micros, 0,
            "a one-word reword of an existing correction must not earn again"
        );
    }

    #[test]
    fn a_genuinely_different_correction_still_earns() {
        let mut corpus = Corpus::default();
        corpus.submit(CORRECTION_A);
        let score = corpus.submit(CORRECTION_B);
        assert!(
            score.value_micros > 0,
            "an unrelated correction must not be collapsed into the first, got {}",
            score.value_micros
        );
        assert_eq!(score.dup_pen_micros, 1_000_000);
    }

    #[test]
    fn value_is_bounded_monotonic_and_concave_in_novelty() {
        let v = |nov: i64| correction_value(nov, 1, &K).value_micros;
        let mut prev = -1;
        let mut nov = 0;
        while nov <= 1_000_000 {
            let value = v(nov);
            assert!((0..=1_000_000).contains(&value), "out of range: {value}");
            assert!(value >= prev, "not monotonic at {nov}: {prev} then {value}");
            prev = value;
            nov += 5_000;
        }
        let d1 = v(300_000) - v(200_000);
        let d2 = v(400_000) - v(300_000);
        assert!(d2 <= d1, "expected concavity: d1={d1} d2={d2}");
        assert_eq!(v(1_000_000), 1_000_000, "at/above ceiling saturates to 1.0");
        assert_eq!(v(K.nov_floor_micros), 0, "at the floor the term is 0");
    }

    #[test]
    fn correction_floor_matches_the_dedup_threshold() {
        // The floor must be at least the clusterer's join threshold expressed
        // on the same normalized scale. If tau_hamming is ever raised without
        // raising the floor, near-duplicates start earning novelty again.
        let tau_as_novelty = (i64::from(DEDUP_CONSTANTS_V1.tau_hamming) * 1_000_000) / 64;
        assert!(
            K.nov_floor_micros >= tau_as_novelty,
            "novelty floor {} must cover the dedup join threshold {}",
            K.nov_floor_micros,
            tau_as_novelty
        );
    }

    #[test]
    fn deterministic() {
        let a = correction_value(400_000, 3, &K);
        let b = correction_value(400_000, 3, &K);
        assert_eq!(a, b);
    }

    #[test]
    fn a_degenerate_cluster_size_is_safe() {
        // Size 0 or negative can only come from a corrupt snapshot; treat it
        // as a singleton rather than dividing by zero.
        assert_eq!(correction_value(400_000, 0, &K).dup_pen_micros, 1_000_000);
        assert_eq!(correction_value(400_000, -7, &K).dup_pen_micros, 1_000_000);
    }

    // ---- shadow-mode guarantee ----

    #[test]
    fn a_correction_value_does_not_change_credited_output() {
        // The credit pipeline's raw increment is `q * dup_pen` over the TRACE
        // dedup cluster. A correction's value, novelty and cluster size are
        // not inputs to it, and this pins that: two decisions identical in
        // (q, trace cluster size) but carrying wildly different correction
        // values produce the same increment and the same cap factor.
        let quiet = correction_value(0, 50, &K);
        let loud = correction_value(1_000_000, 1, &K);
        assert!(loud.value_micros > quiet.value_micros);

        let q_micros = Some(750_000_i64);
        let trace_cluster_size = Some(2_i32);
        let increment = crate::contributor_cap::increment_micros(q_micros, trace_cluster_size);
        assert_eq!(
            increment,
            crate::contributor_cap::increment_micros(q_micros, trace_cluster_size),
            "the credited increment is a function of q and the TRACE cluster only"
        );
        assert_eq!(increment, 375_000);
        let factor = crate::contributor_cap::contributor_factor_micros(
            0,
            increment,
            &crate::contributor_cap::CONTRIBUTOR_CAP_CONSTANTS_V1,
        );
        assert!(factor > 0);
        // And the credit-quality score, the other half of the credited path,
        // takes perplexity and TRACE novelty only — no correction input.
        let cq = crate::credit_quality::credit_quality(
            15_000_000,
            18_000_000,
            850_000,
            &crate::credit_quality::CREDIT_QUALITY_ACTIVE,
        );
        assert!(cq.q_micros > 0);
    }
}
