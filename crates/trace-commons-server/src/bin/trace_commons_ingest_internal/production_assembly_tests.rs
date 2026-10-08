// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use trace_commons_protocol::llm::recording::{TraceFile, TraceResponse, TraceStep};
use trace_commons_protocol::trace_contribution::{
    DeterministicTraceRedactor, RecordedTraceContributionOptions, TraceRedactor,
};

/// The five development markers `validate_production_package` refuses in any
/// artifact of a production package.
const DEVELOPMENT_MARKERS: [&str; 5] = [
    "local_reference",
    "reference_",
    "pipeline-test",
    "mock_",
    "synthetic",
];

fn lookup_from(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map = pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect::<BTreeMap<_, _>>();
    move |key: &str| map.get(key).cloned()
}

fn scorer_env() -> Vec<(&'static str, &'static str)> {
    vec![
        (TRACE_COMMONS_NEAR_AI_MODEL, "Qwen/Qwen3.6-35B-A3B-FP8"),
        (TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF, "-8.5"),
        (TRACE_COMMONS_NEAR_AI_BASE_URL, "https://scorer.invalid/v1"),
        (TRACE_COMMONS_NEAR_AI_API_KEY, "key-one"),
        (TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS, "30"),
        ("TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS", "2048"),
    ]
}

fn with(
    base: &[(&'static str, &'static str)],
    key: &'static str,
    value: &'static str,
) -> Vec<(&'static str, &'static str)> {
    let mut pairs = base
        .iter()
        .filter(|(existing, _)| *existing != key)
        .copied()
        .collect::<Vec<_>>();
    pairs.push((key, value));
    pairs
}

fn scorer_descriptor(pairs: &[(&str, &str)]) -> NearAiScorerDescriptor {
    NearAiScorerDescriptor::from_lookup(&lookup_from(pairs)).expect("descriptor parses")
}

/// Spec 4.2 item 2: the scorer descriptor is a pure function of the model
/// pin and the scorer's own configuration.
#[test]
fn scorer_descriptor_is_a_pure_function_of_the_pin() {
    let base = scorer_env();
    let descriptor = scorer_descriptor(&base);
    assert_eq!(descriptor.bytes(), scorer_descriptor(&base).bytes());
    assert_eq!(
        String::from_utf8(descriptor.bytes()).unwrap(),
        "{\"logprobs_top_k\":1,\"model\":\"Qwen/Qwen3.6-35B-A3B-FP8\",\
         \"schema\":\"trace_commons.near_ai_scorer_descriptor.v1\",\
         \"tail_logprob_cutoff_micros\":-8500000}"
    );
    for (key, value) in [
        (TRACE_COMMONS_NEAR_AI_MODEL, "Qwen/Qwen3.8-27B"),
        (TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF, "-7"),
    ] {
        assert_ne!(
            scorer_descriptor(&with(&base, key, value)).hash(),
            descriptor.hash(),
            "{key} is part of the pin"
        );
    }
    let mut other_top_k = descriptor.clone();
    other_top_k.logprobs_top_k = 2;
    assert_ne!(other_top_k.hash(), descriptor.hash());
    for (key, value) in [
        (TRACE_COMMONS_NEAR_AI_API_KEY, "key-two"),
        (TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS, "90"),
        (TRACE_COMMONS_NEAR_AI_BASE_URL, "https://other.invalid/v1"),
        ("TRACE_COMMONS_GATE_CHUNK_TARGET_TOKENS", "1024"),
    ] {
        assert_eq!(
            scorer_descriptor(&with(&base, key, value)).hash(),
            descriptor.hash(),
            "{key} is not part of the pin"
        );
    }
    // An unset cutoff is the scorer's default, the same value the legacy
    // gate scores with.
    let defaulted = scorer_env()
        .into_iter()
        .filter(|(key, _)| *key != TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF)
        .collect::<Vec<_>>();
    assert_eq!(
        scorer_descriptor(&defaulted).tail_logprob_cutoff,
        TRACE_COMMONS_PERPLEXITY_DEFAULT_TAIL_LOGPROB_CUTOFF
    );
}

/// A missing model or a cutoff that is not a finite number refuses, with a
/// label that names no value.
#[test]
fn scorer_descriptor_refuses_a_missing_or_malformed_pin() {
    let no_model = scorer_env()
        .into_iter()
        .filter(|(key, _)| *key != TRACE_COMMONS_NEAR_AI_MODEL)
        .collect::<Vec<_>>();
    assert_eq!(
        NearAiScorerDescriptor::from_lookup(&lookup_from(&no_model))
            .unwrap_err()
            .to_string(),
        "pipeline_scorer_model_missing"
    );
    for cutoff in ["not-a-number", "NaN", "inf"] {
        assert_eq!(
            NearAiScorerDescriptor::from_lookup(&lookup_from(&with(
                &scorer_env(),
                TRACE_COMMONS_PERPLEXITY_TAIL_LOGPROB_CUTOFF,
                cutoff
            )))
            .unwrap_err()
            .to_string(),
            "pipeline_scorer_tail_cutoff_invalid",
            "{cutoff}"
        );
    }
}

fn embedder_env() -> Vec<(&'static str, &'static str)> {
    vec![
        (TRACE_COMMONS_EMBEDDER_MODEL_ID, "BAAI/bge-large-en-v1.5"),
        (TRACE_COMMONS_EMBEDDER_MAX_TOKENS, "512"),
        (TRACE_COMMONS_VECTOR_INDEX_DIM, "1024"),
        (TRACE_COMMONS_EMBEDDER_CACHE_DIR, "/var/cache/one"),
    ]
}

fn embedder_descriptor(pairs: &[(&str, &str)]) -> FastEmbedDescriptor {
    FastEmbedDescriptor::from_lookup(&lookup_from(pairs)).expect("descriptor parses")
}

/// Spec 4.2 item 3.
#[test]
fn embedder_descriptor_changes_with_model_dim_and_tokens() {
    let base = embedder_env();
    let descriptor = embedder_descriptor(&base);
    assert_eq!(
        String::from_utf8(descriptor.bytes()).unwrap(),
        "{\"matryoshka_dim\":null,\"max_tokens\":512,\"model_id\":\"BAAI/bge-large-en-v1.5\",\
         \"output_dim\":1024,\"schema\":\"trace_commons.fastembed_embedder_descriptor.v1\"}"
    );
    for (key, value) in [
        (TRACE_COMMONS_EMBEDDER_MODEL_ID, "BAAI/bge-small-en-v1.5"),
        (TRACE_COMMONS_EMBEDDER_MAX_TOKENS, "256"),
        (TRACE_COMMONS_VECTOR_INDEX_DIM, "768"),
        (TRACE_COMMONS_EMBEDDER_MATRYOSHKA_DIM, "768"),
    ] {
        assert_ne!(
            embedder_descriptor(&with(&base, key, value)).hash(),
            descriptor.hash(),
            "{key} is part of the descriptor"
        );
    }
    assert_eq!(
        embedder_descriptor(&with(
            &base,
            TRACE_COMMONS_EMBEDDER_CACHE_DIR,
            "/var/cache/two"
        ))
        .hash(),
        descriptor.hash(),
        "the cache directory is not"
    );
    let defaulted = embedder_descriptor(&[]);
    assert_eq!(defaulted.model_id, TRACE_COMMONS_EMBEDDER_DEFAULT_MODEL_ID);
    assert_eq!(
        defaulted.max_tokens,
        TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS
    );
    assert_eq!(defaulted.output_dim, TRACE_COMMONS_VECTOR_INDEX_DEFAULT_DIM);
}

struct FixedScorer;

