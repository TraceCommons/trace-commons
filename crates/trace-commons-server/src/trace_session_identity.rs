// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Canonical source-session identity for account-scoped withdrawal equality.

use sha2::{Digest, Sha256};
use trace_commons_protocol::trace_contribution::{
    SourceSessionIdentity, validate_source_session_identity,
};

pub use trace_commons_protocol::trace_contribution::SourceSessionIdentityError as SourceSessionError;

const DIGEST_DOMAIN: &[u8] = b"trace-commons:source-session:v1\0";

#[derive(Clone, PartialEq, Eq)]
pub struct CanonicalSourceSession {
    adapter: String,
    native_id: String,
}

impl std::fmt::Debug for CanonicalSourceSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("CanonicalSourceSession")
            .field("adapter", &self.adapter)
            .field("native_id", &"[redacted]")
            .finish()
    }
}

pub fn canonical_source_session(
    identity: &SourceSessionIdentity,
) -> Result<CanonicalSourceSession, SourceSessionError> {
    validate_source_session_identity(identity)?;
    Ok(CanonicalSourceSession {
        adapter: identity.adapter.clone(),
        native_id: identity.native_id.clone(),
    })
}

pub fn session_digest(session: &CanonicalSourceSession) -> [u8; 32] {
    let adapter = session.adapter.as_bytes();
    let native_id = session.native_id.as_bytes();
    let mut hasher = Sha256::new();
    hasher.update(DIGEST_DOMAIN);
    hasher.update((adapter.len() as u16).to_be_bytes());
    hasher.update(adapter);
    hasher.update((native_id.len() as u16).to_be_bytes());
    hasher.update(native_id);
    hasher.finalize().into()
}
