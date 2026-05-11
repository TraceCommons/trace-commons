//! Compose `PerplexityScorer` + `Embedder` + `VectorIndex` into a single
//! gate-decision pipeline.

use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::embedder::Embedder;
use crate::perplexity::PerplexityScorer;
use crate::vector_index::VectorIndex;

/// Config for `EnclaveGateOrchestrator`. Floors are inclusive lower bounds in
/// micros; passing means `value >= floor`.
#[derive(Debug, Clone)]
pub struct EnclaveGateOrchestratorConfig {
    pub gate_policy_version: String,
    pub gate_version_hash: String,
    pub perplexity_floor_micros: u64,
    pub tail_fraction_floor_micros: u64,
    pub novelty_floor_micros: u64,
    pub top_k: usize,
}

impl EnclaveGateOrchestratorConfig {
    /// Sensible defaults for tests / dev: floors set so the mock distribution
    /// emits a mix of pass / fail without callers having to tune.
    pub fn mock_default() -> Self {
        Self {
            gate_policy_version: "enclave_mock_v1".into(),
            gate_version_hash: "sha256:enclave_mock_v1".into(),
            perplexity_floor_micros: 0,
            tail_fraction_floor_micros: 0,
            novelty_floor_micros: 0,
            top_k: 8,
        }
    }
}

/// Output of `EnclaveGateOrchestrator::evaluate`. The host-side
/// `EnclaveGateService` maps this into the audit-row shape stored in
/// `trace_gate_decisions`.
#[derive(Debug, Clone, PartialEq)]
pub struct OrchestrationDecision {
    pub gate_policy_version: String,
    pub gate_version_hash: String,
    pub perplexity_micros: u64,
    pub tail_fraction_micros: u64,
    pub perplexity_passed: bool,
    pub novelty_score_micros: u64,
    pub nearest_neighbor_hash: String,
    pub novelty_passed: bool,
    pub embedding_evidence_hash: String,
    pub attestation_chain_hash: String,
    /// `Some(id)` when both gates passed and the orchestrator inserted the
    /// embedding into the vector index; `None` otherwise.
    pub inserted_entry_id: Option<Uuid>,
}

/// Pipelines a plaintext through perplexity + embedding + novelty gates and
/// returns a deterministic `OrchestrationDecision`.
pub struct EnclaveGateOrchestrator<P, E, V> {
    perplexity: P,
    embedder: E,
    index: V,
    cfg: EnclaveGateOrchestratorConfig,
}

impl<P, E, V> EnclaveGateOrchestrator<P, E, V>
where
    P: PerplexityScorer,
    E: Embedder,
    V: VectorIndex,
{
    pub fn new(perplexity: P, embedder: E, index: V, cfg: EnclaveGateOrchestratorConfig) -> Self {
        Self {
            perplexity,
            embedder,
            index,
            cfg,
        }
    }

    pub fn config(&self) -> &EnclaveGateOrchestratorConfig {
        &self.cfg
    }

    /// Remove an entry from the orchestrator's vector index by `entry_id`.
    ///
    /// Returns `Ok(true)` if the entry was found and removed, `Ok(false)` if
    /// no such entry existed (idempotent: already-absent is a satisfied
    /// postcondition). Callers that only need "make sure it's gone" can safely
    /// discard the bool.
    pub fn delete_vector_entry(&self, entry_id: Uuid) -> anyhow::Result<bool> {
        self.index.delete(entry_id)
    }

    /// Evaluate `plaintext` under the orchestrator's gate policy.
    ///
    /// Steps:
    ///  1. Score for perplexity.
    ///  2. Embed.
    ///  3. Query top-k nearest neighbors and compute novelty as
    ///     `1 - max(cosine_similarity)`.
    ///  4. Apply floors → bool pass flags.
    ///  5. If both pass, insert embedding into the index so future traces see
    ///     it; otherwise leave the index untouched.
    pub fn evaluate(
        &self,
        plaintext: &[u8],
        tenant_storage_ref: &str,
    ) -> anyhow::Result<OrchestrationDecision> {
        let perp = self.perplexity.score(plaintext);
        let embedding = self.embedder.embed(plaintext);
        let neighbors = self
            .index
            .nearest(tenant_storage_ref, &embedding, self.cfg.top_k)?;

        // novelty_score = 1 - max similarity, scaled to micros. Cosine sim
        // lives in [-1, 1]; clamp before mapping so the micros stay in
        // [0, 2_000_000].
        let max_sim = neighbors
            .iter()
            .map(|n| n.similarity)
            .fold(f32::NEG_INFINITY, f32::max);
        let novelty_score_f = if max_sim.is_finite() {
            (1.0 - max_sim).max(0.0)
        } else {
            // No existing entries → maximally novel.
            1.0
        };
        let novelty_score_micros = (novelty_score_f.clamp(0.0, 2.0) * 1_000_000.0) as u64;

        let perplexity_passed = perp.aggregate_perplexity_micros >= self.cfg.perplexity_floor_micros
            && perp.tail_fraction_micros >= self.cfg.tail_fraction_floor_micros;
        let novelty_passed = novelty_score_micros >= self.cfg.novelty_floor_micros;

        let nearest_neighbor_hash = hash_neighbors(&neighbors);
        let embedding_evidence_hash = hash_embedding_evidence(
            &self.cfg.gate_policy_version,
            tenant_storage_ref,
            &embedding,
        );
        let attestation_chain_hash = hash_attestation_chain(
            &self.cfg.gate_policy_version,
            &self.cfg.gate_version_hash,
        );

        let mut inserted_entry_id = None;
        if perplexity_passed && novelty_passed {
            let entry_id = Uuid::new_v4();
            self.index
                .insert(entry_id, tenant_storage_ref, &embedding)?;
            inserted_entry_id = Some(entry_id);
        }

        Ok(OrchestrationDecision {
            gate_policy_version: self.cfg.gate_policy_version.clone(),
            gate_version_hash: self.cfg.gate_version_hash.clone(),
            perplexity_micros: perp.aggregate_perplexity_micros,
            tail_fraction_micros: perp.tail_fraction_micros,
            perplexity_passed,
            novelty_score_micros,
            nearest_neighbor_hash,
            novelty_passed,
            embedding_evidence_hash,
            attestation_chain_hash,
            inserted_entry_id,
        })
    }
}