impl trace_commons_gate_api::PerplexityScorer for FixedScorer {
    fn score(&self, _plaintext: &[u8]) -> anyhow::Result<trace_commons_gate_api::PerplexityResult> {
        Ok(trace_commons_gate_api::PerplexityResult {
            aggregate_perplexity_micros: 3_000_000,
            tail_fraction_micros: 100_000,
            tokens_scored: 4,
        })
    }

    fn score_chunk(
        &self,
        _chunk: &[u8],
    ) -> anyhow::Result<trace_commons_gate_api::ChunkPerplexity> {
        Ok(trace_commons_gate_api::ChunkPerplexity {
            sum_nll: 2.0,
            tokens: 2,
            tail_tokens: 0,
            logprobs: vec![-1.0, -1.0],
            token_char_lens: vec![1, 1, 1],
        })
    }
}

struct FixedEmbedder;

impl trace_commons_gate_api::Embedder for FixedEmbedder {
    fn embed(&self, _plaintext: &[u8]) -> anyhow::Result<Vec<f32>> {
        Ok(vec![1.0, 0.0])
    }
}

fn pipeline_scorer() -> NearAiPipelineScorer {
    NearAiPipelineScorer::new(Arc::new(FixedScorer), scorer_descriptor(&scorer_env()))
}

fn pipeline_embedder() -> FastEmbedPipelineEmbedder {
    FastEmbedPipelineEmbedder::new(
        Arc::new(FixedEmbedder),
        embedder_descriptor(&embedder_env()),
    )
}

/// The adapters forward to the component they wrap, the lossless
/// `score_chunk` included, and report the identities spec A-D4 and A-D5
/// pin.
#[test]
fn adapter_identities_are_safe_labels() {
    use trace_commons_gate_api::{
        Embedder, IdentifiedEmbedder, IdentifiedPerplexityScorer, PerplexityScorer,
    };
    let scorer = pipeline_scorer();
    assert_eq!(scorer.dependency_identity(), "near_ai_perplexity_scorer");
    assert!(scorer.production_qualified());
    assert_eq!(scorer.content_descriptor(), scorer.descriptor().bytes());
    assert_eq!(scorer.score_chunk(b"x").unwrap().logprobs, vec![-1.0, -1.0]);
    assert_eq!(scorer.score(b"x").unwrap().tokens_scored, 4);

    let embedder = pipeline_embedder();
    assert_eq!(embedder.dependency_identity(), "fastembed_text_embedder");
    assert_eq!(embedder.model_id(), "BAAI/bge-large-en-v1.5");
    assert!(embedder.production_qualified());
    assert_eq!(embedder.content_descriptor(), embedder.descriptor().bytes());
    assert_eq!(embedder.embed(b"x").unwrap(), vec![1.0, 0.0]);

    for label in [
        scorer.dependency_identity(),
        embedder.dependency_identity(),
        USEARCH_PIPELINE_INDEX_READER_IDENTITY,
        USEARCH_PIPELINE_INDEX_WRITER_IDENTITY,
        INTERNAL_TRACE_CREDIT_ADAPTER_IDENTITY,
    ] {
        assert!(
            trace_commons_server::versioned_pipeline_qualification::is_safe_label(label),
            "{label}"
        );
    }
}

/// `validate_production_package` refuses an artifact carrying any of the five
/// development markers; neither descriptor, nor the compatibility ids the
/// assembler binds, carries one.
#[test]
fn descriptors_carry_no_development_marker() {
    let scorer = scorer_descriptor(&scorer_env());
    let texts = [
        String::from_utf8(scorer.bytes()).unwrap(),
        String::from_utf8(embedder_descriptor(&embedder_env()).bytes()).unwrap(),
        scorer.compatibility_scorer_model_id(),
        PRODUCTION_COMPATIBILITY_PROJECTION_ID.to_string(),
        PRODUCTION_COMPATIBILITY_INDEX_ID.to_string(),
    ];
    for text in &texts {
        for marker in DEVELOPMENT_MARKERS {
            assert!(!text.contains(marker), "{text} contains {marker}");
        }
        assert!(!text.contains("test"), "{text}");
    }
    assert!(
        scorer
            .compatibility_scorer_model_id()
            .starts_with("near_ai:")
    );
}

/// Spec 4.2 item 5: the pipeline index refuses a root equal to, or nested in
/// either direction with, the legacy novelty or dedup root (two indexes on
/// one root corrupt each other's `nearest`).
#[test]
fn pipeline_index_root_must_not_be_the_legacy_root() {
    let legacy = std::path::Path::new("/var/lib/trace-commons-vector-index");
    let dedup = std::path::Path::new("/var/lib/trace-commons-vector-index/../dedup-index");
    for shared in [
        "/var/lib/trace-commons-vector-index",
        "/var/lib/trace-commons-vector-index/",
        "/var/lib/trace-commons-vector-index/pipeline",
        "/var/lib/./trace-commons-vector-index/pipeline",
        "/var/lib",
        "/var/lib/dedup-index",
        "/var/lib/dedup-index/pipeline",
        "/var/lib/other/../trace-commons-vector-index/pipeline",
    ] {
        assert_eq!(
            validate_pipeline_index_root(std::path::Path::new(shared), &[legacy, dedup])
                .unwrap_err()
                .to_string(),
            "pipeline_vector_index_root_shared",
            "{shared}"
        );
    }
    for separate in [
        "/var/lib/trace-commons-pipeline-index",
        "/var/lib/trace-commons-vector-index-pipeline",
        "/srv/pipeline-index",
    ] {
        validate_pipeline_index_root(std::path::Path::new(separate), &[legacy, dedup])
            .unwrap_or_else(|error| panic!("{separate}: {error}"));
    }
}

#[cfg(feature = "near-ai-scorer")]
mod usearch_pipeline_index {
    use super::*;
    use trace_commons_gate_api::pipeline::TenantStorageRef;
    use trace_commons_gate_api::{
        IndexEntryKey, IndexUpsertResult, IndexWriteError, VectorIndexReader, VectorIndexWriter,
    };

    const DIM: usize = 4;

    fn usearch_test_config(
        dim: usize,
    ) -> trace_commons_gate_enclave::vector_index_usearch::UsearchVectorIndexConfig {
        trace_commons_gate_enclave::vector_index_usearch::UsearchVectorIndexConfig {
            dim,
            hnsw_m: 16,
            ef_construction: 64,
            ef_search: 64,
            max_open: 8,
            flush_every: 1_000,
            flush_interval: None,
        }
    }

    fn open(root: &std::path::Path) -> anyhow::Result<UsearchPipelineIndex> {
        UsearchPipelineIndex::open(root, usearch_test_config(DIM))
    }

    fn tenant(n: u8) -> TenantStorageRef {
        TenantStorageRef::new(format!("tenant_sha256:{}", format!("{n:02x}").repeat(16))).unwrap()
    }

    fn key(tenant_ref: &TenantStorageRef, revision: u128, chunk: u32) -> IndexEntryKey {
        IndexEntryKey {
            tenant_storage_ref: tenant_ref.clone(),
            index_id: PRODUCTION_COMPATIBILITY_INDEX_ID.to_string(),
            revision_id: Uuid::from_u128(revision),
            projection_id: PRODUCTION_COMPATIBILITY_PROJECTION_ID.to_string(),
            model_id: "BAAI/bge-large-en-v1.5".to_string(),
            chunk,
        }
    }

    fn unit(axis: usize) -> Vec<f32> {
        let mut vector = vec![0.0; DIM];
        vector[axis] = 1.0;
        vector
    }

