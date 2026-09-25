// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Canonical source-session identity for account-scoped withdrawal equality.

use sha2::{Digest, Sha256};
use trace_commons_protocol::trace_contribution::SourceSessionIdentity;
use uuid::Uuid;

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceSessionError {
    UnsupportedAdapter,
    InvalidNativeId,
}

pub fn canonical_source_session(
    identity: &SourceSessionIdentity,
) -> Result<CanonicalSourceSession, SourceSessionError> {
    let requires_uuid = match identity.adapter.as_str() {
        "codex" | "claude-code" => true,
        "opencode" | "cline" | "gemini-cli" => false,
        _ => return Err(SourceSessionError::UnsupportedAdapter),
    };
    let native_id = identity.native_id.as_bytes();
    if native_id.is_empty()
        || native_id.len() > 128
        || !native_id
            .iter()
            .all(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'-')
    {
        return Err(SourceSessionError::InvalidNativeId);
    }
    if requires_uuid {
        match Uuid::parse_str(&identity.native_id) {
            Ok(parsed) if parsed.hyphenated().to_string() == identity.native_id => {}
            _ => return Err(SourceSessionError::InvalidNativeId),
        }
    }
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
