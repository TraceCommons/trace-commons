//! Update discovery and installation.
//!
//! The manifest is the only thing a client trusts to learn that a new
//! version exists. It is signed because the transport is not: a public
//! bucket is a fine place to put bytes and a poor place to put authority.
pub mod authenticode;
pub mod endpoint;
pub mod fetch;
pub mod manifest;
pub mod source;
pub mod swap;
pub mod version;
