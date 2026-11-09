//! The Terms of Service acceptance this Mac holds, the published version the
//! daemon last saw, and the rule that holds uploads between the two.
//!
//! Design: `docs/superpowers/specs/2026-11-09-terms-scroll-acceptance-design.md`,
//! sections 3 and 5. One file in the state directory, `terms-state.json`
//! (0600, atomic). The published version is persisted, not only held in
//! memory, so a hold survives a restart with no network: a failed fetch never
//! starts a hold, and it never lifts one either.
//!
//! The hold is its own state. It is never the contributor's pause: Resume
//! does not clear it, and accepting does not resume a pause the contributor
//! set.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::config::ConfigStore;

/// The state file's name in the state directory.
pub const TERMS_STATE_FILE: &str = "terms-state.json";

/// `accept` named a version that is no longer the published one.
pub const TERMS_VERSION_STALE: &str = "terms-version-stale";

/// The version, the hash of the bytes the app rendered, and when the
/// contributor accepted them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AcceptedTerms {
    pub version: String,
    pub content_sha256: String,
    pub accepted_at: DateTime<Utc>,
}

/// A tenant the server confirmed the current acceptance for.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordedIn {
    pub tenant_id: String,
    pub recorded_at: DateTime<Utc>,
}

/// The version the site published at the daemon's last successful fetch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublishedTerms {
    pub version: String,
    pub content_sha256: String,
    pub in_effect: bool,
    pub checked_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermsState {
    #[serde(default)]
    pub accepted: Option<AcceptedTerms>,
    #[serde(default)]
    pub recorded_in: Vec<RecordedIn>,
    #[serde(default)]
    pub published: Option<PublishedTerms>,
}

impl TermsState {
    /// The saved state, or the default when no file exists yet.
    pub fn load(store: &ConfigStore) -> Result<TermsState> {
        match store.read_daemon_file(TERMS_STATE_FILE)? {
            None => Ok(TermsState::default()),
            Some(bytes) => serde_json::from_slice(&bytes).context("terms-state-unreadable"),
        }
    }

    /// Write the state atomically at 0600.
    pub fn save(&self, store: &ConfigStore) -> Result<()> {
        let bytes = serde_json::to_vec_pretty(self).context("terms-state-unwritable")?;
        store.write_daemon_file(TERMS_STATE_FILE, &bytes)
    }

    /// Whether uploads are held: an in-effect version is published and it is
    /// not the version accepted here. A draft (`in_effect: false`) never
    /// holds, and nothing holds before the first successful fetch.
    pub fn held(&self) -> bool {
        match &self.published {
            None => false,
            Some(published) => {
                published.in_effect
                    && self.accepted.as_ref().map(|a| a.version.as_str())
                        != Some(published.version.as_str())
            }
        }
    }

    /// The published version owed, when uploads are held for it.
    pub fn version_owed(&self) -> Option<String> {
        if self.held() {
            self.published.as_ref().map(|p| p.version.clone())
        } else {
            None
        }
    }