fn hash_neighbors(neighbors: &[crate::vector_index::NearestNeighbor]) -> String {
    let mut h = Sha256::new();
    h.update(b"trace_gate_enclave.nearest_neighbors.v1\n");
    for n in neighbors {
        h.update(n.entry_id.as_bytes());
        h.update(n.similarity.to_be_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}

fn hash_embedding_evidence(
    gate_policy_version: &str,
    tenant_storage_ref: &str,
    embedding: &[f32],
) -> String {
    let mut h = Sha256::new();
    h.update(b"trace_gate_enclave.embedding_evidence.v1\n");
    h.update(gate_policy_version.as_bytes());
    h.update(b"\n");
    h.update(tenant_storage_ref.as_bytes());
    h.update(b"\n");
    for x in embedding {
        h.update(x.to_be_bytes());
    }
    format!("sha256:{:x}", h.finalize())
}

fn hash_attestation_chain(gate_policy_version: &str, gate_version_hash: &str) -> String {
    let mut h = Sha256::new();
    h.update(b"trace_gate_enclave.attestation_chain.v1\n");
    h.update(gate_policy_version.as_bytes());
    h.update(b"\n");
    h.update(gate_version_hash.as_bytes());
    format!("sha256:{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::embedder::MockEmbedder;
    use crate::perplexity::MockPerplexityScorer;
    use crate::vector_index::MockVectorIndex;

    fn orch_with_floors(
        perplexity_floor_micros: u64,
        tail_fraction_floor_micros: u64,
        novelty_floor_micros: u64,
    ) -> EnclaveGateOrchestrator<MockPerplexityScorer, MockEmbedder, MockVectorIndex> {
        let mut cfg = EnclaveGateOrchestratorConfig::mock_default();
        cfg.perplexity_floor_micros = perplexity_floor_micros;
        cfg.tail_fraction_floor_micros = tail_fraction_floor_micros;
        cfg.novelty_floor_micros = novelty_floor_micros;
        EnclaveGateOrchestrator::new(
            MockPerplexityScorer::new(),
            MockEmbedder::new(),
            MockVectorIndex::new(),
            cfg,
        )
    }

    #[test]
    fn deterministic_for_same_input() {
        let orch_a = orch_with_floors(0, 0, 0);
        let orch_b = orch_with_floors(0, 0, 0);
        let a = orch_a.evaluate(b"hello world", "tenant_a").unwrap();
        let b = orch_b.evaluate(b"hello world", "tenant_a").unwrap();
        // inserted_entry_id is a fresh UUID each run; everything else stable.
        assert_eq!(a.perplexity_micros, b.perplexity_micros);
        assert_eq!(a.tail_fraction_micros, b.tail_fraction_micros);
        assert_eq!(a.novelty_score_micros, b.novelty_score_micros);
        assert_eq!(a.embedding_evidence_hash, b.embedding_evidence_hash);
        assert_eq!(a.attestation_chain_hash, b.attestation_chain_hash);
        assert_eq!(a.perplexity_passed, b.perplexity_passed);
        assert_eq!(a.novelty_passed, b.novelty_passed);
    }

    #[test]
    fn inserted_trace_is_its_own_nearest_neighbor_next_time() {
        let orch = orch_with_floors(0, 0, 0);
        let first = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(first.perplexity_passed && first.novelty_passed);
        assert!(first.inserted_entry_id.is_some());

        // Second evaluation of the SAME plaintext should see the prior entry
        // as a near-perfect (cosine ~ 1.0) neighbor → novelty collapses to
        // near zero.
        let second = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(
            second.novelty_score_micros < 10_000,
            "expected near-zero novelty after self-insert, got {}",
            second.novelty_score_micros
        );
    }

    #[test]
    fn high_novelty_floor_fails_a_duplicate_trace() {
        // Very high novelty floor: a re-submitted trace fails because the
        // index already contains its embedding.
        let orch = orch_with_floors(0, 0, 900_000);
        let first = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(first.novelty_passed, "fresh trace should pass novelty");
        let second = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(
            !second.novelty_passed,
            "duplicate trace should fail under a high novelty floor"
        );
        // Failed → orchestrator did NOT insert.
        assert!(second.inserted_entry_id.is_none());
    }

    #[test]
    fn impossible_perplexity_floor_fails() {
        let orch = orch_with_floors(u64::MAX, 0, 0);
        let d = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(!d.perplexity_passed);
        assert!(d.inserted_entry_id.is_none());
    }

    #[test]
    fn distinct_tenants_do_not_cross_pollinate() {
        let orch = orch_with_floors(0, 0, 900_000);
        // Insert under tenant_a.
        let a = orch.evaluate(b"hello world", "tenant_a").unwrap();
        assert!(a.novelty_passed);
        // Same plaintext under tenant_b → tenant_b's index is empty, so
        // novelty stays at 1.0.
        let b = orch.evaluate(b"hello world", "tenant_b").unwrap();
        assert!(b.novelty_passed);
    }
}