    /// The writer contract `IsolatedPipelineIndex`'s tests cover, against
    /// the usearch-backed index.
    #[test]
    fn usearch_pipeline_index_meets_the_writer_contract() {
        let dir = tempfile::tempdir().unwrap();
        let index = open(dir.path()).unwrap();
        let a = tenant(1);
        let b = tenant(2);
        let index_id = PRODUCTION_COMPATIBILITY_INDEX_ID;

        let first = key(&a, 1, 0);
        assert_eq!(
            index.upsert(&first, &unit(0), "sha256:one").unwrap(),
            IndexUpsertResult::Inserted
        );
        assert_eq!(
            index.upsert(&first, &unit(0), "sha256:one").unwrap(),
            IndexUpsertResult::Unchanged
        );
        assert_eq!(
            index.upsert(&first, &unit(0), "sha256:other").unwrap_err(),
            IndexWriteError::ContentConflict
        );
        assert_eq!(
            index.upsert(&first, &unit(1), "sha256:one").unwrap_err(),
            IndexWriteError::ContentConflict
        );
        index
            .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
            .unwrap();
        index
            .upsert(&key(&b, 3, 0), &unit(0), "sha256:three")
            .unwrap();

        // `nearest` answers the real entry ids, best first, within one
        // tenant, and leaves out the excluded revision.
        let nearest = index.nearest(&a, index_id, &unit(0), 2, None).unwrap();
        assert_eq!(nearest.len(), 2);
        assert_eq!(nearest[0].entry_id, first.entry_id());
        assert!((nearest[0].similarity - 1.0).abs() < 1e-4);
        assert_eq!(nearest[1].entry_id, key(&a, 2, 0).entry_id());
        let excluded = index
            .nearest(&a, index_id, &unit(0), 2, Some(Uuid::from_u128(1)))
            .unwrap();
        assert_eq!(
            excluded
                .iter()
                .map(|neighbor| neighbor.entry_id)
                .collect::<Vec<_>>(),
            vec![key(&a, 2, 0).entry_id()]
        );
        assert!(
            index
                .nearest(&a, "another_index", &unit(0), 2, None)
                .unwrap()
                .is_empty()
        );

        assert_eq!(index.snapshot(&a, index_id).unwrap().cardinality, 2);
        assert_eq!(index.snapshot(&b, index_id).unwrap().cardinality, 1);

        // `invalidate_revision` removes that revision's entries only, and is
        // idempotent.
        assert!(
            index
                .invalidate_revision(&a, index_id, Uuid::from_u128(1))
                .unwrap()
        );
        assert!(
            !index
                .invalidate_revision(&a, index_id, Uuid::from_u128(1))
                .unwrap()
        );
        assert_eq!(index.snapshot(&a, index_id).unwrap().cardinality, 1);
        assert_eq!(index.snapshot(&b, index_id).unwrap().cardinality, 1);
        let after = index.nearest(&a, index_id, &unit(0), 5, None).unwrap();
        assert_eq!(
            after
                .iter()
                .map(|neighbor| neighbor.entry_id)
                .collect::<Vec<_>>(),
            vec![key(&a, 2, 0).entry_id()]
        );
        // A removed entry may be written again.
        assert_eq!(
            index.upsert(&first, &unit(0), "sha256:one").unwrap(),
            IndexUpsertResult::Inserted
        );
    }

    /// The snapshot hash depends on the entries, not the order they were
    /// written in.
    #[test]
    fn usearch_pipeline_snapshot_hash_is_order_independent() {
        let a = tenant(1);
        let index_id = PRODUCTION_COMPATIBILITY_INDEX_ID;
        let entries = [
            (key(&a, 1, 0), unit(0), "sha256:one"),
            (key(&a, 1, 1), unit(1), "sha256:two"),
            (key(&a, 2, 0), unit(2), "sha256:three"),
        ];
        let forward_dir = tempfile::tempdir().unwrap();
        let forward = open(forward_dir.path()).unwrap();
        for (entry, embedding, hash) in &entries {
            forward.upsert(entry, embedding, hash).unwrap();
        }
        let reverse_dir = tempfile::tempdir().unwrap();
        let reverse = open(reverse_dir.path()).unwrap();
        for (entry, embedding, hash) in entries.iter().rev() {
            reverse.upsert(entry, embedding, hash).unwrap();
        }
        let left = forward.snapshot(&a, index_id).unwrap();
        let right = reverse.snapshot(&a, index_id).unwrap();
        assert_eq!(left, right);
        assert_eq!(left.cardinality, 3);
        let empty = open(tempfile::tempdir().unwrap().path())
            .unwrap()
            .snapshot(&a, index_id)
            .unwrap();
        assert_ne!(empty.snapshot_hash, left.snapshot_hash);
        assert_eq!(empty.cardinality, 0);
    }

    /// The manifest is persisted beside the usearch files, so a reopened
    /// index answers the same entries, snapshot and real entry ids.
    #[test]
    fn manifest_survives_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let a = tenant(1);
        let index_id = PRODUCTION_COMPATIBILITY_INDEX_ID;
        let before = {
            let index = open(dir.path()).unwrap();
            index
                .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
                .unwrap();
            index
                .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
                .unwrap();
            index.snapshot(&a, index_id).unwrap()
        };
        let reopened = open(dir.path()).unwrap();
        assert_eq!(reopened.snapshot(&a, index_id).unwrap(), before);
        assert_eq!(
            reopened
                .nearest(&a, index_id, &unit(1), 1, None)
                .unwrap()
                .first()
                .map(|neighbor| neighbor.entry_id),
            Some(key(&a, 2, 0).entry_id())
        );
        assert_eq!(
            reopened
                .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
                .unwrap(),
            IndexUpsertResult::Unchanged
        );
    }

    /// A manifest that does not match its usearch file refuses the start.
    #[test]
    fn truncated_manifest_refuses_open() {
        let dir = tempfile::tempdir().unwrap();
        let a = tenant(1);
        {
            let index = open(dir.path()).unwrap();
            index
                .upsert(&key(&a, 1, 0), &unit(0), "sha256:one")
                .unwrap();
            index
                .upsert(&key(&a, 2, 0), &unit(1), "sha256:two")
                .unwrap();
        }
        let manifests = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| path.to_string_lossy().ends_with(".manifest.json"))
            .collect::<Vec<_>>();
        assert_eq!(manifests.len(), 1);
        let manifest: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&manifests[0]).unwrap()).unwrap();
        let mut truncated = manifest.clone();
        let entries = truncated["entries"].as_object_mut().unwrap();
        let first = entries.keys().next().unwrap().clone();
        entries.remove(&first);
        std::fs::write(&manifests[0], serde_json::to_vec(&truncated).unwrap()).unwrap();
        assert_eq!(
            open(dir.path()).err().unwrap().to_string(),
            "pipeline_vector_index_manifest_mismatch"
        );
        std::fs::write(&manifests[0], b"{not json").unwrap();
        assert_eq!(
            open(dir.path()).err().unwrap().to_string(),
            "pipeline_vector_index_manifest_mismatch"
        );
    }

    /// Every write persists before it returns, and returns well within the
    /// 60 s index write fence margin (measured with a generous bound).
    #[test]
    fn flush_returns_within_the_fence_margin() {
        let dir = tempfile::tempdir().unwrap();
        let index = open(dir.path()).unwrap();
        let a = tenant(1);
        let started = std::time::Instant::now();
        for revision in 0..50 {
            index
                .upsert(
                    &key(&a, revision, 0),
                    &unit((revision % 4) as usize),
                    "sha256:x",
                )
                .unwrap();
        }
        index
            .invalidate_revision(&a, PRODUCTION_COMPATIBILITY_INDEX_ID, Uuid::from_u128(3))
            .unwrap();
        let elapsed = started.elapsed();
        assert!(
            elapsed
                < std::time::Duration::from_secs(
                    trace_commons_server::versioned_pipeline::PIPELINE_INDEX_WRITE_FENCE_MARGIN_SECONDS
                        as u64
                        / 4
                ),
            "{elapsed:?}"
        );
        drop(index);
        assert_eq!(
            open(dir.path())
                .unwrap()
                .snapshot(&a, PRODUCTION_COMPATIBILITY_INDEX_ID)
                .unwrap()
                .cardinality,
            49
        );
    }
}

