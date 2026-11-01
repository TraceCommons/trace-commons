//! Routing and cost data for the inference hops behind a session.
//!
//! The row type, the ledger trait, the attested-call capture (`attested`)
//! and the decorating source (`enriched`) live in the
//! `trace-commons-contributor-sources` leaf crate, next to the adapters they
//! decorate, and are re-exported here at their old paths; that crate's
//! `routing` module documents the rules that hold for all of it. The modules
//! declared below stay in this crate: the IronWire ledger reader, NEAR AI
//! receipt fetching, and the proof and attestation-report checks.

pub use trace_commons_contributor_sources::routing::*;

pub mod attestation_report;
pub mod ironwire;
pub(crate) mod managed_history;
pub mod proof_attestor;
pub mod receipt;

#[cfg(test)]
mod cross_layer_tests;
