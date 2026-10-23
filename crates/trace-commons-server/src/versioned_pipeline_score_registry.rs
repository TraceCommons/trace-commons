// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Score policy injection point for the versioned pipeline.
//!
//! The open build runs three Score policies, all built in: the minimal
//! reference (`trace_commons.score.minimal.v1`), the compatibility policy
//! (`trace_commons.score.compatibility.v1`), and the substance policy of the
//! compatibility family (`trace_commons.score.substance.v1`). An embedding
//! binary that holds a substituted backend supplies its own [`ScorePolicy`]
//! through a
//! [`ScorePolicyRegistry`]: a factory keyed by the implementation id a
//! bundle manifest's Score [`PolicyRef`] names. The registry supplies code
//! for an implementation id; the signed, content-addressed manifest still
//! pins which id, which configuration (`configuration_hash`), and which
//! dependencies (`data_artifact_hashes`) the bundle runs, so a registered
//! factory never widens what a package can name.
//!
//! The ingest service takes its registry from the library entry point
//! (`trace_commons_ingest::IngestOptions::with_score_policies`); the open
//! `trace-commons-ingest` binary passes an empty one. A package naming an
//! implementation id no factory is registered under is refused at bind time
//! with `bundle_policy_not_runnable`; it never falls back to a built-in
//! policy. The built-in ids are reserved and cannot be registered.
//!
//! A registered factory is held only as a trait object, and so are the
//! policies it builds. Like every other identified dependency, a factory is
//! unqualified for production unless it declares otherwise
//! ([`ScorePolicyFactory::production_qualified`]); the per-bundle
//! qualification reports it as the bundle's `score_policy` check, and a
//! tenant bundle is recorded as qualified
//! (`PipelineQualificationStore::qualify_bundle`) only when the factory is
//! qualified and accepts the bundle's Score configuration
//! ([`ScorePolicyFactory::configuration_is_qualifiable`]).
//!
//! [`PolicyRef`]: trace_commons_gate_api::pipeline::PolicyRef

use std::collections::BTreeMap;
use std::sync::Arc;

use trace_commons_gate_api::pipeline::{BundleManifest, ScorePolicy};
use trace_commons_gate_api::{
    IdentifiedEmbedder, IdentifiedIndexReader, IdentifiedPerplexityScorer,
};

use crate::versioned_pipeline_bundle::{MINIMAL_SCORE_IMPLEMENTATION, PipelineTraceCreditEvent};
use crate::versioned_pipeline_compat::COMPATIBILITY_SCORE_IMPLEMENTATION;
use crate::versioned_pipeline_substance::SUBSTANCE_SCORE_IMPLEMENTATION;

/// Safe label for registering a factory under a built-in implementation id.
pub const SCORE_POLICY_IMPLEMENTATION_RESERVED_LABEL: &str = "score_policy_implementation_reserved";
/// Safe label for registering a second factory under the same id.
pub const SCORE_POLICY_IMPLEMENTATION_DUPLICATE_LABEL: &str =
    "score_policy_implementation_duplicate";
/// Safe label for an empty implementation id.
pub const SCORE_POLICY_IMPLEMENTATION_INVALID_LABEL: &str = "score_policy_implementation_invalid";
/// Identity the per-bundle qualification reports for a package whose Score
/// implementation id is neither built in nor registered. Such a package is
/// not runnable either; the check fails closed so it is never qualified.
pub const SCORE_POLICY_MISSING_LABEL: &str = "score_policy_missing";

/// What a factory is given to build one Score policy for one bundle: the
/// same dependencies the compatibility Score policy is built from.
/// `configuration` is the artifact the manifest's Score
/// `configuration_hash` names; `BundlePackage::validate` has already checked
/// that these bytes hash to it, so the factory decodes its own configuration
/// from bytes the signed manifest pins. `scorer` and `embedder` have already
/// been checked against the Score policy ref's `data_artifact_hashes`.
pub struct ScorePolicyDependencies<'a> {
    pub manifest: &'a BundleManifest,
    pub configuration: &'a [u8],
    pub scorer: Arc<dyn IdentifiedPerplexityScorer>,
    pub embedder: Arc<dyn IdentifiedEmbedder>,
    pub index_reader: Arc<dyn IdentifiedIndexReader>,
}

