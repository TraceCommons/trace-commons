// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The qualification harness assembly switch (spec
//! `docs/superpowers/specs/2026-10-08-pipeline-production-assembly-design.md`,
//! B-D1; plan B3).
//!
//! The four package-bearing checks (`PROMOTION_PACKAGE_CHECKS` less the
//! promotion-only three) are emitted by harnesses that build the pipeline
//! service themselves. `TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY` chooses what
//! they build: unset, blank or `reference` keeps the reference assembly
//! exactly as before; `production` builds the service through
//! [`assemble_production_pipeline`], the constructor the deployed ingest
//! uses, and only in a `near-ai-scorer` build
//! (`harness_production_assembly_unavailable` otherwise).
//!
//! A production harness serves a signed production package, never one of
//! its own making: it reads the scorer and embedder descriptors and `main`'s
//! gate configuration back out of the package
//! ([`production_package_pins`]), builds the dependencies those descriptors
//! name, assembles, and refuses (`harness_production_package_mismatch`) a
//! service whose package is not the signed one byte for byte. Only
//! [`harness_dependencies_from_env`] names the NEAR AI, fastembed and usearch
//! types; a test hands [`assemble_harness_production`] qualified doubles
//! through the same trait objects, and the package check makes a double
//! that described itself as something else fail rather than pass.
//!
//! What the switch replaces is the scorer, the embedder, the index, the
//! settlement adapter and its cap, and the package. Authority and the
//! privacy boundary stay the harness's own: they are not bound by the
//! package, and the harness drives fixed test tenants whose results must
//! not depend on a deployment's tenant policies or on a classifier.

use std::path::Path;
use std::sync::Arc;

use trace_commons_gate_api::pipeline::BundlePackage;
use trace_commons_gate_api::{
    Embedder, IdentifiedIndexReader, IdentifiedIndexWriter, PerplexityScorer,
};

use crate::versioned_pipeline::PipelineService;
use crate::versioned_pipeline_authority::{PipelineAuthorityProvider, PipelinePrivacyBoundary};
use crate::versioned_pipeline_bundle::package_compatibility_config;
use crate::versioned_pipeline_compat::{CompatibilityQualification, MainGateConfig};
use crate::versioned_pipeline_production::{
    FastEmbedDescriptor, NearAiScorerDescriptor, PipelineGateComponents, ProductionPipelineInputs,
    assemble_production_pipeline, production_compatibility_package,
};
use crate::versioned_pipeline_qualification::{
    BundlePackageTrustStore, SignedBundlePackage, TrustedBundleKey,
};

/// Selects what the qualification harnesses assemble.
pub const TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY: &str = "TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY";
/// The signed production package a production harness serves, and its
/// trusted key (`promote init`'s copies).
pub const TRACE_COMMONS_PIPELINE_HARNESS_PACKAGE_PATH: &str =
    "TRACE_COMMONS_PIPELINE_HARNESS_PACKAGE_PATH";
pub const TRACE_COMMONS_PIPELINE_HARNESS_TRUSTED_KEY_PATH: &str =
    "TRACE_COMMONS_PIPELINE_HARNESS_TRUSTED_KEY_PATH";
/// Where a production harness opens its usearch pipeline index: a directory
/// `pipeline.py` makes inside the run, never the deployment's
/// `TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT`, which no harness reads.
pub const TRACE_COMMONS_PIPELINE_HARNESS_INDEX_ROOT: &str =
    "TRACE_COMMONS_PIPELINE_HARNESS_INDEX_ROOT";

pub const HARNESS_PRODUCTION_ASSEMBLY_UNAVAILABLE_LABEL: &str =
    "harness_production_assembly_unavailable";
pub const HARNESS_ASSEMBLY_UNKNOWN_LABEL: &str = "harness_assembly_unknown";
pub const HARNESS_PRODUCTION_PACKAGE_INVALID_LABEL: &str = "harness_production_package_invalid";
pub const HARNESS_PRODUCTION_PACKAGE_MISMATCH_LABEL: &str = "harness_production_package_mismatch";
pub const HARNESS_PRODUCTION_PACKAGE_MISSING_LABEL: &str = "harness_production_package_missing";
pub const HARNESS_PRODUCTION_INDEX_ROOT_MISSING_LABEL: &str =
    "harness_production_index_root_missing";

