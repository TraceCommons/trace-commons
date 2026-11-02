//! Signing out clears the NEAR AI half of the connected account.
//!
//! Owner decisions (2026-11-02): Private AI does not outlive the account, so
//! CLI `logout`, unenroll, native sign-out and forget all clear the NEAR AI
//! inference key, its refresh session and the spending-organization record;
//! and the inference key is revoked at NEAR AI on the way out, best effort.
//!
//! The order is clear, then revoke. Clearing never waits on NEAR AI: what the
//! revoke needs is copied under the commit lock, the local clear runs through
//! the cleanup journal ([`super::ceremony::forget_locked`]), and only then is
//! the copied session exchanged for an access token and the keys deleted. A
//! failed or impossible revoke is one label line and nothing else: the clear
//! stands. The exchange's rotated refresh token is dropped, never stored --
//! storing it would bring back the session this has just removed.
//!
//! The pending revocations in the spending-organization record are revoked
//! with the same session, since the record goes with the clear and nothing
//! would be left to retry them.

use std::time::Duration;

use anyhow::Result;

use super::api::{CloudApi, RevokeOutcome};
use super::loopback::SessionTokens;
use crate::config::ConfigStore;
use crate::daemon::settings::DaemonSettings;

/// How long a revoke may take in all, exchange included. A sign-out waits for
/// it, and the clear it follows has already happened.
const REVOKE_BUDGET: Duration = Duration::from_secs(10);

/// The one log label a revoke that did not happen writes.
pub(crate) const LABEL_NOT_REVOKED: &str = "near_ai_sign_out_key_not_revoked";

/// What a revoke needs, copied before the clear. Never logged: `Debug`
/// names nothing.
pub(crate) struct Revocation {
    session: Option<(String, String)>,
    /// `(workspace_id, key_id)`, the stored key first.
    keys: Vec<(String, String)>,
    has_active: bool,
}

impl std::fmt::Debug for Revocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Revocation")
            .field("keys", &self.keys.len())
            .finish_non_exhaustive()
    }
}

impl Revocation {
    /// `None` when there is no key to revoke.
    fn capture(current: Option<&DaemonSettings>, store: &ConfigStore) -> Option<Self> {
        let mut keys = Vec::new();
        let mut has_active = false;
        if let Some(key) = current.and_then(|c| c.near_ai_inference.as_ref()) {
            keys.push((key.workspace_id.clone(), key.key_id.clone()));
            has_active = true;
        }
        // An unreadable record names nothing this can act on.
        if let Ok(record) = super::spend_org::load(store) {
            for entry in record.pending_revocations {
                let pair = (entry.workspace_id, entry.key_id);
                if !keys.contains(&pair) {
                    keys.push(pair);
                }
            }
        }
        if keys.is_empty() {
            return None;
        }
        let session = current
            .and_then(|c| c.near_ai_session.as_ref())
            .map(|s| (s.refresh_token.clone(), s.user_agent.clone()));
        Some(Self {
            session,
            keys,
            has_active,
        })
    }

    /// Revoke, best effort. Answers whether the stored key is gone at NEAR
    /// AI, or `None` when no key was stored (only older pending keys were
    /// tidied). Never fails: anything short of a revoke is one label line.
    pub(crate) async fn run(self, api: Option<&CloudApi>) -> Option<bool> {
        let has_active = self.has_active;
        let reason = match tokio::time::timeout(REVOKE_BUDGET, self.attempt(api)).await {
            Ok(Ok(())) => return has_active.then_some(true),
            Ok(Err(reason)) => reason,
            Err(_) => "timed_out",
        };
        tracing::warn!(target: "near_ai_credential", "near_ai_sign_out_key_not_revoked");
        crate::daemon::diagnostics_log::record_labels(
            tracing::Level::WARN,
            LABEL_NOT_REVOKED,
            &[("reason", reason)],
        );
        has_active.then_some(false)
    }

    async fn attempt(self, api: Option<&CloudApi>) -> Result<(), &'static str> {
        let api = api.ok_or("unavailable")?;
        let (refresh_token, user_agent) = self.session.ok_or("no_session")?;
        let access_token = api
            .refresh_session(&refresh_token, &user_agent)
            .await
            .map_err(|_| "session")?
            .session
            .access_token;
        let tokens = SessionTokens {
            access_token,
            refresh_token: String::new(),
        };
        let mut outcome = Ok(());
        for (index, (workspace_id, key_id)) in self.keys.iter().enumerate() {
            let active = self.has_active && index == 0;
            match api
                .revoke_inference_key(&tokens, workspace_id, key_id)
                .await
            {
                Ok(RevokeOutcome::Revoked | RevokeOutcome::AlreadyGone) => {}
                // A pending key from another account: this session never can.
                Ok(RevokeOutcome::NotThisAccount) if !active => {}
                Ok(RevokeOutcome::NotThisAccount) => outcome = Err("not_this_account"),
                Err(_) if active => outcome = Err("refused"),
                Err(_) => {}
            }
        }
        outcome
    }
}

