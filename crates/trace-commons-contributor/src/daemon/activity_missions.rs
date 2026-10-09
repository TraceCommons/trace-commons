//! Read-only trace-activity catalogue and account progress, with rewards off.
//!
//! Catalogue requests are anonymous and independent of local matching. Status
//! uses only the native account session; every rotation is retained, including
//! on refusals. Neither read changes contribution consent or upload state.

use std::{collections::BTreeSet, time::Duration};

use chrono::{DateTime, Datelike, NaiveDate, Utc};
use serde::de::DeserializeOwned;
use trace_commons_protocol::{
    ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER,
    activity_missions::{
        ActivityCatalogue, ActivityProgress, MAX_PREDICATE_TITLE_CHARS, MAX_PREDICATE_VALUES,
    },
};

use super::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use super::withdraw::{ERR_ACCOUNT_SESSION_REQUIRED, account_session};
use crate::config::{ContributorConfig, allowlist_for, ingest_origin_url};

const MAX_RESPONSE_BYTES: usize = 128 * 1024;
const UNAVAILABLE: &str = "activity-missions-unavailable";
const INVALID: &str = "activity-missions-params-invalid";
const CREDENTIAL_UNAVAILABLE: &str = "commons_credential_storage_unavailable";

struct Call<T> {
    result: Result<T, ()>,
    rotated_token: Option<String>,
}

fn empty_params(req: &Request) -> bool {
    req.params
        .as_object()
        .is_some_and(|object| object.is_empty())
}

pub(super) async fn handle_catalogue(shared: &DaemonShared, req: &Request) -> Response {
    if !empty_params(req) {
        return Response::err(req.id, ERR_BAD_PARAMS, INVALID);
    }
    let Ok(Some(config)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
    };
    let call = fetch::<ActivityCatalogue>(&config, "/v1/activity-missions", None).await;
    match call.result {
        Ok(catalogue) if valid_catalogue(&catalogue) => Response::ok(
            req.id,
            serde_json::json!({
                "catalogue":catalogue,
                "disclosure":crate::consent_copy::ACTIVITY_MISSIONS_DISCLOSURE,
            }),
        ),
        _ => Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE),
    }
}

pub(super) async fn handle_status(shared: &DaemonShared, req: &Request) -> Response {
    if !empty_params(req) {
        return Response::err(req.id, ERR_BAD_PARAMS, INVALID);
    }
    let Ok(Some(config)) = shared.store.load_config() else {
        return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
    };
    let session = match account_session(shared) {
        Ok(Some(session)) => session,
        Ok(None) => return Response::err(req.id, ERR_UNAVAILABLE, ERR_ACCOUNT_SESSION_REQUIRED),
        Err(_) => return Response::err(req.id, ERR_UNAVAILABLE, CREDENTIAL_UNAVAILABLE),
    };
    // Bind the credential to the exact configured authority before sending it.
    // Config and account-session reads can race with identity changes.
    if !same_account_snapshot(shared, &config, &session) {
        return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
    }
    let call = fetch::<ActivityProgress>(
        &config,
        "/v1/account/activity-missions/status",
        Some(&session.session.access_token),
    )
    .await;
    // The account middleware rotates on an error response too. Retain it
    // before classifying HTTP, parsing or domain-validation outcomes.
    let rotated = call.rotated_token.is_some();
    if super::public_run::persist_rotated_token(shared, &session, call.rotated_token).is_err() {
        // A rotation is refused, not lost, when the account was replaced in
        // flight: that is the account changing, not the store failing.
        if !original_account_still_current(shared, &config, &session, rotated) {
            return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
        }
        return Response::err(req.id, ERR_UNAVAILABLE, CREDENTIAL_UNAVAILABLE);
    }
    // A read without a rotation still needs to notice sign-out or identity
    // replacement while the HTTP request was in flight.
    if !original_account_still_current(shared, &config, &session, rotated) {
        return Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE);
    }
    match call.result {
        Ok(status) if valid_progress(&status) => Response::ok(
            req.id,
            serde_json::json!({
                "status":status,
                "disclosure":crate::consent_copy::ACTIVITY_MISSIONS_DISCLOSURE,
            }),
        ),
        _ => Response::err(req.id, ERR_UNAVAILABLE, UNAVAILABLE),
    }
}

