//! The ceremony itself: one attempt at a time, per config directory.
//!
//! The shape follows `account_onboarding` deliberately rather than inventing
//! a second one -- begin, hand back a browser URL and an attempt id, poll for
//! status, cancel -- because a shell that has learned one browser hand-off
//! should not have to learn a different one for the next.
//!
//! Two things are genuinely different and are the reason this is not that
//! file with the URLs changed:
//!
//! - **The external wallet signs only a login message.** NEAR sign-in verifies
//!   the callback locally and submits the proof to Cloud for authentication.
//!   OAuth providers return a session through the existing loopback callback.
//! - **The result is a credential, not an enrollment.** Nothing here touches
//!   `contributor.json`, the device identity, or the account session. A
//!   contributor may obtain an inference key before enrolling, after
//!   enrolling, or without ever enrolling, and this must not quietly generate
//!   a device key as a side effect of buying inference.

use super::api::CloudApi;
use super::loopback;
use crate::config::ConfigStore;
use crate::daemon::nearai_credential::near_wallet::NearWalletError;
use crate::daemon::nearai_credential::near_wallet_loopback::{self, NearWalletListener};
use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential, NearAiSession};
use anyhow::{Result, anyhow, bail};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// How long a contributor has to finish in the browser before the listener
/// gives up and the port is released.
/// The lifecycle word a finished ceremony reports.
///
/// A const because it crosses a module boundary: `mod.rs` matches on it to
/// decide whether a completed sign-in should cycle the proxy onto the new
/// key. As two literals, a rename here left that match dead and silent.
pub const STATUS_COMPLETE: &str = "complete";

const BROWSER_TIMEOUT: Duration = Duration::from_secs(300);
/// Above this many tracked attempts the registry refuses rather than growing.
const MAX_ATTEMPTS: usize = 128;

#[cfg(test)]
#[path = "ceremony_regression_tests.rs"]
mod regression_tests;

/// One ceremony's public state. Never carries the key, the session, or the
/// browser URL -- the URL is returned once, by `begin`, and is not re-served
/// by a status poll that any local caller can make.
#[derive(Clone, Serialize)]
pub struct Status {
    pub attempt_id: String,
    pub status: &'static str,
}

struct Attempt {
    state: Status,
    abort: Option<tokio::task::AbortHandle>,
}

static ATTEMPTS: OnceLock<Mutex<HashMap<PathBuf, Attempt>>> = OnceLock::new();

fn attempts() -> &'static Mutex<HashMap<PathBuf, Attempt>> {
    ATTEMPTS.get_or_init(Default::default)
}

/// How many times the stored credential has been written or removed for a
/// config directory, since this process started.
///
/// This exists because the ceremony finishes somewhere the daemon cannot be
/// reached from. The browser round trip outlives the IPC call that began it,
/// so the last leg runs as a detached task on [`ceremony_runtime`], holding a
/// path and nothing else -- no `DaemonShared`, no settings lock, no channel.
/// All it can do is write the settings document. A daemon that is already
/// running has its own in-memory copy of that document and would never
/// re-read it, so the write would sit on disk until the next daemon start:
/// the contributor completes a ceremony, is told it succeeded, and nothing
/// they can see changes. This counter is the one bit that crosses that gap --
/// `DaemonShared::absorb_near_ai_credential_change` compares it against the
/// last value it observed and re-reads the credential when it has moved.
///
/// A counter and not a flag, for two reasons. Two ceremonies can complete
/// between two reconcile passes, and a flag cleared by the first reader is a
/// notification the second reader never sees; a counter each consumer
/// compares against its own last value is stolen from by nobody.
///
/// Keyed by directory, like [`ATTEMPTS`], and uncapped where that is bounded:
/// an entry appears only for a directory this process has actually written a
/// credential into, which is one per daemon.
static CHANGES: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();

fn changes() -> &'static Mutex<HashMap<PathBuf, u64>> {
    CHANGES.get_or_init(Default::default)
}

