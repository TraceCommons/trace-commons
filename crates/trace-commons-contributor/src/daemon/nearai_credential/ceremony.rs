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
use super::chooser;
use super::loopback;
use crate::config::ConfigStore;
use crate::daemon::nearai_credential::near_wallet::NearWalletError;
use crate::daemon::nearai_credential::near_wallet_loopback::{self, NearWalletListener};
use crate::daemon::settings::{DaemonSettings, NearAiInferenceCredential, NearAiSession};
use crate::daemon::stored_cloud_credentials::StoredCloudCredentials;
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

#[cfg(test)]
#[path = "ceremony_login_tests.rs"]
mod login_tests;

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

/// A legacy-keychain entry was copied into the current store. Same
/// ordering rule as [`record_change`]: the copy is written before this is
/// called, so a daemon that reacts reads the credential, not its absence.
pub(crate) fn record_change_after_migration(dir: &std::path::Path) {
    record_change(dir);
}

/// A switch of spending organization published a new key. Same ordering rule
/// as [`record_change`]: called only after the settings document is written.
pub(crate) fn record_credential_change(dir: &std::path::Path) {
    record_change(dir);
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

/// Keyed by directory, like [`CHANGES`]. Test-only: production code never
/// needs to ask how many browser URLs it minted, but a regression test that
/// only inspected `begin`'s return value could not tell a guard that refused
/// early from one that refused only after the URL was already handed out.
#[cfg(test)]
static BROWSER_URLS: OnceLock<Mutex<HashMap<PathBuf, u64>>> = OnceLock::new();

#[cfg(test)]
fn record_browser_url_served(dir: &std::path::Path) {
    *BROWSER_URLS
        .get_or_init(Default::default)
        .lock()
        .expect("browser url lock")
        .entry(dir.to_path_buf())
        .or_default() += 1;
}

/// How many browser URLs this process has minted for `dir`. Zero unless
/// `begin` actually reached the point of building one.
#[cfg(test)]
pub(crate) fn browser_urls_served(dir: &std::path::Path) -> u64 {
    BROWSER_URLS
        .get_or_init(Default::default)
        .lock()
        .expect("browser url lock")
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
///
/// The prefix is how NEAR AI finds keys made through this app. It shows in the
/// contributor's own key list, so it must not carry the product name.
pub(super) fn key_name(attempt_id: &str) -> String {
    let suffix: String = attempt_id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    format!("near-ai-app-{suffix}")
}

/// One sign-in's browser leg: a login already known, or the chooser page
/// that picks one.
enum BrowserSignIn {
    Ready(Ready),
    /// No login named: the browser is sent to the chooser page on this
    /// listener, and the login picked there continues on it (see
    /// [`chooser`]). `state` binds the page and the choice; it is in the
    /// page's address, so it is never the OAuth deposit's secret. The login
    /// this device's account was last accepted with is listed first.
    Choose {
        listener: std::net::TcpListener,
        state: String,
        account_login: Option<&'static str>,
    },
}

/// A browser leg for a known login.
enum Ready {
    OAuth {
        listener: std::net::TcpListener,
        /// The deposit's binding: in the bounce page's body only, never in a
        /// URL.
        state: String,
        provider: &'static str,
        /// Another choice from the login page this listener served first.
        stale: Option<chooser::StaleChoice>,
    },
    Near(NearWalletListener),
}

impl Ready {
    fn provider(&self) -> &'static str {
        match self {
            Self::OAuth { provider, .. } => provider,
            Self::Near(_) => "near",
        }
    }

    /// `provider` on `listener`: the wallet page served on it, or an OAuth
    /// login returning to it with a fresh deposit secret. `stale`, after the
    /// login page, recognizes another choice from that page, which ends the
    /// attempt instead of leaving it to time out.
    fn on(
        api: &CloudApi,
        provider: &'static str,
        listener: std::net::TcpListener,
        stale: Option<chooser::StaleChoice>,
    ) -> Result<(String, Self)> {
        if provider == "near" {
            let (url, listener) = near_wallet_loopback::adopt(listener, stale)?;
            return Ok((url, Self::Near(listener)));
        }
        let url = api.authorize_url(provider, listener.local_addr()?.port())?;
        Ok((
            url,
            Self::OAuth {
                listener,
                state: loopback::random_state()?,
                provider,
                stale,
            },
        ))
    }
}

impl BrowserSignIn {
    /// `provider`, when named, is one this build offers ([`offered_provider`]).
    /// `account_login` is read by the caller off this runtime.
    async fn prepare(
        api: &CloudApi,
        provider: Option<&'static str>,
        account_login: Option<&'static str>,
    ) -> Result<(String, Self)> {
        let provider = match (provider, crate::channel::NEAR_AI_SIGN_IN_PROVIDERS) {
            (Some(provider), _) => provider,
            // One login (staging): nothing to choose, so no page.
            (None, &[only]) => only,
            (None, _) => {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
                let state = loopback::random_state()?;
                let url = chooser::page_url(listener.local_addr()?.port(), &state);
                return Ok((
                    url,
                    Self::Choose {
                        listener: listener.into_std()?,
                        state,
                        account_login,
                    },
                ));
            }
        };
        if provider == "near" {
            let (url, listener) = near_wallet_loopback::bind().await?;
            return Ok((url, Self::Ready(Ready::Near(listener))));
        }
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await?
            .into_std()?;
        let (url, ready) = Ready::on(api, provider, listener, None)?;
        Ok((url, Self::Ready(ready)))
    }

    /// Serve the chooser until a login is picked, then send the browser to
    /// that login on the same listener. A known login is returned as is.
    async fn chosen(self, api: &CloudApi) -> Result<Ready> {
        let (listener, state, account_login) = match self {
            Self::Ready(ready) => return Ok(ready),
            Self::Choose {
                listener,
                state,
                account_login,
            } => (listener, state, account_login),
        };
        let listener = tokio::net::TcpListener::from_std(listener)?;
        let choice = chooser::receive_choice(
            &listener,
            &state,
            crate::channel::NEAR_AI_SIGN_IN_PROVIDERS,
            account_login,
        )
        .await?;
        let next = listener
            .into_std()
            .map_err(anyhow::Error::from)
            .and_then(|listener| {
                Ready::on(
                    api,
                    choice.provider,
                    listener,
                    Some(chooser::StaleChoice::new(state)),
                )
            });
        match next {
            Ok((location, ready)) => {
                // A browser never sent on cannot finish: fail now, not when
                // the attempt times out.
                if !choice.redirect(&location).await {
                    bail!("near_ai_credential_unavailable")
                }
                Ok(ready)
            }
            Err(error) => {
                choice.refuse().await;
                Err(error)
            }
        }
    }

    /// The third argument is the agent that *created* the session, which the
    /// service matches on every later refresh. The two arms differ on it and
    /// that difference is the point: the OAuth redirect is completed by the
    /// browser, so the browser's agent is what cloud-api recorded, while the
    /// NEAR wallet sign-in is a request this client makes itself.
    ///
    /// `progress` is set to [`FailedStep::Exchange`] while the wallet proof
    /// is traded for a session, and told the login once it is known, which
    /// `settle` stores with the session and the failure line names.
    async fn finish<F, Fut>(self, api: &CloudApi, progress: &Progress, finish: F) -> Result<()>
    where
        F: FnOnce(loopback::SessionTokens, Option<DateTime<Utc>>, String) -> Fut,
        Fut: std::future::Future<Output = Result<()>>,
    {
        let ready = self.chosen(api).await?;
        progress.set_provider(ready.provider());
        match ready {
            Ready::OAuth {
                listener,
                state,
                stale,
                ..
            } => {
                let listener = tokio::net::TcpListener::from_std(listener)?;
                loopback::receive_session_after(listener, &state, stale.as_ref(), |session| {
                    finish(session.tokens, None, session.user_agent)
                })
                .await
            }
            Ready::Near(listener) => {
                let proof = listener.wait().await?;
                progress.set(FailedStep::Exchange);
                let authenticated = api
                    .sign_in_near(proof)
                    .await
                    .map_err(at(FailedStep::Exchange))?;
                finish(
                    authenticated.session,
                    Some(authenticated.refresh_token_expires_at),
                    crate::daemon::nearai_credential::api::USER_AGENT.to_string(),
                )
                .await
            }
        }
    }
}

/// The step of a sign-in that failed after the browser was sent out, as a
/// fixed label for the daemon's error log. Attached to the error as context,
/// so the ceremony's own handling of the error is unchanged.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub(crate) enum FailedStep {
    /// The ceremony's own client could not be built.
    Setup,
    /// The browser leg: the loopback listener, the deposit, or the wallet's
    /// signed callback. Any failure not tagged with another step.
    Callback,
    /// The NEAR wallet proof traded for a NEAR AI session.
    Exchange,
    /// Listing organizations and workspaces, and minting the inference key.
    Mint,
    /// Writing the key and session to settings and OS credential storage.
    Persist,
    /// No sign-in arrived within the browser timeout.
    Timeout,
    /// The enrolled commons refused the login, or could not be asked.
    Commons,
}

