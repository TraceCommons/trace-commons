// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Bounded synthetic novelty smoke test, not a fitted production calibration.
//! Real FastEmbed, standard chunker/aggregation, isolated exact cosine index.
//! Single-chunk controls make mock perplexity token weights irrelevant.
#[cfg(feature = "near-ai-scorer")]
#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    use trace_commons_gate_api::EnclaveGateOrchestratorConfig;
    use trace_commons_gate_enclave::{
        embedder_fastembed::FastEmbedTextEmbedder, orchestrator::EnclaveGateOrchestrator,
        perplexity::MockPerplexityScorer, vector_index::MockVectorIndex,
    };
    let cache = std::env::var("TRACE_COMMONS_EMBEDDER_CACHE_DIR")?;
    let embedder =
        FastEmbedTextEmbedder::try_new("BAAI/bge-large-en-v1.5", cache, None, 512).await?;
    let mut cfg = EnclaveGateOrchestratorConfig::mock_default();
    cfg.perplexity_floor_micros = 0;
    cfg.tail_fraction_floor_micros = 0;
    cfg.novelty_floor_micros = 0;
    cfg.chunk_target_tokens = 2048;
    cfg.chunk_max_tokens = 3072;
    cfg.chunk_cap = 16;
    cfg.chunk_min_tokens = 64;
    cfg.embed_insert_novelty_micros = 50_000;
    let orch = EnclaveGateOrchestrator::new(
        MockPerplexityScorer::new(),
        embedder,
        MockVectorIndex::new(),
        cfg,
    );
    let fixtures: serde_json::Value = serde_json::from_str(include_str!("novelty_controls.json"))?;
    for (i, case) in fixtures.as_array().unwrap().iter().enumerate() {
        let tenant = format!("synthetic-control-{i}");
        let envelope = |text: &str| {
            serde_json::to_vec(&serde_json::json!({"events": [
            {"event_type": "user_message", "redacted_content": "Investigate the problem and verify the repair."},
            {"event_type": "assistant_message", "redacted_content": text}
        ]})).unwrap()
        };
        let base = case["base"].as_str().unwrap();
        let seed = orch.evaluate(&envelope(base), &tenant)?;
        assert_eq!(seed.chunk_count, 1);
        assert_eq!(seed.novelty_score_micros, 1_000_000);
        for kind in ["exact", "near", "distinct", "distinct_same_domain"] {
            let text = if kind == "exact" {
                base
            } else {
                case[kind].as_str().unwrap()
            };
            let decision = orch.evaluate(&envelope(text), &tenant)?;
            assert_eq!(decision.chunk_count, 1);
            println!(
                "{}",
                serde_json::json!({"case": i, "kind": kind,
                "novelty_micros": decision.novelty_score_micros,
                "passes_100000": decision.novelty_score_micros >= 100_000,
                "passes_500000": decision.novelty_score_micros >= 500_000})
            );
            assert_eq!(
                decision.novelty_score_micros >= 100_000,
                kind.starts_with("distinct"),
                "case {i} {kind}: unexpected classification at the 0.10 holding floor"
            );
            assert!(
                decision.novelty_score_micros < 500_000,
                "case {i} {kind}: stale 0.50 floor regression changed"
            );
            for entry in decision.inserted_chunk_entries {
                orch.delete_vector_entry(&tenant, entry.entry_id)?;
            }
        }
    }
    Ok(())
}
#[cfg(not(feature = "near-ai-scorer"))]
fn main() {
    panic!("requires --features near-ai-scorer");
}
