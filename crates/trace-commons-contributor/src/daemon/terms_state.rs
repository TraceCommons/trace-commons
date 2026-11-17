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

use std::sync::atomic::Ordering;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use trace_commons_protocol::terms::{
    TERMS_ACCEPTANCE_PATH, TermsAcceptanceRequest, TermsAcceptanceResponse,
};

use super::ipc::{
    DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, EVENT_STATUS_CHANGED, Request, Response,
};
use crate::config::ConfigStore;

/// The state file's name in the state directory.
pub const TERMS_STATE_FILE: &str = "terms-state.json";

/// `accept` named a version that is no longer the published one.
pub const TERMS_VERSION_STALE: &str = "terms-version-stale";
/// `terms-state.json` could not be written; nothing changed.
pub const TERMS_STATE_WRITE_FAILED: &str = "terms-state-write-failed";
/// `terms_accept` params were not `{version, content_sha256, accepted_at}`.
pub const TERMS_ACCEPT_PARAMS_INVALID: &str = "terms-accept-params-invalid";

/// How long a failed acceptance post waits before the next try, for the
/// same tenant and acceptance. A tenant change or a new acceptance tries at
/// once.
const TERMS_POST_RETRY_SECS: i64 = 15 * 60;

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

/// How often the daemon re-reads the published document after start.
pub(crate) const TERMS_RECHECK_SECS: i64 = 6 * 60 * 60;

/// When the published document is fetched again: due at once, then
/// [`TERMS_RECHECK_SECS`] after each attempt, successful or not.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TermsSchedule {
    next_at: Option<DateTime<Utc>>,
}

impl TermsSchedule {
    /// Due on the first tick.
    pub(crate) fn starting() -> Self {
        Self { next_at: None }
    }

    pub(crate) fn due(&self, now: DateTime<Utc>) -> bool {
        self.next_at.is_none_or(|at| now >= at)
    }

    pub(crate) fn attempted(&mut self, now: DateTime<Utc>) {
        self.next_at = Some(now + chrono::Duration::seconds(TERMS_RECHECK_SECS));
    }
}

/// The daemon's terms pass: re-check the published version when due, then
/// post an owed acceptance.
pub(crate) async fn tick(shared: &DaemonShared, now: DateTime<Utc>, schedule: &mut TermsSchedule) {
    if schedule.due(now) {
        schedule.attempted(now);
        let _ = refresh_terms(shared, now).await;
    }
    let _ = post_pending(shared, now).await;
}

/// Fetch the published document and record its version. A failed fetch
/// logs its label and changes nothing: it never starts a hold and never
/// lifts one.
pub(crate) async fn refresh_terms(shared: &DaemonShared, now: DateTime<Utc>) -> Result<()> {
    let fetched = fetch_for_daemon(shared).await;
    apply_fetch(shared, now, fetched);
    Ok(())
}

#[cfg(not(test))]
async fn fetch_for_daemon(
    _shared: &DaemonShared,
) -> Result<crate::terms::FetchedTerms, &'static str> {
    crate::terms::fetch_published().await
}

/// Tests never reach the live document: they set `terms_fetch_override`.
#[cfg(test)]
async fn fetch_for_daemon(
    shared: &DaemonShared,
) -> Result<crate::terms::FetchedTerms, &'static str> {
    shared
        .terms_fetch_override
        .lock()
        .expect("terms override lock")
        .clone()
        .unwrap_or(Err(crate::terms::TERMS_UNAVAILABLE))
}

/// Record a fetch's outcome. On success the published version, hash and
/// `in_effect` replace the saved ones, `terms_held` follows, and
/// `status_changed` is published when the hold or the owed version moved.
pub(crate) fn apply_fetch(
    shared: &DaemonShared,
    now: DateTime<Utc>,
    fetched: Result<crate::terms::FetchedTerms, &'static str>,
) {
    let fetched = match fetched {
        Ok(fetched) => fetched,
        Err(label) => {
            tracing::warn!(reason = label, "terms not checked");
            return;
        }
    };
    let owed_before = shared.terms.lock().expect("terms lock").version_owed();
    let published = PublishedTerms {
        version: fetched.document.version,
        content_sha256: fetched.content_sha256,
        in_effect: fetched.document.in_effect,
        checked_at: now,
    };
    match store_and_note(shared, |state| {
        state.published = Some(published);
        Ok(())
    }) {
        Ok(((), held_moved)) => {
            let owed_after = shared.terms.lock().expect("terms lock").version_owed();
            if held_moved || owed_before != owed_after {
                shared.publish(EVENT_STATUS_CHANGED, serde_json::json!({}));
            }
        }
        Err(label) => tracing::warn!(reason = label, "terms not recorded"),
    }
}