impl FailedStep {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Setup => "setup",
            Self::Callback => "callback",
            Self::Exchange => "exchange",
            Self::Mint => "mint",
            Self::Persist => "persist",
            Self::Timeout => "timeout",
            Self::Commons => "commons",
        }
    }
}

impl std::fmt::Display for FailedStep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// Where a ceremony is, so a timeout names the step it interrupted -- a
/// mint that ran past the deadline is not a sign-in that never arrived --
/// and so a persist the deadline cannot stop is waited for rather than
/// reported as failed. Starts at [`FailedStep::Callback`], the browser wait.
#[derive(Clone)]
pub(crate) struct Progress {
    step: std::sync::Arc<std::sync::atomic::AtomicU8>,
    /// The login this attempt signs in with, once known: named at the start
    /// or picked on the chooser page. Per attempt, never looked up by
    /// directory. Label only.
    provider: std::sync::Arc<std::sync::OnceLock<&'static str>>,
    /// The persist's outcome once it is running: `None` until it ends, then
    /// its root-cause label on failure. A blocking persist outlives the
    /// future that started it.
    persist: std::sync::Arc<std::sync::Mutex<Option<PersistWatch>>>,
}

type PersistWatch = tokio::sync::watch::Receiver<Option<Result<(), String>>>;

impl Progress {
    pub(crate) fn new() -> Self {
        let progress = Self {
            step: Default::default(),
            provider: Default::default(),
            persist: Default::default(),
        };
        progress.set(FailedStep::Callback);
        progress
    }

    /// Record the attempt's login. The first one stands: a leg has one.
    pub(crate) fn set_provider(&self, provider: &'static str) {
        let _ = self.provider.set(provider);
    }

    /// The attempt's login, or `None` before one was picked.
    pub(crate) fn provider(&self) -> Option<&'static str> {
        self.provider.get().copied()
    }

    pub(super) fn set(&self, step: FailedStep) {
        self.step
            .store(step as u8, std::sync::atomic::Ordering::SeqCst);
    }

    fn get(&self) -> FailedStep {
        match self.step.load(std::sync::atomic::Ordering::SeqCst) {
            x if x == FailedStep::Setup as u8 => FailedStep::Setup,
            x if x == FailedStep::Exchange as u8 => FailedStep::Exchange,
            x if x == FailedStep::Mint as u8 => FailedStep::Mint,
            x if x == FailedStep::Persist as u8 => FailedStep::Persist,
            x if x == FailedStep::Timeout as u8 => FailedStep::Timeout,
            x if x == FailedStep::Commons as u8 => FailedStep::Commons,
            _ => FailedStep::Callback,
        }
    }

    /// Hand the running persist's outcome to [`Self::after_deadline`].
    fn persisting(&self) -> tokio::sync::watch::Sender<Option<Result<(), String>>> {
        let (sender, receiver) = tokio::sync::watch::channel(None);
        *self.persist.lock().unwrap_or_else(|e| e.into_inner()) = Some(receiver);
        sender
    }

    /// The error a timeout ends the ceremony with: [`FailedStep::Timeout`]
    /// while the browser had not answered, else the step it interrupted.
    pub(super) fn timed_out(&self) -> anyhow::Error {
        let step = match self.get() {
            FailedStep::Callback => FailedStep::Timeout,
            step => step,
        };
        at(step)(anyhow!("near_ai_credential_expired"))
    }

    /// What the deadline leaves: a persist that was already running is
    /// waited for, so a sign-in that stored its key reads as complete and
    /// one that did not reads as the persist's own failure; anything else
    /// is [`Self::timed_out`].
    pub(super) async fn after_deadline(&self) -> Result<()> {
        let running = self
            .persist
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(mut running) = running else {
            return Err(self.timed_out());
        };
        let ended = running.wait_for(Option::is_some).await.map(|v| v.clone());
        match ended {
            Ok(Some(Ok(()))) => Ok(()),
            Ok(Some(Err(label))) => Err(at(FailedStep::Persist)(anyhow!(label))),
            // The persist ended without a word: its worker was lost.
            _ => Err(at(FailedStep::Persist)(anyhow!(
                "near_ai_credential_unavailable"
            ))),
        }
    }
}

/// Tag an error with the step it came from.
fn at(step: FailedStep) -> impl FnOnce(anyhow::Error) -> anyhow::Error {
    move |error| error.context(step)
}

/// The reason a failed step logs: the error's control name when it is
/// one -- `near_ai_` and label-shaped by the error log's own rule
/// ([`crate::daemon::diagnostics_log::label_value`]), one definition for
/// both -- else `other`. Checked by shape rather than against a copied
/// list, so a label added later is logged as itself; remote text is never
/// shaped like one.
fn failure_reason(root: &str) -> &str {
    let named = root
        .strip_prefix("near_ai_")
        .is_some_and(|rest| !rest.is_empty());
    if named && crate::daemon::diagnostics_log::label_value(root) == root {
        root
    } else {
        "other"
    }
}

/// The event a failed sign-in logs.
const FAILURE_EVENT: &str = "near_ai_sign_in_failed";

/// The label line's fields for a failed sign-in -- step, reason, provider
/// -- or `None` for a cancel, which is not a failure: the wallet's own
/// cancel, a cancel or forget that won the race with the persist
/// (`near_ai_credential_cancelled`), or a second pick on the login page
/// after the first (`chooser::STALE_CHOICE`), which ends the attempt as the
/// person's own doing. The error's text is checked
/// ([`failure_reason`]) and never copied otherwise.
pub(crate) fn failure_fields(
    error: &anyhow::Error,
    provider: &'static str,
) -> Option<[(&'static str, String); 3]> {
    if ended_by_person(error) {
        return None;
    }
    let root = error.root_cause().to_string();
    let step = error
        .downcast_ref::<FailedStep>()
        .copied()
        .unwrap_or(FailedStep::Callback);
    Some([
        ("step", step.label().to_owned()),
        ("reason", failure_reason(&root).to_owned()),
        ("provider", provider.to_owned()),
    ])
}

/// A sign-in that ended by the person's own doing, not a fault: the
/// wallet's own cancel, a cancel or forget that won the race with the
/// persist (`near_ai_credential_cancelled`), or the login page coming back
/// after a pick (`chooser::STALE_CHOICE`, the root of the wallet's
/// `StaleChoice` too). Its status is `cancelled`, as a cancel's is, so the
/// app adds no failure notice to the browser's own sentence, and it writes
/// no failure line. One rule for both, so they cannot drift.
pub(crate) fn ended_by_person(error: &anyhow::Error) -> bool {
    error.downcast_ref::<NearWalletError>() == Some(&NearWalletError::Cancelled)
        || matches!(
            error.root_cause().to_string().as_str(),
            "near_ai_credential_cancelled" | chooser::STALE_CHOICE
        )
}

/// The line [`record_failure`] writes for `error`, without its timestamp.
#[cfg(test)]
pub(super) fn failure_line(error: &anyhow::Error, provider: &'static str) -> Option<String> {
    let fields = failure_fields(error, provider)?;
    Some(crate::daemon::diagnostics_log::labels_line(
        tracing::Level::WARN,
        FAILURE_EVENT,
        &fields.each_ref().map(|(key, value)| (*key, value.as_str())),
    ))
}

/// One label-only line in the daemon's error log (`daemon-errors.log`)
/// naming the step a sign-in failed at. The shells say only that the
/// sign-in did not finish; this says where.
fn record_failure(error: &anyhow::Error, provider: &'static str) {
    if let Some(fields) = failure_fields(error, provider) {
        crate::daemon::diagnostics_log::record_labels(
            tracing::Level::WARN,
            FAILURE_EVENT,
            &fields.each_ref().map(|(key, value)| (*key, value.as_str())),
        );
    }
}

/// `provider` as this build offers it (`channel::NEAR_AI_SIGN_IN_PROVIDERS`),
/// or `None`. A staging build has no NEAR wallet ceremony to begin.
fn offered_provider(provider: &str) -> Option<&'static str> {
    crate::channel::NEAR_AI_SIGN_IN_PROVIDERS
        .iter()
        .find(|offered| **offered == provider)
        .copied()
}

