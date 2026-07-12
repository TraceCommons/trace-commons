//! Pure cluster-assignment logic for cross-trace dedup. Signal-agnostic: the
//! caller gathers candidate clusters from the cross-tenant simhash scan and/or
//! the dedup vector index and hands them here. OR-match on either signal; tie
//! -> larger cluster (deterministic); no match -> new singleton.

use crate::dedup_simhash::hamming_distance;
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct DedupConstants {
    /// Embedding cosine-DISTANCE threshold in micros (join when a candidate's
    /// cosine distance <= this). Calibrated in shadow; V1 is a starting value.
    pub tau_e_micros: i64,
    /// simhash Hamming-distance threshold (join when <= this).
    pub tau_hamming: u32,
    pub version: i32,
}

pub const DEDUP_CONSTANTS_V1: DedupConstants = DedupConstants {
    tau_e_micros: 150_000, // cosine distance 0.15
    // The simhash tests observe ~7 Hamming distance for a one-token reword
    // and ~9 for the A6 shim, while unrelated text sits at >=18. tau_hamming
    // = 10 clusters near-duplicates/rewords/shims while still separating
    // unrelated content. Starting value for shadow calibration.
    tau_hamming: 10,
    version: 1,
};

#[derive(Debug, Clone, Copy)]
pub struct ClusterCandidate {
    pub cluster_id: Uuid,
    pub size: i64,
    pub simhash: u64,
    pub embed_cosine_micros: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterAssignment {
    Existing(Uuid),
    New,
}

pub fn assign_cluster(
    new_simhash: u64,
    candidates: &[ClusterCandidate],
    k: &DedupConstants,
) -> ClusterAssignment {
    // A candidate matches if EITHER signal is within threshold (OR semantics).
    let mut best: Option<(Uuid, i64)> = None; // (cluster_id, size)
    for c in candidates {
        let simhash_match = hamming_distance(new_simhash, c.simhash) <= k.tau_hamming;
        let embed_match = c.embed_cosine_micros.is_some_and(|d| d <= k.tau_e_micros);
        if simhash_match || embed_match {
            // tie-break: larger cluster wins; on equal size, lower uuid wins for determinism
            let take = match best {
                None => true,
                Some((bid, bsize)) => c.size > bsize || (c.size == bsize && c.cluster_id < bid),
            };
            if take {
                best = Some((c.cluster_id, c.size));
            }
        }
    }
    match best {
        Some((id, _)) => ClusterAssignment::Existing(id),
        None => ClusterAssignment::New,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    const K: DedupConstants = DEDUP_CONSTANTS_V1;
    fn cand(id: Uuid, size: i64, simhash: u64, cos: Option<i64>) -> ClusterCandidate {
        ClusterCandidate {
            cluster_id: id,
            size,
            simhash,
            embed_cosine_micros: cos,
        }
    }

    #[test]
    fn no_candidates_is_new_singleton() {
        assert_eq!(assign_cluster(42, &[], &K), ClusterAssignment::New);
    }

    #[test]
    fn simhash_within_threshold_joins() {
        let id = Uuid::from_u128(1);
        // identical simhash -> Hamming 0 <= tau_hamming
        let c = cand(id, 1, 42, None);
        assert_eq!(
            assign_cluster(42, &[c], &K),
            ClusterAssignment::Existing(id)
        );
    }

    #[test]
    fn simhash_far_and_no_embedding_is_new() {
        let id = Uuid::from_u128(1);
        // Hamming distance >> tau_hamming, no embedding signal
        let c = cand(id, 1, u64::MAX, None);
        assert_eq!(assign_cluster(0, &[c], &K), ClusterAssignment::New);
    }

    #[test]
    fn embedding_within_threshold_joins_even_if_simhash_far() {
        // heavy paraphrase: simhash far, but embedding cosine distance below tau_e
        let id = Uuid::from_u128(2);
        let c = cand(id, 1, u64::MAX, Some(K.tau_e_micros - 1));
        assert_eq!(assign_cluster(0, &[c], &K), ClusterAssignment::Existing(id));
    }

    #[test]
    fn embedding_over_threshold_does_not_join_on_embedding_alone() {
        let id = Uuid::from_u128(2);
        let c = cand(id, 1, u64::MAX, Some(K.tau_e_micros + 1));
        assert_eq!(assign_cluster(0, &[c], &K), ClusterAssignment::New);
    }

    #[test]
    fn tie_breaks_to_larger_cluster() {
        // two clusters both match on simhash; join the larger
        let small = Uuid::from_u128(10);
        let large = Uuid::from_u128(20);
        let cands = [cand(small, 2, 42, None), cand(large, 9, 42, None)];
        assert_eq!(
            assign_cluster(42, &cands, &K),
            ClusterAssignment::Existing(large)
        );
    }
}
