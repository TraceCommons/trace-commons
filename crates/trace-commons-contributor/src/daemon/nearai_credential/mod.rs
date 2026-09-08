//! Obtaining a NEAR AI inference credential, on this machine, for this
//! contributor.
//!
//! The shape of the flow is forced by the service rather than chosen. Every
//! management route on cloud-api is authenticated by a session JWT and every
//! inference route by an `sk-` API key, and the two middlewares are disjoint
//! with no fallback in either direction -- a session cannot make an inference
//! call, and a key cannot read or mint anything. So signing in is not the end
//! of the flow, it is the beginning of one:
//!
//! ```text
//! browser sign in  ->  rt_ refresh token + session JWT   (loopback)
//!                  ->  GET  /v1/organizations            -> org
//!                  ->  GET  /v1/organizations/{org}/workspaces -> workspace
//!                  ->  POST /v1/workspaces/{ws}/api-keys -> sk- key  [store this]
//! ```
//!
//! The last call returns the plaintext key once and accepts an omitted
//! `expires_at`, so the credential we keep does not expire and the session
//! that minted it is discarded immediately. That is the point of doing it this
//! way: an inference credential with no refresh story at all beats a session
//! this daemon would have to keep alive for the rest of its life.

pub mod api;
pub mod loopback;