fn trace_credit_request(atomic_units: u128) -> trace_commons_gate_api::SettlementRequest {
    trace_commons_gate_api::SettlementRequest::new(
        trace_commons_gate_api::pipeline::TenantStorageRef::new(format!(
            "tenant_sha256:{}",
            "ab".repeat(16)
        ))
        .unwrap(),
        Uuid::from_u128(5),
        trace_commons_gate_api::pipeline::InstrumentId::trace_credit(),
        trace_commons_gate_api::pipeline::AtomicUnits::from_raw(atomic_units),
        format!("sha256:{}", "1".repeat(64)),
        format!("sha256:{}", "2".repeat(64)),
    )
    .unwrap()
}

/// Spec 4.2 item 6: the production Trace Credit adapter answers an internal
/// receipt for the expected result, the same one every time, settles only
/// `trace_credit`, and is production-qualified.
#[tokio::test]
async fn internal_trace_credit_adapter_is_idempotent_and_qualified() {
    use trace_commons_gate_api::SettlementAdapter;
    let adapter = InternalTraceCreditSettlementAdapter::new();
    assert_eq!(
        adapter.instrument_id(),
        &trace_commons_gate_api::pipeline::InstrumentId::trace_credit()
    );
    assert_eq!(adapter.adapter_identity(), "internal_trace_credit_ledger");
    assert_eq!(adapter.payout_rail(), "none");
    assert!(adapter.production_qualified());
    let request = trace_credit_request(1_000);
    let first = adapter.settle(&request).await.unwrap();
    let second = adapter.settle(&request).await.unwrap();
    assert_eq!(first, second);
    assert!(first.answers(&request));
    assert_eq!(first.result_ref_hash(), request.expected_result_ref_hash());
    assert_eq!(first.external_receipt_hash(), None, "no external effect");
}

async fn policy_test_envelope(
    scopes: &[ConsentScope],
    card_scope: ConsentScope,
    uses: &[TraceAllowedUse],
) -> TraceContributionEnvelope {
    let trace = TraceFile {
        model_name: "fixture-model".to_string(),
        memory_snapshot: Vec::new(),
        http_exchanges: Vec::new(),
        steps: vec![TraceStep {
            request_hint: None,
            response: TraceResponse::UserInput {
                content: "Please inspect the workspace".to_string(),
            },
            expected_tool_results: Vec::new(),
            timestamp: None,
        }],
    };
    let raw = trace_commons_protocol::trace_contribution::RawTraceContribution::from_recorded_trace(
        &trace,
        RecordedTraceContributionOptions {
            include_message_text: true,
            pseudonymous_contributor_id: Some("sha256:contributor".to_string()),
            tenant_scope_ref: Some("tenant_sha256:client".to_string()),
            ..Default::default()
        },
    );
    let mut envelope = DeterministicTraceRedactor::default()
        .redact_trace(raw)
        .await
        .expect("redaction should succeed");
    envelope.consent.scopes = scopes.to_vec();
    envelope.trace_card.consent_scope = card_scope;
    envelope.trace_card.allowed_uses = uses.to_vec();
    envelope
}

fn policy_test_auth(tenant_id: &str) -> TenantAuth {
    TenantAuth {
        tenant_id: tenant_id.to_string(),
        role: TokenRole::Contributor,
        principal_ref: static_token_principal_ref("contributor-token"),
        legacy_principal_ref: None,
        expires_at: None,
        auth_method: TraceAuthMethod::StaticToken,
        signed_claim_issuer: None,
        signed_claim_audiences: BTreeSet::new(),
        signed_claim_subject: None,
        allowed_consent_scopes: BTreeSet::new(),
        allowed_uses: BTreeSet::new(),
    }
}

/// Spec 4.2 item 7 (O-A1): for a tenant present in
/// `TRACE_COMMONS_TENANT_POLICIES`, one present with empty allowlists, and
/// one absent, under both values of
/// `TRACE_COMMONS_REQUIRE_TENANT_SUBMISSION_POLICY`, the pipeline's grant
/// check (`SubmissionAuthority::permits`) answers what the legacy admission
/// check (`enforce_tenant_submission_policy`) answers, and the authority
/// carries the tenant's policy, which the pipeline's `NoveltyUtility` leg
/// reads as `main`'s utility credit check reads it. A tenant whose policy
/// `main` reads from the database is answered `None` (the pipeline refuses
/// it with `authority_control_missing`), because this provider cannot read
/// the database.
#[tokio::test]
async fn tenant_policy_authority_matches_legacy_admission() {
    let mut policies = BTreeMap::new();
    policies.insert(
        "tenant-present".to_string(),
        TenantSubmissionPolicy {
            allowed_consent_scopes: BTreeSet::from([ConsentScope::DebuggingEvaluation]),
            allowed_uses: BTreeSet::from([TraceAllowedUse::Evaluation]),
        },
    );
    policies.insert(
        "tenant-empty".to_string(),
        TenantSubmissionPolicy {
            allowed_consent_scopes: BTreeSet::new(),
            allowed_uses: BTreeSet::new(),
        },
    );
    policies.insert(
        "tenant-db".to_string(),
        TenantSubmissionPolicy {
            allowed_consent_scopes: BTreeSet::new(),
            allowed_uses: BTreeSet::new(),
        },
    );
    let policies = Arc::new(policies);
    let inside = policy_test_envelope(
        &[ConsentScope::DebuggingEvaluation],
        ConsentScope::DebuggingEvaluation,
        &[TraceAllowedUse::Evaluation],
    )
    .await;
    let outside_scope = policy_test_envelope(
        &[ConsentScope::ModelTraining],
        ConsentScope::ModelTraining,
        &[TraceAllowedUse::Evaluation],
    )
    .await;
    let outside_use = policy_test_envelope(
        &[ConsentScope::DebuggingEvaluation],
        ConsentScope::DebuggingEvaluation,
        &[TraceAllowedUse::ModelTraining],
    )
    .await;

    for require_policy in [false, true] {
        let provider = TenantPolicyPipelineAuthorityProvider::new(
            policies.clone(),
            require_policy,
            Arc::new(|tenant_id: &str| tenant_id == "tenant-db"),
        );
        assert!(
            trace_commons_server::versioned_pipeline_authority::PipelineAuthorityProvider::production_qualified(&provider)
        );
        assert_eq!(
            trace_commons_server::versioned_pipeline_authority::PipelineAuthorityProvider::authority_for_tenant(&provider, "tenant-db"),
            None
        );
        for tenant_id in ["tenant-present", "tenant-empty", "tenant-absent"] {
            let authority =
                trace_commons_server::versioned_pipeline_authority::PipelineAuthorityProvider::authority_for_tenant(&provider, tenant_id)
                    .expect("an env-policy tenant has an authority");
            assert_eq!(authority.require_policy, require_policy);
            assert_eq!(
                authority.tenant,
                trace_commons_server::trace_authority::SubmissionAllowlists::default(),
                "the per-token claim allowlist is per request; the handler enforces it before routing"
            );
            assert_eq!(
                authority.policy,
                policies.get(tenant_id).map(|policy| {
                    trace_commons_server::trace_authority::SubmissionAllowlists {
                        allowed_consent_scopes: policy.allowed_consent_scopes.clone(),
                        allowed_uses: policy.allowed_uses.clone(),
                    }
                }),
                "{tenant_id}"
            );
            for (case, envelope) in [
                ("inside", &inside),
                ("outside_scope", &outside_scope),
                ("outside_use", &outside_use),
            ] {
                let legacy = enforce_tenant_submission_policy(
                    &policy_test_auth(tenant_id),
                    envelope,
                    policies.get(tenant_id),
                    require_policy,
                )
                .is_ok();
                let mut scopes = envelope.consent.scopes.clone();
                if !scopes.contains(&envelope.trace_card.consent_scope) {
                    scopes.push(envelope.trace_card.consent_scope);
                }
                let pipeline = authority.permits(&scopes, &envelope.trace_card.allowed_uses);
                assert_eq!(
                    legacy, pipeline,
                    "{tenant_id} {case} require_policy={require_policy}"
                );
            }
        }
    }

    // Where the two checks differ: legacy admission requires every requested
    // scope to be allowed, `permits` any one. Legacy admission runs first
    // and refuses such a submission before it is routed, so the pipeline
    // never sees it.
    let mixed = policy_test_envelope(
        &[
            ConsentScope::DebuggingEvaluation,
            ConsentScope::ModelTraining,
        ],
        ConsentScope::DebuggingEvaluation,
        &[TraceAllowedUse::Evaluation],
    )
    .await;
    assert!(
        enforce_tenant_submission_policy(
            &policy_test_auth("tenant-present"),
            &mixed,
            policies.get("tenant-present"),
            false,
        )
        .is_err()
    );
}