/// A failed post, and when the same post may be tried again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PostRetry {
    tenant_id: String,
    accepted: AcceptedTerms,
    not_before: DateTime<Utc>,
}

/// Apply `change` to a copy of the daemon's terms state, save the copy, and
/// only then make it the state in force, so a failed write changes nothing.
/// Keeps `terms_held` in step. Returns the change's own result and whether
/// the hold moved; publishing `status_changed` is the caller's call.
pub(crate) fn store_and_note<R>(
    shared: &DaemonShared,
    change: impl FnOnce(&mut TermsState) -> Result<R, &'static str>,
) -> Result<(R, bool), &'static str> {
    let (result, held_now) = {
        let mut guard = shared.terms.lock().expect("terms lock");
        let mut next = guard.clone();
        let result = change(&mut next)?;
        next.save(&shared.store)
            .map_err(|_| TERMS_STATE_WRITE_FAILED)?;
        *guard = next;
        (result, guard.held())
    };
    let before = shared.terms_held.swap(held_now, Ordering::SeqCst);
    Ok((result, before != held_now))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcceptParams {
    version: String,
    content_sha256: String,
    accepted_at: DateTime<Utc>,
}

/// IPC `terms_accept` as the socket answers it: when the version named is
/// not the cached published one, the site is checked again first, then
/// [`handle_accept`] decides. The cached version only moves on the six-hourly
/// re-check, while the app reads the live document, so without this a
/// version published between two re-checks would be refused as stale until
/// the next one. A failed re-check changes nothing, and the refusal stands.
/// No re-check in a developer dry run.
pub(crate) async fn handle_accept_async(shared: &DaemonShared, req: &Request) -> Response {
    let named = serde_json::from_value::<AcceptParams>(req.params.clone())
        .ok()
        .filter(|params| {
            TermsAcceptanceRequest {
                terms_version: params.version.clone(),
                content_sha256: params.content_sha256.clone(),
                accepted_at: params.accepted_at,
            }
            .validate(Utc::now())
            .is_ok()
        })
        .map(|params| params.version);
    let lags = {
        let state = shared.terms.lock().expect("terms lock");
        matches!((&state.published, &named), (Some(published), Some(version)) if &published.version != version)
    };
    // A developer dry run reaches no network, as the tick's own re-check
    // does not: the cached version decides.
    if lags && !shared.dev_dry_run {
        let _ = refresh_terms(shared, Utc::now()).await;
    }
    handle_accept(shared, req)
}

/// IPC `terms_accept {version, content_sha256, accepted_at}`: replace the
/// acceptance with what the app rendered and lift the hold. Answers
/// `{"held": bool}`. Refused with `terms-version-stale` when another version
/// is published now, and with the protocol's labels for a malformed version,
/// hash or a time in the future. Never touches the contributor's pause. The
/// post to each org is the tick's (`post_pending`).
pub(crate) fn handle_accept(shared: &DaemonShared, req: &Request) -> Response {
    let Ok(params) = serde_json::from_value::<AcceptParams>(req.params.clone()) else {
        return Response::err(req.id, ERR_BAD_PARAMS, TERMS_ACCEPT_PARAMS_INVALID);
    };
    let request = TermsAcceptanceRequest {
        terms_version: params.version,
        content_sha256: params.content_sha256,
        accepted_at: params.accepted_at,
    };
    if let Err(label) = request.validate(Utc::now()) {
        return Response::err(req.id, ERR_BAD_PARAMS, label);
    }
    let accepted = AcceptedTerms {
        version: request.terms_version,
        content_sha256: request.content_sha256,
        accepted_at: request.accepted_at,
    };
    let lifted = match store_and_note(shared, |state| state.accept(accepted)) {
        Ok(((), moved)) => moved,
        Err(TERMS_VERSION_STALE) => {
            return Response::err(req.id, ERR_BAD_PARAMS, TERMS_VERSION_STALE);
        }
        Err(label) => return Response::err(req.id, ERR_UNAVAILABLE, label),
    };
    // Uploads resume at once rather than at the next poll: the ordinary
    // pass, with every other gate (the contributor's pause among them).
    if lifted {
        shared.request_upload_at(Utc::now());
    }
    // A new acceptance is posted at the next tick, not after an old backoff.
    *shared.terms_post_retry.lock().expect("terms retry lock") = None;
    shared.publish(EVENT_STATUS_CHANGED, serde_json::json!({}));
    let held = shared.terms_held.load(Ordering::SeqCst);
    Response::ok(req.id, serde_json::json!({ "held": held }))
}

