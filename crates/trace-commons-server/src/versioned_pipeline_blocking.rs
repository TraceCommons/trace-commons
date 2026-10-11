// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The bounded blocking pool a versioned pipeline Score evaluation runs on
//! (#1140).
//!
//! A Score evaluation calls the scorer, the embedder and the index reader,
//! which are synchronous gate-api contracts: a local embedder is CPU-bound
//! for the length of every chunk, and a production scorer makes blocking
//! network calls. The pipeline worker runs on ingest's own runtime, the one
//! serving HTTP, so the evaluation runs on the blocking pool rather than on
//! a runtime worker thread. It also runs under a process-wide permit, so a
//! burst of Scores (the worker, an operator's `process_run`, a harness) holds
//! at most `PIPELINE_SCORE_EVALUATION_CONCURRENCY` blocking threads; the rest
//! wait for a permit on the runtime, where waiting holds no thread.

use std::sync::{Arc, OnceLock};

use tokio::sync::Semaphore;

/// How many Score evaluations may hold a blocking thread at once, process
/// wide.
pub const PIPELINE_SCORE_EVALUATION_CONCURRENCY: usize = 4;

/// The evaluation did not return a result: its blocking task panicked or
/// was cancelled, or no permit could be taken. Each caller reports it under
/// its own label.
#[derive(Debug)]
pub(crate) struct ScoreEvaluationTaskFailed;

fn score_evaluation_permits() -> Arc<Semaphore> {
    static PERMITS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    PERMITS
        .get_or_init(|| Arc::new(Semaphore::new(PIPELINE_SCORE_EVALUATION_CONCURRENCY)))
        .clone()
}

/// Runs `evaluate` on the blocking pool under a Score evaluation permit.
///
/// The permit moves into the blocking task and is released when `evaluate`
/// returns, not when the caller stops waiting: a caller dropped mid-flight
/// (`join_or_abort` at shutdown) cannot free a permit whose thread is still
/// busy, so the bound counts threads actually held.
pub(crate) async fn run_score_evaluation<T, F>(evaluate: F) -> Result<T, ScoreEvaluationTaskFailed>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    let permit = score_evaluation_permits()
        .acquire_owned()
        .await
        .map_err(|_| ScoreEvaluationTaskFailed)?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        evaluate()
    })
    .await
    .map_err(|_| ScoreEvaluationTaskFailed)
}