/// What a qualification harness assembles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessAssembly {
    /// The reference scorer, embedder, index and recording adapters: every
    /// harness exactly as before the switch existed.
    Reference,
    /// [`assemble_production_pipeline`] over the dependencies a signed
    /// production package names.
    Production,
}

impl HarnessAssembly {
    /// The value the evidence records as `harness_assembly`.
    pub fn label(self) -> &'static str {
        match self {
            Self::Reference => "reference",
            Self::Production => "production",
        }
    }

    /// `raw` is the variable's value; `near_ai_scorer_compiled` whether this
    /// build has `near-ai-scorer`, without which no production dependency
    /// can be built.
    pub fn from_value(raw: Option<&str>, near_ai_scorer_compiled: bool) -> anyhow::Result<Self> {
        match raw.map(str::trim).unwrap_or_default() {
            "" | "reference" => Ok(Self::Reference),
            "production" => {
                anyhow::ensure!(
                    near_ai_scorer_compiled,
                    HARNESS_PRODUCTION_ASSEMBLY_UNAVAILABLE_LABEL
                );
                Ok(Self::Production)
            }
            _ => anyhow::bail!(HARNESS_ASSEMBLY_UNKNOWN_LABEL),
        }
    }

    /// [`Self::from_value`] of the process environment and this build.
    pub fn from_env() -> anyhow::Result<Self> {
        Self::from_value(
            std::env::var(TRACE_COMMONS_PIPELINE_HARNESS_ASSEMBLY)
                .ok()
                .as_deref(),
            cfg!(feature = "near-ai-scorer"),
        )
    }
}

/// What a production package says its assembly is built from: the two
/// descriptors its Score policy names and stores, and `main`'s gate
/// configuration its compatibility configuration holds.
#[derive(Debug, Clone, PartialEq)]
pub struct ProductionPackagePins {
    pub scorer_descriptor: NearAiScorerDescriptor,
    pub embedder_descriptor: FastEmbedDescriptor,
    pub main_gate: MainGateConfig,
}

/// Reads [`ProductionPackagePins`] out of `package` and proves them: the
/// production package those pins build must be `package` itself. Anything
/// else -- a reference or test package, a production package whose
/// descriptors this build cannot reproduce byte for byte, one built with
/// another projection or index id -- is refused.
pub fn production_package_pins(package: &BundlePackage) -> anyhow::Result<ProductionPackagePins> {
    let invalid = || anyhow::anyhow!(HARNESS_PRODUCTION_PACKAGE_INVALID_LABEL);
    let config = package_compatibility_config(package).ok_or_else(invalid)?;
    anyhow::ensure!(
        config.qualification == CompatibilityQualification::ProductionCompatible,
        HARNESS_PRODUCTION_PACKAGE_INVALID_LABEL
    );
    let [scorer_hash, embedder_hash] = package.manifest.score.data_artifact_hashes.as_slice()
    else {
        return Err(invalid());
    };
    let artifact = |hash: &String| package.artifacts.get(hash).ok_or_else(invalid);
    let pins = ProductionPackagePins {
        scorer_descriptor: NearAiScorerDescriptor::from_bytes(artifact(scorer_hash)?)
            .ok_or_else(invalid)?,
        embedder_descriptor: FastEmbedDescriptor::from_bytes(artifact(embedder_hash)?)
            .ok_or_else(invalid)?,
        main_gate: config.main_gate(),
    };
    let rebuilt = production_compatibility_package(
        &pins.scorer_descriptor,
        &pins.embedder_descriptor,
        &pins.main_gate,
    )
    .map_err(|_| invalid())?;
    anyhow::ensure!(
        rebuilt == *package,
        HARNESS_PRODUCTION_PACKAGE_MISMATCH_LABEL
    );
    Ok(pins)
}

/// The signed package at `package_path`, verified against the trusted key
/// at `key_path`. Every failure is a label; no path is ever printed.
pub fn verified_package(package_path: &Path, key_path: &Path) -> anyhow::Result<BundlePackage> {
    let missing = || anyhow::anyhow!(HARNESS_PRODUCTION_PACKAGE_MISSING_LABEL);
    let invalid = || anyhow::anyhow!(HARNESS_PRODUCTION_PACKAGE_INVALID_LABEL);
    let signed: SignedBundlePackage =
        serde_json::from_slice(&std::fs::read(package_path).map_err(|_| missing())?)
            .map_err(|_| invalid())?;
    let key: TrustedBundleKey =
        serde_json::from_slice(&std::fs::read(key_path).map_err(|_| missing())?)
            .map_err(|_| invalid())?;
    BundlePackageTrustStore::new([key])
        .and_then(|trust| trust.verify(&signed))
        .map_err(|_| invalid())?;
    Ok(signed.package)
}

