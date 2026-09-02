//! Attestation verification primitives shared between the hosted server and
//! the client-side crates that ship inside third-party agent harnesses.
//!
//! This crate is `MIT OR Apache-2.0`. It exists so that a contributor can
//! verify an attestation *before* handing over raw bytes, which means the
//! verification code cannot live behind the AGPL boundary that
//! `trace-commons-server` sits on.

pub mod address;
pub mod measurements;
pub mod quote;
/// NEAR AI inference-receipt verification, and the EIP-191 signer recovery it
/// is built on.
///
/// Behind the `receipt` feature because `k256` and `sha3` are the only
/// dependencies of this crate that quote verification does not need, and they
/// bring nineteen transitive crates with them. A client that ships inside a
/// third-party agent harness and only verifies *quotes* should not pay for a
/// curve implementation.
///
/// **It is not only receipts.** [`receipt::recover_eip191_signer`] is the same
/// operation a witness certificate's signature needs, so a client that
/// verifies a certificate must enable this feature and does pay that cost.
/// [`address::decode_address`] is deliberately outside it.
#[cfg(feature = "receipt")]
pub mod receipt;