/// Clear the NEAR AI credential, its session and the spending-organization
/// record, and answer what a revoke needs. The caller holds the commit lock.
///
/// `current` is the resolved credential as the caller holds it. It is used
/// only when it is still what disk names: a stale copy revokes nothing.
pub(crate) fn clear_locked(
    store: &ConfigStore,
    current: Option<&DaemonSettings>,
) -> Result<(bool, Option<Revocation>)> {
    let disk = DaemonSettings::load(store)?;
    let current = current.filter(|current| {
        crate::daemon::cloud_credential_lifecycle::ensure_current(&disk, current).is_ok()
    });
    let revocation = Revocation::capture(current, store);
    let removed = super::ceremony::forget_locked(store)?;
    Ok((removed, revocation))
}

/// [`clear_locked`] for a caller with no running daemon: CLI `logout`, which
/// stops the daemon first. The same steps as `ceremony::forget`, which it
/// replaces there, plus the copy a revoke needs.
///
/// The inner result is the OS cleanup. An entry that cannot be deleted is an
/// error there, as in `ceremony::forget`: the reference stays journaled, and
/// logout stops before wiping the journal that names it. The revocation is
/// answered either way, since the local clear has happened.
pub(crate) fn clear_store(store: &ConfigStore) -> Result<(Option<Revocation>, Result<()>)> {
    // Read before the commit lock: an OS read may prompt, and the commit
    // lock covers local publication only.
    let current = resolve_stored(store);
    let locks = super::session::coordination(store.dir())?;
    let commit = locks
        .commit
        .lock()
        .map_err(|_| anyhow::anyhow!("near_ai_credential_unavailable"))?;
    let (_, revocation) = clear_locked(store, current.as_ref())?;
    drop(commit);
    crate::daemon::cloud_credential_lifecycle::sweep_legacy_cloud_entries(store);
    let cleaned = crate::daemon::cloud_credential_lifecycle::cleanup_native(store);
    Ok((revocation, cleaned))
}

/// The stored credential with its secrets, for a caller with no running
/// daemon to ask. May read the OS credential store; never migrates or writes.
pub(crate) fn resolve_stored(store: &ConfigStore) -> Option<DaemonSettings> {
    let settings = DaemonSettings::load(store).ok()?;
    if settings.cloud_credentials.is_none() {
        return Some(settings);
    }
    crate::daemon::cloud_credential_lifecycle::native(store)
        .ok()?
        .resolve(settings)
        .ok()
}

/// The client a revoke uses. Tests install their own per directory; a test
/// build never reaches the live service.
#[cfg(not(test))]
pub(crate) fn api(_store: &ConfigStore) -> Option<CloudApi> {
    CloudApi::live().ok()
}

#[cfg(test)]
pub(crate) fn api(store: &ConfigStore) -> Option<CloudApi> {
    test_api::registry()
        .lock()
        .unwrap()
        .get(store.dir())
        .cloned()
}

#[cfg(test)]
pub(crate) mod test_api {
    use super::*;
    use std::collections::HashMap;
    use std::path::PathBuf;
    use std::sync::{Mutex, OnceLock};

    pub(super) fn registry() -> &'static Mutex<HashMap<PathBuf, CloudApi>> {
        static REGISTRY: OnceLock<Mutex<HashMap<PathBuf, CloudApi>>> = OnceLock::new();
        REGISTRY.get_or_init(Default::default)
    }

    /// Point this directory's revokes at a stub.
    pub(crate) fn install(store: &ConfigStore, origin: &str) {
        registry().lock().unwrap().insert(
            store.dir().to_path_buf(),
            CloudApi::for_test(origin).unwrap(),
        );
    }
}

/// A connected NEAR AI half on disk, a NEAR AI stub that records what it
/// is sent, and the checks every sign-out path's test makes.
#[cfg(test)]
pub(crate) mod test_support {
    use super::*;
    use crate::daemon::credential_store::{CredentialError, CredentialReference};
    use crate::daemon::settings::{NearAiInferenceCredential, NearAiSession};
    use axum::{
        Json, Router,
        extract::Request,
        http::StatusCode,
        routing::{delete, post},
    };
    use std::sync::{Arc, Mutex};

    pub(crate) const KEY: &str = "sk-disconnect-secret-key";
    pub(crate) const REFRESH: &str = "rt_disconnect-secret-session";
    pub(crate) const ACCESS: &str = "at-disconnect-secret-access";

    /// Every request the stub saw: method, path and bearer.
    pub(crate) type Seen = Arc<Mutex<Vec<String>>>;