/// Note that this directory's stored credential is not what it was.
///
/// Called only after the settings document has been written, never before: a
/// daemon that reacted to this and then read the old document would cycle its
/// proxy onto the credential it already had, and would not look again.
fn record_change(dir: &std::path::Path) {
    *changes()
        .lock()
        .expect("credential change lock")
        .entry(dir.to_path_buf())
        .or_default() += 1;
}

#[cfg(test)]
pub(crate) fn record_change_for_test(dir: &std::path::Path) {
    record_change(dir);
}

/// How many credential changes this process has made for `dir`.
///
/// Zero for a directory nothing has written, which is the ordinary case: a
/// daemon whose contributor ran no ceremony this run compares zero against
/// zero on every pass and touches the disk not at all.
#[must_use]
pub fn change_count(dir: &std::path::Path) -> u64 {
    changes()
        .lock()
        .expect("credential change lock")
        .get(dir)
        .copied()
        .unwrap_or(0)
}

/// A runtime that outlives the IPC call that started the ceremony.
///
/// The embedded FFI builds a short-lived runtime per call, and the browser
/// round trip -- a contributor typing a password and approving an OAuth
/// consent screen -- routinely outlives it by minutes. The same reasoning as
/// `account_onboarding`, and the same fix.
fn ceremony_runtime() -> Result<&'static tokio::runtime::Runtime> {
    static RUNTIME: OnceLock<Option<tokio::runtime::Runtime>> = OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(1)
                .thread_name("near-ai-credential")
                .enable_all()
                .build()
                .ok()
        })
        .as_ref()
        .ok_or_else(|| anyhow!("near_ai_credential_unavailable"))
}

/// The name this device's key carries in the contributor's own NEAR AI key
/// list, so they can tell which machine minted it and revoke one of several.
///
/// It is unique per attempt because the service answers `409` on a duplicate
/// name within a workspace, and a contributor who re-runs the ceremony after
/// losing a key must not be blocked by the name of the one they lost.
fn key_name(attempt_id: &str) -> String {
    let suffix: String = attempt_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    format!("trace-commons-{suffix}")
}

enum BrowserSignIn {
    OAuth {
        listener: std::net::TcpListener,
        state: String,
    },
    Near(NearWalletListener),
}

