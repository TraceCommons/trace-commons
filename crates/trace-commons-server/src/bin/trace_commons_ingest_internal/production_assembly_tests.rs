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