    /// Replace the acceptance. Refused with [`TERMS_VERSION_STALE`] when a
    /// different version is published now: the person read something that
    /// is no longer current. Clears `recorded_in`, so the new acceptance is
    /// posted again to every org.
    pub fn accept(&mut self, terms: AcceptedTerms) -> Result<(), &'static str> {
        if let Some(published) = &self.published
            && published.version != terms.version
        {
            return Err(TERMS_VERSION_STALE);
        }
        self.accepted = Some(terms);
        self.recorded_in.clear();
        Ok(())
    }

    /// Whether the acceptance still has to be posted for `tenant_id`.
    pub fn needs_post(&self, tenant_id: &str) -> bool {
        self.accepted.is_some() && !self.recorded_in.iter().any(|r| r.tenant_id == tenant_id)
    }

    /// The server confirmed the acceptance for `tenant_id`.
    pub fn mark_recorded(&mut self, tenant_id: &str, recorded_at: DateTime<Utc>) {
        if let Some(existing) = self
            .recorded_in
            .iter_mut()
            .find(|r| r.tenant_id == tenant_id)
        {
            existing.recorded_at = recorded_at;
            return;
        }
        self.recorded_in.push(RecordedIn {
            tenant_id: tenant_id.to_string(),
            recorded_at,
        });
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 9, 12, 0, 0).unwrap()
    }

    fn hash(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    fn published(version: &str, in_effect: bool) -> PublishedTerms {
        PublishedTerms {
            version: version.into(),
            content_sha256: hash('a'),
            in_effect,
            checked_at: t0(),
        }
    }

    fn accepted(version: &str) -> AcceptedTerms {
        AcceptedTerms {
            version: version.into(),
            content_sha256: hash('a'),
            accepted_at: t0(),
        }
    }

    #[test]
    fn no_published_version_never_holds() {
        let state = TermsState::default();
        assert!(!state.held());
        let state = TermsState {
            accepted: Some(accepted("v1")),
            ..Default::default()
        };
        assert!(!state.held());
        assert_eq!(state.version_owed(), None);
    }

    #[test]
    fn draft_version_never_holds() {
        let state = TermsState {
            published: Some(published("2026-08-22-draft", false)),
            ..Default::default()
        };
        assert!(!state.held());
        assert_eq!(state.version_owed(), None);
    }

    #[test]
    fn in_effect_version_holds_until_accepted() {
        let mut state = TermsState {
            accepted: Some(accepted("v1")),
            published: Some(published("v2", true)),
            ..Default::default()
        };
        assert!(state.held());
        assert_eq!(state.version_owed().as_deref(), Some("v2"));
        state.accept(accepted("v2")).unwrap();
        assert!(!state.held());
        assert_eq!(state.version_owed(), None);
    }

    #[test]
    fn user_with_no_record_is_held() {
        let state = TermsState {
            accepted: None,
            published: Some(published("v1", true)),
            ..Default::default()
        };
        assert!(state.held());
    }

    #[test]
    fn accept_clears_recorded_in() {
        let mut state = TermsState {
            accepted: Some(accepted("v1")),
            published: Some(published("v2", true)),
            ..Default::default()
        };
        state.mark_recorded("tenant-a", t0());
        assert!(!state.needs_post("tenant-a"));
        state.accept(accepted("v2")).unwrap();
        assert!(state.recorded_in.is_empty());
        assert!(state.needs_post("tenant-a"));
    }

    #[test]
    fn accept_refuses_stale_version() {
        let mut state = TermsState {
            accepted: Some(accepted("v1")),
            published: Some(published("v3", true)),
            ..Default::default()
        };
        state.mark_recorded("tenant-a", t0());
        assert_eq!(state.accept(accepted("v2")), Err(TERMS_VERSION_STALE));
        // Nothing moved.
        assert_eq!(state.accepted, Some(accepted("v1")));
        assert_eq!(state.recorded_in.len(), 1);
        assert!(state.held());
    }

    #[test]
    fn needs_post_only_with_an_acceptance() {
        let mut state = TermsState::default();
        assert!(!state.needs_post("tenant-a"));
        state.accept(accepted("v1")).unwrap();
        assert!(state.needs_post("tenant-a"));
        state.mark_recorded("tenant-a", t0());
        assert!(!state.needs_post("tenant-a"));
        assert!(state.needs_post("tenant-b"));
    }

    #[test]
    fn round_trips_with_0600() {
        let (_dir, store) = crate::config::tests_support::temp_store();
        assert_eq!(TermsState::load(&store).unwrap(), TermsState::default());
        let mut state = TermsState {
            accepted: Some(accepted("v1")),
            published: Some(published("v1", true)),
            ..Default::default()
        };
        state.mark_recorded("tenant-a", t0());
        state.save(&store).unwrap();
        assert_eq!(TermsState::load(&store).unwrap(), state);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            let mode = std::fs::metadata(store.daemon_path(TERMS_STATE_FILE))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }
    }
}