fn same_account_snapshot(
    shared: &DaemonShared,
    config: &ContributorConfig,
    session: &crate::account_auth::LoadedAccountSession,
) -> bool {
    super::commons_credentials::account_snapshot(&shared.store, config)
        .is_ok_and(|snapshot| snapshot == session.snapshot)
}

fn original_account_still_current(
    shared: &DaemonShared,
    config: &ContributorConfig,
    original: &crate::account_auth::LoadedAccountSession,
    rotated: bool,
) -> bool {
    let Ok(Some(current)) = account_session(shared) else {
        return false;
    };
    // Keep the original context; a reload is validation, never adoption. A
    // browser sign-in can replace the account without first clearing it.
    current.session.account_id == original.session.account_id
        && current.session.expires_at == original.session.expires_at
        && same_account_snapshot(shared, config, &current)
        && if rotated {
            current.snapshot.same_lifecycle(&original.snapshot)
        } else {
            current.snapshot == original.snapshot
        }
}

fn valid_label(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
}

// Validate the known semantic contract independently of additive response
// metadata, which deserialization discards. The policy itself stays strict.
fn valid_catalogue(catalogue: &ActivityCatalogue) -> bool {
    if catalogue.schema_version != 1
        || catalogue.kind != "trace_activity"
        || catalogue.rewards_enabled
        || catalogue.credit_points_pending.is_some()
        || catalogue.credit_condition != "mission_credit_ledger_unavailable"
    {
        return false;
    }
    match &catalogue.policy {
        Some(policy) => {
            catalogue.state == "configured"
                && policy
                    .digest()
                    .is_ok_and(|digest| catalogue.policy_sha256.as_deref() == Some(digest.as_str()))
        }
        None => catalogue.state == "unconfigured" && catalogue.policy_sha256.is_none(),
    }
}

fn valid_progress(status: &ActivityProgress) -> bool {
    let today = status.observed_at.date_naive();
    let elapsed_days = (today - status.coverage_starts_on).num_days();
    if status.schema_version != 1
        || status.kind != "trace_activity"
        || status.source != "account_contributed_submissions"
        || status.rewards_enabled
        || status.credit_points_pending.is_some()
        || status.credit_condition != "mission_credit_ledger_unavailable"
        || status.policy_sha256.len() != 64
        || !status
            .policy_sha256
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        || !(0..366).contains(&elapsed_days)
        || today.with_day(1) != Some(status.month_starts_on)
        || status
            .level
            .as_deref()
            .is_some_and(|level| !valid_label(level))
        || (!status.levels_configured && status.level.is_some())
    {
        return false;
    }
    match (&status.daily, status.completed_days, status.current_streak) {
        (None, None, None) => {}
        (Some(daily), Some(completed), Some(streak))
            if daily.day == today
                && valid_label(&daily.mission_id)
                && daily.required_contributions > 0
                && daily.complete
                    == (daily.contributions >= u64::from(daily.required_contributions))
                && status.monthly_contributions >= daily.contributions
                && u64::from(completed) <= (elapsed_days + 1) as u64
                && streak <= completed
                && (!daily.complete || (completed > 0 && streak > 0)) => {}
        _ => return false,
    }
    if let Some(badges) = &status.badges {
        let mut ids = BTreeSet::new();
        if badges.is_empty()
            || badges.len() > 128
            || badges
                .iter()
                .any(|badge| !valid_label(&badge.id) || !ids.insert(&badge.id))
        {
            return false;
        }
    }
    true
}

