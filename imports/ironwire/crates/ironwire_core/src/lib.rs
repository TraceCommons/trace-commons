//! IronWire core: the vocabulary every other crate agrees on.
//!
//! This crate depends on nothing else of ours and does no I/O. It holds the
//! wire protocols, the capability model that decides whether a route can
//! preserve a request's semantics, the observed-quota ledger, and the routing
//! policy.
//!
//! The one idea worth internalising before reading further: IronWire has two
//! lanes (see `docs/DESIGN.md` §2). The **native** lane forwards bytes without
//! re-encoding them, so its fidelity is 1.0 by construction. The **translated**
//! lane is capability-gated and *refuses* rather than degrades. Types here are
//! shaped to make the second lane's refusals cheap and total.
#![warn(missing_docs)]

pub mod admission;
pub mod atomic;
pub mod capability;
pub mod confidence;
pub mod config;
pub mod config_edit;
pub mod discovery;
pub mod error;
pub mod peek;
pub mod policy;
pub mod protocol;
pub mod quota;
pub mod quota_store;

pub use capability::{Capabilities, Ineligible, ReasoningNeed, RequestRequirements};
pub use config::{Config, PathsConfig, ResilienceConfig, ServerConfig, UpdateConfig};
pub use error::{Error, Result};
pub use peek::{IdentityMarkers, RequestPeek};
pub use policy::{ConversationKey, RouteDecision, Rung};
pub use protocol::{BackendId, BackendKind, Facade, ModelTier, Protocol};
pub use quota::{Headroom, QuotaSnapshot};

/// Default loopback port. Chosen to be memorable and outside the common
/// dev-server range; the daemon binds `127.0.0.1` only (`docs/TRUST.md` I1).
pub const DEFAULT_PORT: u16 = 8463;

/// Opt-in, byte-preserving token capture request augmentation.
pub mod token_capture;