/// [`verified_package`] at the two harness package variables of `lookup`.
pub fn verified_package_from_lookup(
    lookup: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<BundlePackage> {
    let package = lookup(TRACE_COMMONS_PIPELINE_HARNESS_PACKAGE_PATH)
        .ok_or_else(|| anyhow::anyhow!(HARNESS_PRODUCTION_PACKAGE_MISSING_LABEL))?;
    let key = lookup(TRACE_COMMONS_PIPELINE_HARNESS_TRUSTED_KEY_PATH)
        .ok_or_else(|| anyhow::anyhow!(HARNESS_PRODUCTION_PACKAGE_MISSING_LABEL))?;
    verified_package(Path::new(&package), Path::new(&key))
}

/// The scorer, embedder and index a production harness holds. The harness
/// on the operator host fills it from [`harness_dependencies_from_env`]; a
/// test fills it with qualified doubles.
#[derive(Clone)]
pub struct HarnessDependencies {
    pub scorer: Arc<dyn PerplexityScorer>,
    pub embedder: Arc<dyn Embedder>,
    pub index_reader: Arc<dyn IdentifiedIndexReader>,
    pub index_writer: Arc<dyn IdentifiedIndexWriter>,
}

/// The gate components of a production harness: `dependencies` under the
/// package's own descriptors, with the harness's `authority` and `privacy`.
pub fn harness_production_components(
    pins: &ProductionPackagePins,
    dependencies: HarnessDependencies,
    authority: Arc<dyn PipelineAuthorityProvider>,
    privacy: Arc<dyn PipelinePrivacyBoundary>,
) -> Arc<PipelineGateComponents> {
    Arc::new(PipelineGateComponents {
        scorer: dependencies.scorer,
        scorer_descriptor: pins.scorer_descriptor.clone(),
        embedder: dependencies.embedder,
        embedder_descriptor: pins.embedder_descriptor.clone(),
        index_reader: dependencies.index_reader,
        index_writer: dependencies.index_writer,
        index_root_shared_with_legacy: false,
        authority,
        tenant_policy_count: 0,
        privacy: Some(privacy),
        privacy_backend: None,
    })
}

/// [`assemble_production_pipeline`] over `inputs`, refused
/// (`harness_production_package_mismatch`) unless the service it built
/// serves exactly `expected`: the check a production harness's result
/// rests on, since that result names `expected`.
pub fn assemble_harness_production(
    expected: &BundlePackage,
    inputs: ProductionPipelineInputs,
) -> anyhow::Result<PipelineService> {
    let service = assemble_production_pipeline(inputs)?;
    anyhow::ensure!(
        service.default_package() == expected,
        HARNESS_PRODUCTION_PACKAGE_MISMATCH_LABEL
    );
    Ok(service)
}

/// The production dependencies `pins` name, built for a harness on the
/// operator host (spec B-D1: real NEAR AI scoring, real bge embedding, a
/// usearch index in the run directory). The model, cutoff, top-k and the
/// embedder's settings come from the package; `lookup` supplies only what
/// the package does not bind: the NEAR AI endpoint, key and timeout, and the
/// embedder's model cache. The index opens at `index_root` with ingest's
/// default HNSW settings, which no package binds.
#[cfg(feature = "near-ai-scorer")]
pub async fn harness_dependencies_from_env(
    pins: &ProductionPackagePins,
    index_root: &Path,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<HarnessDependencies> {
    use trace_commons_gate_enclave::embedder_fastembed::FastEmbedTextEmbedder;
    use trace_commons_gate_enclave::vector_index_usearch::UsearchVectorIndexConfig;
    use trace_commons_gate_enclave::{NearAiPerplexityScorer, NearAiScorerConfig};

    use crate::versioned_pipeline_production::UsearchPipelineIndex;

    let value = |var: &str| {
        lookup(var)
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())
    };
    let base_url = value("TRACE_COMMONS_NEAR_AI_BASE_URL")
        .ok_or_else(|| anyhow::anyhow!("harness_near_ai_base_url_missing"))?;
    let api_key = value("TRACE_COMMONS_NEAR_AI_API_KEY")
        .ok_or_else(|| anyhow::anyhow!("harness_near_ai_api_key_missing"))?;
    let timeout_seconds = match value("TRACE_COMMONS_NEAR_AI_TIMEOUT_SECONDS") {
        Some(raw) => raw
            .parse::<u64>()
            .ok()
            .filter(|seconds| *seconds > 0)
            .ok_or_else(|| anyhow::anyhow!("harness_near_ai_timeout_invalid"))?,
        None => 60,
    };
    let scorer_config = NearAiScorerConfig {
        base_url,
        model: pins.scorer_descriptor.model.clone(),
        api_key,
        tail_logprob_cutoff: pins.scorer_descriptor.tail_logprob_cutoff,
        logprobs_top_k: pins.scorer_descriptor.logprobs_top_k,
        timeout: std::time::Duration::from_secs(timeout_seconds),
    };
    // reqwest's blocking client must not be built on an async task.
    let scorer =
        tokio::task::spawn_blocking(move || NearAiPerplexityScorer::try_new(scorer_config))
            .await
            .map_err(|_| anyhow::anyhow!("harness_near_ai_scorer_init_failed"))?
            .map_err(|_| anyhow::anyhow!("harness_near_ai_scorer_init_failed"))?;
    let embedder_descriptor = &pins.embedder_descriptor;
    let cache_dir = value("TRACE_COMMONS_EMBEDDER_CACHE_DIR")
        .unwrap_or_else(|| "/var/cache/trace-commons-embedder".to_string());
    let embedder = FastEmbedTextEmbedder::try_new(
        embedder_descriptor.model_id.clone(),
        &cache_dir,
        embedder_descriptor.matryoshka_dim,
        embedder_descriptor.max_tokens,
    )
    .await
    .map_err(|_| anyhow::anyhow!("harness_embedder_init_failed"))?;
    anyhow::ensure!(
        embedder.output_dim() == embedder_descriptor.output_dim,
        "harness_embedder_dimension_mismatch"
    );
    std::fs::create_dir_all(index_root)
        .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
    let index = Arc::new(UsearchPipelineIndex::open(
        index_root,
        UsearchVectorIndexConfig {
            dim: embedder_descriptor.output_dim,
            hnsw_m: 16,
            ef_construction: 200,
            ef_search: 50,
            max_open: 32,
            flush_every: 32,
            flush_interval: None,
        },
    )?);
    Ok(HarnessDependencies {
        scorer: Arc::new(scorer),
        embedder: Arc::new(embedder),
        index_reader: index.clone(),
        index_writer: index,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spec 5.2 item 3: unset, blank and `reference` select the reference
    /// assembly; `production` selects the production assembly only in a
    /// `near-ai-scorer` build and refuses with
    /// `harness_production_assembly_unavailable` in any other; anything else
    /// refuses with `harness_assembly_unknown`.
    #[test]
    fn harness_production_assembly_requires_the_feature() {
        for raw in [None, Some(""), Some("  "), Some("reference")] {
            assert_eq!(
                HarnessAssembly::from_value(raw, false).unwrap(),
                HarnessAssembly::Reference
            );
            assert_eq!(
                HarnessAssembly::from_value(raw, true).unwrap(),
                HarnessAssembly::Reference
            );
        }
        assert_eq!(
            HarnessAssembly::from_value(Some("production"), false)
                .unwrap_err()
                .to_string(),
            HARNESS_PRODUCTION_ASSEMBLY_UNAVAILABLE_LABEL
        );
        assert_eq!(
            HarnessAssembly::from_value(Some(" production "), true).unwrap(),
            HarnessAssembly::Production
        );
        assert_eq!(
            HarnessAssembly::from_value(Some("Production"), true)
                .unwrap_err()
                .to_string(),
            HARNESS_ASSEMBLY_UNKNOWN_LABEL
        );
        // What this build does with the variable set to `production`.
        let compiled =
            HarnessAssembly::from_value(Some("production"), cfg!(feature = "near-ai-scorer"));
        if cfg!(feature = "near-ai-scorer") {
            assert_eq!(compiled.unwrap(), HarnessAssembly::Production);
        } else {
            assert_eq!(
                compiled.unwrap_err().to_string(),
                "harness_production_assembly_unavailable"
            );
        }
        assert_eq!(HarnessAssembly::Reference.label(), "reference");
        assert_eq!(HarnessAssembly::Production.label(), "production");
    }

    pub(crate) fn pilot_pins() -> ProductionPackagePins {
        ProductionPackagePins {
            scorer_descriptor: NearAiScorerDescriptor {
                model: "Qwen/Qwen3.6-35B-A3B-FP8".to_string(),
                tail_logprob_cutoff: -8.25,
                logprobs_top_k: 1,
            },
            embedder_descriptor: FastEmbedDescriptor {
                model_id: "BAAI/bge-large-en-v1.5".to_string(),
                output_dim: 1024,
                max_tokens: 512,
                matryoshka_dim: Some(256),
            },
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
                novelty_utility_microcredits: 2_500_000,
            },
        }
    }

    /// A production package gives back exactly the pins it was built from,
    /// so a harness rebuilds the package the deployment's env file named.
    #[test]
    fn production_package_pins_round_trip_the_package() {
        let pins = pilot_pins();
        let package = production_compatibility_package(
            &pins.scorer_descriptor,
            &pins.embedder_descriptor,
            &pins.main_gate,
        )
        .unwrap();
        assert_eq!(production_package_pins(&package).unwrap(), pins);
        let mut without_matryoshka = pins.clone();
        without_matryoshka.embedder_descriptor.matryoshka_dim = None;
        let package = production_compatibility_package(
            &without_matryoshka.scorer_descriptor,
            &without_matryoshka.embedder_descriptor,
            &without_matryoshka.main_gate,
        )
        .unwrap();
        assert_eq!(
            production_package_pins(&package).unwrap(),
            without_matryoshka
        );
    }

    /// The harness refuses a reference package, and any production package
    /// it cannot rebuild exactly.
    #[test]
    fn production_package_pins_refuse_what_they_cannot_rebuild() {
        use crate::versioned_pipeline_bundle::MinimalPolicyBundle;
        use crate::versioned_pipeline_compat::CompatibilityBundleConfig;
        use trace_commons_gate_api::{ReferenceEmbedder, ReferencePerplexityScorer};

        let label =
            |package: &BundlePackage| production_package_pins(package).unwrap_err().to_string();
        let reference = MinimalPolicyBundle::compatibility_package(
            &CompatibilityBundleConfig::local_reference(),
            &ReferencePerplexityScorer::new(),
            &ReferenceEmbedder::new(),
        )
        .unwrap();
        assert_eq!(label(&reference), HARNESS_PRODUCTION_PACKAGE_INVALID_LABEL);

        let pins = pilot_pins();
        let package = production_compatibility_package(
            &pins.scorer_descriptor,
            &pins.embedder_descriptor,
            &pins.main_gate,
        )
        .unwrap();
        // A descriptor with a field this build does not know.
        let mut extended = package.clone();
        let scorer_hash = extended.manifest.score.data_artifact_hashes[0].clone();
        let mut descriptor: serde_json::Value =
            serde_json::from_slice(&extended.artifacts[&scorer_hash]).unwrap();
        descriptor["attestation_host"] = serde_json::json!("example");
        extended
            .artifacts
            .insert(scorer_hash, serde_json::to_vec(&descriptor).unwrap());
        assert_eq!(label(&extended), HARNESS_PRODUCTION_PACKAGE_INVALID_LABEL);
        // The scorer and embedder descriptors in the other order.
        let mut swapped = package.clone();
        swapped.manifest.score.data_artifact_hashes.reverse();
        assert_eq!(label(&swapped), HARNESS_PRODUCTION_PACKAGE_INVALID_LABEL);
        // Another projection id: every pin decodes, the rebuild differs.
        let mut config =
            crate::versioned_pipeline_bundle::package_compatibility_config(&package).unwrap();
        config.projection_id = "compatibility_projection_v2".to_string();
        let other = MinimalPolicyBundle::compatibility_package(
            &config,
            &crate::versioned_pipeline_production::NearAiPipelineScorer::new(
                Arc::new(ReferencePerplexityScorer::new()),
                pins.scorer_descriptor.clone(),
            ),
            &crate::versioned_pipeline_production::FastEmbedPipelineEmbedder::new(
                Arc::new(ReferenceEmbedder::new()),
                pins.embedder_descriptor.clone(),
            ),
        )
        .unwrap();
        assert_eq!(label(&other), HARNESS_PRODUCTION_PACKAGE_MISMATCH_LABEL);
    }
}