async fn fetch<T: DeserializeOwned>(
    config: &ContributorConfig,
    path: &str,
    token: Option<&str>,
) -> Call<T> {
    let failed = || Call {
        result: Err(()),
        rotated_token: None,
    };
    let Ok(url) = ingest_origin_url(&config.ingest_url, path) else {
        return failed();
    };
    let literal_loopback = match url.host() {
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        _ => false,
    };
    if !(url.scheme() == "https" || (url.scheme() == "http" && literal_loopback))
        || allowlist_for(config.allowed_hosts.as_deref())
            .check(&url)
            .is_err()
    {
        return failed();
    }
    let Ok(client) = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(20))
        .build()
    else {
        return failed();
    };
    let request = client.get(url);
    let request = match token {
        Some(token) => request.bearer_auth(token),
        None => request,
    };
    let Ok(mut response) = request.send().await else {
        return failed();
    };
    let rotated_token = token.and_then(|_| {
        response
            .headers()
            .get(ACCOUNT_NATIVE_ROTATED_TOKEN_HEADER)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned)
    });
    if rotated_token
        .as_ref()
        .is_some_and(|token| token.len() > 4096)
    {
        return failed();
    }
    let refused = || Call {
        result: Err(()),
        rotated_token: rotated_token.clone(),
    };
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|length| length > MAX_RESPONSE_BYTES as u64)
    {
        return refused();
    }
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if bytes.len().saturating_add(chunk.len()) <= MAX_RESPONSE_BYTES => {
                bytes.extend_from_slice(&chunk)
            }
            Ok(None) => break,
            _ => return refused(),
        }
    }
    Call {
        result: serde_json::from_slice(&bytes).map_err(|_| ()),
        rotated_token,
    }
}

// ---- the contribution-mission slot ----

// Whatever a published version-1 predicate may carry, the client catalogue
// accepts, so a policy the server publishes cannot make `receive_catalogue`
// refuse (and empty) the slot.
const _: () = assert!(MAX_PREDICATE_VALUES <= crate::contribution_missions::MAX_CRITERION_VALUES);
const _: () = assert!(MAX_PREDICATE_TITLE_CHARS <= crate::contribution_missions::MAX_TEXT_CHARS);

/// How often the published activity catalogue is fetched for the
/// contribution-mission slot: four times inside
/// [`super::mission_matching::MISSION_CATALOGUE_MAX_AGE`], so one or two
/// failed fetches do not let a live slot age out.
pub(crate) const MISSION_SLOT_REFRESH: Duration = Duration::from_secs(6 * 60 * 60);

/// When the slot's next fetch is due. A local of the daemon's tick loop.
///
/// Its inputs are the clock and whether a config exists, nothing about the
/// queue or local activity, so when it runs says nothing about this Mac's
/// work. Due at once on start; after an attempt, whatever its outcome, not
/// again for [`MISSION_SLOT_REFRESH`], so an unreachable server is not asked
/// on every tick. Without a config it stays due, and a schedule kept for one
/// enrollment does not carry over to the next, so the first tick after
/// enrollment fetches.
#[derive(Debug, Clone, Default)]
pub(crate) struct MissionSlotSchedule {
    next_at: Option<DateTime<Utc>>,
    /// The enrollment `next_at` was set for: its ingest URL and device key
    /// id. Memory only, never logged. A new enrollment mints a new device
    /// key, so enrolling again between two ticks still reads as a change.
    enrollment: Option<(String, String)>,
}

impl MissionSlotSchedule {
    pub(crate) fn due(&self, now: DateTime<Utc>) -> bool {
        let Some(next_at) = self.next_at else {
            return true;
        };
        let interval = chrono::TimeDelta::from_std(MISSION_SLOT_REFRESH).unwrap_or_default();
        // A clock that moved back would otherwise postpone the next fetch
        // past the slot's own age limit.
        now >= next_at || next_at - now > interval
    }

    pub(crate) fn attempted(&mut self, now: DateTime<Utc>) {
        self.next_at = chrono::TimeDelta::from_std(MISSION_SLOT_REFRESH)
            .ok()
            .and_then(|interval| now.checked_add_signed(interval));
    }

    pub(crate) fn unconfigured(&mut self) {
        self.next_at = None;
        self.enrollment = None;
    }

    /// Note the enrollment this tick runs under. One that differs from the
    /// enrollment the schedule was kept for makes it due at once.
    pub(crate) fn enrolled(&mut self, config: &ContributorConfig) {
        let current = (config.ingest_url.clone(), config.device_key_id.clone());
        if self.enrollment.as_ref() != Some(&current) {
            self.next_at = None;
            self.enrollment = Some(current);
        }
    }
}