impl BrowserSignIn {
    async fn prepare(api: &CloudApi, provider: &str) -> Result<(String, Self)> {
        if provider == "near" {
            let (url, listener) = near_wallet_loopback::bind().await?;
            return Ok((url, Self::Near(listener)));
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let url = api.authorize_url(provider, listener.local_addr()?.port())?;
        Ok((
            url,
            Self::OAuth {
                listener: listener.into_std()?,
                state: loopback::random_state()?,
            },
        ))
    }

    async fn receive(
        self,
        api: &CloudApi,
    ) -> Result<(loopback::SessionTokens, Option<DateTime<Utc>>)> {
        match self {
            Self::OAuth { listener, state } => {
                let listener = tokio::net::TcpListener::from_std(listener)?;
                let session = loopback::receive_session(listener, &state).await?;
                Ok((session, None))
            }
            Self::Near(listener) => {
                let authenticated = api.sign_in_near(listener.wait().await?).await?;
                Ok((
                    authenticated.session,
                    Some(authenticated.refresh_token_expires_at),
                ))
            }
        }
    }
}

/// Begin a ceremony: bind the listener, hand back where to send the browser.
///
/// The listener is bound *before* anything is returned, so the port in the
/// callback URL is one this process already holds. Building the URL first and
/// binding afterwards would leave a window in which another process could take
/// the port and receive the session.
pub async fn begin(store: &ConfigStore, provider: &str) -> Result<serde_json::Value> {
    let api = CloudApi::live()?;
    let attempt_id = loopback::random_state()?;
    let dir = store.dir().to_path_buf();
    let (browser_url, sign_in) = BrowserSignIn::prepare(&api, provider).await?;

    {
        let mut map = attempts().lock().expect("ceremony state lock");
        if map
            .get(&dir)
            .is_some_and(|a| matches!(a.state.status, "starting" | "waiting_for_browser"))
        {
            bail!("near_ai_credential_busy")
        }
        if map.len() > MAX_ATTEMPTS && !map.contains_key(&dir) {
            bail!("near_ai_credential_busy")
        }
        map.insert(
            dir.clone(),
            Attempt {
                state: Status {
                    attempt_id: attempt_id.clone(),
                    status: "waiting_for_browser",
                },
                abort: None,
            },
        );
    }

    // Hand the listener across as a std listener: it must be re-registered on
    // the ceremony runtime's reactor, not the caller's, or a dropped IPC
    // runtime takes the half-finished ceremony down with it.
    let finished_id = attempt_id.clone();
    let finished_dir = dir.clone();
    let task = ceremony_runtime()?.spawn(async move {
        let outcome = async {
            let api = CloudApi::live()?;
            let (session, expires_at) =
                tokio::time::timeout(BROWSER_TIMEOUT, sign_in.receive(&api))
                    .await
                    .map_err(|_| anyhow!("near_ai_credential_expired"))??;
            // A fresh client, on this runtime: never a connection pool
            // attached to the reactor the caller may already have dropped.
            let minted = api
                .mint_inference_key(&session, &key_name(&finished_id))
                .await?;
            // The session is kept, not dropped. Inference does not need it --
            // the key above is non-expiring and self-sufficient -- but the
            // balance does: every management route on cloud-api is
            // session-only. What is retained is the refresh token alone, and
            // `NearAiSession` states what that is authority over.
            let persist_dir = finished_dir.clone();
            let persist_id = finished_id.clone();
            tokio::task::spawn_blocking(move || {
                persist_attempt(
                    &persist_dir,
                    &persist_id,
                    minted,
                    session.refresh_token,
                    expires_at,
                )
            })
            .await
            .map_err(|_| anyhow!("near_ai_credential_unavailable"))?
        }
        .await;
        let mut map = attempts().lock().expect("ceremony state lock");
        if let Some(entry) = map.get_mut(&finished_dir).filter(|a| {
            a.state.attempt_id == finished_id && a.state.status == "waiting_for_browser"
        }) {
            entry.state.status = match outcome {
                Ok(()) => STATUS_COMPLETE,
                Err(error)
                    if error.downcast_ref::<NearWalletError>()
                        == Some(&NearWalletError::Cancelled) =>
                {
                    "cancelled"
                }
                Err(_) => "failed",
            };
            entry.abort = None;
        }
    });
    if let Some(entry) = attempts()
        .lock()
        .expect("ceremony state lock")
        .get_mut(&dir)
        .filter(|a| a.state.attempt_id == attempt_id)
    {
        entry.abort = Some(task.abort_handle());
    }
    Ok(serde_json::json!({
        "attempt_id": attempt_id,
        "status": "waiting_for_browser",
        "browser_url": browser_url,
    }))
}

/// Write the minted key, and the session that reads the balance, into the
/// daemon's own settings.
///
/// Read-modify-write of the whole settings document, which is what every other
/// writer of this file does. It reads from disk rather than from the in-memory
/// copy so a ceremony that finished while the daemon was doing something else
/// cannot silently revert an unrelated setting.
///
/// Both records are written together, and both are overwritten. That is what
/// makes a contributor whose session expired able to simply run the ceremony
/// again: they do not have to forget first, and a stale session cannot survive
/// beside a freshly minted key.
///
/// No expiry is recorded for the refresh token here because the OAuth finish
/// does not supply one -- it arrives in a URL fragment with the access token
/// and nothing else. The first exchange fills it in.
///
/// **Test-only, and deliberately not the path a ceremony takes.** Production
/// writes through [`persist_attempt`], which holds the same locks and then
/// additionally refuses a mint whose attempt is no longer the waiting one --
/// a cancel or a forget that landed while the browser leg was in flight. This
/// entry point omits that check, so it still writes the credential the way the
/// ceremony's final step does, and still exercises the coupling that this
/// function and not an IPC call is what a running daemon has to notice, but it
/// cannot stand in for the guard. A test about the guard must drive `begin` or
/// call `persist_attempt` directly.
#[cfg(test)]
pub(crate) fn persist(
    dir: &std::path::Path,
    minted: super::api::MintedKey,
    refresh_token: String,
) -> Result<()> {
    persist_checked(dir, minted, refresh_token, None, || Ok(()))
}

fn persist_attempt(
    dir: &std::path::Path,
    attempt_id: &str,
    minted: crate::daemon::nearai_credential::api::MintedKey,
    refresh_token: String,
    expires_at: Option<DateTime<Utc>>,
) -> Result<()> {
    persist_checked(dir, minted, refresh_token, expires_at, || {
        let map = attempts()
            .lock()
            .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
        if !map.get(dir).is_some_and(|attempt| {
            attempt.state.attempt_id == attempt_id && attempt.state.status == "waiting_for_browser"
        }) {
            return Err(anyhow!("near_ai_credential_cancelled"));
        }
        Ok(())
    })
}

fn persist_checked(
    dir: &std::path::Path,
    minted: crate::daemon::nearai_credential::api::MintedKey,
    refresh_token: String,
    expires_at: Option<DateTime<Utc>>,
    accept: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let store = ConfigStore::open(dir.to_path_buf())?;
    let expected = DaemonSettings::load(&store)?;
    let inference = Some(NearAiInferenceCredential {
        key: minted.key,
        key_id: minted.key_id,
        key_prefix: minted.key_prefix,
        organization_id: minted.organization_id,
        workspace_id: minted.workspace_id,
        minted_at: Utc::now(),
    });
    let session = Some(NearAiSession {
        refresh_token,
        refresh_token_expires_at: expires_at,
        stored_at: Utc::now(),
    });

    crate::daemon::cloud_credential_lifecycle::native(&store)?.replace(
        &expected,
        inference,
        session,
        accept,
        |_| {
            record_change(dir);
            Ok(())
        },
    )?;
    Ok(())
}

/// This directory's current attempt, if the caller names the right one.
pub fn status(dir: &std::path::Path, attempt_id: Option<&str>) -> Option<Status> {
    let map = attempts().lock().expect("ceremony state lock");
    map.get(dir)
        .filter(|a| attempt_id == Some(a.state.attempt_id.as_str()))
        .map(|a| a.state.clone())
}

/// Whether this directory has an attempt, and how far it got -- without
/// saying which attempt it is.
///
/// [`status`] answers only a caller that can name the attempt, and that
/// guard stays: the attempt id is the value the served page carries back,
/// and the browser URL is handed out once and never re-served.
///
/// This is the other question, and it is not the same one. A shell that was
/// restarted, or a second shell on the same machine, cannot name the attempt
/// and still has to know whether one is in flight -- because the alternative
/// is showing a contributor "no sign-in here" while a browser tab of theirs
/// is waiting, which invites them to start a second ceremony and end up with
/// a second key they did not want and will not find. What this hands back is
/// a lifecycle word and nothing else: it names no attempt, carries no URL,
/// and cannot be used to cancel anything.
pub fn attempt_status(dir: &std::path::Path) -> Option<&'static str> {
    let map = attempts().lock().expect("ceremony state lock");
    map.get(dir).map(|a| a.state.status)
}