/// Wraps `IsolatedPipelineIndex` and reports itself production-qualified,
/// standing in for the usearch-backed index in a default-features build.
struct QualifiedIsolatedIndex(
    Arc<trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex>,
);

impl trace_commons_gate_api::VectorIndexReader for QualifiedIsolatedIndex {
    fn snapshot(
        &self,
        tenant_storage_ref: &trace_commons_gate_api::pipeline::TenantStorageRef,
        index_id: &str,
    ) -> anyhow::Result<trace_commons_gate_api::IndexSnapshot> {
        self.0.snapshot(tenant_storage_ref, index_id)
    }

    fn nearest(
        &self,
        tenant_storage_ref: &trace_commons_gate_api::pipeline::TenantStorageRef,
        index_id: &str,
        embedding: &[f32],
        k: usize,
        exclude_revision: Option<Uuid>,
    ) -> anyhow::Result<Vec<trace_commons_gate_api::NearestNeighbor>> {
        self.0
            .nearest(tenant_storage_ref, index_id, embedding, k, exclude_revision)
    }
}

impl trace_commons_gate_api::VectorIndexWriter for QualifiedIsolatedIndex {
    fn upsert(
        &self,
        key: &trace_commons_gate_api::IndexEntryKey,
        embedding: &[f32],
        content_hash: &str,
    ) -> Result<trace_commons_gate_api::IndexUpsertResult, trace_commons_gate_api::IndexWriteError>
    {
        self.0.upsert(key, embedding, content_hash)
    }

    fn invalidate_revision(
        &self,
        tenant_storage_ref: &trace_commons_gate_api::pipeline::TenantStorageRef,
        index_id: &str,
        revision_id: Uuid,
    ) -> Result<bool, trace_commons_gate_api::IndexWriteError> {
        self.0
            .invalidate_revision(tenant_storage_ref, index_id, revision_id)
    }
}

impl trace_commons_gate_api::IdentifiedIndexReader for QualifiedIsolatedIndex {
    fn dependency_identity(&self) -> &str {
        USEARCH_PIPELINE_INDEX_READER_IDENTITY
    }

    fn production_qualified(&self) -> bool {
        true
    }
}

impl trace_commons_gate_api::IdentifiedIndexWriter for QualifiedIsolatedIndex {
    fn dependency_identity(&self) -> &str {
        USEARCH_PIPELINE_INDEX_WRITER_IDENTITY
    }

    fn production_qualified(&self) -> bool {
        true
    }
}

/// A privacy boundary that reports itself qualified and a prose-PII
/// classifier, standing in for `ClassifierRedactorPipelinePrivacyBoundary`
/// over a real backend. Finds nothing.
struct ClassifyingPrivacy;

#[async_trait::async_trait]
impl trace_commons_server::versioned_pipeline_authority::PipelinePrivacyBoundary
    for ClassifyingPrivacy
{
    async fn rescrub(
        &self,
        _envelope: &mut TraceContributionEnvelope,
    ) -> anyhow::Result<Vec<ResidualRiskCondition>> {
        Ok(Vec::new())
    }

    fn production_qualified(&self) -> bool {
        true
    }

    fn classifies_prose_pii(&self) -> bool {
        true
    }
}

/// `PipelineGateComponents` filled with qualified doubles: what the
/// `near-ai-scorer` environment builder fills with the NEAR AI scorer, the
/// fastembed embedder and the usearch index.
fn test_components(
    privacy: Option<
        Arc<dyn trace_commons_server::versioned_pipeline_authority::PipelinePrivacyBoundary>,
    >,
) -> Arc<PipelineGateComponents> {
    let index = Arc::new(QualifiedIsolatedIndex(
        trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex::new(),
    ));
    Arc::new(PipelineGateComponents {
        scorer: Arc::new(FixedScorer),
        scorer_descriptor: scorer_descriptor(&scorer_env()),
        embedder: Arc::new(FixedEmbedder),
        embedder_descriptor: embedder_descriptor(&with(
            &embedder_env(),
            TRACE_COMMONS_VECTOR_INDEX_DIM,
            "2",
        )),
        index_reader: index.clone(),
        index_writer: index,
        index_root_shared_with_legacy: false,
        authority: Arc::new(TenantPolicyPipelineAuthorityProvider::new(
            Arc::new(BTreeMap::new()),
            false,
            Arc::new(|_: &str| false),
        )),
        tenant_policy_count: 0,
        privacy_backend: privacy
            .is_some()
            .then_some(trace_commons_protocol::trace_contribution::PrivacyFilterBackendTag::NearAi),
        privacy,
    })
}

fn classifying_privacy()
-> Option<Arc<dyn trace_commons_server::versioned_pipeline_authority::PipelinePrivacyBoundary>> {
    Some(Arc::new(ClassifyingPrivacy))
}

async fn backend_without_a_database() -> Arc<PgBackend> {
    let unused_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    Arc::new(
        PgBackend::new(&DatabaseConfig::from_postgres_url(
            &format!("postgres://nobody@127.0.0.1:{unused_port}/none"),
            1,
        ))
        .await
        .unwrap(),
    )
}

async fn assembly_fixture(
    dir: &tempfile::TempDir,
) -> (TraceCorpusDbConnections, ConfiguredTraceArtifactStore) {
    let backend = backend_without_a_database().await;
    let connections = TraceCorpusDbConnections {
        database: backend.clone() as Arc<dyn Database>,
        postgres: backend,
    };
    let crypto = SecretsCrypto::new(SecretString::from(
        trace_commons_server::secrets::keychain::generate_master_key_hex(),
    ))
    .unwrap();
    let store = ConfiguredTraceArtifactStore::legacy(Arc::new(
        LocalEncryptedTraceArtifactStore::new(dir.path(), crypto),
    ));
    (connections, store)
}

/// `main`'s gate configuration for these tests: the pilot template's floors
/// (0, 0, 500000) and `main`'s defaults.
const MAIN_GATE: trace_commons_server::versioned_pipeline_compat::MainGateConfig =
    trace_commons_server::versioned_pipeline_compat::MainGateConfig {
        perplexity_floor_micros: Some(0),
        tail_fraction_floor_micros: Some(0),
        novelty_floor_micros: Some(500_000),
        embed_insert_novelty_micros: 50_000,
        top_k: 5,
        chunk_target_tokens: 2048,
        chunk_max_tokens: 3072,
        chunk_cap: 16,
        chunk_min_tokens: 64,
        novelty_utility_microcredits: 0,
    };

fn issuer_checks() -> PipelineNoveltyUtilityChecks {
    PipelineNoveltyUtilityChecks {
        issuer_principal_ref: Some("principal_sha256:pipeline_issuer".to_string()),
        ..PipelineNoveltyUtilityChecks::default()
    }
}

