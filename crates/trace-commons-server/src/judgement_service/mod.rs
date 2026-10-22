// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The Insights judgement service, as a library.
//!
//! The service runs three pinned judgements (`task`, `event_kind`,
//! `finished`) over one session's judgement digest on a NEAR AI model in a
//! TEE, meters the operator's spend per account, and returns labels with the
//! receipt evidence the daemon shows. Design:
//! `docs/superpowers/specs/2026-10-20-insights-judgement-service-design.md`
//! (spec PR #1501). This module is delivery slice 3, "Service core (library
//! only)". There is no binary, no HTTP surface, no enclave seam and no model
//! HTTP adapter here; those are slices 5 and 6.
//!
//! - [`model`]: the [`model::JudgementModel`] adapter trait, held as
//!   `Arc<dyn JudgementModel>` so a different model substitutes at the seam.
//! - [`mock`]: `MockJudgementModel`, hash-derived and signing real
//!   `provider_tee`-shaped receipts with a test key. Compiled only under
//!   `cfg(test)` or the `judgement-development-models` feature.
//! - [`prompts`]: the three prompts, compiled in, and their
//!   `prompt_version` pins.
//! - [`catalog`]: the NEAR AI catalog entry parser and the hourly-refresh
//!   state with its 24-hour staleness rule.
//! - [`production`]: the compiled-in production allowlist and the
//!   production check (allowlist, catalog flags, signer pin, rotation
//!   latch).
//! - [`receipt_check`]: the per-call wrapper: reserve, call, verify the
//!   receipt, settle, and only then read the label out of the verified
//!   body. A rotation-shaped receipt failure latches the model.
//! - [`limiter`] and [`ledger`]: the account-bound spend limiter and its
//!   sealed on-disk ledger, the service's only state.
//! - [`claim`]: the offline verifier for the judgement claim.
//! - [`digest`]: the one thing the service reads out of a digest, the
//!   number of marked events.
//! - [`service`]: one request end to end, over the pieces above, with the
//!   hash-only log line.
//!
//! Logging is hash-only (spec section 6): a random request id,
//! `digest_sha256`, the model id, the refusal label or `ok`, and a latency
//! bucket. Never the account, a key derived from it, a label, a confidence,
//! a prompt, the digest, the claim, or NEAR AI response text.

pub mod catalog;
pub mod claim;
pub mod digest;
pub mod ledger;
pub mod limiter;
#[cfg(any(test, feature = "judgement-development-models"))]
pub mod mock;
pub mod model;
pub mod production;
pub mod prompts;
pub mod receipt_check;
pub mod service;
