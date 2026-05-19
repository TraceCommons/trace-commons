//! Shared HTTP client surface for trace-commons-server operator binaries.
//!
//! This crate is consumed by the four operator binaries
//! (`trace-commons-review`, `trace-commons-worker`, `trace-commons-admin`,
//! `trace-commons-tenant`). It owns:
//!
//! - The HTTP transport ([`Client`]), including bearer-token resolution,
//!   defense-in-depth host allowlist enforcement, and typed error mapping
//!   for the server's `{"error": "<Label>"}` refusal envelope.
//! - Foundational types shared across all operator audiences (scopes,
//!   pagination, JSON-or-text rendering).
//!
//! Per-endpoint typed request/response structs land in the binary PRs
//! (#2 reviewer + admin, #3 worker + tenant). Each binary's `Cargo.toml`
//! adds the structs it needs in its own module; structs that are reused
//! across binaries migrate here.

pub mod client;
pub mod error;
pub mod format;
pub mod host_allowlist;

pub use client::Client;
pub use error::{Error, Result};
