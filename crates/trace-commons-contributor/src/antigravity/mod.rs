//! Import command support for Antigravity IDE trajectories.
//!
//! This is not a `TraceSource`: the daemon never sees Antigravity directly.
//! The `discover` probe here finds the IDE's local language server API,
//! `client` reads the conversations it serves, `convert` turns them into
//! Trajectory-v1 records, and `import` stages those through the existing
//! `trajectory` source instead.
//!
//! `dead_code` stays allowed at the module level for the descriptive fields
//! and probe internals the import path does not itself read (`git_root`,
//! `step_count`, `Candidate`), which are part of the recorded API surface
//! and are asserted by this module's own tests.
#![allow(dead_code)]

mod client;
mod convert;
mod endpoint;
// The only submodule with a caller outside this module: `commands` drives
// the import. Everything else is reached from inside here, so there are no
// re-exports to drift out of sync with what is actually used.
pub(crate) mod import;
