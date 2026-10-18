// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Hosted ingest service and the entry point for embedding builds.

pub mod ingest;
pub use ingest::{IngestOptions, OPEN_BUILD_BIN_NAME, run, run_with_score_policies};
