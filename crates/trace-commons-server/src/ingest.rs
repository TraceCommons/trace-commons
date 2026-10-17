// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The hosted ingest service (`trace-commons-ingest`) as a library entry
//! point.
//!
//! [`run`] does everything the `trace-commons-ingest` binary's `main` does:
//! it reads the command line and the environment, assembles the state and
//! the pipeline runtime, and serves until shutdown. The open binary
//! (`src/bin/trace_commons_ingest_main.rs`) is a thin `main` that calls it
//! with [`IngestOptions::open_build`], whose Score policy registry is empty.
//!
//! A build that registers its own Score policies (the proprietary-backend
//! seam in `crates/trace-commons-gate-api/README.md`) builds its own binary
//! over the same entry and passes its factories in
//! [`IngestOptions::with_score_policies`]. The registry reaches every place
//! ingest assembles a pipeline service or a qualification store: the
//! production pipeline assembly (`ProductionPipelineInputs::score_policies`)
//! and the qualification store, which qualifies what the service can bind.
//! A registered factory is still unqualified for production unless it
//! declares otherwise
//! ([`ScorePolicyFactory::production_qualified`](crate::versioned_pipeline_score_registry::ScorePolicyFactory::production_qualified)).
//!
//! Nothing else differs between the two builds; in particular
//! `GET /v1/source` is served the same way by both.

use crate::versioned_pipeline_score_registry::ScorePolicyRegistry;

/// The service itself. The file stays at its historical path under
/// `src/bin/`, with its sibling modules in `trace_commons_ingest_internal/`;
/// it is compiled here, into the library, and is no longer a binary target.
#[path = "bin/trace-commons-ingest.rs"]
pub(crate) mod app;

/// The name the open binary reports (`--version`, and the build identity).
pub const OPEN_BUILD_BIN_NAME: &str = "trace-commons-ingest";

/// What a binary starts ingest with.
#[derive(Clone)]
pub struct IngestOptions {
    bin_name: &'static str,
    score_policies: ScorePolicyRegistry,
}

impl IngestOptions {
    /// The open build: no registered Score policies, and the open binary's
    /// name.
    pub fn open_build() -> Self {
        Self {
            bin_name: OPEN_BUILD_BIN_NAME,
            score_policies: ScorePolicyRegistry::new(),
        }
    }

    /// The Score policy factories the pipeline service and the
    /// qualification store can bind, replacing any set before.
    pub fn with_score_policies(mut self, score_policies: ScorePolicyRegistry) -> Self {
        self.score_policies = score_policies;
        self
    }

    /// The binary name `--version` reports. A binary built over this entry
    /// passes its own (`env!("CARGO_BIN_NAME")`).
    pub fn with_bin_name(mut self, bin_name: &'static str) -> Self {
        self.bin_name = bin_name;
        self
    }

    pub fn score_policies(&self) -> &ScorePolicyRegistry {
        &self.score_policies
    }

    pub fn bin_name(&self) -> &'static str {
        self.bin_name
    }
}

impl Default for IngestOptions {
    fn default() -> Self {
        Self::open_build()
    }
}

/// Runs ingest as the `trace-commons-ingest` binary does: command-line
/// subcommands, configuration from the environment, then serve.
pub async fn run(options: IngestOptions) -> anyhow::Result<()> {
    app::run_ingest_main(options).await
}

/// [`run`] with the open-build options and `score_policies` registered.
pub async fn run_with_score_policies(score_policies: ScorePolicyRegistry) -> anyhow::Result<()> {
    run(IngestOptions::open_build().with_score_policies(score_policies)).await
}
