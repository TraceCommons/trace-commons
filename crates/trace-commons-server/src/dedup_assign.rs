// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Pure cluster-assignment logic for cross-trace dedup. The caller gathers
//! candidate clusters from the cross-tenant simhash scan and/or the dedup
//! vector index and hands them here, each carrying the signal version it was
//! derived under. A candidate stamped differently from the incoming row is
//! refused before any distance is computed: the two numbers are not measuring
//! the same thing, so a small distance between them is a coincidence rather
//! than evidence. Among the rest: OR-match on either signal; tie -> larger
//! cluster (deterministic); no match -> new singleton.
//!
//! # The version bump is a credit event, not just a clustering one
//!
//! Refusing across versions is the right semantics and it has a consequence
//! that must be handled before any renderer or simhash constant moves.
//!
//! The moment `CANONICAL_RENDER_VERSION` (or `DEDUP_SIMHASH_ALGORITHM`)
//! changes, new submissions carry the new stamp while the entire stored
//! corpus still carries the old one. Every candidate is then refused, so
//! every resubmission of an already-stored trace clusters as a singleton:
//! `dedup_cluster_size = 1`, `dup_pen = 1`, and the per-contributor cap's
//! `R = sum(q * dup_pen)` counts a duplicate at full weight. For the length
//! of that window, resubmitting is worth more than it was before this
//! module started refusing -- silently, and to anyone who tries it.
//!
//! So the re-derivation pass is not a tidying step that can follow the bump
//! at leisure. Two rules:
//!
//! 1. The pass completes before the constant flips in production.
//! 2. The pass and the new constant never ship in the same binary, or there
//!    is no ordering to enforce.
//!
//! If a deployment cannot honour both, the fallback is to withhold or flag
//! credit for any decision whose `effective_signal_version` is not the
//! build's current stamp, so a stale-version row cannot earn a
//! duplicate-free `dup_pen` while the corpus is mixed.

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

/// The `dedup_signal_version` a row recorded before the column existed is
/// read as. Every pre-column row was written by the enclave path with the v1
/// renderer and the v1 simhash, except deterministic-service rows, which come
/// from development and test services; reading the whole NULL set as v1 for
/// the transition window is the honest reading, and the re-derivation pass
/// overwrites all of it.
///
/// FROZEN. It is deliberately a literal and not composed from
/// `CANONICAL_RENDER_VERSION` + `DEDUP_SIMHASH_ALGORITHM`: those two name
/// what the code renders TODAY, and recomposing this from them would silently
/// re-label every historical row on the next bump — which is the exact defect
/// the column exists to prevent.
pub const LEGACY_DEDUP_SIGNAL_VERSION: &str = "events.v1+fnv1a-2shingle.v1";

/// The stamp for a `GateDecision` that is a synthetic re-hydration rather
/// than a scored trace -- one built only to call a downstream emitter, whose
/// `dedup_simhash` is a placeholder `0`.
///
/// Deliberately not [`LEGACY_DEDUP_SIGNAL_VERSION`]. No writer persists a
/// copy carrying this, but a stamp is only worth anything if the value that
/// would do the most damage on the day someone does is not the one sitting
/// there. Under the legacy stamp, a simhash of `0` is Hamming-close to every
/// low-weight v1 signal in the corpus, so a placeholder would be the single
/// best-connected node in the graph. Under a name no real derivation ever
/// produces, `assign_cluster` refuses it against every candidate.
pub const PLACEHOLDER_DEDUP_SIGNAL_VERSION: &str = "placeholder.not-a-derivation";

#[derive(Debug, Clone, Copy)]
pub struct ClusterCandidate<'a> {
    pub cluster_id: Uuid,
    pub size: i64,
    pub simhash: u64,
    pub embed_cosine_micros: Option<i64>,
    /// The signal version this cluster's REPRESENTATIVE was derived under —
    /// a cluster's version is the version of the row that created it. A
    /// caller reading rows from storage gets this from
    /// [`crate::trace_corpus_storage::DedupSignalRow::effective_signal_version`],
    /// which is where a stored `NULL` is decoded.
    pub signal_version: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterAssignment {
    Existing(Uuid),
    New,
}

