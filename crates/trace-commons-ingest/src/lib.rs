// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hosted ingest service and the entry point for embedding builds.

// `test-support` builds the fixtures in `ingest::app::test_support`, whose
// env-var setup needs `unsafe`. No shipped build may enable that feature
// (`scripts/ci/check-release-test-support.sh`), so every shipped build still
// forbids unsafe code.
#![cfg_attr(not(any(test, feature = "test-support")), forbid(unsafe_code))]

pub mod ingest;
pub use ingest::{IngestOptions, OPEN_BUILD_BIN_NAME, run, run_with_score_policies};

/// Test fixtures for packages that test this service from outside it. Only
/// under the `test-support` feature, which no shipped build may enable.
#[cfg(feature = "test-support")]
#[doc(hidden)]
pub use ingest::app::test_support;
