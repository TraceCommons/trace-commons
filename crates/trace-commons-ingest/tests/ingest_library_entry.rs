// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ingest service's library entry point (`trace_commons_ingest`).
//!
//! A build that registers its own Score policies (the proprietary-backend
//! seam in `crates/trace-commons-gate-api/README.md`) builds its own
//! `trace-commons-ingest` binary over this entry and passes its registry in
//! [`IngestOptions`]. The open binary passes the open-build options, whose
//! registry is empty.
//!
//! None of these tests needs a database: the production assembly is built
//! over doubles and a backend pointed at a closed port, as
//! `production_assembly_is_constructible_from_the_library` builds it.

use std::sync::Arc;

use secrecy::SecretString;
use trace_commons_gate_api::pipeline::ScorePolicy;
use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer};
use trace_commons_ingest::{IngestOptions, OPEN_BUILD_BIN_NAME};
use trace_commons_server::config::DatabaseConfig;
use trace_commons_server::db::postgres::PgBackend;
use trace_commons_server::secrets::SecretsCrypto;
use trace_commons_server::trace_artifact_store::{
    LocalEncryptedTraceArtifactStore, TraceArtifactStore,
};
use trace_commons_server::trace_authority::{SubmissionAllowlists, SubmissionAuthority};
use trace_commons_server::versioned_pipeline::{
    PipelineLeaseConfig, PipelineNoveltyUtilityChecks, PipelineService,
};
use trace_commons_server::versioned_pipeline_authority::StaticPipelineAuthorityProvider;
use trace_commons_server::versioned_pipeline_bundle::PipelineTraceCreditEvent;
use trace_commons_server::versioned_pipeline_compat::MainGateConfig;
use trace_commons_server::versioned_pipeline_index::IsolatedPipelineIndex;
use trace_commons_server::versioned_pipeline_production::{
    FastEmbedDescriptor, NearAiScorerDescriptor, PipelineGateComponentParts,
    PipelineGateComponents, ProductionPipelineInputs, assemble_production_pipeline,
};
use trace_commons_server::versioned_pipeline_score_registry::{
    ScorePolicyDependencies, ScorePolicyFactory, ScorePolicyRegistry,
};

const TEST_SCORE_IMPLEMENTATION: &str = "test.ingest_library_entry.score.v1";

/// A Score policy factory registered only to be found again. It is not
/// production-qualified, and it builds nothing: no bundle in these tests
/// names its implementation id.
struct RegisteredTestScorePolicy;

impl ScorePolicyFactory for RegisteredTestScorePolicy {
    fn dependency_identity(&self) -> &str {
        "ingest_library_entry_test_score_policy"
    }

    fn trace_credit_event(&self) -> PipelineTraceCreditEvent {
        PipelineTraceCreditEvent::PipelineScore
    }

    fn build(
        &self,
        _dependencies: ScorePolicyDependencies<'_>,
    ) -> Result<Arc<dyn ScorePolicy>, &'static str> {
        Err("ingest_library_entry_test_score_policy_not_built")
    }
}

fn registry_with_test_policy() -> ScorePolicyRegistry {
    ScorePolicyRegistry::new()
        .with_factory(
            TEST_SCORE_IMPLEMENTATION,
            Arc::new(RegisteredTestScorePolicy),
        )
        .expect("a non-reserved id registers")
}

