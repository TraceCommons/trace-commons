//! TraceDAO hosted server crate.
//!
//! The first extraction pass keeps server binaries in this crate and uses
//! Ironclaw as a temporary protocol/storage dependency. Shared types should move
//! into a dedicated protocol crate before this becomes the production server.

pub const TRACEDAO_SERVER_EXTRACTION_STAGE: &str = "ironclaw-bridge";