/// The contribution-mission catalogue a published activity catalogue gives
/// the local matcher, as [`super::mission_matching::receive_catalogue`]
/// reads it: one mission per activity mission that carries a supported
/// predicate, with the predicate as its criteria.
///
/// `None` when there is nothing to match on -- the policy is unconfigured,
/// `today` is outside its dates, or no mission carries a supported
/// predicate. A mission without one is left out rather than read as "no
/// criteria", which would fit every session.
pub(crate) fn contribution_catalogue(
    catalogue: &ActivityCatalogue,
    today: NaiveDate,
) -> Option<serde_json::Value> {
    let policy = catalogue.policy.as_ref()?;
    if today < policy.starts_on || today >= policy.ends_before {
        return None;
    }
    let missions: Vec<serde_json::Value> = policy
        .missions
        .iter()
        .filter_map(|mission| {
            let predicate = mission.supported_predicate()?;
            Some(serde_json::json!({
                "mission_id": mission.id,
                "title": mission.title,
                "criteria": {
                    "tools": predicate.tools,
                    "tool_families": predicate.tool_families,
                    "languages": predicate.languages,
                    "min_sessions": predicate.min_sessions,
                },
            }))
        })
        .collect();
    if missions.is_empty() {
        return None;
    }
    Some(serde_json::json!({
        "schema_version": crate::contribution_missions::CONTRIBUTION_MISSION_CATALOGUE_SCHEMA_VERSION,
        "missions": missions,
    }))
}

/// Refresh the contribution-mission slot from the published activity
/// catalogue, when [`MissionSlotSchedule`] says so.
///
/// The same anonymous `GET /v1/activity-missions` every contributor sends,
/// through the same origin, scheme and allowlist checks as
/// `activity_missions_catalogue`; nothing local goes with it, and matching
/// stays on this Mac. Three outcomes:
///
/// - **Fetch failed** (transport, status, bounds, a digest that does not
///   verify): the slot is left as it is and ages out on its own after
///   `MISSION_CATALOGUE_MAX_AGE`, never kept past it.
/// - **Nothing to match on**: the slot is emptied, so `mission_fit` is
///   absent, never a zero standing for "fits nothing".
/// - **Missions with a supported predicate**: they replace the slot.
///
/// Nothing is logged: the slot is the whole of its effect. Shells are told
/// through the queue and status events when what they render could change.
pub(crate) async fn refresh_mission_slot(
    shared: &DaemonShared,
    now: DateTime<Utc>,
    schedule: &mut MissionSlotSchedule,
) {
    use super::mission_matching::{clear_catalogue, live_catalogue, receive_catalogue};
    // The config is read before the schedule is consulted: an unenroll
    // between two due attempts must still reset it, or enrolling again
    // inside the interval would wait out the old enrollment's schedule.
    let Ok(Some(config)) = shared.store.load_config() else {
        schedule.unconfigured();
        return;
    };
    schedule.enrolled(&config);
    if !schedule.due(now) {
        return;
    }
    schedule.attempted(now);
    let call = fetch::<ActivityCatalogue>(&config, "/v1/activity-missions", None).await;
    let Ok(catalogue) = call.result else {
        return;
    };
    if !valid_catalogue(&catalogue) {
        return;
    }
    // An unenroll, or a move to another server, while the request was in
    // flight: what came back belongs to an enrollment that is gone.
    let still_enrolled = shared
        .store
        .load_config()
        .ok()
        .flatten()
        .is_some_and(|current| current.ingest_url == config.ingest_url);
    if !still_enrolled {
        return;
    }
    let before = live_catalogue(&shared.mission_catalogue, now);
    match contribution_catalogue(&catalogue, now.date_naive()) {
        // A refusal empties the slot inside `receive_catalogue`; the bounds
        // asserted above keep a published predicate from causing one.
        Some(raw) => {
            let _ = receive_catalogue(&shared.mission_catalogue, &raw, now);
        }
        None => clear_catalogue(&shared.mission_catalogue),
    }
    if live_catalogue(&shared.mission_catalogue, now) != before {
        shared.publish(super::ipc::EVENT_QUEUE_CHANGED, serde_json::json!({}));
        shared.publish(super::ipc::EVENT_STATUS_CHANGED, serde_json::json!({}));
    }
}
