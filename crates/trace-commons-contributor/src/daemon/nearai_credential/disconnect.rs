//! Signing out clears the NEAR AI half of the connected account.
//!
//! Owner decisions (2026-11-02): Private AI does not outlive the account, so
//! CLI `logout`, unenroll, native sign-out and forget all clear the NEAR AI
//! inference key, its refresh session and the spending-organization record;
//! and the inference key is revoked at NEAR AI on the way out, best effort.
//!
//! The order is clear, then revoke. Clearing never waits on NEAR AI: what the
//! revoke needs is copied under the commit lock (the key's ids from the
//! non-secret metadata, the session from the secret when it can be read), the local clear runs through
//! the cleanup journal ([`super::ceremony::forget_locked`]), and only then is
//! the copied session exchanged for an access token and the keys deleted. A
//! failed or impossible revoke is one label line and nothing else: the clear
//! stands. The exchange's rotated refresh token is dropped, never stored --
//! storing it would bring back the session this has just removed.
//!
//! The pending revocations in the spending-organization record are revoked
//! with the same session, since the record goes with the clear and nothing
//! would be left to retry them.

use std::path::PathBuf;
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
    /// The daemon directory, for its refresh lock.
    dir: PathBuf,
    /// Whether disk named a stored key. Read from disk, not from the secret,
    /// so a key whose secret could not be read still counts as stored.
    stored: bool,
    /// The stored key's `(workspace_id, key_id)`, from the non-secret
    /// metadata. `None` with `stored` only for a record that names none.
    active: Option<(String, String)>,
    /// The refresh token and the agent it is bound to. `None` when the
    /// secret could not be read, or no session was stored.
    session: Option<(String, String)>,
    /// Older keys the spending-organization record still meant to revoke.
    pending: Vec<(String, String)>,
}

impl std::fmt::Debug for Revocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Revocation")
            .field("stored", &self.stored)
            .field("pending", &self.pending.len())
            .finish_non_exhaustive()
    }
}

impl Revocation {
    /// `None` when there is nothing to revoke: no stored key and no pending
    /// one.
    ///
    /// `disk` is the settings file as it is now; `current` the resolved
    /// secrets the caller could read, already checked against `disk`.
    fn capture(
        store: &ConfigStore,
        disk: &DaemonSettings,
        current: Option<&DaemonSettings>,
    ) -> Option<Self> {
        let metadata_ids = disk
            .cloud_credentials
            .as_ref()
            .and_then(|metadata| metadata.inference_ids());
        let plaintext_ids = disk
            .near_ai_inference
            .as_ref()
            .map(|key| (key.workspace_id.clone(), key.key_id.clone()));
        let stored = disk
            .cloud_credentials
            .as_ref()
            .is_some_and(|metadata| metadata.has_inference())
            || disk.near_ai_inference.is_some();
        let active = metadata_ids.or(plaintext_ids);
        let mut pending = Vec::new();
        // An unreadable record names nothing this can act on.
        if let Ok(record) = super::spend_org::load(store) {
            for entry in record.pending_revocations {
                let pair = (entry.workspace_id, entry.key_id);
                if active.as_ref() != Some(&pair) && !pending.contains(&pair) {
                    pending.push(pair);
                }
            }
        }
        if !stored && pending.is_empty() {
            return None;
        }
        // A legacy plaintext document carries its own session.
        let session = current
            .and_then(|current| current.near_ai_session.as_ref())
            .or(disk.near_ai_session.as_ref())
            .map(|s| (s.refresh_token.clone(), s.user_agent.clone()));
        Some(Self {
            dir: store.dir().to_path_buf(),
            stored,
            active,
            session,
            pending,
        })
    }

    /// Revoke, best effort. Answers whether the stored key is gone at NEAR
    /// AI, or `None` when no key was stored (only older pending keys were
    /// tidied). A stored key that could not be revoked -- including one whose
    /// secret could not be read -- is `Some(false)`, never `None`. Never
    /// fails: a stored key not revoked is one label line.
    pub(crate) async fn run(self, api: Option<&CloudApi>) -> Option<bool> {
        let stored = self.stored;
        let reason = match tokio::time::timeout(REVOKE_BUDGET, self.attempt(api)).await {
            Ok(Ok(())) => return stored.then_some(true),
            Ok(Err(reason)) => reason,
            Err(_) => "timed_out",
        };
        if stored {
            tracing::warn!(target: "near_ai_credential", "near_ai_sign_out_key_not_revoked");
            crate::daemon::diagnostics_log::record_labels(
                tracing::Level::WARN,
                LABEL_NOT_REVOKED,
                &[("reason", reason)],
            );
        }
        stored.then_some(false)
    }