/// Abandon an attempt still waiting on the browser, releasing its port.
///
/// `attempt_id` is optional, and the two cases mean different things:
///
/// - `Some(id)` names an attempt, and it must be the one in flight. A caller
///   that names the wrong attempt is refused exactly as before -- it is asking
///   about a ceremony this directory is not running.
/// - `None` cancels whatever this directory has in flight, without naming it.
///
/// **Why an unnamed cancel is allowed.** The id is handed out once, by
/// `begin`, to the caller that started the ceremony. A shell restarted while
/// the daemon kept running -- the ordinary case, not an edge one -- holds no
/// id, and [`attempt_status`] deliberately still tells it a sign-in is in
/// flight. Requiring an id to cancel therefore promised an action the
/// transport could not perform: every shell drew a Cancel control that could
/// not be sent, and had to either lie about why or silently do nothing.
///
/// It is not a widening of what this socket permits. [`forget`] already takes
/// no id at all and does strictly more -- it discards a minted, valid
/// credential -- while this only abandons a sign-in that has not finished. And
/// the existence of an attempt is already served to every caller by
/// `attempt_status`, by design. What stays guarded is the id itself: an
/// unnamed cancel is answered with a lifecycle word alone, never with the
/// attempt id, so this cannot be used to *learn* one.
pub fn cancel(dir: &std::path::Path, attempt_id: Option<&str>) -> Option<Status> {
    let locks = crate::daemon::nearai_credential::session::coordination(dir).ok()?;
    let _commit = locks.commit.lock().ok()?;
    cancel_locked(dir, attempt_id)
}