/// The inputs a production boot hands `assemble_ingest_pipeline_runtime`,
/// each overridable by a case.
struct Boot {
    production_required: bool,
    tenants_processed: bool,
    allow_test_dependencies: bool,
    unqualified_routing_allowed: bool,
    near_payout_controls: PipelineNearPayoutControls,
    main_gate: trace_commons_server::versioned_pipeline_compat::MainGateConfig,
    components: Option<Arc<PipelineGateComponents>>,
}

impl Boot {
    fn production() -> Self {
        Self {
            production_required: true,
            tenants_processed: true,
            allow_test_dependencies: false,
            unqualified_routing_allowed: false,
            near_payout_controls: PipelineNearPayoutControls {
                settlement_mode: PipelineNearSettlementMode::Disabled,
                require_adapter_auth: false,
            },
            main_gate: MAIN_GATE,
            components: Some(test_components(classifying_privacy())),
        }
    }

    async fn assemble(
        self,
        assembler: &dyn IngestPipelineRuntimeAssembler,
    ) -> anyhow::Result<Arc<PipelineService>> {
        let dir = tempfile::tempdir().unwrap();
        let (connections, store) = assembly_fixture(&dir).await;
        assemble_ingest_pipeline_runtime_with_components(
            Some(assembler),
            Some(&connections),
            Some(&store),
            self.production_required,
            PipelineLeaseConfig::default(),
            self.tenants_processed,
            self.allow_test_dependencies,
            self.unqualified_routing_allowed,
            None,
            StdDuration::from_secs(60),
            self.near_payout_controls,
            &issuer_checks(),
            self.main_gate,
            self.components,
        )
        .map(|service| service.expect("an assembler was given"))
    }
}

/// Spec 4.2 item 8: through `assemble_ingest_pipeline_runtime`, a correctly
/// configured production assembly passes every startup refusal --
/// production-qualified, `main`'s gate configuration, no zero floor, the
/// privacy requirement met -- and binds the compatibility bundle under the
/// production ids.
#[tokio::test]
async fn production_assembler_passes_every_startup_refusal() {
    let service = Boot::production()
        .assemble(&ProductionPipelineAssembler)
        .await
        .expect("a correctly configured production assembly starts");
    assert!(pipeline_runtime_is_production_qualified(&service));
    pipeline_runtime::validate_pipeline_privacy_filter_requirement(true, &service).unwrap();
    assert!(service.binds_compatibility_bundle());
    assert!(!service.payout_enabled());
    let qualification = service
        .bundle_qualification(service.default_package())
        .unwrap();
    assert!(qualification.blockers().is_empty(), "{qualification:?}");
    assert_eq!(qualification.scorer.identity, "near_ai_perplexity_scorer");
    assert_eq!(qualification.embedder.identity, "fastembed_text_embedder");
    assert_eq!(
        qualification.settlement_adapters["trace_credit"].identity,
        "internal_trace_credit_ledger"
    );
    trace_commons_server::versioned_pipeline_qualification::validate_production_package(
        service.default_package(),
    )
    .expect("the production package carries no development marker");
    let config = service.compatibility_config().unwrap();
    assert!(config.matches_main_gate(&MAIN_GATE));
    assert_eq!(
        config.scorer_model_id,
        scorer_descriptor(&scorer_env()).compatibility_scorer_model_id()
    );
    assert_eq!(config.projection_id, PRODUCTION_COMPATIBILITY_PROJECTION_ID);
    assert_eq!(config.index_id, PRODUCTION_COMPATIBILITY_INDEX_ID);
}

/// An assembly that binds floors other than `main`'s: the production
/// assembler with the context's gate configuration shifted first.
struct ShiftedFloorsAssembler;

impl IngestPipelineRuntimeAssembler for ShiftedFloorsAssembler {
    fn assemble(
        &self,
        mut context: pipeline_runtime::IngestPipelineRuntimeContext,
    ) -> anyhow::Result<Arc<PipelineService>> {
        context.main_gate.novelty_floor_micros = Some(400_000);
        ProductionPipelineAssembler.assemble(context)
    }
}

/// Spec 4.2 item 9: each startup refusal still fires for a misconfigured
/// production deployment.
#[tokio::test]
async fn production_assembler_refuses_what_it_must() {
    let refusal =
        |result: anyhow::Result<Arc<PipelineService>>| result.err().expect("refused").to_string();

    let mut zero = MAIN_GATE;
    zero.novelty_floor_micros = Some(0);
    assert_eq!(
        refusal(
            Boot {
                main_gate: zero,
                ..Boot::production()
            }
            .assemble(&ProductionPipelineAssembler)
            .await
        ),
        "compatibility_zero_floor"
    );

    assert_eq!(
        refusal(Boot::production().assemble(&ShiftedFloorsAssembler).await),
        "pipeline_runtime_main_gate_config_mismatch"
    );

    // No privacy backend: not production-qualified while tenants are routed,
    // and, started anyway for tests, refused by the privacy requirement.
    assert_eq!(
        refusal(
            Boot {
                components: Some(test_components(None)),
                ..Boot::production()
            }
            .assemble(&ProductionPipelineAssembler)
            .await
        ),
        "pipeline_runtime_dependencies_not_production_qualified"
    );
    let without_privacy = Boot {
        components: Some(test_components(None)),
        production_required: false,
        allow_test_dependencies: true,
        ..Boot::production()
    }
    .assemble(&ProductionPipelineAssembler)
    .await
    .unwrap();
    assert_eq!(
        pipeline_runtime::validate_pipeline_privacy_filter_requirement(true, &without_privacy)
            .unwrap_err()
            .to_string(),
        "pipeline_privacy_filter_required"
    );

    assert_eq!(
        refusal(
            Boot {
                production_required: false,
                allow_test_dependencies: true,
                unqualified_routing_allowed: true,
                ..Boot::production()
            }
            .assemble(&ProductionPipelineAssembler)
            .await
        ),
        "pipeline_unqualified_routing_with_production_runtime"
    );

    assert_eq!(
        refusal(
            Boot {
                components: None,
                ..Boot::production()
            }
            .assemble(&ProductionPipelineAssembler)
            .await
        ),
        "pipeline_production_components_missing"
    );
}

/// Spec 4.2 item 10: the production runtime is an explicit opt-in on top of
/// the `near-ai-scorer` build and the `enclave_near_ai` gate, and payout
/// stays off whatever `main`'s NEAR settlement mode is.
#[tokio::test]
async fn runtime_selection_is_opt_in() {
    let select = |raw: Option<&str>, gate: Option<&str>, near_ai: bool| {
        pipeline_runtime_selection(raw, gate, near_ai).map_err(|error| error.to_string())
    };
    for unset in [None, Some(""), Some("  ")] {
        for near_ai in [false, true] {
            assert_eq!(
                select(unset, Some("enclave_near_ai"), near_ai),
                Ok(PipelineRuntimeSelection::None)
            );
        }
    }
    assert_eq!(
        select(Some("production"), Some("enclave_near_ai"), true),
        Ok(PipelineRuntimeSelection::Production)
    );
    assert_eq!(
        select(Some(" production "), Some("enclave_near_ai"), true),
        Ok(PipelineRuntimeSelection::Production)
    );
    assert_eq!(
        select(Some("production"), Some("enclave_near_ai"), false),
        Err("pipeline_runtime_production_requires_near_ai_scorer".to_string())
    );
    for gate in [None, Some(""), Some("in_memory"), Some("enclave_local_gpu")] {
        assert_eq!(
            select(Some("production"), gate, true),
            Err("pipeline_runtime_production_requires_enclave_near_ai".to_string()),
            "{gate:?}"
        );
    }
    for other in ["Production", "prod", "reference", "none"] {
        assert_eq!(
            select(Some(other), Some("enclave_near_ai"), true),
            Err("pipeline_runtime_selection_unknown".to_string()),
            "{other}"
        );
    }
    assert_eq!(PipelineRuntimeSelection::None.label(), "none");
    assert_eq!(PipelineRuntimeSelection::Production.label(), "production");
    assert!(ProductionPipelineAssembler.needs_gate_components());

    for settlement_mode in [
        PipelineNearSettlementMode::Disabled,
        PipelineNearSettlementMode::DryRun,
        PipelineNearSettlementMode::Http,
    ] {
        let service = Boot {
            near_payout_controls: PipelineNearPayoutControls {
                settlement_mode,
                require_adapter_auth: true,
            },
            ..Boot::production()
        }
        .assemble(&ProductionPipelineAssembler)
        .await
        .unwrap();
        assert!(!service.payout_enabled(), "{settlement_mode:?}");
        assert_eq!(service.payout_controls(), None);
    }
}