/// The production assembly over doubles, under the registry `options`
/// carries: the inputs ingest's production assembler hands the library.
async fn production_pipeline_for(options: &IngestOptions) -> PipelineService {
    let unused_port = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let backend = Arc::new(
        PgBackend::new(&DatabaseConfig::from_postgres_url(
            &format!("postgres://nobody@127.0.0.1:{unused_port}/none"),
            1,
        ))
        .await
        .unwrap(),
    );
    let dir = tempfile::tempdir().unwrap();
    let artifact_store: Arc<dyn TraceArtifactStore> =
        Arc::new(LocalEncryptedTraceArtifactStore::new(
            dir.path(),
            SecretsCrypto::new(SecretString::from(
                "ingest-library-entry-test-master-key-32b".to_string(),
            ))
            .unwrap(),
        ));
    let index = IsolatedPipelineIndex::new();
    let components = Arc::new(PipelineGateComponents::with_unqualified_adapters(
        PipelineGateComponentParts {
            scorer: Arc::new(ReferencePerplexityScorer::new()),
            scorer_descriptor: NearAiScorerDescriptor {
                model: "test/ingest-library-entry-scorer".to_string(),
                tail_logprob_cutoff: -8.0,
                logprobs_top_k: 1,
            },
            embedder: Arc::new(ReferenceEmbedder::new()),
            embedder_descriptor: FastEmbedDescriptor {
                model_id: "test/ingest-library-entry-embedder".to_string(),
                output_dim: 1024,
                max_tokens: 512,
                matryoshka_dim: None,
            },
            index_reader: index.clone(),
            index_writer: index,
            index_root_shared_with_legacy: false,
            authority: Arc::new(StaticPipelineAuthorityProvider::test_only(
                SubmissionAuthority {
                    tenant: SubmissionAllowlists::default(),
                    policy: None,
                    require_policy: false,
                },
            )),
            tenant_policy_count: 0,
            privacy: None,
            privacy_backend: None,
        },
    ));
    assemble_production_pipeline(ProductionPipelineInputs {
        backend,
        artifact_store,
        object_store_name: "local".to_string(),
        lease_config: PipelineLeaseConfig::default(),
        novelty_utility_checks: PipelineNoveltyUtilityChecks::default(),
        unqualified_routing_allowed: false,
        main_gate: MainGateConfig {
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
        },
        components,
        score_policies: options.score_policies().clone(),
    })
    .expect("the production assembly builds over doubles")
}

/// A registry passed through the library entry's options reaches the
/// assembled production pipeline service, which is where the pipeline and
/// the qualification store read it.
#[tokio::test]
async fn a_registered_score_policy_reaches_the_assembled_pipeline_service() {
    let options = IngestOptions::open_build().with_score_policies(registry_with_test_policy());
    assert!(
        options
            .score_policies()
            .get(TEST_SCORE_IMPLEMENTATION)
            .is_some()
    );

    let service = production_pipeline_for(&options).await;
    let registered = service
        .score_policies()
        .get(TEST_SCORE_IMPLEMENTATION)
        .expect("the registered factory reaches the service");
    assert_eq!(
        registered.dependency_identity(),
        "ingest_library_entry_test_score_policy"
    );
    assert!(!registered.production_qualified());
}

/// The open build's options carry an empty registry and the open binary's
/// name, so the open binary assembles exactly what it assembled before the
/// entry point moved into the library.
#[tokio::test]
async fn the_open_build_passes_an_empty_registry() {
    let options = IngestOptions::open_build();
    assert!(options.score_policies().is_empty());
    assert_eq!(options.bin_name(), OPEN_BUILD_BIN_NAME);
    assert_eq!(OPEN_BUILD_BIN_NAME, "trace-commons-ingest");
    assert!(IngestOptions::default().score_policies().is_empty());

    let service = production_pipeline_for(&options).await;
    assert!(service.score_policies().is_empty());
}

/// The open binary is a thin `main` that starts the library entry with the
/// open-build options and nothing else: no registry of its own.
#[test]
fn the_open_binary_starts_the_library_entry_with_the_open_build_options() {
    let source = include_str!("../src/bin/trace_commons_ingest_main.rs");
    let code: String = source
        .lines()
        .filter(|line| !line.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("trace_commons_ingest::run(IngestOptions::open_build())"),
        "the open binary must start the library entry with the open-build options"
    );
    assert!(!code.contains("with_score_policies"));
    assert!(!code.contains("ScorePolicyRegistry"));
}
