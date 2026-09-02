//! Attestation verification primitives shared between the hosted server and
//! the client-side crates that ship inside third-party agent harnesses.
//!
//! This crate is `MIT OR Apache-2.0`. It exists so that a contributor can
//! verify an attestation *before* handing over raw bytes, which means the
//! verification code cannot live behind the AGPL boundary that
//! `trace-commons-server` sits on.

pub mod measurements;
pub mod quote;
pub mod receipt;