/// Spec 4.2 item 11: with the selection unset, `main` passes no assembler,
/// so the default build serves no pipeline and readiness says why.
#[tokio::test]
async fn default_build_serves_no_pipeline() {
    use tower::ServiceExt;

    assert_eq!(
        pipeline_runtime_selection(None, None, cfg!(feature = "near-ai-scorer")).unwrap(),
        PipelineRuntimeSelection::None
    );
    assert!(PipelineRuntimeSelection::None.assembler().is_none());
    let dir = tempfile::tempdir().unwrap();
    let (connections, store) = assembly_fixture(&dir).await;
    assert!(
        assemble_ingest_pipeline_runtime_with_components(
            PipelineRuntimeSelection::None.assembler(),
            Some(&connections),
            Some(&store),
            false,
            PipelineLeaseConfig::default(),
            false,
            false,
            false,
            None,
            StdDuration::from_secs(60),
            Boot::production().near_payout_controls,
            &issuer_checks(),
            MAIN_GATE,
            None,
        )
        .unwrap()
        .is_none()
    );

    let state = super::super::tests::test_state_with_options(
        dir.path().to_path_buf(),
        None,
        None,
        false,
        false,
        false,
        false,
    );
    assert_eq!(
        state.pipeline_runtime_selection,
        PipelineRuntimeSelection::None
    );
    let response = pipeline_runtime::build_pipeline_app(state)
        .oneshot(
            axum::http::Request::get("/v1/pipeline/readiness")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(body["reason"], "pipeline_runtime_absent");
}

/// Spec A-D3 / plan A2: splitting the NEAR AI gate builder into components
/// and orchestrator leaves the legacy gate version hash byte-identical. The
/// NEAR AI arm's inputs (`max_tokens` 0, the hosted model id as the
/// perplexity model) for a fixed configuration, pinned to the value the
/// unsplit builder computed.
#[test]
fn near_ai_gate_version_hash_is_pinned() {
    assert_eq!(
        compute_gate_version_hash(
            "gate-policy-v3",
            0,
            0,
            500_000,
            5,
            "Qwen/Qwen3.6-35B-A3B-FP8",
            0,
            -8.0,
            "BAAI/bge-large-en-v1.5",
            512,
            None,
            1024,
            2048,
            3072,
            16,
            64,
            50_000,
        ),
        "sha256:04908a88d9ae4f27e1b974d5b16b50cdddc2efd0524b4b002f8b3c1e9fd1922c"
    );
}

fn gate_pins() -> LegacyGatePins {
    LegacyGatePins {
        model: "Qwen/Qwen3.6-35B-A3B-FP8".to_string(),
        tail_logprob_cutoff: -8.5,
        embedder_model_id: "BAAI/bge-large-en-v1.5".to_string(),
        embedder_output_dim: 1024,
        embedder_max_tokens: 512,
        embedder_matryoshka_dim: None,
    }
}

/// The package names the scorer and embedder by their descriptors; a start
/// whose descriptors would name other values than the gate was built with
/// is refused, so the package can never name a scorer that is not the one
/// running.
#[test]
fn descriptors_must_match_the_gate_they_name() {
    let scorer = scorer_descriptor(&scorer_env());
    let embedder = embedder_descriptor(&embedder_env());
    ensure_descriptors_match_gate(&scorer, &embedder, &gate_pins()).unwrap();
    let refused = |pins: LegacyGatePins| {
        ensure_descriptors_match_gate(&scorer, &embedder, &pins)
            .unwrap_err()
            .to_string()
    };
    assert_eq!(
        refused(LegacyGatePins {
            model: "Qwen/Qwen3.8-27B".to_string(),
            ..gate_pins()
        }),
        "pipeline_scorer_descriptor_mismatch"
    );
    assert_eq!(
        refused(LegacyGatePins {
            tail_logprob_cutoff: -8.0,
            ..gate_pins()
        }),
        "pipeline_scorer_descriptor_mismatch"
    );
    for pins in [
        LegacyGatePins {
            embedder_model_id: "BAAI/bge-small-en-v1.5".to_string(),
            ..gate_pins()
        },
        LegacyGatePins {
            embedder_output_dim: 768,
            ..gate_pins()
        },
        LegacyGatePins {
            embedder_max_tokens: 256,
            ..gate_pins()
        },
        LegacyGatePins {
            embedder_matryoshka_dim: Some(768),
            ..gate_pins()
        },
    ] {
        assert_eq!(refused(pins), "pipeline_embedder_descriptor_mismatch");
    }
}

/// `TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT` is required under the
/// production selection and checked against the novelty root and the dedup
/// root as the legacy gate resolves them (the dedup default is
/// `<novelty>/../dedup-index`).
#[test]
fn pipeline_index_root_is_required_and_separate() {
    assert_eq!(
        pipeline_index_root_from(&lookup_from(&[]))
            .unwrap_err()
            .to_string(),
        "pipeline_vector_index_root_missing"
    );
    assert_eq!(
        pipeline_index_root_from(&lookup_from(&[(
            TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT,
            "  "
        )]))
        .unwrap_err()
        .to_string(),
        "pipeline_vector_index_root_missing"
    );
    for (pairs, root) in [
        (vec![], "/var/lib/trace-commons-vector-index/pipeline"),
        (vec![], "/var/lib/dedup-index"),
        (
            vec![(TRACE_COMMONS_VECTOR_INDEX_ROOT, "/srv/novelty")],
            "/srv/dedup-index/pipeline",
        ),
        (
            vec![(TRACE_COMMONS_DEDUP_VECTOR_INDEX_ROOT, "/srv/dedup")],
            "/srv/dedup",
        ),
    ] {
        let mut pairs = pairs;
        pairs.push((TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT, root));
        assert_eq!(
            pipeline_index_root_from(&lookup_from(&pairs))
                .unwrap_err()
                .to_string(),
            "pipeline_vector_index_root_shared",
            "{root}"
        );
    }
    assert_eq!(
        pipeline_index_root_from(&lookup_from(&[(
            TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT,
            "/var/lib/trace-commons-pipeline-index"
        )]))
        .unwrap(),
        PathBuf::from("/var/lib/trace-commons-pipeline-index")
    );
}

fn production_infrastructure()
-> trace_commons_server::versioned_pipeline_qualification::ProductionInfrastructureProfile {
    use trace_commons_server::versioned_pipeline_qualification::ProductionAdapterKind;
    trace_commons_server::versioned_pipeline_qualification::ProductionInfrastructureProfile {
        authoritative_metadata: ProductionAdapterKind::Production,
        artifact_store: ProductionAdapterKind::Production,
        key_wrapper: ProductionAdapterKind::Production,
        authentication: ProductionAdapterKind::Production,
        plaintext_fallback: false,
        best_effort_database_mirror: false,
        static_bearer_authentication: false,
        hs256_bridge_authentication: false,
        unversioned_policy_dependencies: false,
        live_external_payout_enabled: false,
    }
}

fn revision() -> String {
    format!("sha256:{}", "c".repeat(64))
}

fn emit_vars(dir: &std::path::Path, code_revision: &str) -> PipelineCheckVars {
    PipelineCheckVars {
        dir: Some(dir.to_string_lossy().into_owned()),
        run_id: Some("qproduction".to_string()),
        code_revision_hash: Some(code_revision.to_string()),
    }
}

fn read_json(path: std::path::PathBuf) -> serde_json::Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

/// Spec 4.2 item 12 (A-D11): the startup path emits
/// `pipeline_production_adapters` once, naming the default package, with
/// label-and-digest-only evidence whose hash recomputes; a second boot does
/// not refuse; a revision other than the build's refuses the start; a
/// missing privacy backend or development infrastructure emits `fail` with
/// the blockers.
#[tokio::test]
async fn production_adapters_check_is_emitted_once_with_its_evidence() {
    use trace_commons_server::versioned_pipeline_qualification::{
        PipelineCheckStatus, evidence_hash, package_digests,
    };
    let components = test_components(classifying_privacy());
    let service = Boot {
        components: Some(components.clone()),
        ..Boot::production()
    }
    .assemble(&ProductionPipelineAssembler)
    .await
    .unwrap();

    // Not requested: nothing written, the start continues.
    let quiet = tempfile::tempdir().unwrap();
    assert_eq!(
        emit_production_adapters_check(
            PipelineCheckVars::default(),
            Some(&revision()),
            &service,
            &components,
            production_infrastructure(),
            "disabled",
        )
        .unwrap(),
        ProductionAdaptersEmit::NotRequested
    );
    assert_eq!(std::fs::read_dir(quiet.path()).unwrap().count(), 0);

    let dir = tempfile::tempdir().unwrap();
    let emit = |vars: PipelineCheckVars, deployed: Option<&str>| {
        emit_production_adapters_check(
            vars,
            deployed,
            &service,
            &components,
            production_infrastructure(),
            "disabled",
        )
    };
    assert_eq!(
        emit(emit_vars(dir.path(), &revision()), Some(&revision())).unwrap(),
        ProductionAdaptersEmit::Emitted(PipelineCheckStatus::Pass)
    );
    let result = read_json(dir.path().join("pipeline_production_adapters.result.json"));
    let evidence = read_json(
        dir.path()
            .join("pipeline_production_adapters.evidence.json"),
    );
    let digests = package_digests(service.default_package()).unwrap();
    assert_eq!(result["status"], "pass");
    assert_eq!(result["run_id"], "qproduction");
    assert_eq!(result["code_revision_hash"], revision());
    assert_eq!(result["package_hash"], digests.package_hash);
    assert_eq!(result["dependency_digest"], digests.dependency_digest);
    assert_eq!(result["safe_blockers"], serde_json::json!([]));
    assert_eq!(result["evidence_hash"], evidence_hash(&evidence).unwrap());

    let profile = trace_commons_server::versioned_pipeline_qualification::ProductionDependencyProfile::for_bundle(
        &service,
        service.default_package(),
        production_infrastructure(),
    )
    .unwrap();
    let scorer_hash = scorer_descriptor(&scorer_env()).hash();
    let embedder_hash = components.embedder_descriptor.hash();
    assert_eq!(
        evidence,
        serde_json::json!({
            "schema": "trace_commons.pipeline_production_adapters.v1",
            "runtime_identity_digest": profile.runtime_identity_digest().unwrap(),
            "scorer": {"identity": "near_ai_perplexity_scorer", "descriptor_hash": scorer_hash, "qualified": true},
            "embedder": {"identity": "fastembed_text_embedder", "descriptor_hash": embedder_hash, "output_dim": 2, "qualified": true},
            "index_reader": {"identity": "usearch_pipeline_index_reader", "qualified": true},
            "index_writer": {"identity": "usearch_pipeline_index_writer", "qualified": true},
            "index_root_shared_with_legacy": false,
            "settlement_adapters": {"trace_credit": {"identity": "internal_trace_credit_ledger", "payout_rail": "none", "qualified": true}},
            "payout_enabled": false,
            "near_settlement_mode": "disabled",
            "authority": {"qualified": true, "tenant_policy_count": 0},
            "privacy": {"backend": "near_ai", "classifies_prose_pii": true, "qualified": true},
            "compatibility_configuration_qualifiable": true,
            "infrastructure_blockers": [],
            "bundle_blockers": [],
        })
    );
    let text = evidence.to_string();
    for clear in ["Qwen", "bge", "scorer.invalid", "key-one", "tenant-"] {
        assert!(!text.contains(clear), "{clear} in {text}");
    }

    // A second boot: logged, not refused, and the first result stands.
    let first = std::fs::read(dir.path().join("pipeline_production_adapters.result.json")).unwrap();
    assert_eq!(
        emit(emit_vars(dir.path(), &revision()), Some(&revision())).unwrap(),
        ProductionAdaptersEmit::AlreadyEmitted
    );
    assert_eq!(
        std::fs::read(dir.path().join("pipeline_production_adapters.result.json")).unwrap(),
        first
    );

    // A revision other than the build's, or a build with none: refused.
    let other = tempfile::tempdir().unwrap();
    let other_revision = format!("sha256:{}", "d".repeat(64));
    for deployed in [Some(other_revision.as_str()), None] {
        assert_eq!(
            emit(emit_vars(other.path(), &revision()), deployed)
                .unwrap_err()
                .to_string(),
            "pipeline_check_revision_mismatch"
        );
    }
    // A partial environment: refused, as `PipelineCheckEmitter` refuses it.
    assert_eq!(
        emit(
            PipelineCheckVars {
                run_id: None,
                ..emit_vars(other.path(), &revision())
            },
            Some(&revision())
        )
        .unwrap_err()
        .to_string(),
        "pipeline_check_environment_incomplete"
    );
    assert_eq!(std::fs::read_dir(other.path()).unwrap().count(), 0);

    // No privacy backend: `fail`, naming why.
    let without = test_components(None);
    let unqualified = Boot {
        components: Some(without.clone()),
        production_required: false,
        allow_test_dependencies: true,
        ..Boot::production()
    }
    .assemble(&ProductionPipelineAssembler)
    .await
    .unwrap();
    let failing = tempfile::tempdir().unwrap();
    let mut development = production_infrastructure();
    development.artifact_store =
        trace_commons_server::versioned_pipeline_qualification::ProductionAdapterKind::Development;
    assert_eq!(
        emit_production_adapters_check(
            emit_vars(failing.path(), &revision()),
            Some(&revision()),
            &unqualified,
            &without,
            development,
            "http",
        )
        .unwrap(),
        ProductionAdaptersEmit::Emitted(PipelineCheckStatus::Fail)
    );
    let result = read_json(
        failing
            .path()
            .join("pipeline_production_adapters.result.json"),
    );
    let evidence = read_json(
        failing
            .path()
            .join("pipeline_production_adapters.evidence.json"),
    );
    assert_eq!(result["status"], "fail");
    assert_eq!(
        result["safe_blockers"],
        serde_json::json!([
            "artifact_store_not_production",
            "pipeline_privacy_filter_required",
            "runtime_privacy_not_production",
        ])
    );
    assert_eq!(evidence["privacy"]["backend"], "none");
    assert_eq!(evidence["near_settlement_mode"], "http");
    assert_eq!(
        evidence["bundle_blockers"],
        serde_json::json!(["runtime_privacy_not_production"])
    );
    assert_eq!(
        evidence["infrastructure_blockers"],
        serde_json::json!(["artifact_store_not_production"])
    );
}
