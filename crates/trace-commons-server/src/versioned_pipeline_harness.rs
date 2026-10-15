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
//! `assemble_production_pipeline`, the constructor the deployed ingest
//! uses, and only in a `near-ai-scorer` build
//! (`harness_production_assembly_unavailable` otherwise).
//!
//! A production harness serves a signed production package, never one of
//! its own making: it reads the scorer and embedder descriptors and `main`'s
//! gate configuration back out of the package
//! ([`production_package_pins`]), takes its components from
//! `PipelineGateComponents::from_env` -- the constructor the deployed ingest
//! uses, and the only one whose scorer and embedder adapters are
//! production-qualified -- assembles, and refuses
//! (`harness_production_package_mismatch`) a service whose package is not the
//! signed one byte for byte, which is also what refuses an env file whose
//! descriptors are not the package's. [`harness_dependencies_from_env`] is the
//! only production source; a test hands [`assemble_harness_production`]
//! doubles ([`HarnessDependencies::Doubles`]), which are never
//! production-qualified, whatever they report.
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

use crate::versioned_pipeline::{PipelineCrashPoint, PipelineService};
use crate::versioned_pipeline_authority::{PipelineAuthorityProvider, PipelinePrivacyBoundary};
use crate::versioned_pipeline_bundle::package_compatibility_config;
use crate::versioned_pipeline_compat::{CompatibilityQualification, MainGateConfig};
use crate::versioned_pipeline_production::{
    FastEmbedDescriptor, NearAiScorerDescriptor, PipelineGateComponentParts,
    PipelineGateComponents, ProductionPipelineInputs, production_compatibility_package,
    production_pipeline_builder,
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

pub const HARNESS_PRODUCTION_ASSEMBLY_UNAVAILABLE_LABEL: &str =
    "harness_production_assembly_unavailable";
pub const HARNESS_ASSEMBLY_UNKNOWN_LABEL: &str = "harness_assembly_unknown";
pub const HARNESS_PRODUCTION_PACKAGE_INVALID_LABEL: &str = "harness_production_package_invalid";
pub const HARNESS_PRODUCTION_PACKAGE_MISMATCH_LABEL: &str = "harness_production_package_mismatch";
pub const HARNESS_PRODUCTION_PACKAGE_MISSING_LABEL: &str = "harness_production_package_missing";

/// What a qualification harness assembles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessAssembly {
    /// The reference scorer, embedder, index and recording adapters: every
    /// harness exactly as before the switch existed.
    Reference,
    /// `assemble_production_pipeline` over the dependencies a signed
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

/// Test doubles for a production harness's scorer, embedder and index.
#[derive(Clone)]
pub struct HarnessDoubles {
    pub scorer: Arc<dyn PerplexityScorer>,
    pub embedder: Arc<dyn Embedder>,
    pub index_reader: Arc<dyn IdentifiedIndexReader>,
    pub index_writer: Arc<dyn IdentifiedIndexWriter>,
}

/// What a production harness builds its gate components from.
#[derive(Clone)]
pub enum HarnessDependencies {
    /// Doubles, wrapped under the package's descriptors by
    /// `PipelineGateComponents::with_unqualified_adapters`: never
    /// production-qualified. Tests only.
    Doubles(HarnessDoubles),
    /// The components `PipelineGateComponents::from_env` built
    /// ([`harness_dependencies_from_env`]): the real NEAR AI scorer,
    /// fastembed embedder and usearch index, production-qualified.
    FromEnv(Arc<PipelineGateComponents>),
}

impl HarnessDependencies {
    pub fn index_reader(&self) -> Arc<dyn IdentifiedIndexReader> {
        match self {
            Self::Doubles(doubles) => doubles.index_reader.clone(),
            Self::FromEnv(components) => components.parts().index_reader.clone(),
        }
    }

    pub fn index_writer(&self) -> Arc<dyn IdentifiedIndexWriter> {
        match self {
            Self::Doubles(doubles) => doubles.index_writer.clone(),
            Self::FromEnv(components) => components.parts().index_writer.clone(),
        }
    }
}

/// The gate components of a production harness, with the harness's
/// `authority` and `privacy`: doubles under the package's own descriptors
/// (unqualified), or `from_env`'s components with only those two replaced
/// (`PipelineGateComponents::with_boundaries`).
pub fn harness_production_components(
    pins: &ProductionPackagePins,
    dependencies: HarnessDependencies,
    authority: Arc<dyn PipelineAuthorityProvider>,
    privacy: Arc<dyn PipelinePrivacyBoundary>,
) -> Arc<PipelineGateComponents> {
    match dependencies {
        HarnessDependencies::Doubles(doubles) => Arc::new(
            PipelineGateComponents::with_unqualified_adapters(PipelineGateComponentParts {
                scorer: doubles.scorer,
                scorer_descriptor: pins.scorer_descriptor.clone(),
                embedder: doubles.embedder,
                embedder_descriptor: pins.embedder_descriptor.clone(),
                index_reader: doubles.index_reader,
                index_writer: doubles.index_writer,
                index_root_shared_with_legacy: false,
                authority,
                tenant_policy_count: 0,
                privacy: Some(privacy),
                privacy_backend: None,
            }),
        ),
        HarnessDependencies::FromEnv(components) => {
            Arc::new(components.with_boundaries(authority, privacy))
        }
    }
}

/// [`crate::versioned_pipeline_production::assemble_production_pipeline`] over `inputs`, refused
/// (`harness_production_package_mismatch`) unless the service it built
/// serves exactly `expected`: the check a production harness's result
/// rests on, since that result names `expected`.
pub fn assemble_harness_production(
    expected: &BundlePackage,
    inputs: ProductionPipelineInputs,
) -> anyhow::Result<PipelineService> {
    assemble_harness_production_crashing_at(expected, inputs, None)
}

/// [`assemble_harness_production`] with `crash_point` set on the
/// production builder: the restore drill's seed stops its second lifetime
/// at a durable Settle selection. Nothing else differs from the deployed
/// assembly.
pub fn assemble_harness_production_crashing_at(
    expected: &BundlePackage,
    inputs: ProductionPipelineInputs,
    crash_point: Option<PipelineCrashPoint>,
) -> anyhow::Result<PipelineService> {
    let mut builder = production_pipeline_builder(inputs)?;
    if let Some(point) = crash_point {
        builder = builder.with_crash_point(point);
    }
    let service = builder.build()?;
    anyhow::ensure!(
        service.default_package() == expected,
        HARNESS_PRODUCTION_PACKAGE_MISMATCH_LABEL
    );
    Ok(service)
}

/// The production components for a harness on the operator host (spec
/// B-D1): `PipelineGateComponents::from_env`, exactly as the deployed ingest
/// calls it, with no tenant policy (the harness replaces the authority and
/// the privacy boundary with its own). It reads the process environment:
/// the descriptors, the NEAR AI endpoint, key and model, the embedder, and
/// `TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT`, which `pipeline.py promote
/// package-checks` points inside the run. Every missing or inconsistent
/// setting refuses with `from_env`'s own label; nothing falls back to a
/// reference component.
#[cfg(feature = "near-ai-scorer")]
pub async fn harness_dependencies_from_env() -> anyhow::Result<HarnessDependencies> {
    use std::collections::BTreeMap;

    use crate::versioned_pipeline_production::{PipelineComponentInputs, std_env_lookup};

    // `pipeline.py` points the root at a directory inside the run that
    // does not exist yet; the deployment's is made by its installer.
    create_harness_index_root(&std_env_lookup)?;
    let (components, _shared) = PipelineGateComponents::from_env(PipelineComponentInputs {
        tenant_policies: Arc::new(BTreeMap::new()),
        require_tenant_submission_policy: false,
        db_policy_reads: Arc::new(|_: &str| false),
        // The harness replaces this authority with its own
        // (`with_boundaries`), so no tenant policy is read from a database.
        db_policies: None,
    })
    .await?;
    Ok(HarnessDependencies::FromEnv(components))
}

/// Creates the pipeline index root `lookup` names, after the same checks
/// `from_env` makes of it (required, never a legacy root), and returns it.
pub fn create_harness_index_root(
    lookup: &dyn Fn(&str) -> Option<String>,
) -> anyhow::Result<std::path::PathBuf> {
    let root = crate::versioned_pipeline_production::pipeline_index_root_from(lookup)?;
    std::fs::create_dir_all(&root)
        .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
    Ok(root)
}

/// A second usearch pipeline index for the restore drill's rebuild, beside
/// the live one: `<TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT>-rebuilt`, with
/// the settings `from_env` opens the live one with. A rebuild target only;
/// nothing is scored against it.
#[cfg(feature = "near-ai-scorer")]
pub fn open_harness_rebuild_index()
-> anyhow::Result<Arc<crate::versioned_pipeline_production::UsearchPipelineIndex>> {
    use crate::versioned_pipeline_production::{
        pipeline_index_root_from, std_env_lookup, usearch_index_config_from_env,
    };
    let live = pipeline_index_root_from(&std_env_lookup)?;
    let name = live
        .file_name()
        .ok_or_else(|| anyhow::anyhow!("pipeline_vector_index_open_failed"))?
        .to_string_lossy()
        .into_owned();
    let root = live.with_file_name(format!("{name}-rebuilt"));
    std::fs::create_dir_all(&root)
        .map_err(|_| anyhow::anyhow!("pipeline_vector_index_open_failed"))?;
    Ok(Arc::new(
        crate::versioned_pipeline_production::UsearchPipelineIndex::open(
            &root,
            usearch_index_config_from_env()?,
        )?,
    ))
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

    /// The harness makes the run's pipeline index root before `from_env`
    /// opens it, after `from_env`'s own checks: a missing root, or one that
    /// is a legacy root, is refused and nothing is created.
    #[test]
    fn harness_index_root_is_created_after_from_envs_checks() {
        use crate::versioned_pipeline_production::{
            PIPELINE_VECTOR_INDEX_ROOT_MISSING_LABEL, TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT,
            TRACE_COMMONS_VECTOR_INDEX_ROOT,
        };
        let dir = tempfile::tempdir().unwrap();
        let root = dir
            .path()
            .join("indexes")
            .join("pipeline_restore_drill")
            .join("seed");
        let root_text = root.to_string_lossy().into_owned();
        let lookup = |var: &str| {
            (var == TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT).then(|| root_text.clone())
        };
        assert_eq!(create_harness_index_root(&lookup).unwrap(), root);
        assert!(root.is_dir());

        let missing = |_: &str| None;
        assert_eq!(
            create_harness_index_root(&missing).unwrap_err().to_string(),
            PIPELINE_VECTOR_INDEX_ROOT_MISSING_LABEL
        );
        let legacy = dir.path().join("legacy");
        let legacy_text = legacy.to_string_lossy().into_owned();
        let shared = |var: &str| {
            (var == TRACE_COMMONS_PIPELINE_VECTOR_INDEX_ROOT
                || var == TRACE_COMMONS_VECTOR_INDEX_ROOT)
                .then(|| legacy_text.clone())
        };
        assert!(create_harness_index_root(&shared).is_err());
        assert!(!legacy.exists());
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
