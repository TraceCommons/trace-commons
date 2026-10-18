// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `trace-commons-ingest` binary: the library's ingest entry point
//! (`trace_commons_ingest`) with the open-build options, which
//! register no Score policies. The service itself is
//! `src/bin/trace-commons-ingest.rs`, compiled into the library.

use trace_commons_ingest::IngestOptions;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    trace_commons_ingest::run(IngestOptions::open_build()).await
}