/// What one [`post_pending`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PostOutcome {
    /// No config, or no tenant in it.
    NoTenant,
    /// Nothing accepted, or this tenant already confirmed it.
    NothingOwed,
    /// The same post failed recently; its backoff has not run out.
    Waiting,
    /// No account session: a quiet no-op until there is one.
    NoSession,
    /// Posted and confirmed for this tenant.
    Recorded,
    /// Posted, but the acceptance was replaced meanwhile; the new one posts
    /// on a later tick.
    Superseded,
    /// Posted and refused or unanswered, with a fixed label.
    Failed(&'static str),
}

/// Post the acceptance to ingest for the configured tenant when that tenant
/// has not confirmed it yet. A missing session is a quiet no-op; any other
/// failure logs its label only and is tried again after a backoff.
pub(crate) async fn post_pending(shared: &DaemonShared, now: DateTime<Utc>) -> Result<()> {
    match post_pending_outcome(shared, now).await {
        PostOutcome::Failed(label) => {
            tracing::warn!(reason = label, "terms acceptance not recorded");
        }
        PostOutcome::Recorded => tracing::info!("terms acceptance recorded"),
        _ => {}
    }
    Ok(())
}

pub(crate) async fn post_pending_outcome(shared: &DaemonShared, now: DateTime<Utc>) -> PostOutcome {
    let Ok(Some(cfg)) = shared.store.load_config() else {
        return PostOutcome::NoTenant;
    };
    let tenant = cfg.tenant_id.clone();
    if tenant.is_empty() {
        return PostOutcome::NoTenant;
    }
    // Snapshot, then release the lock before any await.
    let accepted = {
        let terms = shared.terms.lock().expect("terms lock");
        if !terms.needs_post(&tenant) {
            return PostOutcome::NothingOwed;
        }
        match terms.accepted.clone() {
            Some(accepted) => accepted,
            None => return PostOutcome::NothingOwed,
        }
    };
    if shared
        .terms_post_retry
        .lock()
        .expect("terms retry lock")
        .as_ref()
        .is_some_and(|retry| {
            retry.tenant_id == tenant && retry.accepted == accepted && now < retry.not_before
        })
    {
        return PostOutcome::Waiting;
    }
    let body = TermsAcceptanceRequest {
        terms_version: accepted.version.clone(),
        content_sha256: accepted.content_sha256.clone(),
        accepted_at: accepted.accepted_at,
    };
    let reply = crate::account_contribution::call::<_, TermsAcceptanceResponse>(
        &shared.store,
        &cfg,
        Method::POST,
        TERMS_ACCEPTANCE_PATH,
        Some(&body),
        None,
    )
    .await;
    let retry_later = |shared: &DaemonShared| {
        *shared.terms_post_retry.lock().expect("terms retry lock") = Some(PostRetry {
            tenant_id: tenant.clone(),
            accepted: accepted.clone(),
            not_before: now + chrono::Duration::seconds(TERMS_POST_RETRY_SECS),
        });
    };
    match reply {
        Ok(response) => {
            // Mark only the acceptance that was posted: one replaced while
            // the post was in flight has not been sent anywhere yet.
            let marked = store_and_note(shared, |state| {
                if state.accepted.as_ref() == Some(&accepted) {
                    state.mark_recorded(&tenant, response.recorded_at);
                    Ok(true)
                } else {
                    Ok(false)
                }
            });
            match marked {
                Ok((true, _)) => {
                    *shared.terms_post_retry.lock().expect("terms retry lock") = None;
                    PostOutcome::Recorded
                }
                Ok((false, _)) => PostOutcome::Superseded,
                Err(label) => {
                    retry_later(shared);
                    PostOutcome::Failed(label)
                }
            }
        }
        Err(error) => {
            // `call`'s errors are fixed labels; anything else is reduced to
            // one, since an `anyhow` chain can carry a path.
            let label = match error.to_string().as_str() {
                "account-sign-in-required" => return PostOutcome::NoSession,
                "account-session-changed" => "account-session-changed",
                "account-contribution-refused" => "account-contribution-refused",
                "account-contribution-unavailable" => "account-contribution-unavailable",
                _ => "terms-post-failed",
            };
            retry_later(shared);
            PostOutcome::Failed(label)
        }
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

/// The daemon side: IPC, status and the post to ingest.
#[cfg(test)]
mod daemon_tests {
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    };

    use axum::{Json, Router, routing::post};

    use super::*;
    use crate::account_auth;
    use crate::daemon::cloud_credential_test_support::MemoryBackend;
    use crate::daemon::commons_credentials;
    use crate::daemon::ipc::{handle_request, handle_request_async};

    fn hash(c: char) -> String {
        std::iter::repeat_n(c, 64).collect()
    }

    fn fixture() -> (tempfile::TempDir, DaemonShared) {
        let (dir, store) = crate::config::tests_support::temp_store();
        commons_credentials::install_test_backend(&store, Arc::new(MemoryBackend::default()));
        let shared = DaemonShared::load(store).unwrap();
        (dir, shared)
    }

    fn publish(shared: &DaemonShared, version: &str, in_effect: bool) {
        store_and_note(shared, |state| {
            state.published = Some(PublishedTerms {
                version: version.into(),
                content_sha256: hash('b'),
                in_effect,
                checked_at: Utc::now(),
            });
            Ok(())
        })
        .unwrap();
    }

    fn req(method: &str, params: serde_json::Value) -> Request {
        Request {
            id: 7,
            method: method.into(),
            params,
        }
    }

    fn accept_params(version: &str) -> serde_json::Value {
        serde_json::json!({
            "version": version,
            "content_sha256": hash('b'),
            "accepted_at": Utc::now(),
        })
    }

    fn drain(rx: &mut tokio::sync::broadcast::Receiver<crate::daemon::ipc::Event>) -> Vec<String> {
        let mut names = Vec::new();
        while let Ok(event) = rx.try_recv() {
            names.push(event.event);
        }
        names
    }

    #[test]
    fn accept_lifts_the_hold_and_publishes_status() {
        let (_dir, shared) = fixture();
        publish(&shared, "v2", true);
        assert!(shared.terms_held.load(Ordering::SeqCst));
        let mut rx = shared.events.subscribe();
        let reply = handle_request(&shared, &req("terms_accept", accept_params("v2")));
        assert!(reply.error.is_none(), "{:?}", reply.error);
        assert_eq!(reply.result.unwrap()["held"], false);
        assert!(!shared.terms_held.load(Ordering::SeqCst));
        assert!(drain(&mut rx).iter().any(|e| e == EVENT_STATUS_CHANGED));
        // Persisted, so a restart keeps it.
        let saved = TermsState::load(&shared.store).unwrap();
        assert_eq!(saved.accepted.as_ref().unwrap().version, "v2");
        assert!(!saved.held());
    }

    #[test]
    fn accept_refuses_a_version_that_is_no_longer_published() {
        let (_dir, shared) = fixture();
        publish(&shared, "v-b", true);
        let reply = handle_request(&shared, &req("terms_accept", accept_params("v-a")));
        let error = reply.error.expect("a stale version is refused");
        assert_eq!(error.code, ERR_BAD_PARAMS);
        assert_eq!(error.message, TERMS_VERSION_STALE);
        assert!(shared.terms_held.load(Ordering::SeqCst), "the hold stays");
        assert!(shared.terms.lock().unwrap().accepted.is_none());
    }

    fn live(shared: &DaemonShared, fetched: Result<&str, &'static str>) {
        *shared.terms_fetch_override.lock().unwrap() =
            Some(fetched.map(|version| crate::terms::FetchedTerms {
                document: trace_commons_protocol::terms::TermsDocument {
                    schema_version: trace_commons_protocol::terms::TERMS_SCHEMA_VERSION.into(),
                    version: version.into(),
                    effective: "2026-12-01".into(),
                    in_effect: true,
                    title: "Terms".into(),
                    blocks: vec![trace_commons_protocol::terms::TermsBlock::Paragraph {
                        text: "Terms text.".into(),
                    }],
                },
                content_sha256: hash('b'),
            }));
    }

    /// The daemon's cached published version lags the site: the person
    /// read the newer one, which is current. The daemon checks the site
    /// again before refusing, and accepts.
    #[tokio::test]
    async fn accept_of_a_version_newer_than_the_cached_one_rechecks_and_succeeds() {
        let (_dir, shared) = fixture();
        publish(&shared, "v-b", true);
        live(&shared, Ok("v-c"));
        let reply = handle_request_async(&shared, &req("terms_accept", accept_params("v-c"))).await;
        assert!(reply.error.is_none(), "{:?}", reply.error);
        assert_eq!(reply.result.unwrap()["held"], false);
        assert!(!shared.terms_held.load(Ordering::SeqCst));
        let saved = TermsState::load(&shared.store).unwrap();
        assert_eq!(saved.accepted.as_ref().unwrap().version, "v-c");
        assert_eq!(saved.published.as_ref().unwrap().version, "v-c");
    }

    /// The re-check confirms the cached version: the person read an older
    /// one, and is refused.
    #[tokio::test]
    async fn accept_still_refused_when_the_recheck_confirms_another_version() {
        let (_dir, shared) = fixture();
        publish(&shared, "v-b", true);
        live(&shared, Ok("v-b"));
        let reply = handle_request_async(&shared, &req("terms_accept", accept_params("v-a"))).await;
        assert_eq!(reply.error.expect("refused").message, TERMS_VERSION_STALE);
        assert!(shared.terms_held.load(Ordering::SeqCst));
        assert!(shared.terms.lock().unwrap().accepted.is_none());
    }

    /// A failed re-check changes nothing and the refusal stands: a version
    /// the daemon cannot confirm is never recorded.
    #[tokio::test]
    async fn accept_refused_when_the_recheck_fails() {
        let (_dir, shared) = fixture();
        publish(&shared, "v-b", true);
        live(&shared, Err(crate::terms::TERMS_UNAVAILABLE));
        let reply = handle_request_async(&shared, &req("terms_accept", accept_params("v-c"))).await;
        assert_eq!(reply.error.expect("refused").message, TERMS_VERSION_STALE);
        assert!(shared.terms_held.load(Ordering::SeqCst));
        assert_eq!(
            shared
                .terms
                .lock()
                .unwrap()
                .published
                .as_ref()
                .unwrap()
                .version,
            "v-b"
        );
    }

    /// A developer dry run reaches no network (`DEV_DRY_RUN_LOCAL_METHODS`
    /// lists `terms_accept` as local-only), so there is no re-check and the
    /// cached version decides.
    #[tokio::test]
    async fn accept_does_not_recheck_in_a_developer_dry_run() {
        let (_dir, mut shared) = fixture();
        shared.dev_dry_run = true;
        publish(&shared, "v-b", true);
        live(&shared, Ok("v-c"));
        let reply = handle_request_async(&shared, &req("terms_accept", accept_params("v-c"))).await;
        assert_eq!(reply.error.expect("refused").message, TERMS_VERSION_STALE);
        let published = shared.terms.lock().unwrap().published.clone().unwrap();
        assert_eq!(published.version, "v-b", "nothing was fetched");
    }

    #[test]
    fn accept_refuses_bad_params_with_protocol_labels() {
        let (_dir, shared) = fixture();
        let mut params = accept_params("v1");
        params["content_sha256"] = serde_json::json!(hash('B'));
        let reply = handle_request(&shared, &req("terms_accept", params));
        assert_eq!(reply.error.unwrap().message, "terms-hash-invalid");
        let mut params = accept_params("v1");
        params["accepted_at"] = serde_json::json!(Utc::now() + chrono::Duration::minutes(6));
        let reply = handle_request(&shared, &req("terms_accept", params));
        assert_eq!(reply.error.unwrap().message, "terms-accepted-at-future");
        let mut params = accept_params("v1");
        params["extra"] = serde_json::json!(true);
        let reply = handle_request(&shared, &req("terms_accept", params));
        assert_eq!(reply.error.unwrap().message, TERMS_ACCEPT_PARAMS_INVALID);
    }

    #[test]
    fn accept_does_not_resume_a_user_pause() {
        let (_dir, shared) = fixture();
        publish(&shared, "v2", true);
        let paused = handle_request(&shared, &req("pause", serde_json::json!({})));
        assert!(paused.error.is_none(), "{:?}", paused.error);
        let reply = handle_request(&shared, &req("terms_accept", accept_params("v2")));
        assert!(reply.error.is_none(), "{:?}", reply.error);
        assert!(shared.paused.load(Ordering::SeqCst), "still paused");
        assert_eq!(shared.status_value()["paused"], true);
        assert_eq!(shared.status_value()["terms_hold"], false);
    }

    #[test]
    fn resume_does_not_lift_the_terms_hold() {
        let (_dir, shared) = fixture();
        publish(&shared, "v2", true);
        let paused = handle_request(&shared, &req("pause", serde_json::json!({})));
        assert!(paused.error.is_none());
        let resumed = handle_request(&shared, &req("resume", serde_json::json!({})));
        assert!(resumed.error.is_none());
        assert!(!shared.paused.load(Ordering::SeqCst));
        assert!(shared.terms_held.load(Ordering::SeqCst));
        assert_eq!(shared.status_value()["terms_hold"], true);
    }

    #[test]
    fn status_reports_terms_hold_and_owed_version() {
        let (_dir, shared) = fixture();
        let status = shared.status_value();
        assert_eq!(status["terms_hold"], false);
        assert!(status["terms_version_owed"].is_null());
        publish(&shared, "2026-12-01", true);
        let status = shared.status_value();
        assert_eq!(status["terms_hold"], true);
        assert_eq!(status["terms_version_owed"], "2026-12-01");
        // A draft is never owed.
        publish(&shared, "2026-12-02-draft", false);
        let status = shared.status_value();
        assert_eq!(status["terms_hold"], false);
        assert!(status["terms_version_owed"].is_null());
    }

    #[test]
    fn terms_accept_is_advertised_and_allowed_in_a_dry_run() {
        assert!(crate::daemon::ipc::METHODS.contains(&"terms_accept"));
        assert!(crate::daemon::ipc::DEV_DRY_RUN_LOCAL_METHODS.contains(&"terms_accept"));
    }

    /// A fake ingest that counts terms posts and remembers each body.
    struct FakeIngest {
        url: String,
        posts: Arc<AtomicUsize>,
        bodies: Arc<Mutex<Vec<serde_json::Value>>>,
    }

    async fn fake_ingest() -> FakeIngest {
        let posts = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let (count, seen) = (Arc::clone(&posts), Arc::clone(&bodies));
        let app = Router::new().route(
            TERMS_ACCEPTANCE_PATH,
            post(move |Json(body): Json<serde_json::Value>| {
                count.fetch_add(1, Ordering::SeqCst);
                seen.lock().unwrap().push(body);
                async { Json(serde_json::json!({ "recorded_at": "2026-11-09T12:00:00Z" })) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        FakeIngest {
            url: format!("http://{address}"),
            posts,
            bodies,
        }
    }

    fn enroll(shared: &DaemonShared, ingest_url: &str, tenant: &str) {
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = ingest_url.into();
        cfg.tenant_id = tenant.into();
        shared.store.save_config(&cfg).unwrap();
        // Rebind the session to the config just saved, as
        // `account_contribution`'s rotation test does after a change.
        account_auth::clear_token(&shared.store).unwrap();
        publish_native_session(&shared.store, "account-a");
    }

    /// As `account_contribution`'s tests do: a session bound to the
    /// config saved just before.
    fn publish_native_session(store: &ConfigStore, account: &str) {
        let session = account_auth::AccountSession {
            access_token: "synthetic-native-token".into(),
            expires_at: Utc::now() + chrono::TimeDelta::hours(6),
            account_id: account.into(),
        };
        let expected =
            commons_credentials::snapshot(store, commons_credentials::Kind::Account).unwrap();
        commons_credentials::replace(
            store,
            &expected,
            &serde_json::to_vec(&session).unwrap(),
            None,
        )
        .unwrap();
    }

    #[tokio::test]
    async fn posts_once_per_tenant() {
        let (_dir, shared) = fixture();
        let ingest = fake_ingest().await;
        enroll(&shared, &ingest.url, "tenant-a");
        let reply = handle_request_async(&shared, &req("terms_accept", accept_params("v1"))).await;
        assert!(reply.error.is_none(), "{:?}", reply.error);
        assert_eq!(
            post_pending_outcome(&shared, Utc::now()).await,
            PostOutcome::Recorded
        );
        assert_eq!(
            post_pending_outcome(&shared, Utc::now()).await,
            PostOutcome::NothingOwed
        );
        assert_eq!(ingest.posts.load(Ordering::SeqCst), 1);
        let body = ingest.bodies.lock().unwrap()[0].clone();
        assert_eq!(body["terms_version"], "v1");
        assert_eq!(body["content_sha256"], hash('b'));
        let saved = TermsState::load(&shared.store).unwrap();
        assert_eq!(saved.recorded_in.len(), 1);
        assert_eq!(saved.recorded_in[0].tenant_id, "tenant-a");
    }

    #[tokio::test]
    async fn posts_again_when_the_tenant_changes() {
        let (_dir, shared) = fixture();
        let ingest = fake_ingest().await;
        enroll(&shared, &ingest.url, "tenant-a");
        handle_request(&shared, &req("terms_accept", accept_params("v1")));
        assert_eq!(
            post_pending_outcome(&shared, Utc::now()).await,
            PostOutcome::Recorded
        );
        // Signed in to another org.
        enroll(&shared, &ingest.url, "tenant-b");
        assert_eq!(
            post_pending_outcome(&shared, Utc::now()).await,
            PostOutcome::Recorded
        );
        assert_eq!(ingest.posts.load(Ordering::SeqCst), 2);
        let tenants: Vec<String> = shared
            .terms
            .lock()
            .unwrap()
            .recorded_in
            .iter()
            .map(|r| r.tenant_id.clone())
            .collect();
        assert_eq!(tenants, ["tenant-a", "tenant-b"]);
    }

    #[tokio::test]
    async fn no_session_is_a_quiet_no_op() {
        let (_dir, shared) = fixture();
        let ingest = fake_ingest().await;
        let mut cfg = crate::commands::unenrolled_preview_config();
        cfg.ingest_url = ingest.url.clone();
        cfg.tenant_id = "tenant-a".into();
        shared.store.save_config(&cfg).unwrap();
        handle_request(&shared, &req("terms_accept", accept_params("v1")));
        assert_eq!(
            post_pending_outcome(&shared, Utc::now()).await,
            PostOutcome::NoSession
        );
        assert_eq!(ingest.posts.load(Ordering::SeqCst), 0);
        // Not treated as a failure: no backoff was recorded, so a sign-in
        // posts on the very next tick.
        assert!(shared.terms_post_retry.lock().unwrap().is_none());
        assert!(shared.terms.lock().unwrap().needs_post("tenant-a"));
    }

    #[tokio::test]
    async fn a_failed_post_backs_off_and_keeps_the_tenant_unrecorded() {
        let (_dir, shared) = fixture();
        // Bound and dropped: nothing listens here.
        let closed = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}", closed.local_addr().unwrap());
        drop(closed);
        enroll(&shared, &url, "tenant-a");
        handle_request(&shared, &req("terms_accept", accept_params("v1")));
        let now = Utc::now();
        assert!(matches!(
            post_pending_outcome(&shared, now).await,
            PostOutcome::Failed(_)
        ));
        assert_eq!(
            post_pending_outcome(&shared, now + chrono::Duration::seconds(60)).await,
            PostOutcome::Waiting
        );
        assert!(shared.terms.lock().unwrap().needs_post("tenant-a"));
    }
}