/// Builds a Score policy for a bundle whose manifest names this factory's
/// implementation id. Implemented by an embedding binary; the open build
/// registers none.
///
/// A registered policy runs under the same Score runtime controls as the
/// compatibility Score policy
/// ([`score_runs_under_runtime_controls`](crate::versioned_pipeline_bundle::score_runs_under_runtime_controls)):
///
/// - its Score takes the tenant's Score lock, and reads the index commands
///   of the tenant's runs that Settle has not applied as neighbours;
/// - `main`'s duplicate short-circuits decide its Settle selection, keep a
///   duplicate out of the index, and withhold a duplicate's Trace Credit
///   leg, so a duplicate earns nothing whatever the policy awarded;
/// - Settle writes its `trace_gate_decisions` row from its Score evidence.
///   The row needs `perplexity_micros`, `tail_fraction_micros`,
///   `novelty_score_micros`, `quality_passed`, `novelty_passed`, and
///   `nearest_neighbor_hash` on the policy's `ScoreEvidence`, as the
///   compatibility evidence carries them. A policy whose evidence lacks one
///   fails its run closed at Settle
///   (`pipeline_gate_decision_evidence_incomplete`) before any leg pays;
///   the server never fills one in.
///
/// A policy may hold a trace for operator review instead of pricing it
/// (`ScoreOutput::held_for_review`). A reviewer resolves the hold through
/// the pipeline review routes. On an approval the policy runs again with
/// `ScoreInput::operator_review` set (the decision and the hold label,
/// nothing that names the reviewer) and must price the trace: holding again
/// fails the run closed (`pipeline_score_hold_repeated`). On a rejection the
/// policy is not run; the server commits `ScoreOutput::rejected_after_hold`,
/// and the run settles with no credit, no index entry and no gate decision
/// row.
///
/// Production qualification: a bundle naming this factory's id is recorded
/// as qualified (`validate_production_package_with_score_policies`) only
/// when [`Self::production_qualified`] is `true` and
/// [`Self::configuration_is_qualifiable`] accepts its Score configuration
/// bytes. Both default to `false`.
pub trait ScorePolicyFactory: Send + Sync {
    /// A safe label that names the factory in hash-only reports.
    fn dependency_identity(&self) -> &str;

    /// Which Trace Credit ledger event a bundle built by this factory
    /// settles under (see [`PipelineTraceCreditEvent`]). Declared by the
    /// factory, not inferred by the server.
    fn trace_credit_event(&self) -> PipelineTraceCreditEvent;

    /// Builds the policy. An error is a safe label; the bundle refuses the
    /// package as `bundle_package_invalid` whatever the label.
    fn build(
        &self,
        dependencies: ScorePolicyDependencies<'_>,
    ) -> Result<Arc<dyn ScorePolicy>, &'static str>;

    /// `false` by default. Readiness and qualification fail closed on
    /// `false`.
    fn production_qualified(&self) -> bool {
        false
    }

    /// Whether `configuration` (the bytes the manifest's Score
    /// `configuration_hash` names) may be qualified for production -- the
    /// counterpart of `CompatibilityBundleConfig::is_qualifiable`. `false`
    /// by default.
    fn configuration_is_qualifiable(&self, configuration: &[u8]) -> bool {
        let _ = configuration;
        false
    }
}

/// Score policy factories an embedding binary supplies, keyed by the
/// implementation id a bundle manifest's Score policy ref names. Empty by
/// default, which is what the open build runs.
#[derive(Clone, Default)]
pub struct ScorePolicyRegistry {
    factories: BTreeMap<String, Arc<dyn ScorePolicyFactory>>,
}

impl ScorePolicyRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registers `factory` under `implementation_id`. The built-in Score
    /// implementation ids are reserved, an id may be registered once, and an
    /// empty id is refused.
    pub fn with_factory(
        mut self,
        implementation_id: &str,
        factory: Arc<dyn ScorePolicyFactory>,
    ) -> Result<Self, &'static str> {
        if implementation_id.trim().is_empty() {
            return Err(SCORE_POLICY_IMPLEMENTATION_INVALID_LABEL);
        }
        if is_built_in_score_implementation(implementation_id) {
            return Err(SCORE_POLICY_IMPLEMENTATION_RESERVED_LABEL);
        }
        if self.factories.contains_key(implementation_id) {
            return Err(SCORE_POLICY_IMPLEMENTATION_DUPLICATE_LABEL);
        }
        self.factories
            .insert(implementation_id.to_string(), factory);
        Ok(self)
    }

    /// The factory registered under `implementation_id`, if any. Never
    /// answers for a built-in id.
    pub fn get(&self, implementation_id: &str) -> Option<&Arc<dyn ScorePolicyFactory>> {
        self.factories.get(implementation_id)
    }

    pub fn is_empty(&self) -> bool {
        self.factories.is_empty()
    }
}

/// Whether `implementation_id` is one of the built-in Score policies.
pub fn is_built_in_score_implementation(implementation_id: &str) -> bool {
    implementation_id == COMPATIBILITY_SCORE_IMPLEMENTATION
        || implementation_id == MINIMAL_SCORE_IMPLEMENTATION
        || implementation_id == SUBSTANCE_SCORE_IMPLEMENTATION
}