    async fn attempt(self, api: Option<&CloudApi>) -> Result<(), &'static str> {
        let api = api.ok_or("unavailable")?;
        let (refresh_token, user_agent) = self.session.ok_or("no_session")?;
        // NEAR AI rotates the refresh token on use, so this exchange takes the
        // same per-directory lock as every other one. An exchange that copied
        // the session before the clear and is still in flight finishes first;
        // its rotation then cannot be stored (the session it expected is
        // gone), and this one presents a token NEAR AI no longer accepts and
        // answers `session`. Never two uses of one token at once.
        let access_token = {
            let locks = super::session::coordination(&self.dir).map_err(|_| "unavailable")?;
            let _refresh = locks.refresh_lock().await;
            api.refresh_session(&refresh_token, &user_agent)
                .await
                .map_err(|_| "session")?
                .session
                .access_token
        };
        let tokens = SessionTokens {
            access_token,
            refresh_token: String::new(),
        };
        let mut outcome = if self.stored && self.active.is_none() {
            Err("no_key_ids")
        } else {
            Ok(())
        };
        if let Some((workspace_id, key_id)) = &self.active {
            match api
                .revoke_inference_key(&tokens, workspace_id, key_id)
                .await
            {
                Ok(RevokeOutcome::Revoked | RevokeOutcome::AlreadyGone) => {}
                Ok(RevokeOutcome::NotThisAccount) => outcome = Err("not_this_account"),
                Err(_) => outcome = Err("refused"),
            }
        }
        // Older keys: tidied where this session can, never a reason the
        // stored key's answer changes. Another account's (`403`) never can.
        for (workspace_id, key_id) in &self.pending {
            let _ = api
                .revoke_inference_key(&tokens, workspace_id, key_id)
                .await;
        }
        outcome
    }
}

/// Clear the NEAR AI credential, its session and the spending-organization
/// record, and answer what a revoke needs. The caller holds the commit lock.
///
/// `current` is the resolved credential as the caller holds it. Its session
/// is used only when it is still what disk names. Whether a key was stored,
/// and its ids, come from disk, so a secret that could not be read still
/// answers "stored, not revoked".
pub(crate) fn clear_locked(
    store: &ConfigStore,
    current: Option<&DaemonSettings>,
) -> Result<(bool, Option<Revocation>)> {
    let disk = DaemonSettings::load(store)?;
    let current = current.filter(|current| {
        crate::daemon::cloud_credential_lifecycle::ensure_current(&disk, current).is_ok()
    });
    let revocation = Revocation::capture(store, &disk, current);
    let removed = super::ceremony::forget_locked(store)?;
    Ok((removed, revocation))
}

/// What [`clear_store`] did.
#[derive(Debug)]
pub(crate) struct StoreCleared {
    /// A credential or session was stored and is gone. Read by the tests'
    /// `ceremony::forget`; no shipping caller needs it.
    #[cfg_attr(not(test), allow(dead_code))]
    pub removed: bool,
    /// What a revoke needs, when there is anything to revoke.
    pub revocation: Option<Revocation>,
    /// The OS cleanup. An entry that cannot be deleted is an error here: the
    /// reference stays journaled, and logout stops before wiping the journal
    /// that names it. The local clear has happened either way.
    pub cleaned: Result<()>,
}

/// [`clear_locked`] for a caller with no running daemon (CLI `logout`, which
/// stops the daemon first, and `account logout` when none answers).
///
/// The secret is read before the commit lock: an OS read may prompt, and the
/// commit lock covers local publication only. A read that fails or is
/// declined still clears, and the revoke then answers "stored, not revoked".
///
/// The legacy sweep runs before `cleanup_native`, and the order matters.
/// `clear_locked` has already cleared `cloud_credentials`, so every journal
/// reference is an orphan now, but `cleanup_native` drains the journal as it
/// deletes -- run after it, the sweep finds nothing left to do. This is a
/// contributor-initiated moment, the only kind that may spend an
/// authorization prompt on the legacy store.
pub(crate) fn clear_store(store: &ConfigStore) -> Result<StoreCleared> {
    let current = resolve_stored(store);
    let locks = super::session::coordination(store.dir())?;
    let commit = locks
        .commit
        .lock()
        .map_err(|_| anyhow::anyhow!("near_ai_credential_unavailable"))?;
    let (removed, revocation) = clear_locked(store, current.as_ref())?;
    drop(commit);
    crate::daemon::cloud_credential_lifecycle::sweep_legacy_cloud_entries(store);
    let cleaned = crate::daemon::cloud_credential_lifecycle::cleanup_native(store);
    Ok(StoreCleared {
        removed,
        revocation,
        cleaned,
    })
}

/// The stored credential with its secrets, for a caller with no running
/// daemon to ask. May read the OS credential store; never migrates or writes.
/// `None` when the bundle cannot be read.
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
