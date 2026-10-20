//! The signed provider-catalog channel.
//!
//! IronWire depends on values it does not control: a `anthropic-beta` flag, an
//! API version string, a model catalogue, the prefix that identifies a client.
//! When a provider changes one, every deployed IronWire breaks at once — and
//! shipping a binary through five package ecosystems and waiting for users to
//! upgrade is far too slow a fix.
//!
//! So those values live in a **signed, versioned document** that the daemon can
//! refresh independently of the binary. Data, not code.
//!
//! The design constraint that makes this safe is in [`schema`]: **no type here
//! can express a host, a URL, or a filesystem path.** Base URLs and the
//! credential→host binding stay compiled in, so whoever holds the signing key
//! cannot redirect a subscription token (`docs/TRUST.md` I2). Read the module
//! docs before adding a field.
//!
//! See `docs/UPDATES.md` for how this fits alongside notify-only updates.
#![warn(missing_docs)]

pub mod schema;
pub mod store;

pub use schema::{AnthropicCatalog, Catalog, ClientIdentityCatalog, ModelEntry, SCHEMA_VERSION};
pub use store::{CatalogError, CatalogStore, SignedCatalog};

/// Public key this build trusts for catalog documents.
///
/// Baked in on purpose: a key fetched at runtime is not a root of trust. Its
/// private half lives in the release signing infrastructure, not in this
/// repository — see `docs/UPDATES.md`.
///
/// The placeholder below is deliberately **not** a usable key. Until release
/// signing exists, every document fails verification and the daemon runs on the
/// compiled-in defaults, which is the correct failure direction.
pub const CATALOG_PUBLIC_KEY: [u8; 32] = [0u8; 32];