/// What a sign-in does once the browser handed over a session: mint this
/// device's key and store it with the session. `progress` follows each
/// step, so a timeout names the one it interrupted.
pub(super) async fn complete(
    api: &CloudApi,
    dir: &std::path::Path,
    attempt_id: &str,
    session: loopback::SessionTokens,
    expires_at: Option<DateTime<Utc>>,
    user_agent: String,
    progress: &Progress,
) -> Result<()> {
    progress.set(FailedStep::Mint);
    // A remembered spending organization is preferred when the account
    // still has it, so signing in again lands where the contributor chose.
    // The listing is the one the mint already makes: no extra call, and no
    // new way for a sign-in to fail.
    let preferred = ConfigStore::open(dir.to_path_buf())
        .ok()
        .and_then(|store| super::spend_org::chosen_organization(&store));
    let choice = match preferred.as_deref() {
        Some(preferred) => super::api::OrganizationChoice::Prefer(preferred),
        None => super::api::OrganizationChoice::FirstActive,
    };
    let (minted, preferred_listed) = api
        .mint_inference_key_listing(&session, &key_name(attempt_id), choice)
        .await
        .map_err(at(FailedStep::Mint))?;
    // From here on, every way out that does not store the key revokes it.
    let unstored = UnstoredKey::new(api, &session, &minted);
    // A device whose Trace Commons account signs in through NEAR AI asks the
    // commons about this login before it replaces the stored one (owner,
    // 2026-11-02). Beside it, and in the same step, the new login's identity
    // is read once (one `GET /v1/users/me`) to be recorded with the key.
    // Best effort: unknown falls back to the organization listing and to
    // tagging lazily, and never fails the sign-in.
    progress.set(FailedStep::Commons);
    let store = ConfigStore::open(dir.to_path_buf()).map_err(at(FailedStep::Commons))?;
    let (check, new_identity) = tokio::join!(
        crate::daemon::account_session_renewal::check_fresh_login_live(
            &store,
            api,
            &session.access_token,
        ),
        super::spend_org::read_identity(&store, api, &session),
    );
    let identities = SignInIdentities {
        previous: super::spend_org::last_identity(&store),
        new: new_identity,
    };
    settle(
        dir,
        attempt_id,
        minted,
        session,
        expires_at,
        user_agent,
        preferred,
        preferred_listed,
        identities,
        check,
        progress,
        unstored,
    )
    .await
}

/// The NEAR AI identities around a sign-in, as
/// [`super::spend_org::identity_digest`]s: the one recorded for the stored
/// login, and the new login's.
#[derive(Debug, Clone, Default)]
pub(crate) struct SignInIdentities {
    pub(crate) previous: Option<String>,
    pub(crate) new: Option<String>,
}

/// A key minted for a sign-in and not yet stored. Dropped while armed --
/// the commons refused the login, the sign-in was cancelled or ran out of
/// time, or the persist failed -- it revokes the key, so no way out leaves
/// an unused key in the NEAR AI account. Disarmed once the key is stored.
pub(crate) struct UnstoredKey {
    revoke: Option<UnstoredRevoke>,
}

struct UnstoredRevoke {
    api: CloudApi,
    access_token: String,
    workspace_id: String,
    key_id: String,
    runtime: tokio::runtime::Handle,
}

impl UnstoredRevoke {
    async fn run(self) {
        let session = loopback::SessionTokens {
            access_token: self.access_token,
            refresh_token: String::new(),
        };
        if self
            .api
            .revoke_inference_key(&session, &self.workspace_id, &self.key_id)
            .await
            .is_err()
        {
            tracing::warn!(target: "near_ai_credential", "near_ai_unbound_key_not_revoked");
        }
    }
}

impl UnstoredKey {
    pub(crate) fn new(
        api: &CloudApi,
        session: &loopback::SessionTokens,
        minted: &super::api::MintedKey,
    ) -> Self {
        Self {
            revoke: tokio::runtime::Handle::try_current()
                .ok()
                .map(|runtime| UnstoredRevoke {
                    api: api.clone(),
                    access_token: session.access_token.clone(),
                    workspace_id: minted.workspace_id.clone(),
                    key_id: minted.key_id.clone(),
                    runtime,
                }),
        }
    }

    /// The key is stored: nothing to revoke.
    fn stored(mut self) {
        self.revoke = None;
    }

    /// Revoke now, and wait for the answer.
    async fn revoke_now(mut self) {
        if let Some(revoke) = self.revoke.take() {
            revoke.run().await;
        }
    }
}

impl Drop for UnstoredKey {
    fn drop(&mut self) {
        if let Some(revoke) = self.revoke.take() {
            let runtime = revoke.runtime.clone();
            runtime.spawn(revoke.run());
        }
    }
}

/// The attempt word for a login the enrolled commons refused, most often a
/// different NEAR AI account from the one this device joined with. The
/// stored login is kept.
pub const STATUS_DIFFERENT_ACCOUNT: &str = "different_account";

/// The control name [`settle`] fails with for [`STATUS_DIFFERENT_ACCOUNT`].
const DIFFERENT_ACCOUNT: &str = "near_ai_credential_different_account";

/// The control name [`settle`] fails with when the commons could not be
/// asked.
const COMMONS_UNAVAILABLE: &str = "near_ai_credential_commons_unavailable";

