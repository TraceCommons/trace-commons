//! Private, local Insights for the Trace Commons contributor: deterministic
//! observations from session files the contributor selected, and the stores
//! and service that keep them.
//!
//! Split out of `trace-commons-contributor` so client builds that touch the
//! rest of the contributor do not recompile Insights and its unit tests. The
//! contributor re-exports [`insights`] at its original path
//! (`trace_commons_contributor::insights`); the module keeps its name here so
//! its own `crate::insights::...` paths are unchanged.
//!
//! It depends on the session parsers and routing types in
//! `trace-commons-contributor-sources` and on the OS and file primitives in
//! `trace-commons-contributor-platform`, never on the contributor itself.
//!
//! Items marked `#[doc(hidden)]` were crate-private before the split and are
//! public only so the contributor crate can keep calling them. They are not
//! a supported interface.
//!
//! # `test-credential-store`
//!
//! TEST-ONLY, forwarded from the contributor's feature of the same name, and
//! forwarded on to the platform crate. Under it the default digest-key
//! custody refuses (as it already does in this crate's own `cfg(test)`), and
//! the OS credential store refuses to open. This crate's unit tests use a
//! seeded in-memory digest key; a dependent crate's tests that need keyed
//! series pass their own `InMemoryDigestKeyStore` to
//! `LocalInsightStore::open_with_digest_keys`.

pub mod insights;

// The session adapters and routing types, under the names the Insights
// modules have always used for them.
pub(crate) use trace_commons_contributor_sources::{routing, source};
