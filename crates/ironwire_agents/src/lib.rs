//! Pointing a coding agent's own config file at IronWire, without taking it over.
//!
//! Everything that knows where an agent keeps its settings, and how to edit that
//! file politely, lives here — the two agents IronWire ships knowing about and
//! the ones a signed catalog introduces. It is a crate rather than a module of
//! the binary because the control API has to answer "which tools does this
//! machine have, and are they pointed at us" as well, and a second copy of
//! those paths in `ironwire_proxy` is exactly the drift this arrangement exists
//! to prevent.
//!
//! Three rules run through all of it, and they are what make editing a file the
//! user owns acceptable at all:
//!
//! - **Never rewrite a file we cannot parse.** Their syntax error must not come
//!   back looking like ours.
//! - **Fill an empty slot; leave a full one alone.** A value already there is
//!   another proxy or a deliberate choice, and taking it over would move
//!   someone's traffic without telling them.
//! - **Remove only what we put there.**

pub mod catalog;
pub mod claude_settings;
pub mod codex_config;
pub mod endpoint;
pub mod tools;