/// What a minted login becomes, given what the commons made of it.
///
/// Not bound (a first run, another tenant): stored as it always was.
/// Unchecked: stored, but never recorded as the account's login.
/// Accepted and Verified are the two outcomes where the commons accepted
/// the login for this account, and the only two that record it as the
/// account's login (`login_record`, from the attempt's own `Progress`).
/// Accepted: stored, then the account session it earned is stored too.
/// Verified (signed out): stored, with no account session.
/// Refused or unasked: nothing is stored, the key just minted is revoked
/// where NEAR AI allows, and the stored login stays as it was.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn settle(
    dir: &std::path::Path,
    attempt_id: &str,
    minted: super::api::MintedKey,
    session: loopback::SessionTokens,
    expires_at: Option<DateTime<Utc>>,
    user_agent: String,
    preferred: Option<String>,
    preferred_listed: bool,
    identities: SignInIdentities,
    check: crate::daemon::account_session_renewal::FreshLogin,
    progress: &Progress,
    unstored: UnstoredKey,
) -> Result<()> {
    use crate::daemon::account_session_renewal::{FreshLogin, commit_fresh_login};
    // Refused or unasked: the new key is revoked, never recorded for a
    // later sweep -- that runs with the stored login's session, which is
    // another account's.
    let refused = |label: &'static str| at(FailedStep::Commons)(anyhow!(label));
    // The commons accepted this login for this account without an account
    // session to store: remembered as the account's once it is stored, as an
    // accepted one is.
    let verified = matches!(check, FreshLogin::Verified);
    let accepted = match check {
        // Verified on a signed-out device: stored, with no account session.
        FreshLogin::NotBound | FreshLogin::Verified => None,
        // Stored, never recorded as the account's login (no commit, no
        // record of an accepted login); one label line for now.
        FreshLogin::Unchecked(reason) => {
            crate::daemon::diagnostics_log::record_labels(
                tracing::Level::WARN,
                "near_ai_sign_in_unchecked",
                &[("reason", reason)],
            );
            None
        }
        FreshLogin::Accepted(accepted) => Some(accepted),
        FreshLogin::Refused => {
            unstored.revoke_now().await;
            return Err(refused(DIFFERENT_ACCOUNT));
        }
        FreshLogin::Unavailable => {
            unstored.revoke_now().await;
            return Err(refused(COMMONS_UNAVAILABLE));
        }
    };
    progress.set(FailedStep::Persist);
    let persisted = progress.persisting();
    // The session is kept, not dropped. Inference does not need it -- the
    // key is non-expiring and self-sufficient -- but the balance does:
    // every management route on cloud-api is session-only. What is retained
    // is the refresh token alone, and `NearAiSession` states what that is
    // authority over.
    let persist_dir = dir.to_path_buf();
    let persist_id = attempt_id.to_string();
    let login = progress.provider();
    let minted_key_id = minted.key_id.clone();
    let persist = move || -> Result<()> {
        persist_attempt(
            &persist_dir,
            &persist_id,
            minted,
            session.refresh_token,
            expires_at,
            user_agent,
            login,
        )?;
        unstored.stored();
        // A sign-in to another account forgets what belonged to the old one,
        // once its key is stored.
        if let Ok(store) = ConfigStore::open(persist_dir) {
            super::spend_org::forget_other_account(
                &store,
                preferred.as_deref(),
                preferred_listed,
                identities.previous.as_deref(),
                identities.new.as_deref(),
            );
            // An unread identity leaves the recorded one alone: it names the
            // replaced key, so it can never be taken for this login's.
            if let Some(new) = identities.new.as_deref() {
                super::spend_org::record_identity(&store, &minted_key_id, new);
            }
        }
        Ok(())
    };
    tokio::task::spawn_blocking(move || {
        let outcome = persist();
        let _ = persisted.send(Some(
            outcome
                .as_ref()
                .map(|_| ())
                .map_err(|error| error.root_cause().to_string()),
        ));
        outcome
    })
    .await
    .map_err(|_| anyhow!("near_ai_credential_unavailable"))
    .and_then(|persisted| persisted)
    .map_err(at(FailedStep::Persist))?;
    // Best effort: the NEAR AI login is stored, so the sign-in has landed
    // whatever happens here, and Sign in again renews the account session
    // with it if this write is lost.
    // `None` when nothing was accepted, so nothing was to be committed.
    let committed = match accepted {
        Some(accepted) => Some(match ConfigStore::open(dir.to_path_buf()) {
            Ok(store) => commit_fresh_login(&store, *accepted).await.is_ok(),
            Err(_) => false,
        }),
        None => None,
    };
    if committed == Some(false) {
        tracing::warn!(target: "near_ai_credential", "near_ai_account_session_not_stored");
    }
    // The one place a sign-in records the account's login: the commons
    // accepted this attempt's login (Accepted, its session stored, or
    // Verified). The sign-in page lists it first next time. Label only, off
    // this runtime's one worker.
    if verified || committed == Some(true) {
        if let Ok(store) = ConfigStore::open(dir.to_path_buf()) {
            let _ = tokio::task::spawn_blocking(move || {
                super::login_record::accepted_by_commons(&store, login);
            })
            .await;
        }
    }
    Ok(())
}

/// The control name a settled error failed with, for tests.
#[cfg(test)]
pub(crate) fn different_account_label() -> &'static str {
    DIFFERENT_ACCOUNT
}

/// Register `attempt_id` as this directory's waiting ceremony, as `begin`
/// does, so a test can drive [`complete`] without a browser.
#[cfg(test)]
pub(crate) fn mark_waiting_for_test(dir: &std::path::Path, attempt_id: &str) {
    attempts().lock().expect("ceremony state lock").insert(
        dir.to_path_buf(),
        Attempt {
            state: Status {
                attempt_id: attempt_id.into(),
                status: "waiting_for_browser",
            },
            abort: None,
        },
    );
}

/// Begin a ceremony: bind the listener, hand back where to send the browser.
///
/// The listener is bound *before* anything is returned, so the port in the
/// callback URL is one this process already holds. Building the URL first and
/// binding afterwards would leave a window in which another process could take
/// the port and receive the session.
pub async fn begin(store: &ConfigStore, provider: &str) -> Result<serde_json::Value> {
    // One lookup: the provider as this build lists it, which `prepare` and
    // the failure line both take, so neither carries the caller's text.
    let Some(provider) = offered_provider(provider) else {
        bail!("near_ai_credential_provider_unknown")
    };
    begin_with(store, Some(provider)).await
}

/// Begin a ceremony that names no login: the browser opens the chooser page
/// listing this build's logins (`channel::NEAR_AI_SIGN_IN_PROVIDERS`), or goes
/// straight to the only one when the build offers one.
pub async fn begin_in_browser(store: &ConfigStore) -> Result<serde_json::Value> {
    begin_with(store, None).await
}

/// The failure line's provider while no login has been picked on the
/// chooser page yet.
const PROVIDER_NOT_CHOSEN: &str = "not_chosen";

/// The provider a failure line names: the attempt's login, or
/// [`PROVIDER_NOT_CHOSEN`] before one was picked.
fn failure_provider(progress: &Progress) -> &'static str {
    progress.provider().unwrap_or(PROVIDER_NOT_CHOSEN)
}