fn cancel_locked(dir: &std::path::Path, attempt_id: Option<&str>) -> Option<Status> {
    let mut map = attempts().lock().ok()?;
    let entry = map
        .get_mut(dir)
        // `None` matches the attempt in flight; `Some` must name it.
        .filter(|a| attempt_id.is_none_or(|id| id == a.state.attempt_id.as_str()))?;
    if matches!(entry.state.status, "starting" | "waiting_for_browser") {
        entry.state.status = "cancelled";
        if let Some(task) = &entry.abort {
            task.abort();
        }
        entry.abort = None;
    }
    Some(entry.state.clone())
}

/// Forget a stored credential **and the session stored beside it**.
///
/// Both, always, and never one without the other. A contributor who forgets a
/// credential believes they have taken this machine's access to their NEAR AI
/// account away; a refresh token left behind would keep exactly the wider half
/// of that access -- the half that can mint further keys and read the account
/// -- while the screen said the credential was gone. That is the worst outcome
/// this pair of records can produce, so it is one `take` beside the other with
/// no condition between them.
///
/// Local only, and it says so: the key remains valid at the service until the
/// contributor revokes it there, and this does not revoke it for them. It now
/// *could* -- revoking needs a session and a session is retained -- but a
/// forget that also called the service would be a network operation on a path
/// a contributor expects to be instant and offline, and one that fails
/// halfway leaves them unsure which of the two happened. `key_id` is kept
/// precisely so a contributor can find the right key in their own list and
/// revoke it there deliberately.
pub fn forget(store: &ConfigStore) -> Result<bool> {
    let locks = crate::daemon::nearai_credential::session::coordination(store.dir())?;
    let _commit = locks
        .commit
        .lock()
        .map_err(|_| anyhow::anyhow!("near_ai_credential_unavailable"))?;
    let removed = forget_locked(store)?;
    drop(_commit);
    crate::daemon::cloud_credential_lifecycle::cleanup_native(store)?;
    Ok(removed)
}

