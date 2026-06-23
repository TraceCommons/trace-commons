//! Login-with-NEAR (Slice 3a) account ceremony support.
//!
//! This module hosts the server-side NEAR sign-in flow: issuing a NEP-413
//! challenge, verifying the wallet-signed message against the configured
//! `recipient`, resolving the signing public key to a tenant, and minting the
//! account session. It is fail-closed and tenant-scoped like the rest of the
//! account surface.
//!
//! Scaffolding only at this slice point (Task 3): the NEP-413 payload encoding,
//! signature verification, NEAR RPC access-key resolution, and the begin/finish
//! handlers are filled in by later Slice 3a tasks (Task 4+). The configuration
//! (`config::NearConfig`) and the `CeremonyState::NearChallenge` variant that
//! this module will consume already exist.