    /// Store a key and session through the OS-store lifecycle, and a
    /// spending-organization record with one pending revocation. Answers the
    /// keychain reference the bundle went to.
    pub(crate) fn seed(store: &ConfigStore) -> CredentialReference {
        let mut settings = DaemonSettings::load(store).unwrap();
        settings.near_ai_inference = Some(NearAiInferenceCredential {
            key: KEY.into(),
            key_id: "key-1".into(),
            key_prefix: "sk-dis".into(),
            organization_id: "org-1".into(),
            workspace_id: "ws-1".into(),
            minted_at: chrono::Utc::now(),
        });
        settings.near_ai_session = Some(NearAiSession {
            refresh_token: REFRESH.into(),
            refresh_token_expires_at: None,
            stored_at: chrono::Utc::now(),
            user_agent: "Mozilla/5.0 Test".into(),
        });
        settings.save_for_test(store).unwrap();
        super::super::spend_org::record_pending(
            store,
            &[super::super::spend_org::PendingRevocation::new(
                "ws-0", "key-old",
            )],
        )
        .unwrap();
        assert!(
            store
                .daemon_path(super::super::spend_org::SPEND_ORG_FILE)
                .exists()
        );
        let reference = DaemonSettings::load(store)
            .unwrap()
            .cloud_credentials
            .expect("stored in the OS store")
            .reference();
        assert!(
            crate::daemon::cloud_credential_test_support::backend(store.dir())
                .unwrap()
                .read(&reference)
                .is_ok()
        );
        reference
    }

    /// A NEAR AI stub. `refresh` and `revoke` are the statuses its two
    /// routes answer.
    pub(crate) async fn stub(refresh: StatusCode, revoke: StatusCode) -> (String, Seen) {
        let seen: Seen = Arc::default();
        let recorded = Arc::clone(&seen);
        let router = Router::new()
            .route(
                "/v1/users/me/access-tokens",
                post(move || async move {
                    let body = serde_json::json!({
                        "access_token": ACCESS,
                        "refresh_token": "rt_rotated-secret",
                        "refresh_token_expiration": "2030-01-01T00:00:00Z",
                    });
                    (refresh, Json(body))
                }),
            )
            .route(
                "/v1/workspaces/{workspace}/api-keys/{key}",
                delete(move || async move { revoke }),
            )
            .layer(axum::middleware::from_fn(
                move |request: Request, next: axum::middleware::Next| {
                    let seen = Arc::clone(&recorded);
                    async move {
                        let bearer = request
                            .headers()
                            .get("authorization")
                            .and_then(|value| value.to_str().ok())
                            .unwrap_or_default()
                            .to_string();
                        seen.lock().unwrap().push(format!(
                            "{} {} {bearer}",
                            request.method(),
                            request.uri().path()
                        ));
                        next.run(request).await
                    }
                },
            ));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        (format!("http://{address}"), seen)
    }

    /// The revoke went out with the session that was cleared: the old
    /// refresh token bought the access token every delete carried, for the
    /// stored key and the pending one.
    pub(crate) fn assert_revoked_with_old_session(seen: &Seen) {
        let seen = seen.lock().unwrap().clone();
        assert_eq!(
            seen.first().map(String::as_str),
            Some(format!("POST /v1/users/me/access-tokens Bearer {REFRESH}").as_str()),
            "{seen:?}"
        );
        assert!(
            seen.contains(&format!(
                "DELETE /v1/workspaces/ws-1/api-keys/key-1 Bearer {ACCESS}"
            )),
            "{seen:?}"
        );
        assert!(
            seen.contains(&format!(
                "DELETE /v1/workspaces/ws-0/api-keys/key-old Bearer {ACCESS}"
            )),
            "{seen:?}"
        );
    }

    /// Nothing of the NEAR AI half is left: no bundle in the OS store, no
    /// reference in the settings or the cleanup journal, no spending-org
    /// record.
    pub(crate) fn assert_cleared(store: &ConfigStore, reference: &CredentialReference) {
        let settings = DaemonSettings::load(store).unwrap();
        assert!(settings.cloud_credentials.is_none());
        assert!(settings.near_ai_inference.is_none());
        assert!(settings.near_ai_session.is_none());
        assert!(matches!(
            crate::daemon::cloud_credential_test_support::backend(store.dir())
                .unwrap()
                .read(reference),
            Err(CredentialError::NoEntry)
        ));
        let journal = store
            .read_daemon_file("cloud-credential-cleanup.json")
            .unwrap()
            .map(|bytes| serde_json::from_slice::<serde_json::Value>(&bytes).unwrap());
        if let Some(journal) = journal {
            assert_eq!(
                journal["references"].as_array().map(Vec::len).unwrap_or(0),
                0,
                "{journal}"
            );
        }
        assert!(
            !store
                .daemon_path(super::super::spend_org::SPEND_ORG_FILE)
                .exists()
        );
    }
}

#[cfg(test)]
#[path = "disconnect_tests.rs"]
mod tests;
