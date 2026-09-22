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
//!
//! As of 2026-09-21 nothing contributor-visible and nothing in a ledger reads
//! `dedup_cluster_size` (it feeds two shadow contributor-cap columns that
//! nothing reads back), so the fallback is not a column today. It is instead
//! a stated precondition on the settlement sub-project: a reader that turns
//! `dedup_cluster_size` into money must refuse a row whose
//! `effective_signal_version` is not the build's stamp, in the same way
//! [`assign_cluster`] refuses a candidate. The re-derivation pass
//! (`POST /v1/admin/rederive-dedup`) is what brings a corpus onto one stamp.
//!
//! # One sweep, two callers
//!
//! [`sweep_clusters`] is the batch form of [`assign_cluster`]: rows in
//! `decided_at` order, each assigned against one candidate per cluster keyed
//! by the cluster's first member (first-member representative linkage: no
//! chaining, no drift, and deterministic given the order). Both the
//! recluster route and the re-derivation pass call it, so the two cannot
//! disagree on membership, and it runs without PostgreSQL.

use std::collections::HashMap;

use crate::dedup_simhash::{DedupAlgorithm, hamming_distance};
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

/// Constants for [`DedupAlgorithm::V1`]. Kept as that algorithm's named
/// constants after the inline path moves on: the re-derivation pass can
/// still target v1 for a rollback, and it clusters v1 rows under these.
pub const DEDUP_CONSTANTS_V1: DedupConstants = DedupConstants {
    tau_e_micros: 150_000, // cosine distance 0.15
    // The simhash tests observe ~7 Hamming distance for a one-token reword
    // and ~9 for the A6 shim, while unrelated text sits at >=18. tau_hamming
    // = 10 clusters near-duplicates/rewords/shims while still separating
    // unrelated content. Starting value for shadow calibration.
    //
    // Those tests never exercise a text long enough for occurrence counts
    // to matter; on the pilot corpus this threshold put 75% of clustered
    // rows in one cluster. See `dedup_simhash::DedupAlgorithm::V2`.
    tau_hamming: 10,
    version: 1,
};

/// Constants for [`DedupAlgorithm::V2`], the set-semantic 3-shingle simhash.
///
/// `tau_hamming = 8`: for a unit-weight simhash the expected Hamming
/// distance over 64 bits between two feature sets with Jaccard `J` is
/// `64 * arccos(2J / (1 + J)) / pi`, so J 0.9 (a late fork, a light edit)
/// sits at 6.7 +/- 2.4 and J 0.6 (same-project sessions sharing a large
/// file read) at 14.7 +/- 3.4. At 10 the J 0.6 pair falls inside about one
/// time in nine; at 8 about one in twenty-five, while the J 0.9 pair still
/// lands inside about three times in four. The re-derivation dry run reports
/// the would-be cluster-size distribution at `DRY_RUN_CANDIDATE_TAU_HAMMING`
/// so the value can be moved before the write.
///
/// `tau_e_micros = 30_000`: the embedding arm is unwired, but its previous
/// threshold (cosine distance 0.15) is the MEDIAN distance between two
/// unrelated traces on bge-large-en-v1.5 (measured novelty on the pilot: p05
/// 0.123, p50 0.164, p95 0.291), so the first day it is wired at 0.15 it
/// chains the corpus on its own. 0.03 is four times below the fifth
/// percentile of unrelated pairs: an embedding-only join has to mean
/// near-identical on this embedder, not in the same neighbourhood. Confirmed
/// or moved by the arm's own dry run when it is wired.
pub const DEDUP_CONSTANTS_V2: DedupConstants = DedupConstants {
    tau_e_micros: 30_000,
    tau_hamming: 8,
    version: 2,
};

/// Candidate `tau_hamming` values the re-derivation dry run reports the
/// would-be cluster-size distribution for (the analogue of the perplexity
/// dry run's candidate floors). The expected shape is a large mass at 20+
/// and a small mass at 0-6 with little between; the threshold goes in the
/// valley. A populated 8-14 band means the signal needs escalating
/// (author-kind weighting, then MinHash), not the threshold moving.
pub const DRY_RUN_CANDIDATE_TAU_HAMMING: [u32; 6] = [4, 6, 8, 10, 12, 14];

impl DedupAlgorithm {
    /// The clustering constants that belong to this algorithm. A tau is a
    /// property of the signal it thresholds, so it travels with the name.
    pub const fn constants(self) -> &'static DedupConstants {
        match self {
            DedupAlgorithm::V1 => &DEDUP_CONSTANTS_V1,
            DedupAlgorithm::V2 => &DEDUP_CONSTANTS_V2,
        }
    }
}

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