async fn begin_with(store: &ConfigStore, named: Option<&'static str>) -> Result<serde_json::Value> {
    // Before anything that reaches the network or a browser: a ceremony this
    // process could not store is a wasted sign-in at the service. The probe
    // is an OS read that can wait on a prompt or a locked store, so it runs
    // on a blocking worker, construction included, never on this runtime.
    // The account's login, for the chooser page, is read on the same worker.
    let probe_store = store.clone();
    let account_login = tokio::task::spawn_blocking(move || {
        crate::daemon::cloud_credential_lifecycle::store_is_reachable(&probe_store)
            .map(|()| super::login_record::commons_login(&probe_store))
    })
    .await
    .map_err(|_| anyhow::anyhow!("near_ai_credential_storage_unavailable"))??;
    let api = CloudApi::live()?;
    let attempt_id = loopback::random_state()?;
    let dir = store.dir().to_path_buf();
    let (browser_url, sign_in) = BrowserSignIn::prepare(&api, named, account_login).await?;
    #[cfg(test)]
    record_browser_url_served(&dir);

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
    let progress = Progress::new();
    if let Some(named) = named {
        progress.set_provider(named);
    }
    let failure_progress = progress.clone();
    let task = ceremony_runtime()?.spawn(async move {
        let outcome = async {
            let api = CloudApi::live().map_err(at(FailedStep::Setup))?;
            let api_ref = &api;
            let finished_id = &finished_id;
            let finished_dir = &finished_dir;
            let progress_ref = &progress;
            let finished = tokio::time::timeout(
                BROWSER_TIMEOUT,
                sign_in.finish(&api, &progress, |session, expires_at, user_agent| {
                    complete(
                        api_ref,
                        finished_dir,
                        finished_id,
                        session,
                        expires_at,
                        user_agent,
                        progress_ref,
                    )
                }),
            )
            .await;
            match finished {
                Ok(finished) => finished,
                // A persist already running is waited for, not abandoned.
                Err(_) => progress.after_deadline().await,
            }
        }
        .await;
        if let Err(error) = &outcome {
            let provider = failure_provider(&failure_progress);
            record_failure(error, provider);
        }
        let mut map = attempts().lock().expect("ceremony state lock");
        if let Some(entry) = map.get_mut(&finished_dir).filter(|a| {
            a.state.attempt_id == finished_id && a.state.status == "waiting_for_browser"
        }) {
            entry.state.status = match outcome {
                Ok(()) => STATUS_COMPLETE,
                Err(error) if ended_by_person(&error) => "cancelled",
                Err(error) if error.root_cause().to_string() == DIFFERENT_ACCOUNT => {
                    STATUS_DIFFERENT_ACCOUNT
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
    user_agent: String,
) -> Result<()> {
    persist_checked(
        dir,
        minted,
        refresh_token,
        None,
        user_agent,
        None,
        || Ok(()),
    )
}

fn persist_attempt(
    dir: &std::path::Path,
    attempt_id: &str,
    minted: crate::daemon::nearai_credential::api::MintedKey,
    refresh_token: String,
    expires_at: Option<DateTime<Utc>>,
    user_agent: String,
    login: Option<&'static str>,
) -> Result<()> {
    persist_checked(
        dir,
        minted,
        refresh_token,
        expires_at,
        user_agent,
        login,
        || {
            let map = attempts()
                .lock()
                .map_err(|_| anyhow!("near_ai_credential_unavailable"))?;
            if !map.get(dir).is_some_and(|attempt| {
                attempt.state.attempt_id == attempt_id
                    && attempt.state.status == "waiting_for_browser"
            }) {
                return Err(anyhow!("near_ai_credential_cancelled"));
            }
            Ok(())
        },
    )
}

fn persist_checked(
    dir: &std::path::Path,
    minted: crate::daemon::nearai_credential::api::MintedKey,
    refresh_token: String,
    expires_at: Option<DateTime<Utc>>,
    user_agent: String,
    login: Option<&'static str>,
    accept: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let store = ConfigStore::open(dir.to_path_buf())?;
    let key_id = minted.key_id.clone();
    let expected = DaemonSettings::load(&store)?;
    let superseded = expected
        .cloud_credentials
        .as_ref()
        .map(StoredCloudCredentials::reference);
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
        user_agent,
    });

    crate::daemon::cloud_credential_lifecycle::native(&store)?.replace(
        &expected,
        inference,
        session,
        accept,
        |_| {
            record_change(dir);
            // Which login this session came from, bound to the key minted
            // with it, in the same commit. Without a login the old record is
            // cleared, so it can never describe this session.
            match login {
                Some(login) => super::login_record::record_stored_login(&store, login, &key_id),
                None => super::login_record::clear_stored_login(&store),
            }
            Ok(())
        },
    )?;
    // The ceremony tail. `superseded` is the reference `expected` pointed at
    // before this replace -- no longer active now, so a contributor who just
    // completed a sign-in is the one person for whom a keychain prompt over
    // it is explicable. See the sweep's doc.
    //
    // This must delete by that captured reference, not by walking the
    // journal the way the forget paths do. `replace`'s own trailing
    // `cleanup_locked` (see cloud_credential_lifecycle.rs) already ran and
    // deleted every non-active journal reference from the data-protection
    // backend before we got here. `CredentialStore::delete` maps a missing
    // entry to success, and `superseded` only ever lived in the legacy
    // keychain, so by the time we reach this line the journal has already
    // been pruned down to just the active reference -- there is nothing left
    // for a journal walk to find. Only the reference captured above, by
    // value, still names the legacy entry. Do not "simplify" this back to
    // `sweep_legacy_cloud_entries`; that call is a no-op here by
    // construction, not by bug.
    if let Some(superseded) = superseded {
        crate::daemon::cloud_credential_lifecycle::sweep_legacy_cloud_reference(&store, superseded);
    }
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
        // `None` matches only an attempt still in flight; `Some` must name
        // the attempt, in flight or finished.
        .filter(|a| match attempt_id {
            Some(id) => id == a.state.attempt_id.as_str(),
            None => matches!(a.state.status, "starting" | "waiting_for_browser"),
        })?;
    if matches!(entry.state.status, "starting" | "waiting_for_browser") {
        entry.state.status = "cancelled";
        if let Some(task) = &entry.abort {
            task.abort();
        }
        entry.abort = None;
    }
    Some(entry.state.clone())
}

/// The local clear without the revoke, for tests that only need the clear.
/// Production clears through `disconnect::clear_store` or the daemon's
/// `disconnect`, which also revoke.
#[cfg(test)]
pub(crate) fn forget(store: &ConfigStore) -> Result<bool> {
    let cleared = super::disconnect::clear_store(store)?;
    cleared.cleaned?;
    Ok(cleared.removed)
}

pub(super) fn forget_locked(store: &ConfigStore) -> Result<bool> {
    cancel_locked(store.dir(), None);
    let had = crate::daemon::cloud_credential_lifecycle::forget_locked(store)?;
    // The spending-organization record goes with the session it needs.
    super::spend_org::clear_locked(store);
    // So does what was known about the login that made the session.
    super::login_record::clear_stored_login(store);
    // And so does what was kept about its credit.
    super::arrivals::clear_locked(store);
    // And the payout org names read with it.
    crate::daemon::payout_link::clear_org_names_locked(store);
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
        settings.max_reuploads = 7;
        settings.save_for_test(&store).unwrap();

        persist(
            dir.path(),
            minted(),
            "rt_session-secret".into(),
            "Mozilla/5.0 Test".into(),
        )
        .unwrap();
        let stored = DaemonSettings::load_with_cloud_credentials(&store).unwrap();
        assert_eq!(stored.max_reuploads, 7);
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
        let started = begin(&store, "google").await.unwrap();
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
        let started = begin(&store, "google").await.unwrap();
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

    /// An attempt that has already finished is not "in flight" either.
    ///
    /// The registry keeps a finished attempt in its directory, so an unnamed
    /// cancel that matched on presence alone would answer a made-up-looking
    /// `cancelled` (or `complete`) for a sign-in that was not running. An
    /// unnamed cancel means "stop what is running"; with nothing running it
    /// finds nothing. A cancel that names the finished attempt keeps its
    /// answer: that caller already holds the id and is asking about it.
    #[tokio::test]
    async fn an_unnamed_cancel_after_the_attempt_finished_finds_nothing() {
        let (dir, store) = temp_store();
        let started = begin(&store, "google").await.unwrap();
        let attempt_id = started["attempt_id"].as_str().unwrap().to_string();
        assert_eq!(
            cancel(dir.path(), Some(&attempt_id)).map(|s| s.status),
            Some("cancelled")
        );

        assert!(
            cancel(dir.path(), None).is_none(),
            "an unnamed cancel answered for a sign-in that was not running"
        );
        // The finished attempt is untouched and still answers its own id.
        assert_eq!(attempt_status(dir.path()), Some("cancelled"));
        assert_eq!(
            cancel(dir.path(), Some(&attempt_id)).map(|s| s.status),
            Some("cancelled")
        );
    }

    #[tokio::test]
    async fn one_ceremony_at_a_time_per_directory_and_a_cancel_frees_it() {
        let (dir, store) = temp_store();
        let started = begin(&store, "google").await.unwrap();
        let attempt_id = started["attempt_id"].as_str().unwrap().to_string();
        assert_eq!(started["status"], "waiting_for_browser");

        // The URL sends the browser at the pinned service and names a
        // loopback callback this process is already listening on.
        let url = reqwest::Url::parse(started["browser_url"].as_str().unwrap()).unwrap();
        let pinned =
            reqwest::Url::parse(crate::daemon::nearai_credential::api::CLOUD_API_ORIGIN).unwrap();
        assert_eq!(url.host_str(), pinned.host_str());
        #[cfg(not(feature = "near-ai-staging"))]
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
            begin(&store, "google").await.unwrap_err().to_string(),
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
        assert!(begin(&store, "google").await.is_ok());
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
        assert!(begin(&store, "google").await.is_ok());
        cancel(
            dir.path(),
            status(dir.path(), None)
                .map(|s| s.attempt_id)
                .as_deref()
                .or(Some("")),
        );
    }

    /// The channel gate sits in the ceremony, not only in the IPC handler,
    /// so no caller in a staging build can begin the NEAR wallet sign-in.
    #[cfg(feature = "near-ai-staging")]
    #[tokio::test]
    async fn a_staging_ceremony_refuses_the_near_wallet_and_github() {
        let (dir, store) = temp_store();
        for provider in ["near", "github"] {
            assert_eq!(
                begin(&store, provider).await.unwrap_err().to_string(),
                "near_ai_credential_provider_unknown"
            );
            assert!(status(dir.path(), None).is_none());
        }
    }

    #[test]
    fn a_key_name_is_unique_per_attempt_and_safe_in_a_url_path() {
        let first = key_name(&loopback::random_state().unwrap());
        let second = key_name(&loopback::random_state().unwrap());
        assert_ne!(
            first, second,
            "a 409 on a duplicate name would block a retry"
        );
        assert_eq!(first.len(), "near-ai-app-".len() + 8);
        for name in [first, second, key_name("")] {
            let suffix = name.strip_prefix("near-ai-app-").expect("prefix");
            assert!(
                suffix.len() <= 8 && suffix.bytes().all(|b| b.is_ascii_alphanumeric()),
                "{name}"
            );
            assert!(name.len() <= 128);
            assert!(!name.to_ascii_lowercase().contains("trace-commons"));
            assert!(
                name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'),
                "{name}"
            );
        }
    }

    /// A staging build begins no NEAR wallet ceremony
    /// (`a_staging_ceremony_refuses_the_near_wallet`).
    #[cfg(not(feature = "near-ai-staging"))]
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
        assert!(begin(&store, "google").await.is_err());
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

    /// `begin`'s reachability probe is an OS read, which can wait on a
    /// keychain prompt or a slow Secret Service. It must run on a blocking
    /// worker, not on the IPC runtime's thread.
    ///
    /// On a current-thread runtime a probe run inline holds the only thread,
    /// so the task spawned here cannot run until the probe returns; the
    /// backend waits for that task and reports whether it ever ran.
    #[tokio::test(flavor = "current_thread")]
    async fn the_reachability_probe_does_not_hold_the_runtime_thread() {
        use crate::daemon::credential_store::{
            CredentialError, CredentialReference, SecretBackend,
        };
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};

        struct WaitingBackend {
            released: Arc<AtomicBool>,
            saw_release: AtomicBool,
        }
        impl SecretBackend for WaitingBackend {
            fn read(&self, _: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                while std::time::Instant::now() < deadline {
                    if self.released.load(Ordering::SeqCst) {
                        self.saw_release.store(true, Ordering::SeqCst);
                        break;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                // Unentitled either way, so `begin` stops before the network.
                Err(CredentialError::Unentitled)
            }
            fn write(&self, _: &CredentialReference, _: &[u8]) -> Result<(), CredentialError> {
                Err(CredentialError::Unavailable)
            }
            fn delete(&self, _: &CredentialReference) -> Result<(), CredentialError> {
                Err(CredentialError::Unavailable)
            }
        }

        let (_dir, store) = temp_store();
        let released = Arc::new(AtomicBool::new(false));
        let backend = Arc::new(WaitingBackend {
            released: released.clone(),
            saw_release: AtomicBool::new(false),
        });
        crate::daemon::cloud_credential_test_support::install_backend(&store, backend.clone());
        let releaser = tokio::spawn(async move { released.store(true, Ordering::SeqCst) });

        let error = begin(&store, "google").await.unwrap_err();
        releaser.await.unwrap();

        assert_eq!(error.to_string(), "near_ai_credential_storage_unentitled");
        assert!(
            backend.saw_release.load(Ordering::SeqCst),
            "the probe ran on the runtime thread and starved every other task"
        );
    }

    /// `persist_checked`'s sweep is a distinct site from the journal-walking
    /// one in `cloud_credential_lifecycle.rs`, and it is macOS-only for the
    /// same reason `sweep_legacy_cloud_entries`'s body is: elsewhere it is a
    /// no-op and a test of it would pass without exercising anything.
    #[cfg(target_os = "macos")]
    mod legacy_sweep_at_the_ceremony_tail {
        use super::*;
        use crate::daemon::cloud_credential_test_support::install_legacy_backend;
        use crate::daemon::credential_store::{
            CredentialError, CredentialReference, SecretBackend,
        };
        use std::sync::{Arc, Mutex};

        #[derive(Default)]
        struct RecordingLegacyBackend(Mutex<Vec<CredentialReference>>);

        impl SecretBackend for RecordingLegacyBackend {
            fn read(&self, _reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
                Err(CredentialError::NoEntry)
            }
            fn write(
                &self,
                _reference: &CredentialReference,
                _bytes: &[u8],
            ) -> Result<(), CredentialError> {
                Err(CredentialError::Unavailable)
            }
            fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
                self.0.lock().unwrap().push(*reference);
                Ok(())
            }
        }

        fn preexisting_credentials() -> DaemonSettings {
            let now = Utc::now();
            DaemonSettings {
                private_inference: true,
                near_ai_inference: Some(NearAiInferenceCredential {
                    key: "sk-legacy-secret".into(),
                    key_id: "legacy-key".into(),
                    key_prefix: "sk-lega".into(),
                    organization_id: "org-legacy".into(),
                    workspace_id: "ws-legacy".into(),
                    minted_at: now,
                }),
                near_ai_session: Some(NearAiSession {
                    refresh_token: "rt-legacy".into(),
                    refresh_token_expires_at: None,
                    stored_at: now,
                    user_agent: "Mozilla/5.0 Test".into(),
                }),
                ..Default::default()
            }
        }

        /// This is the case the whole feature exists for: a contributor's
        /// first re-sign-in after upgrading from a pre-migration build. Their
        /// existing credential lives in the legacy keychain; the ceremony
        /// mints a new one, `replace` publishes it to the data-protection
        /// backend and (via its own trailing `cleanup_locked`) prunes the old
        /// reference out of the journal -- leaving nothing for a journal walk
        /// to find. Only the reference this test captured before the
        /// ceremony, by value, still names the legacy entry, and that is what
        /// `persist_checked` must hand to the legacy backend directly.
        ///
        /// Against the code before this change, `persist_checked` called
        /// `sweep_legacy_cloud_entries` (the journal-walking sweep) here,
        /// which finds nothing to delete once the journal has been pruned to
        /// the active reference -- so this assertion fails red on that code.
        #[test]
        fn a_re_sign_in_sweeps_the_superseded_reference_by_value() {
            let (dir, store) = temp_store();
            let mut settings = preexisting_credentials();
            settings.save_for_test(&store).unwrap();
            let superseded = settings.cloud_credentials.as_ref().unwrap().reference();

            let legacy = Arc::new(RecordingLegacyBackend::default());
            install_legacy_backend(&store, legacy.clone());

            persist(
                dir.path(),
                minted(),
                "rt-new-session-secret".into(),
                "Mozilla/5.0 Test".into(),
            )
            .unwrap();

            assert_eq!(
                *legacy.0.lock().unwrap(),
                vec![superseded],
                "the ceremony tail must sweep the reference it captured \
                 before replace, not rely on the journal replace has already \
                 pruned"
            );
        }

        /// A contributor's very first ceremony: there is no prior credential
        /// to supersede, so `expected.cloud_credentials` is `None` and there
        /// is nothing to sweep. This must not prompt for anything and must
        /// not hand the legacy backend a bogus reference.
        #[test]
        fn a_first_ever_ceremony_sweeps_nothing() {
            let (dir, store) = temp_store();
            let legacy = Arc::new(RecordingLegacyBackend::default());
            install_legacy_backend(&store, legacy.clone());

            persist(
                dir.path(),
                minted(),
                "rt-first-session-secret".into(),
                "Mozilla/5.0 Test".into(),
            )
            .unwrap();

            assert_eq!(
                *legacy.0.lock().unwrap(),
                Vec::new(),
                "there is no superseded reference on a first ceremony"
            );
        }

        /// A legacy backend whose every delete fails with one fixed error.
        struct RefusingLegacyBackend {
            error: CredentialError,
            deletes: Mutex<Vec<CredentialReference>>,
        }

        impl SecretBackend for RefusingLegacyBackend {
            fn read(&self, _reference: &CredentialReference) -> Result<Vec<u8>, CredentialError> {
                Err(CredentialError::NoEntry)
            }
            fn write(
                &self,
                _reference: &CredentialReference,
                _bytes: &[u8],
            ) -> Result<(), CredentialError> {
                Err(CredentialError::Unavailable)
            }
            fn delete(&self, reference: &CredentialReference) -> Result<(), CredentialError> {
                self.deletes.lock().unwrap().push(*reference);
                Err(self.error)
            }
        }

        /// A re-sign-in that happens after the credential already left the
        /// legacy store (migrated, or a second re-sign-in): the legacy
        /// backend answers the sweep's delete with `NoEntry`. And a legacy
        /// store that refuses outright -- the contributor denied the
        /// keychain prompt -- answers `Unavailable`. Neither may fail a
        /// ceremony whose new credential is already published.
        ///
        /// The delete is asserted to have been attempted, so this cannot
        /// pass by never reaching the sweep (as it did with no legacy
        /// backend installed at all).
        #[test]
        fn a_legacy_delete_that_fails_does_not_fail_the_ceremony() {
            for error in [CredentialError::NoEntry, CredentialError::Unavailable] {
                let (dir, store) = temp_store();
                let mut settings = preexisting_credentials();
                settings.save_for_test(&store).unwrap();
                let superseded = settings.cloud_credentials.as_ref().unwrap().reference();

                let legacy = Arc::new(RefusingLegacyBackend {
                    error,
                    deletes: Mutex::default(),
                });
                install_legacy_backend(&store, legacy.clone());

                persist(
                    dir.path(),
                    minted(),
                    "rt-second-session-secret".into(),
                    "Mozilla/5.0 Test".into(),
                )
                .unwrap_or_else(|e| panic!("persist failed on a legacy {error:?}: {e}"));

                assert_eq!(
                    *legacy.deletes.lock().unwrap(),
                    vec![superseded],
                    "the sweep must have reached the legacy backend ({error:?})"
                );
                let stored = DaemonSettings::load_with_cloud_credentials(&store).unwrap();
                assert_eq!(
                    stored.near_ai_session.unwrap().refresh_token,
                    "rt-second-session-secret",
                    "the new credential is published ({error:?})"
                );
            }
        }
    }

    /// A secret-looking string standing in for anything an error could
    /// carry: a remote body, a token, an organization id.
    const SECRET: &str = "rt_SECRET-org_1234-user@example.com";

    fn line(error: &anyhow::Error, provider: &'static str) -> Option<String> {
        failure_line(error, provider)
    }

    /// Each step's failure is logged with its own label and the error's
    /// control name, and nothing of any other error text.
    #[test]
    fn each_failed_step_logs_its_label_and_no_error_text() {
        for (step, label) in [
            (FailedStep::Setup, "setup"),
            (FailedStep::Exchange, "exchange"),
            (FailedStep::Mint, "mint"),
            (FailedStep::Persist, "persist"),
            (FailedStep::Timeout, "timeout"),
        ] {
            assert_eq!(
                line(
                    &at(step)(anyhow!("near_ai_credential_no_organization")),
                    "github"
                )
                .unwrap(),
                format!(
                    "WARN near_ai_sign_in_failed step={label} \
                     reason=near_ai_credential_no_organization provider=github"
                )
            );
            let unknown = line(&at(step)(anyhow!("{SECRET}")), "google").unwrap();
            assert_eq!(
                unknown,
                format!("WARN near_ai_sign_in_failed step={label} reason=other provider=google")
            );
            assert!(!unknown.contains("SECRET") && !unknown.contains("org_1234"));
        }
    }

    /// The reason is any `near_ai_` control name, by shape, so a storage
    /// label from the platform crate is logged as itself; anything not
    /// shaped like one is `other`.
    #[test]
    fn the_reason_is_any_control_name_and_nothing_else() {
        for label in [
            "near_ai_credential_storage_verification_failed",
            "near_ai_credential_storage_binding_mismatch",
            "near_ai_credential_migration_unsupported",
            "near_ai_wallet_already_used",
            "near_ai_key_1234",
        ] {
            assert_eq!(failure_reason(label), label);
        }
        for text in [
            SECRET,
            "near_ai_Credential",
            "near_ai_credential-x",
            "near_ai_",
            "storage_unavailable",
            &format!("near_ai_{}", "a".repeat(64)),
        ] {
            assert_eq!(failure_reason(text), "other", "{text}");
        }
    }

    /// An error no step tagged is the browser leg's. A cancel is not a
    /// failure and logs nothing: the wallet's own, or a cancel or forget
    /// that won the race with the persist.
    #[test]
    fn an_untagged_error_is_the_callback_and_a_cancel_logs_nothing() {
        assert_eq!(
            line(&anyhow!("near_ai_credential_callback_invalid"), "near").unwrap(),
            "WARN near_ai_sign_in_failed step=callback \
             reason=near_ai_credential_callback_invalid provider=near"
        );
        let wallet: anyhow::Error = NearWalletError::Cancelled.into();
        assert_eq!(line(&at(FailedStep::Exchange)(wallet), "near"), None);
        assert_eq!(
            line(
                &at(FailedStep::Persist)(anyhow!("near_ai_credential_cancelled")),
                "github"
            ),
            None
        );
    }

    /// The tag is context only: the ceremony's handling of the error, which
    /// downcasts it and reads nothing else, is unchanged.
    #[test]
    fn a_tagged_error_still_downcasts_to_what_it_wraps() {
        let wallet: anyhow::Error = NearWalletError::Cancelled.into();
        let tagged = at(FailedStep::Exchange)(wallet);
        assert_eq!(
            tagged.downcast_ref::<NearWalletError>(),
            Some(&NearWalletError::Cancelled)
        );
    }

    /// A timeout while the browser had not answered is `timeout`; one that
    /// interrupted a step names that step.
    #[test]
    fn a_timeout_names_the_step_it_interrupted() {
        let progress = Progress::new();
        assert_eq!(
            line(&progress.timed_out(), "near").unwrap(),
            "WARN near_ai_sign_in_failed step=timeout reason=near_ai_credential_expired provider=near"
        );
        for (step, label) in [
            (FailedStep::Exchange, "exchange"),
            (FailedStep::Mint, "mint"),
            (FailedStep::Persist, "persist"),
        ] {
            progress.set(step);
            assert!(
                line(&progress.timed_out(), "near")
                    .unwrap()
                    .contains(&format!("step={label} ")),
                "{label}"
            );
        }
    }

    /// A deadline that fires while the persist runs waits for it: a persist
    /// that stored the key reads as complete, one that failed reads as its
    /// own failure, and a deadline with no persist running is the step it
    /// interrupted.
    #[tokio::test]
    async fn a_deadline_waits_for_a_running_persist() {
        let progress = Progress::new();
        assert_eq!(
            line(&progress.after_deadline().await.unwrap_err(), "near").unwrap(),
            "WARN near_ai_sign_in_failed step=timeout reason=near_ai_credential_expired provider=near"
        );

        let progress = Progress::new();
        progress.set(FailedStep::Persist);
        let persisted = progress.persisting();
        let waiting = tokio::spawn({
            let progress = progress.clone();
            async move { progress.after_deadline().await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished(), "the deadline waits for the persist");
        persisted.send(Some(Ok(()))).unwrap();
        waiting
            .await
            .unwrap()
            .expect("a stored sign-in is complete");

        let progress = Progress::new();
        progress.set(FailedStep::Persist);
        progress
            .persisting()
            .send(Some(Err(
                "near_ai_credential_storage_verification_failed".into()
            )))
            .unwrap();
        assert_eq!(
            line(&progress.after_deadline().await.unwrap_err(), "github").unwrap(),
            "WARN near_ai_sign_in_failed step=persist \
             reason=near_ai_credential_storage_verification_failed provider=github"
        );
    }

    /// A NEAR AI stub for the mint: the organizations listed, and how long
    /// the key mint takes.
    async fn mint_stub(organizations: Vec<(&'static str, bool)>, mint_delay: Duration) -> CloudApi {
        use axum::{
            Json, Router,
            extract::Path,
            routing::{get, post},
        };
        let listing = serde_json::json!({
            "organizations": organizations.iter().map(|(id, active)| serde_json::json!({
                "id": id, "name": "Example org", "role": "owner", "is_active": active,
                "owner_id": "u", "settings": {},
                "created_at": "2026-09-08T00:00:00Z", "updated_at": "2026-09-08T00:00:00Z",
            })).collect::<Vec<_>>(),
            "total": organizations.len(), "limit": 100, "offset": 0,
        });
        let router = Router::new()
            .route(
                "/v1/organizations",
                get(move || {
                    let listing = listing.clone();
                    async move { Json(listing) }
                }),
            )
            .route(
                "/v1/organizations/{org}/workspaces",
                get(|Path(org): Path<String>| async move {
                    Json(serde_json::json!({
                        "workspaces": [{
                            "id": format!("ws-{org}"), "name": "default", "organization_id": org,
                            "created_by_user_id": "u", "is_active": true,
                            "created_at": "2026-09-08T00:00:00Z", "updated_at": "2026-09-08T00:00:00Z",
                        }],
                        "total": 1, "limit": 50, "offset": 0,
                    }))
                }),
            )
            .route(
                "/v1/workspaces/{workspace}/api-keys",
                post(move |Path(workspace): Path<String>| async move {
                    tokio::time::sleep(mint_delay).await;
                    Json(serde_json::json!({
                        "id": format!("key-{workspace}"), "key": "sk-minted-secret",
                        "key_prefix": "sk-min", "workspace_id": workspace,
                        "created_by_user_id": "u", "created_at": "2026-09-08T00:00:00Z",
                        "is_active": true,
                    }))
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        CloudApi::for_test(&origin).unwrap()
    }

    fn browser_session() -> loopback::SessionTokens {
        loopback::SessionTokens {
            access_token: "session-jwt".into(),
            refresh_token: "rt_synthetic_new".into(),
        }
    }

    async fn run_complete(
        api: &CloudApi,
        dir: &std::path::Path,
        progress: &Progress,
    ) -> Result<()> {
        complete(
            api,
            dir,
            "attempt-labels",
            browser_session(),
            None,
            "Mozilla/5.0 Test".into(),
            progress,
        )
        .await
    }

    /// A sign-in whose account has no organization fails at the mint, and
    /// the line says so.
    #[tokio::test]
    async fn an_account_without_an_organization_logs_the_mint() {
        let (_dir, store) = temp_store();
        mark_waiting_for_test(store.dir(), "attempt-labels");
        let api = mint_stub(vec![], Duration::ZERO).await;
        let error = run_complete(&api, store.dir(), &Progress::new())
            .await
            .unwrap_err();
        assert_eq!(
            line(&error, "github").unwrap(),
            "WARN near_ai_sign_in_failed step=mint \
             reason=near_ai_credential_no_organization provider=github"
        );
    }

    /// A cancel that landed while the mint was in flight refuses the
    /// persist, and logs nothing.
    #[tokio::test]
    async fn a_cancel_that_wins_the_race_with_the_persist_logs_nothing() {
        let (_dir, store) = temp_store();
        mark_waiting_for_test(store.dir(), "attempt-labels");
        cancel(store.dir(), Some("attempt-labels"));
        let api = mint_stub(vec![("org-a", true)], Duration::ZERO).await;
        let error = run_complete(&api, store.dir(), &Progress::new())
            .await
            .unwrap_err();
        assert_eq!(
            error.root_cause().to_string(),
            "near_ai_credential_cancelled"
        );
        assert_eq!(line(&error, "github"), None);
    }

    /// A mint that runs past the deadline is logged as the mint, not as a
    /// sign-in that never arrived.
    #[tokio::test]
    async fn a_slow_mint_past_the_deadline_is_logged_as_the_mint() {
        let (_dir, store) = temp_store();
        mark_waiting_for_test(store.dir(), "attempt-labels");
        let api = mint_stub(vec![("org-a", true)], Duration::from_secs(5)).await;
        let progress = Progress::new();
        let outcome = tokio::time::timeout(
            Duration::from_millis(200),
            run_complete(&api, store.dir(), &progress),
        )
        .await;
        assert!(outcome.is_err(), "the deadline fired");
        assert!(
            line(&progress.timed_out(), "google")
                .unwrap()
                .contains("step=mint ")
        );
    }

    /// A completed sign-in stores the key and logs nothing.
    #[tokio::test]
    async fn a_completed_sign_in_stores_the_key() {
        let (_dir, store) = temp_store();
        mark_waiting_for_test(store.dir(), "attempt-labels");
        let api = mint_stub(vec![("org-a", true)], Duration::ZERO).await;
        run_complete(&api, store.dir(), &Progress::new())
            .await
            .unwrap();
        let settings =
            crate::daemon::settings::DaemonSettings::load_with_cloud_credentials(&store).unwrap();
        assert_eq!(settings.near_ai_inference.unwrap().organization_id, "org-a");
    }

    /// The tags `complete` cannot show: the client the ceremony builds, the
    /// wallet exchange, and the timeout and the failure line in `begin`.
    /// Matched on the source with its line endings normalized (a Windows
    /// checkout has CRLF), each within its own function or impl, so no
    /// match can be this test's own text.
    #[test]
    fn begin_and_the_wallet_leg_tag_their_steps() {
        let source = include_str!("ceremony.rs").replace("\r\n", "\n");
        let section = |from: &str, to: &str| {
            source
                .split(from)
                .nth(1)
                .and_then(|rest| rest.split(to).next())
                .unwrap()
                .to_owned()
        };
        let begin = section("pub async fn begin(", "\nfn persist_attempt(");
        for tag in [
            "CloudApi::live().map_err(at(FailedStep::Setup))?",
            "Err(_) => progress.after_deadline().await,",
            "record_failure(error, provider);",
            "complete(",
        ] {
            assert!(begin.contains(tag), "begin lost {tag}");
        }
        // The commons is asked about the login before `settle` stores it,
        // and `settle` stores nothing before the answer (owner, 2026-11-02).
        let complete = section("pub(super) async fn complete(", "\n/// The attempt word");
        let asked = complete.find("check_fresh_login_live(").unwrap();
        assert!(asked < complete.find("settle(").unwrap());
        let settle = section(
            "pub(crate) async fn settle(",
            "\n/// The control name a settled",
        );
        let persist = settle.find("persist_attempt(").unwrap();
        for arm in ["FreshLogin::Refused => {", "FreshLogin::Unavailable => {"] {
            assert!(settle.find(arm).unwrap() < persist, "{arm}");
        }
        // The minted key is guarded from the mint on, and disarmed only once
        // the persist stored it.
        assert!(complete.find("UnstoredKey::new(").unwrap() < asked);
        assert!(settle.find("unstored.stored();").unwrap() > persist);
        let wallet = section("impl BrowserSignIn {", "\n}\n");
        assert!(wallet.contains("progress.set(FailedStep::Exchange);"));
        assert!(wallet.contains(".map_err(at(FailedStep::Exchange))?;"));
    }

    /// The label line lands in the warn/error log with a timestamp, and
    /// carries only label-shaped values.
    #[test]
    fn the_label_line_is_appended_to_the_error_log() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir
            .path()
            .join(crate::daemon::diagnostics_log::DIAGNOSTICS_LOG_FILE);
        let log = crate::daemon::diagnostics_log::DiagnosticsLog::new(path.clone(), 1024 * 1024);
        crate::daemon::diagnostics_log::record_labels_to(
            &log,
            tracing::Level::WARN,
            "near_ai_sign_in_failed",
            &[("step", "mint"), ("reason", "other"), ("provider", SECRET)],
        );
        let written = std::fs::read_to_string(path).unwrap();
        assert!(
            written.ends_with(
                " WARN near_ai_sign_in_failed step=mint reason=other provider=invalid\n"
            ),
            "{written}"
        );
    }
}
