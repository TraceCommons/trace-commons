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
//! - **Nothing is signed here.** The enrollment ceremony spends forty lines
//!   proving that the bytes it is about to sign are the bytes the protocol
//!   says, recomputed from a server-issued challenge. There is no challenge in
//!   this flow and nothing to sign; the local binding is the `state` value the
//!   served page carries back, and that is all it is.
//! - **The result is a credential, not an enrollment.** Nothing here touches
//!   `contributor.json`, the device identity, or the account session. A
//!   contributor may obtain an inference key before enrolling, after
//!   enrolling, or without ever enrolling, and this must not quietly generate
//!   a device key as a side effect of buying inference.

use super::api::CloudApi;
use super::loopback;
use crate::config::ConfigStore;
use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential};
use anyhow::{Result, anyhow, bail};
use chrono::Utc;
use serde::Serialize;
use std::{
    collections::HashMap,
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::Duration,
};

/// How long a contributor has to finish in the browser before the listener
/// gives up and the port is released.
const BROWSER_TIMEOUT: Duration = Duration::from_secs(300);
/// Above this many tracked attempts the registry refuses rather than growing.
const MAX_ATTEMPTS: usize = 128;

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

/// Begin a ceremony: bind the listener, hand back where to send the browser.
///
/// The listener is bound *before* anything is returned, so the port in the
/// callback URL is one this process already holds. Building the URL first and
/// binding afterwards would leave a window in which another process could take
/// the port and receive the session.
pub async fn begin(store: &ConfigStore, provider: &str) -> Result<serde_json::Value> {
    let api = CloudApi::live()?;
    let attempt_id = loopback::random_state()?;
    let state = loopback::random_state()?;
    let dir = store.dir().to_path_buf();

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let browser_url = api.authorize_url(provider, listener.local_addr()?.port())?;

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
    let listener = listener.into_std()?;
    let finished_id = attempt_id.clone();
    let finished_dir = dir.clone();
    let task = ceremony_runtime()?.spawn(async move {
        let outcome = async {
            let listener = tokio::net::TcpListener::from_std(listener)?;
            let session =
                tokio::time::timeout(BROWSER_TIMEOUT, loopback::receive_session(listener, &state))
                    .await
                    .map_err(|_| anyhow!("near_ai_credential_expired"))??;
            // A fresh client, on this runtime: never a connection pool
            // attached to the reactor the caller may already have dropped.
            let minted = CloudApi::live()?
                .mint_inference_key(&session, &key_name(&finished_id))
                .await?;
            // The session dies here, unused and unstored. Everything past
            // this point needs only the key.
            drop(session);
            persist(&finished_dir, minted)
        }
        .await;
        let mut map = attempts().lock().expect("ceremony state lock");
        if let Some(entry) = map.get_mut(&finished_dir).filter(|a| {
            a.state.attempt_id == finished_id && a.state.status == "waiting_for_browser"
        }) {
            entry.state.status = if outcome.is_ok() {
                "complete"
            } else {
                "failed"
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

/// Write the minted key into the daemon's own settings.
///
/// Read-modify-write of the whole settings document, which is what every other
/// writer of this file does. It reads from disk rather than from the in-memory
/// copy so a ceremony that finished while the daemon was doing something else
/// cannot silently revert an unrelated setting.
fn persist(dir: &std::path::Path, minted: super::api::MintedKey) -> Result<()> {
    let store = ConfigStore::open(dir.to_path_buf())?;
    let mut settings = DaemonSettings::load(&store)?;
    settings.near_ai_inference = Some(NearAiInferenceCredential {
        key: minted.key,
        key_id: minted.key_id,
        key_prefix: minted.key_prefix,
        organization_id: minted.organization_id,
        workspace_id: minted.workspace_id,
        minted_at: Utc::now(),
    });
    settings.save(&store)
}

/// This directory's current attempt, if the caller names the right one.
pub fn status(dir: &std::path::Path, attempt_id: Option<&str>) -> Option<Status> {
    let map = attempts().lock().expect("ceremony state lock");
    map.get(dir)
        .filter(|a| attempt_id == Some(a.state.attempt_id.as_str()))
        .map(|a| a.state.clone())
}

/// Abandon an attempt still waiting on the browser, releasing its port.
pub fn cancel(dir: &std::path::Path, attempt_id: Option<&str>) -> Option<Status> {
    let mut map = attempts().lock().expect("ceremony state lock");
    let entry = map
        .get_mut(dir)
        .filter(|a| attempt_id == Some(a.state.attempt_id.as_str()))?;
    if matches!(entry.state.status, "starting" | "waiting_for_browser") {
        entry.state.status = "cancelled";
        if let Some(task) = &entry.abort {
            task.abort();
        }
        entry.abort = None;
    }
    Some(entry.state.clone())
}

/// Forget a stored credential.
///
/// Local only, and it says so: the key remains valid at the service until the
/// contributor revokes it there, and this cannot revoke it for them, because
/// revoking needs a session and the session was discarded when the key was
/// minted. `key_id` is kept precisely so a contributor can find the right key
/// in their own list.
pub fn forget(store: &ConfigStore) -> Result<bool> {
    let mut settings = DaemonSettings::load(store)?;
    let had = settings.near_ai_inference.take().is_some();
    settings.save(store)?;
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
        let mut settings = DaemonSettings::load(&store).unwrap();
        settings.max_uploads_per_day = 7;
        settings.save(&store).unwrap();

        persist(dir.path(), minted()).unwrap();
        let stored = DaemonSettings::load(&store).unwrap();
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

        assert!(forget(&store).unwrap());
        assert!(
            DaemonSettings::load(&store)
                .unwrap()
                .near_ai_inference
                .is_none()
        );
        // Forgetting twice is not an error, and the second says it removed
        // nothing rather than claiming a revocation it did not perform.
        assert!(!forget(&store).unwrap());
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

    /// The provider is gated in one place, `authorize_url`, which is where
    /// the whole callback contract is checked. `begin` deliberately does not
    /// re-check it: a second copy of the rule was there, it refused with the
    /// same label at the same moment, and no test could tell the two apart --
    /// which is exactly the kind of control that rots unnoticed. The cost of
    /// the single gate is that a listener is bound and immediately dropped on
    /// the refusal path.
    #[tokio::test]
    async fn an_unknown_provider_is_refused_and_leaves_no_ceremony_behind() {
        let (dir, store) = temp_store();
        assert_eq!(
            begin(&store, "near").await.unwrap_err().to_string(),
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
}