/// One row handed to [`sweep_clusters`]. The caller supplies rows in
/// `decided_at ASC, decision_id ASC` order; the sweep does not sort.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepRow<'a> {
    pub simhash: u64,
    /// The stamp the simhash was derived under (a stored `NULL` already
    /// decoded to the legacy name by the caller).
    pub signal_version: &'a str,
    /// The cluster id this row is stored under, if any. When the row OPENS
    /// a cluster it keeps this id unless an earlier row in the same sweep
    /// already claimed it; a joining row takes its cluster's id regardless.
    /// This is what makes a sweep over a converged corpus reproduce itself
    /// exactly, so a rerun writes nothing. `None` mints a fresh id.
    pub stored_cluster_id: Option<Uuid>,
}

/// Where one row landed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SweepAssignment {
    pub cluster_id: Uuid,
    /// Hamming distance from this row's simhash to the nearest representative
    /// among the SAME-stamped clusters formed before it, whether or not it
    /// joined one. `None` for the first row under its stamp. Measured against
    /// representatives, not members, because that is the distance the
    /// assignment is decided on; the dry run histograms it to show where a
    /// threshold should sit.
    pub nearest_representative_hamming: Option<u32>,
}

/// Output of [`sweep_clusters`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SweepResult {
    /// One entry per input row, in input order.
    pub assignments: Vec<SweepAssignment>,
    /// Final membership of every cluster the sweep formed, computed after
    /// the whole sweep so every member is written with the same total.
    pub sizes: HashMap<Uuid, i64>,
}

impl SweepResult {
    /// Final size of `cluster_id`; `0` for an id the sweep never formed.
    pub fn size_of(&self, cluster_id: Uuid) -> i64 {
        self.sizes.get(&cluster_id).copied().unwrap_or(0)
    }
}

