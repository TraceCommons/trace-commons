//! The optional privacy filter: reversible substitution on the request path.
//!
//! Off by default. `docs/PRIVACY.md` is the design and the critique of the
//! design; `docs/TRUST.md` I7 is the promise that governs how it is described.
//!
//! The short version of both: this is a **risk reducer with a known
//! false-negative rate**, not a guarantee, and nothing in this crate or the UI
//! above it may claim otherwise. A privacy tool that manufactures confidence is
//! worse than none, because it changes what people are willing to paste into an
//! agent.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

pub mod detect;
pub mod mint;
pub mod reverse;
pub mod substitute;

pub use detect::{Detector, Finding, Tiers};
pub use mint::{Class, Map, Salt};
pub use reverse::{ReverseError, Reverser};
pub use substitute::{Exemptions, Substituted, substitute};
