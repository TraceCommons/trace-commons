// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The inline cross-trace dedup assignment, shared by the legacy gate
//! (`evaluate_and_record_gate` in the ingest service) and the versioned
//! pipeline's Settle (`PipelineService`), so the two paths cluster a new
//! decision the same way and cannot drift.
//!
//! One call reads the clusters within the active algorithm's tau of the new
//! simhash (`Database::list_dedup_cluster_candidates`, V145), assigns the
//! decision to one of them or to a new cluster (`dedup_assign::assign_cluster`,
//! which refuses a differently stamped candidate itself), and writes the four
//! dedup columns in one statement, so the stamp and the simhash land together
//! (V57).
//!
//! Best-effort throughout: a failed candidate read clusters against nothing,
//! and a failed write leaves the row as it was. Both log hash-only and never
//! fail the caller's decision. A row left NULL reads as not first in any
//! cluster, so earned trust fails closed on it until the re-derivation pass
//! (`POST /v1/admin/rederive-dedup`) reaches it.

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::db::Database;
use crate::dedup_assign::{ClusterAssignment, ClusterCandidate, LEGACY_DEDUP_SIGNAL_VERSION};
use crate::dedup_simhash::ACTIVE_DEDUP_ALGORITHM;
use crate::trace_corpus_storage::DedupAssignmentWrite;

/// How [`record_inline_dedup`] writes the assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineDedupWrite {
    /// Overwrite the four columns: the legacy gate, which stamps the row it
    /// inserted a moment earlier.
    Overwrite,
    /// Write only while the row has no cluster (`dedup_cluster_id IS NULL`):
    /// the pipeline, whose Settle row insert is `ON CONFLICT DO NOTHING` and
    /// can be offered again, and whose row the re-derivation pass may already
    /// have stamped. A second stamp never moves a row between clusters.
    OnlyIfUnset,
}

/// What [`record_inline_dedup`] did. Returned for tests and callers that
/// count; no caller has to act on it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InlineDedupOutcome {
    /// The four columns were written with this assignment.
    Written {
        dedup_cluster_id: Uuid,
        dedup_cluster_size: i32,
        dedup_signal_version: String,
    },
    /// `OnlyIfUnset` found the row already clustered, or no such row.
    AlreadySet,
    /// The write failed; the row is as it was. Logged hash-only.
    WriteFailed,
}

/// The stamp a derivation carries, normalised the way stored rows are read:
/// NULL and empty alike are the legacy name (`effective_signal_version`). A
/// gate service returning `""` would otherwise match no candidate at all, and
/// every submission would be a permanent singleton.
pub fn normalise_signal_version(signal_version: &str) -> String {
    if signal_version.is_empty() {
        LEGACY_DEDUP_SIGNAL_VERSION.to_string()
    } else {
        signal_version.to_string()
    }
}

fn tenant_hash(tenant_id: &str) -> String {
    format!(
        "sha256:{}",
        hex::encode(Sha256::digest(tenant_id.as_bytes()))
    )
}

/// Assign `decision_id` (tenant `tenant_id`) to a dedup cluster by `simhash`
/// under `signal_version`, and write the four dedup columns. See the module
/// documentation for the failure behaviour.
///
/// The constants are the active algorithm's: both callers derive under
/// `ACTIVE_DEDUP_ALGORITHM`, so its tau applies. One candidate per CLUSTER,
/// keyed by the cluster's representative (earliest-decided member) simhash,
/// with the cluster's size across every tenant -- the same candidates the
/// recluster pass assigns against. Only this row's size snapshot is set;
/// existing members' sizes are refreshed by the recluster pass.
pub async fn record_inline_dedup(
    db: &dyn Database,
    tenant_id: &str,
    decision_id: Uuid,
    simhash: i64,
    signal_version: &str,
    write: InlineDedupWrite,
) -> InlineDedupOutcome {
    let signal_version = normalise_signal_version(signal_version);
    let constants = ACTIVE_DEDUP_ALGORITHM.constants();
    let cluster_rows = match db
        .list_dedup_cluster_candidates(simhash, constants.tau_hamming)
        .await
    {
        Ok(rows) => rows,
        Err(_) => {
            tracing::warn!(
                error_class = "dedup_cluster_candidate_read_failed",
                tenant_hash = %tenant_hash(tenant_id),
                "dedup cluster candidate read failed; clustering against no candidates"
            );
            Vec::new()
        }
    };
    let candidates: Vec<ClusterCandidate<'_>> = cluster_rows
        .iter()
        .map(|row| ClusterCandidate {
            cluster_id: row.dedup_cluster_id,
            size: row.member_count,
            simhash: row.representative_simhash as u64,
            embed_cosine_micros: None,
            signal_version: row.effective_signal_version(),
        })
        .collect();
    let cluster_id = match crate::dedup_assign::assign_cluster(
        simhash as u64,
        &signal_version,
        &candidates,
        constants,
    ) {
        ClusterAssignment::Existing(id) => id,
        ClusterAssignment::New => Uuid::new_v4(),
    };
    let existing_size = cluster_rows
        .iter()
        .find(|row| row.dedup_cluster_id == cluster_id)
        .map_or(0, |row| row.member_count);
    let dedup_cluster_size = i32::try_from(existing_size + 1).unwrap_or(i32::MAX);
    let assignment = DedupAssignmentWrite {
        dedup_simhash: simhash,
        dedup_cluster_id: cluster_id,
        dedup_cluster_size,
        dedup_signal_version: signal_version.clone(),
    };
    let written = match write {
        InlineDedupWrite::Overwrite => db
            .update_trace_gate_decision_dedup(tenant_id, decision_id, assignment)
            .await
            .map(|()| true),
        InlineDedupWrite::OnlyIfUnset => {
            db.update_trace_gate_decision_dedup_if_unset(tenant_id, decision_id, assignment)
                .await
        }
    };
    match written {
        Ok(true) => InlineDedupOutcome::Written {
            dedup_cluster_id: cluster_id,
            dedup_cluster_size,
            dedup_signal_version: signal_version,
        },
        Ok(false) => InlineDedupOutcome::AlreadySet,
        Err(_) => {
            tracing::warn!(
                error_class = "dedup_inline_write_failed",
                tenant_hash = %tenant_hash(tenant_id),
                "shadow dedup inline write failed (non-fatal)"
            );
            InlineDedupOutcome::WriteFailed
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_stamp_reads_as_the_legacy_name_and_others_are_kept() {
        assert_eq!(normalise_signal_version(""), LEGACY_DEDUP_SIGNAL_VERSION);
        assert_eq!(
            normalise_signal_version("events.v1+fnv1a-3shingle-set.v2"),
            "events.v1+fnv1a-3shingle-set.v2"
        );
    }

    #[test]
    fn the_tenant_hash_never_carries_the_tenant_id() {
        let hash = tenant_hash("tenant-secret-name");
        assert!(hash.starts_with("sha256:"), "{hash}");
        assert!(!hash.contains("tenant-secret-name"), "{hash}");
        assert_eq!(hash.len(), "sha256:".len() + 64);
    }
}