/// Assign every row to a cluster in one deterministic pass. Pure: no I/O.
///
/// Walking `rows` in order, each row is offered one [`ClusterCandidate`] per
/// cluster formed so far -- keyed by that cluster's first member's simhash
/// and stamp, with its running size -- and [`assign_cluster`] decides.
/// `assign_cluster` refuses a differently stamped candidate itself, so a
/// mixed-stamp corpus clusters within each stamp and never across. Simhash
/// only: `embed_cosine_micros` is `None` throughout, as in both production
/// callers.
pub fn sweep_clusters(rows: &[SweepRow<'_>], k: &DedupConstants) -> SweepResult {
    // Insertion-ordered so the candidate list, and therefore any tie-break
    // that reaches the uuid comparison, is built the same way every run.
    let mut representatives: Vec<(Uuid, u64, &str)> = Vec::new();
    let mut sizes: HashMap<Uuid, i64> = HashMap::new();
    let mut assignments = Vec::with_capacity(rows.len());

    for row in rows {
        let candidates: Vec<ClusterCandidate<'_>> = representatives
            .iter()
            .map(|(cluster_id, simhash, version)| ClusterCandidate {
                cluster_id: *cluster_id,
                size: sizes.get(cluster_id).copied().unwrap_or(0),
                simhash: *simhash,
                embed_cosine_micros: None,
                signal_version: version,
            })
            .collect();
        let nearest_representative_hamming = candidates
            .iter()
            .filter(|c| c.signal_version == row.signal_version)
            .map(|c| hamming_distance(row.simhash, c.simhash))
            .min();
        let cluster_id = match assign_cluster(row.simhash, row.signal_version, &candidates, k) {
            ClusterAssignment::Existing(id) => id,
            ClusterAssignment::New => {
                let id = match row.stored_cluster_id {
                    Some(stored) if !sizes.contains_key(&stored) => stored,
                    _ => Uuid::new_v4(),
                };
                representatives.push((id, row.simhash, row.signal_version));
                id
            }
        };
        *sizes.entry(cluster_id).or_insert(0) += 1;
        assignments.push(SweepAssignment {
            cluster_id,
            nearest_representative_hamming,
        });
    }

    SweepResult { assignments, sizes }
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

    /// The inline path clusters under the active algorithm's constants, and
    /// since the flip those are v2's: tau 8, embedding arm ceiling 0.03.
    #[test]
    fn the_inline_path_clusters_under_v2_constants() {
        let k = crate::dedup_simhash::ACTIVE_DEDUP_ALGORITHM.constants();
        assert_eq!(k.version, 2);
        assert_eq!(k.tau_hamming, 8);
        assert_eq!(k.tau_e_micros, 30_000);
        assert!(std::ptr::eq(k, &DEDUP_CONSTANTS_V2));
        // The v1 constants are unchanged: a rollback pass still clusters v1
        // rows under them.
        assert_eq!(DEDUP_CONSTANTS_V1.tau_hamming, 10);
        assert_eq!(DEDUP_CONSTANTS_V1.tau_e_micros, 150_000);
    }

    #[test]
    fn each_algorithm_maps_to_its_own_constants() {
        assert_eq!(DedupAlgorithm::V1.constants().version, 1);
        assert_eq!(DedupAlgorithm::V1.constants().tau_hamming, 10);
        assert_eq!(DedupAlgorithm::V2.constants().version, 2);
        assert_eq!(DedupAlgorithm::V2.constants().tau_hamming, 8);
        assert_eq!(DedupAlgorithm::V2.constants().tau_e_micros, 30_000);
        // The dry run brackets both algorithms' thresholds.
        assert!(DRY_RUN_CANDIDATE_TAU_HAMMING.contains(&DEDUP_CONSTANTS_V1.tau_hamming));
        assert!(DRY_RUN_CANDIDATE_TAU_HAMMING.contains(&DEDUP_CONSTANTS_V2.tau_hamming));
        assert!(
            DRY_RUN_CANDIDATE_TAU_HAMMING
                .windows(2)
                .all(|w| w[0] < w[1])
        );
    }

    // ---- sweep_clusters ----

    fn row(simhash: u64, version: &str) -> SweepRow<'_> {
        SweepRow {
            simhash,
            signal_version: version,
            stored_cluster_id: None,
        }
    }

    /// A corpus of five: three within tau of the first row, two far from
    /// everything (and from each other).
    fn five_rows() -> Vec<SweepRow<'static>> {
        vec![
            row(0, V1),
            row(0b111, V1),          // Hamming 3 from row 0
            row(u64::MAX, V1),       // Hamming 64 from row 0
            row(0b11_0000, V1),      // Hamming 2 from row 0
            row(u64::MAX >> 20, V1), // 44 bits: far from everything
        ]
    }

    #[test]
    fn sweep_sizes_sum_to_the_row_count_and_members_share_one_size() {
        let rows = five_rows();
        let result = sweep_clusters(&rows, &K);
        assert_eq!(result.assignments.len(), rows.len());
        assert_eq!(result.sizes.values().sum::<i64>(), rows.len() as i64);
        let first = result.assignments[0].cluster_id;
        assert_eq!(result.assignments[1].cluster_id, first);
        assert_eq!(result.assignments[3].cluster_id, first);
        assert_ne!(result.assignments[2].cluster_id, first);
        assert_ne!(result.assignments[4].cluster_id, first);
        assert_ne!(
            result.assignments[2].cluster_id,
            result.assignments[4].cluster_id
        );
        assert_eq!(result.size_of(first), 3);
        assert_eq!(result.size_of(result.assignments[2].cluster_id), 1);
        assert_eq!(result.sizes.len(), 3);
        // Every member reads the same final size, wherever it sat in the
        // sweep.
        for a in &result.assignments {
            assert!(result.size_of(a.cluster_id) >= 1);
        }
    }

    #[test]
    fn sweeping_its_own_output_reproduces_it() {
        let rows = five_rows();
        let first = sweep_clusters(&rows, &K);
        let again: Vec<SweepRow<'_>> = rows
            .iter()
            .zip(&first.assignments)
            .map(|(r, a)| SweepRow {
                stored_cluster_id: Some(a.cluster_id),
                ..*r
            })
            .collect();
        let second = sweep_clusters(&again, &K);
        assert_eq!(second, first, "a converged corpus re-sweeps to itself");
    }

    #[test]
    fn fresh_ids_are_minted_without_stored_ones_and_kept_when_unclaimed() {
        let rows = five_rows();
        let a = sweep_clusters(&rows, &K);
        let b = sweep_clusters(&rows, &K);
        // Same membership, different ids: nothing to prefer, so each sweep
        // mints its own.
        assert_ne!(a.assignments[0].cluster_id, b.assignments[0].cluster_id);
        assert_eq!(a.sizes.len(), b.sizes.len());

        // A stored id is kept by the row that opens a cluster...
        let kept = Uuid::from_u128(77);
        let mut with_stored = rows.clone();
        with_stored[2].stored_cluster_id = Some(kept);
        let r = sweep_clusters(&with_stored, &K);
        assert_eq!(r.assignments[2].cluster_id, kept);

        // ...unless an earlier row already claimed it: then a fresh id.
        with_stored[0].stored_cluster_id = Some(kept);
        let r = sweep_clusters(&with_stored, &K);
        assert_eq!(r.assignments[0].cluster_id, kept);
        assert_ne!(r.assignments[2].cluster_id, kept);
        // And a joining member's stored id is irrelevant: it takes its
        // cluster's.
        with_stored[1].stored_cluster_id = Some(Uuid::from_u128(88));
        let r = sweep_clusters(&with_stored, &K);
        assert_eq!(r.assignments[1].cluster_id, kept);
    }

    #[test]
    fn a_mixed_stamp_corpus_never_places_two_stamps_in_one_cluster() {
        const V2: &str = "events.v1+fnv1a-3shingle-set.v2";
        let rows = vec![
            row(0, V1),
            row(0, V2),
            row(0b1, V1),
            row(0b1, V2),
            row(0, DEDUP_DETERMINISTIC_FOR_TEST),
        ];
        let r = sweep_clusters(&rows, &K);
        assert_eq!(r.assignments[0].cluster_id, r.assignments[2].cluster_id);
        assert_eq!(r.assignments[1].cluster_id, r.assignments[3].cluster_id);
        assert_ne!(r.assignments[0].cluster_id, r.assignments[1].cluster_id);
        assert_ne!(r.assignments[4].cluster_id, r.assignments[0].cluster_id);
        assert_ne!(r.assignments[4].cluster_id, r.assignments[1].cluster_id);
        assert_eq!(r.sizes.len(), 3);
        // Distance to the nearest representative is measured within a stamp
        // only: the first row of each stamp has none, and a v2 row at
        // Hamming 0 from a v1 representative does not see it.
        assert_eq!(r.assignments[0].nearest_representative_hamming, None);
        assert_eq!(r.assignments[1].nearest_representative_hamming, None);
        assert_eq!(r.assignments[2].nearest_representative_hamming, Some(1));
        assert_eq!(r.assignments[3].nearest_representative_hamming, Some(1));
        assert_eq!(r.assignments[4].nearest_representative_hamming, None);
    }

    const DEDUP_DETERMINISTIC_FOR_TEST: &str =
        crate::trace_gate_service::DETERMINISTIC_DEDUP_SIGNAL_VERSION;

    #[test]
    fn nearest_representative_distance_is_to_representatives_not_members() {
        // Row 1 joins row 0's cluster at distance 3. Row 2 is at distance 1
        // from row 1 (a MEMBER) but distance 4 from row 0 (the
        // representative): first-member linkage measures 4, and row 2 is
        // still within tau so it joins.
        let rows = vec![row(0, V1), row(0b111, V1), row(0b1111, V1)];
        let r = sweep_clusters(&rows, &K);
        assert_eq!(r.assignments[1].nearest_representative_hamming, Some(3));
        assert_eq!(r.assignments[2].nearest_representative_hamming, Some(4));
        assert_eq!(r.assignments[2].cluster_id, r.assignments[0].cluster_id);
        // A far row reports its distance to the nearest representative even
        // though it opens a new cluster.
        let rows = vec![row(0, V1), row(u64::MAX, V1)];
        let r = sweep_clusters(&rows, &K);
        assert_eq!(r.assignments[1].nearest_representative_hamming, Some(64));
        assert_ne!(r.assignments[1].cluster_id, r.assignments[0].cluster_id);
    }

    #[test]
    fn sweep_honours_the_constants_it_is_given() {
        let rows = vec![row(0, V1), row(0b1_1111_1111, V1)]; // Hamming 9
        let v1 = sweep_clusters(&rows, DedupAlgorithm::V1.constants());
        let v2 = sweep_clusters(&rows, DedupAlgorithm::V2.constants());
        assert_eq!(v1.sizes.len(), 1, "9 <= 10 joins under v1 constants");
        assert_eq!(v2.sizes.len(), 2, "9 > 8 does not join under v2 constants");
    }

    #[test]
    fn an_empty_sweep_is_empty() {
        let r = sweep_clusters(&[], &K);
        assert!(r.assignments.is_empty());
        assert!(r.sizes.is_empty());
    }
}
