//! Session-store parsers and the inference routing overlay for the Trace
//! Commons contributor.
//!
//! This is a leaf crate split out of `trace-commons-contributor`, which
//! re-exports both modules at their original paths
//! (`trace_commons_contributor::source` and
//! `trace_commons_contributor::routing`). It exists so daemon edits do not
//! recompile the parsers; it has no behaviour of its own beyond what those
//! modules always had.
//!
//! Items marked `#[doc(hidden)]` were crate-private before the split and are
//! public only so the contributor crate can keep calling them. They are not
//! a supported interface.

pub mod routing;
pub mod source;
