// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

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
    assert_eq!(defaulted.max_tokens, TRACE_COMMONS_EMBEDDER_DEFAULT_MAX_TOKENS);
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
    FastEmbedPipelineEmbedder::new(Arc::new(FixedEmbedder), embedder_descriptor(&embedder_env()))
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
    assert!(scorer.compatibility_scorer_model_id().starts_with("near_ai:"));
}
