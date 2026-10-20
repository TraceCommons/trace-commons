//! The IronWire loopback daemon.
//!
//! Mounts the provider façades and the control API on one `127.0.0.1` listener
//! (`docs/TRUST.md` I1 — there is deliberately no way to bind anything else).
#![warn(missing_docs)]

pub mod control;
pub mod events;
pub mod facade;
pub mod pipeline;
pub mod privacy;
pub mod proof;
pub mod resilience;
pub mod server;
pub mod shutdown;
pub mod spend;
pub mod state;
pub mod statusline;

pub use server::{ServeError, serve};
pub use state::{AppState, BackendRegistry};

pub mod embed;
