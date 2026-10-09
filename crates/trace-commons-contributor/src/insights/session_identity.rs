//! Which harness session a saved snapshot records, as a keyed digest.
//!
//! The reimport overlap rule in [`super::week_rollup`] needs to tell a
//! session imported twice from two sessions that ran at the same time. Time
//! alone cannot: two Codex rollouts in two terminals overlap exactly as an
//! import and its reimport do. The session ID the harness wrote into its own
//! transcript can. It is stored only as a keyed digest under the store's
//! digest key (owner decision D15, open), beside that key's fingerprint, so a
//! digest made under another key is never compared.

use serde::{Deserialize, Serialize};
use trace_commons_protocol::insights_usage_series::{
    DigestKeyStore, KeyedDigest, harness_session_digest, key_fingerprint,
};

use super::SourceFormat;

pub const SESSION_IDENTITY_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedSessionIdentity {
    pub schema_version: u32,
    /// Names the digest key `session` was made under, without revealing it.
    pub key_fingerprint: KeyedDigest,
    pub session: KeyedDigest,
}

impl SavedSessionIdentity {
    pub fn validate(&self) -> anyhow::Result<()> {
        if self.schema_version != SESSION_IDENTITY_SCHEMA_VERSION {
            anyhow::bail!("insights_session_identity_invalid");
        }
        Ok(())
    }
}

/// The harness name the digest is made under: the same session ID written by
/// two harnesses is two sessions.
fn harness(format: SourceFormat) -> Option<&'static str> {
    match format {
        SourceFormat::ClaudeCode => Some(crate::source::SOURCE_CLAUDE_CODE),
        SourceFormat::Codex => Some(crate::source::SOURCE_CODEX),
        SourceFormat::Trajectory => None,
    }
}

/// The one session ID the transcript records: Codex's `session_meta` id, or
/// the `sessionId` every Claude Code record carries. `None` when there is
/// none, or more than one, so an ambiguous file falls back to the overlap
/// rule rather than being trusted as one session.
pub(crate) fn harness_session_id(format: SourceFormat, bytes: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(bytes).ok()?;
    let mut found: Option<String> = None;
    for line in text.lines().filter(|line| !line.trim().is_empty()) {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            continue;
        };
        let id = match format {
            SourceFormat::Codex if value["type"] == "session_meta" => {
                value["payload"]["id"].as_str()
            }
            SourceFormat::ClaudeCode => value["sessionId"].as_str(),
            _ => None,
        };
        let Some(id) = id.filter(|id| !id.is_empty()) else {
            continue;
        };
        match &found {
            None => found = Some(id.to_string()),
            Some(seen) if seen == id => {}
            Some(_) => return None,
        }
    }
    found
}

/// The identity a saved snapshot carries. No session ID, no key: a file that
/// records none never creates one.
pub(crate) fn saved_session_identity(
    format: SourceFormat,
    bytes: &[u8],
    keys: &dyn DigestKeyStore,
) -> Option<SavedSessionIdentity> {
    let harness = harness(format)?;
    let session_id = harness_session_id(format, bytes)?;
    let key = keys.load_or_create().ok()?;
    Some(SavedSessionIdentity {
        schema_version: SESSION_IDENTITY_SCHEMA_VERSION,
        key_fingerprint: key_fingerprint(&key),
        session: harness_session_digest(&key, harness, &session_id),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_protocol::insights_usage_series::InMemoryDigestKeyStore;

    fn codex(id: &str) -> Vec<u8> {
        format!(
            "{{\"type\":\"session_meta\",\"payload\":{{\"id\":\"{id}\"}}}}\n{{\"type\":\"response_item\",\"payload\":{{}}}}\n"
        )
        .into_bytes()
    }

    fn claude(ids: &[&str]) -> Vec<u8> {
        ids.iter()
            .map(|id| format!("{{\"type\":\"user\",\"sessionId\":\"{id}\"}}\n"))
            .collect::<String>()
            .into_bytes()
    }

    fn keys() -> InMemoryDigestKeyStore {
        InMemoryDigestKeyStore::with_seed([0x11; 32])
    }

    #[test]
    fn the_session_id_is_read_from_each_harness() {
        assert_eq!(
            harness_session_id(SourceFormat::Codex, &codex("c-1")).as_deref(),
            Some("c-1")
        );
        assert_eq!(
            harness_session_id(SourceFormat::ClaudeCode, &claude(&["a-1", "a-1"])).as_deref(),
            Some("a-1")
        );
        assert_eq!(
            harness_session_id(SourceFormat::Trajectory, &codex("c-1")),
            None
        );
    }

    #[test]
    fn two_session_ids_in_one_file_are_no_identity() {
        assert_eq!(
            harness_session_id(SourceFormat::ClaudeCode, &claude(&["a-1", "a-2"])),
            None
        );
    }

    #[test]
    fn one_session_imported_twice_has_one_identity_and_two_sessions_have_two() {
        let keys = keys();
        let first = saved_session_identity(SourceFormat::Codex, &codex("c-1"), &keys).unwrap();
        let mut grown = codex("c-1");
        grown.extend_from_slice(b"{\"type\":\"response_item\",\"payload\":{}}\n");
        let again = saved_session_identity(SourceFormat::Codex, &grown, &keys).unwrap();
        let other = saved_session_identity(SourceFormat::Codex, &codex("c-2"), &keys).unwrap();
        assert_eq!(first, again);
        assert_ne!(first.session, other.session);
        assert_eq!(first.key_fingerprint, other.key_fingerprint);
        first.validate().unwrap();
    }

    #[test]
    fn the_same_id_from_two_harnesses_is_two_sessions() {
        let keys = keys();
        let codex = saved_session_identity(SourceFormat::Codex, &codex("same"), &keys).unwrap();
        let claude =
            saved_session_identity(SourceFormat::ClaudeCode, &claude(&["same"]), &keys).unwrap();
        assert_ne!(codex.session, claude.session);
    }

    #[test]
    fn a_file_with_no_session_id_never_creates_a_key() {
        let keys = keys();
        let bare = b"{\"type\":\"session_meta\",\"payload\":{}}\n";
        assert_eq!(
            saved_session_identity(SourceFormat::Codex, bare, &keys),
            None
        );
        assert!(keys.load().unwrap().is_none());
    }

    #[test]
    fn the_session_id_is_never_stored_readable() {
        let identity =
            saved_session_identity(SourceFormat::Codex, &codex("c-readable-id"), &keys()).unwrap();
        let stored = serde_json::to_string(&identity).unwrap();
        assert!(!stored.contains("c-readable-id"), "{stored}");
    }
}
