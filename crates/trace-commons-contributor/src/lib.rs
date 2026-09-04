//! Contributor-side client for trace-commons-server: discovers local coding
//! agent transcripts, redacts them through the deterministic pipeline, and
//! submits TraceContributionEnvelopes under instance-vouched per-user
//! identities.

pub mod account_auth;
pub(crate) mod antigravity;
pub mod brand;
pub mod commands;
pub mod config;
pub mod consent;
pub mod daemon;
pub mod envelope;
pub mod identity;
pub mod issuer_client;
pub mod picker;
pub mod pricing;
pub mod routing;
pub mod routing_copy;
pub mod source;
pub mod source_copy;
pub mod submit;
pub mod update;
pub mod watch_events;
pub mod withdraw;
pub mod witness;
pub mod witness_copy;