pub fn assign_cluster(
    new_simhash: u64,
    new_version: &str,
    candidates: &[ClusterCandidate<'_>],
    k: &DedupConstants,
) -> ClusterAssignment {
    // A candidate matches if EITHER signal is within threshold (OR semantics).
    let mut best: Option<(Uuid, i64)> = None; // (cluster_id, size)
    for c in candidates {
        // Refused BEFORE any distance is computed, and refused here rather
        // than in each caller's candidate filter: a cluster derived by a
        // different renderer or a different simhash algorithm is not a
        // candidate at any Hamming distance, and a gate that lives in the
        // callers is a gate the next call site can forget. Both signals are
        // covered, not only the simhash — an embedding produced under one
        // renderer is no more comparable than a simhash is.
        if c.signal_version != new_version {
            continue;
        }
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
    /// Every candidate in a test that is not ABOUT versioning carries the
    /// same stamp as the incoming row, which is what a single-version corpus
    /// looks like.
    const V1: &str = LEGACY_DEDUP_SIGNAL_VERSION;
    fn cand(id: Uuid, size: i64, simhash: u64, cos: Option<i64>) -> ClusterCandidate<'static> {
        ClusterCandidate {
            cluster_id: id,
            size,
            simhash,
            embed_cosine_micros: cos,
            signal_version: V1,
        }
    }
    fn cand_v(id: Uuid, size: i64, simhash: u64, version: &str) -> ClusterCandidate<'_> {
        ClusterCandidate {
            cluster_id: id,
            size,
            simhash,
            embed_cosine_micros: None,
            signal_version: version,
        }
    }

    #[test]
    fn no_candidates_is_new_singleton() {
        assert_eq!(assign_cluster(42, V1, &[], &K), ClusterAssignment::New);
    }

    #[test]
    fn simhash_within_threshold_joins() {
        let id = Uuid::from_u128(1);
        // identical simhash -> Hamming 0 <= tau_hamming
        let c = cand(id, 1, 42, None);
        assert_eq!(
            assign_cluster(42, V1, &[c], &K),
            ClusterAssignment::Existing(id)
        );
    }

    #[test]
    fn simhash_far_and_no_embedding_is_new() {
        let id = Uuid::from_u128(1);
        // Hamming distance >> tau_hamming, no embedding signal
        let c = cand(id, 1, u64::MAX, None);
        assert_eq!(assign_cluster(0, V1, &[c], &K), ClusterAssignment::New);
    }

    #[test]
    fn embedding_within_threshold_joins_even_if_simhash_far() {
        // heavy paraphrase: simhash far, but embedding cosine distance below tau_e
        let id = Uuid::from_u128(2);
        let c = cand(id, 1, u64::MAX, Some(K.tau_e_micros - 1));
        assert_eq!(
            assign_cluster(0, V1, &[c], &K),
            ClusterAssignment::Existing(id)
        );
    }

    #[test]
    fn embedding_over_threshold_does_not_join_on_embedding_alone() {
        let id = Uuid::from_u128(2);
        let c = cand(id, 1, u64::MAX, Some(K.tau_e_micros + 1));
        assert_eq!(assign_cluster(0, V1, &[c], &K), ClusterAssignment::New);
    }

    /// The gate this module exists for: an IDENTICAL simhash under a
    /// different stamp is not a candidate. Hamming distance 0 is the
    /// strongest possible match on the number, so if the version check is
    /// removed this is the assertion that has to fail.
    #[test]
    fn an_identical_simhash_under_a_different_version_never_joins() {
        let id = Uuid::from_u128(3);
        let c = cand_v(id, 9, 42, "events.v2+fnv1a-2shingle.v1");
        assert_eq!(assign_cluster(42, V1, &[c], &K), ClusterAssignment::New);
    }

    /// The other half of the same claim: the refusal is about the stamp and
    /// nothing else, so the same pair under one stamp joins.
    #[test]
    fn an_identical_simhash_under_the_same_version_joins() {
        let id = Uuid::from_u128(3);
        let c = cand_v(id, 9, 42, V1);
        assert_eq!(
            assign_cluster(42, V1, &[c], &K),
            ClusterAssignment::Existing(id)
        );
    }

    /// A differently stamped candidate is refused on the EMBEDDING side too,
    /// not only on the simhash: an embedding produced under one renderer is
    /// no more comparable than a simhash produced under it.
    #[test]
    fn a_different_version_is_refused_even_when_the_embedding_matches() {
        let id = Uuid::from_u128(4);
        let c = ClusterCandidate {
            cluster_id: id,
            size: 9,
            simhash: u64::MAX,
            embed_cosine_micros: Some(K.tau_e_micros - 1),
            signal_version: "events.v2+fnv1a-2shingle.v1",
        };
        assert_eq!(assign_cluster(0, V1, &[c], &K), ClusterAssignment::New);
    }

    /// Version scoping must not become a tie-break: among same-stamped
    /// candidates the larger cluster still wins, and a differently stamped
    /// larger cluster does not beat a same-stamped smaller one.
    #[test]
    fn a_larger_cluster_under_another_version_loses_to_a_smaller_matching_one() {
        let mine = Uuid::from_u128(10);
        let theirs = Uuid::from_u128(20);
        let cands = [
            cand_v(mine, 1, 42, V1),
            cand_v(theirs, 99, 42, "events.v2+fnv1a-2shingle.v1"),
        ];
        assert_eq!(
            assign_cluster(42, V1, &cands, &K),
            ClusterAssignment::Existing(mine)
        );
    }

    /// The two stamps this build actually writes, named by symbol: a
    /// deterministic service's digest window and the enclave's composed
    /// render+simhash. They must never cluster together, and the reason is
    /// not a threshold -- a `digest-prefix.v1` value is a window of a
    /// decision digest, not a simhash of any text, so a Hamming distance
    /// between the two is a comparison of unrelated numbers.
    #[test]
    fn the_two_stamps_this_build_writes_never_cluster_together() {
        assert_ne!(
            crate::trace_gate_service::DETERMINISTIC_DEDUP_SIGNAL_VERSION,
            LEGACY_DEDUP_SIGNAL_VERSION,
            "the deterministic stamp and the enclave stamp must stay distinct"
        );
        let id = Uuid::from_u128(5);
        let deterministic = cand_v(
            id,
            9,
            42,
            crate::trace_gate_service::DETERMINISTIC_DEDUP_SIGNAL_VERSION,
        );
        assert_eq!(
            assign_cluster(42, LEGACY_DEDUP_SIGNAL_VERSION, &[deterministic], &K),
            ClusterAssignment::New,
            "an enclave decision must not join a deterministic service's cluster"
        );
        let enclave = cand_v(id, 9, 42, LEGACY_DEDUP_SIGNAL_VERSION);
        assert_eq!(
            assign_cluster(
                42,
                crate::trace_gate_service::DETERMINISTIC_DEDUP_SIGNAL_VERSION,
                &[enclave],
                &K
            ),
            ClusterAssignment::New,
            "and the refusal holds in the other direction too"
        );
    }

    #[test]
    fn tie_breaks_to_larger_cluster() {
        // two clusters both match on simhash; join the larger
        let small = Uuid::from_u128(10);
        let large = Uuid::from_u128(20);
        let cands = [cand(small, 2, 42, None), cand(large, 9, 42, None)];
        assert_eq!(
            assign_cluster(42, V1, &cands, &K),
            ClusterAssignment::Existing(large)
        );
    }
}
