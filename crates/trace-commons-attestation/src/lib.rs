//! Attestation verification primitives shared between the hosted server and
//! the client-side crates that ship inside third-party agent harnesses.
//!
//! This crate is `MIT OR Apache-2.0`. It exists so that a contributor can
//! verify an attestation *before* handing over raw bytes, which means the
//! verification code cannot live behind the AGPL boundary that
//! `trace-commons-server` sits on.

pub mod address;
/// EIP-191 signer recovery. Ungated: a redaction-witness certificate is
/// signed this way and is not a NEAR AI receipt, so a client that verifies
/// one must not have to enable a receipt feature to do it.
pub mod eip191;
pub mod measurements;
pub mod quote;
/// NEAR AI inference-receipt parsing and verification.
///
/// Behind the `receipt` feature, which the hosted server turns on and a
/// contributor client does not: a client makes no inference calls and holds
/// no receipts.
///
/// **The feature saves no packages, and is not meant to.** The signer
/// recovery this module is built on moved to [`eip191`], which is ungated,
/// because a redaction-witness certificate is signed the same way -- so
/// `k256` and `sha3` are unconditional dependencies of this crate and every
/// consumer pays for them. What the feature buys is honesty about which half
/// of the crate a caller uses, and a smaller compiled surface for one that
/// only needs the other.
#[cfg(feature = "receipt")]
pub mod receipt;
