//! The background upload daemon.
//!
//! The daemon watches the local coding-agent session roots, decides which
//! sessions are finished and uploadable, tells the contributor about them,
//! uploads the ones they approve, and auto-uploads the projects they have
//! explicitly opted in. It serves a versioned IPC contract so native tray and
//! window applications can drive all of that without reimplementing any of it.
//!
//! Every upload takes the same path an interactive `submit` takes, via
//! `submit::SubmitContext`. There is no second pipeline.
//!
//! Privacy posture, which the rest of this module tree is built to preserve:
//!
//! - A local filesystem path appears only in `daemon-queue.jsonl` and
//!   `daemon-state.json`. It never reaches a receipt, a history record, a log
//!   line, or the wire. Consumers get `project_label`.
//! - Nothing is uploaded from a project the contributor has not opted in, and
//!   sessions whose working directory cannot be resolved can never be opted in
//!   at all.
//! - A configured privacy filter that is unavailable stops the pipeline. It
//!   never degrades to sending unfiltered text.

pub mod settings;
pub mod state;