pub(super) fn forget_locked(store: &ConfigStore) -> Result<bool> {
    cancel_locked(store.dir(), None);
    let had = crate::daemon::cloud_credential_lifecycle::forget_locked(store)?;
    if had {
        // Reconciliation also adopts the session. Only an inference-key
        // change advances the proxy generation there.
        record_change(store.dir());
    }
    Ok(had)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::tests_support::temp_store;
    use crate::daemon::nearai_credential::loopback::CALLBACK_PATH;

    fn minted() -> super::super::api::MintedKey {
        super::super::api::MintedKey {
            key: "sk-minted-secret".into(),
            key_id: "key-1".into(),
            key_prefix: "sk-min".into(),
            organization_id: "org-1".into(),
            workspace_id: "ws-1".into(),
        }
    }

    #[test]
    fn a_minted_key_lands_in_the_daemons_own_slot_and_nowhere_else() {
        let (dir, store) = temp_store();
        // A setting written before the ceremony must survive it: this writes
        // the whole document back, so a read-modify-write that read the wrong
        // copy would revert it.
        let mut settings = DaemonSettings::load_with_cloud_credentials(&store).unwrap();
        settings.max_uploads_per_day = 7;
        settings.save_for_test(&store).unwrap();

        persist(dir.path(), minted(), "rt_session-secret".into()).unwrap();
        let stored = DaemonSettings::load_with_cloud_credentials(&store).unwrap();
        assert_eq!(stored.max_uploads_per_day, 7);
        let credential = stored.near_ai_inference.unwrap();
        assert_eq!(credential.key, "sk-minted-secret");
        assert_eq!(credential.key_id, "key-1");
        assert_eq!(credential.workspace_id, "ws-1");

        // Nothing enrollment-shaped was created. Obtaining inference must not
        // generate a device identity or an enrollment as a side effect.
        assert!(store.load_config().unwrap().is_none());
        assert!(
            crate::identity::DeviceIdentity::load(&store)
                .unwrap()
                .is_none()
        );

        // The session is retained, because the balance is session-only.
        let session = DaemonSettings::load_with_cloud_credentials(&store)
            .unwrap()
            .near_ai_session
            .unwrap();
        assert_eq!(session.refresh_token, "rt_session-secret");
        assert!(
            session.refresh_token_expires_at.is_none(),
            "the OAuth finish supplies no expiry; the first exchange does"
        );

        assert!(forget(&store).unwrap());
        let after = DaemonSettings::load_with_cloud_credentials(&store).unwrap();
        assert!(after.near_ai_inference.is_none());
        // A residual session after a forget would leave the daemon holding
        // the wider of the two credentials while the contributor believed
        // they had revoked both.
        assert!(after.near_ai_session.is_none());
        // Forgetting twice is not an error, and the second says it removed
        // nothing rather than claiming a revocation it did not perform.
        assert!(!forget(&store).unwrap());
    }

    /// A shell that cannot name the attempt can still stop it.
    ///
    /// This is the ordinary case, not an exotic one: the id is handed out once
    /// by `begin`, so a shell restarted while the daemon kept running holds
    /// none -- and `attempt_status` still tells it a sign-in is in flight.
    /// Before this, that state was unactionable, and every shell drew a Cancel
    /// it could not send.
    #[tokio::test]
    async fn a_caller_that_cannot_name_the_attempt_can_still_cancel_it() {
        let (dir, store) = temp_store();
        let started = begin(&store, "github").await.unwrap();
        let attempt_id = started["attempt_id"].as_str().unwrap().to_string();
        assert_eq!(attempt_status(dir.path()), Some("waiting_for_browser"));

        let cancelled = cancel(dir.path(), None).expect("an unnamed cancel reaches the attempt");
        assert_eq!(cancelled.status, "cancelled");
        // The same ceremony, not a new one: cancelling without naming it must
        // not be a way to start over.
        assert_eq!(cancelled.attempt_id, attempt_id);
        assert_eq!(attempt_status(dir.path()), Some("cancelled"));
    }

    /// Naming the wrong attempt is still refused.
    ///
    /// Allowing an unnamed cancel must not degrade into ignoring the id when
    /// one is given: a caller that names an attempt this directory is not
    /// running is asking about something else, and answering it would let a
    /// wrong id read as success.
    #[tokio::test]
    async fn naming_an_attempt_this_directory_is_not_running_is_still_refused() {
        let (dir, store) = temp_store();
        let started = begin(&store, "github").await.unwrap();
        let attempt_id = started["attempt_id"].as_str().unwrap().to_string();

        assert!(cancel(dir.path(), Some("some-other-attempt")).is_none());
        // Untouched by the refusal.
        assert_eq!(attempt_status(dir.path()), Some("waiting_for_browser"));
        assert_eq!(
            cancel(dir.path(), Some(&attempt_id)).map(|s| s.status),
            Some("cancelled")
        );
    }

    /// With nothing in flight, an unnamed cancel finds nothing rather than
    /// inventing something to report.
    #[tokio::test]
    async fn an_unnamed_cancel_with_no_ceremony_in_flight_is_refused() {
        let (dir, _store) = temp_store();
        assert!(cancel(dir.path(), None).is_none());
    }

    #[tokio::test]
    async fn one_ceremony_at_a_time_per_directory_and_a_cancel_frees_it() {
        let (dir, store) = temp_store();
        let started = begin(&store, "github").await.unwrap();
        let attempt_id = started["attempt_id"].as_str().unwrap().to_string();
        assert_eq!(started["status"], "waiting_for_browser");

        // The URL sends the browser at the pinned service and names a
        // loopback callback this process is already listening on.
        let url = reqwest::Url::parse(started["browser_url"].as_str().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("cloud-api.near.ai"));
        let callback = url
            .query_pairs()
            .find(|(k, _)| k == "frontend_callback")
            .unwrap()
            .1
            .into_owned();
        let port: u16 = reqwest::Url::parse(&callback)
            .unwrap()
            .port()
            .expect("an ephemeral port this process holds");
        assert!(
            tokio::net::TcpStream::connect(("127.0.0.1", port))
                .await
                .is_ok(),
            "the listener must be bound before the URL is handed out, or \
             another process can take the port and receive the session"
        );
        assert!(callback.ends_with(CALLBACK_PATH), "{callback}");

        // A second attempt in the same directory is refused rather than
        // evicting the first, whose listener is still holding a port.
        assert_eq!(
            begin(&store, "github").await.unwrap_err().to_string(),
            "near_ai_credential_busy"
        );

        assert_eq!(
            status(dir.path(), Some(&attempt_id)).unwrap().status,
            "waiting_for_browser"
        );
        // A status poll that does not name the attempt gets nothing: the
        // ceremony's existence is not readable by every caller of the socket.
        assert!(status(dir.path(), None).is_none());
        assert!(status(dir.path(), Some("someone-elses")).is_none());

        assert_eq!(
            cancel(dir.path(), Some(&attempt_id)).unwrap().status,
            "cancelled"
        );
        // And now, and only now, another may begin.
        assert!(begin(&store, "github").await.is_ok());
    }

    /// The wallet branch has its own transport; unsupported OAuth names still
    /// fail the authorize_url gate before registering an attempt.
    #[tokio::test]
    async fn an_unknown_provider_is_refused_and_leaves_no_ceremony_behind() {
        let (dir, store) = temp_store();
        assert_eq!(
            begin(&store, "unsupported-provider")
                .await
                .unwrap_err()
                .to_string(),
            "near_ai_credential_provider_unknown"
        );
        assert!(status(dir.path(), None).is_none());
        // A refused attempt must not leave the directory looking busy.
        assert!(begin(&store, "github").await.is_ok());
        cancel(
            dir.path(),
            status(dir.path(), None)
                .map(|s| s.attempt_id)
                .as_deref()
                .or(Some("")),
        );
    }

    #[test]
    fn a_key_name_is_unique_per_attempt_and_safe_in_a_url_path() {
        let first = key_name(&loopback::random_state().unwrap());
        let second = key_name(&loopback::random_state().unwrap());
        assert_ne!(
            first, second,
            "a 409 on a duplicate name would block a retry"
        );
        for name in [first, second, key_name("")] {
            assert!(name.starts_with("trace-commons-"));
            assert!(
                name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "{name}"
            );
        }
    }

    #[tokio::test]
    async fn near_wallet_start_binds_before_open_and_cancel_releases_the_listener() {
        let (dir, store) = temp_store();
        let started = begin(&store, "near").await.unwrap();
        let id = started["attempt_id"].as_str().unwrap();
        let url = reqwest::Url::parse(started["browser_url"].as_str().unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("127.0.0.1"));
        assert_eq!(
            url.path(),
            crate::daemon::nearai_credential::near_wallet::CALLBACK_PATH
        );
        assert!(url.query().is_none());
        assert_eq!(url.fragment().unwrap().len(), 43);
        let address = ("127.0.0.1", url.port().unwrap());
        assert!(tokio::net::TcpListener::bind(address).await.is_err());
        assert!(begin(&store, "github").await.is_err());
        assert!(store.load_config().unwrap().is_none());
        assert_eq!(cancel(dir.path(), Some(id)).unwrap().status, "cancelled");
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if tokio::net::TcpListener::bind(address).await.is_ok() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        assert_eq!(status(dir.path(), Some(id)).unwrap().status, "cancelled");
    }
}
