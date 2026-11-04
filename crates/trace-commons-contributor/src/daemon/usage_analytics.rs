//! Anonymous usage counts, US only (DRAFT, NEEDS APPROVAL, 2026-11-04).
//!
//! A shell reports that a screen was opened or a control used
//! (`record_usage_event {event}`); the daemon adds one to that event's total
//! for the current ISO week, and sends finished weeks' totals to the commons
//! (`POST /v1/usage-events`) with one anonymous install id. Only names on
//! [`trace_commons_protocol::usage_analytics::USAGE_EVENTS`] are counted.
//!
//! Counting is on only when both hold:
//!
//! 1. the `share_usage_data` setting is on (the default);
//! 2. the commons answered `GET /v1/public/usage-region` with
//!    `analytics_allowed: true` within [`REGION_VALID_FOR`]. The server
//!    decides from the request's IP address, US only, and fails closed.
//!
//! Otherwise a report is dropped, and the totals on disk are deleted. The
//! region is asked again every [`REGION_RECHECK`] (sooner after a failed ask)
//! while the setting is on.
//!
//! The install id is a random UUIDv4 made here the first time counting turns
//! on and kept in this module's file. It is not derived from, stored with or
//! sent beside any device key, account, enrollment, path or hardware id: its
//! only use is telling one install's weeks from another's, so returning
//! users can be counted. Turning the setting off deletes the file, install id
//! included; turning it back on makes a new one. Nothing else is ever stored
//! or sent: no paths, prompts, file names, model names or trace content.

use std::collections::BTreeMap;

use chrono::{DateTime, Duration, Utc};
use reqwest::{StatusCode, Url};
use serde::{Deserialize, Serialize};
use trace_commons_operator_client::host_allowlist::HostAllowlist;
use trace_commons_protocol::usage_analytics::{
    USAGE_BATCH_MAX_ROWS, USAGE_COUNT_MAX, USAGE_EVENTS_PATH, USAGE_REGION_MAX_BYTES,
    USAGE_REGION_PATH, UsageEventBatch, UsageEventRow, UsageRegion, is_usage_event, iso_week_of,
    new_install_id,
};
use uuid::Uuid;

use crate::config::{ConfigStore, allowlist_for};

/// The daemon file holding the region answer, the install id and the
/// weekly totals.
pub const USAGE_ANALYTICS_FILE: &str = "usage-analytics.json";

/// How often the region is asked again after an answer.
pub const REGION_RECHECK: Duration = Duration::days(3);

/// How soon the region is asked again after the ask failed.
pub const REGION_RETRY: Duration = Duration::hours(6);

/// How long an answer is trusted. Past it nothing is counted until the
/// region is asked again.
pub const REGION_VALID_FOR: Duration = Duration::days(7);

/// The most finished weeks kept while the commons cannot be reached. Older
/// ones are dropped, never sent late.
pub const RETAIN_WEEKS: usize = 8;

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
const CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// `record_usage_event` refusals. Fixed labels.
pub const ERR_USAGE_EVENT_UNKNOWN: &str = "usage-event-unknown";

/// The last region answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegionCheck {
    /// The commons said US.
    pub allowed: bool,
    /// When to ask again.
    pub next_check: DateTime<Utc>,
    /// When the answer stops counting.
    pub valid_until: DateTime<Utc>,
}

/// What `usage-analytics.json` holds.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageState {
    #[serde(default)]
    pub region: Option<RegionCheck>,
    /// The anonymous install id; made the first time counting turns on.
    #[serde(default)]
    pub install_id: Option<Uuid>,
    /// ISO week -> event -> count.
    #[serde(default)]
    pub weeks: BTreeMap<String, BTreeMap<String, u64>>,
}

impl UsageState {
    /// Whether a report made `now` is counted, given the setting.
    #[must_use]
    pub fn counting(&self, share_usage_data: bool, now: DateTime<Utc>) -> bool {
        share_usage_data
            && self
                .region
                .is_some_and(|region| region.allowed && now < region.valid_until)
    }

    /// Whether the region should be asked `now`.
    #[must_use]
    pub fn region_due(&self, now: DateTime<Utc>) -> bool {
        self.region.is_none_or(|region| now >= region.next_check)
    }

    /// The install id, made now if there is none.
    pub fn install_id(&mut self) -> Uuid {
        *self.install_id.get_or_insert_with(new_install_id)
    }

    /// Add one to `event` for the week holding `now`.
    pub fn count(&mut self, event: &str, now: DateTime<Utc>) {
        let week = iso_week_of(now.date_naive());
        let total = self
            .weeks
            .entry(week)
            .or_default()
            .entry(event.to_owned())
            .or_insert(0);
        *total = (*total + 1).min(USAGE_COUNT_MAX);
    }

    /// The finished weeks (every week but the one holding `now`), oldest
    /// first, as one batch. `None` when there are none or no install id.
    #[must_use]
    pub fn finished_batch(&self, now: DateTime<Utc>) -> Option<(Vec<String>, UsageEventBatch)> {
        let install_id = self.install_id?;
        let current = iso_week_of(now.date_naive());
        let mut weeks = Vec::new();
        let mut rows = Vec::new();
        for (week, events) in &self.weeks {
            if *week == current {
                continue;
            }
            if rows.len() + events.len() > USAGE_BATCH_MAX_ROWS {
                break;
            }
            weeks.push(week.clone());
            rows.extend(
                events
                    .iter()
                    .filter(|(_, count)| **count > 0)
                    .map(|(event, count)| UsageEventRow {
                        event: event.clone(),
                        count: *count,
                        week: week.clone(),
                    }),
            );
        }
        (!weeks.is_empty()).then_some((weeks, UsageEventBatch { install_id, rows }))
    }

    /// Keep at most [`RETAIN_WEEKS`] finished weeks, newest kept.
    pub fn prune(&mut self, now: DateTime<Utc>) {
        let current = iso_week_of(now.date_naive());
        let finished: Vec<String> = self
            .weeks
            .keys()
            .filter(|week| **week != current)
            .cloned()
            .collect();
        let excess = finished.len().saturating_sub(RETAIN_WEEKS);
        for week in finished.into_iter().take(excess) {
            self.weeks.remove(&week);
        }
    }
}

/// Read the state. A missing or unreadable file is an empty state: nothing is
/// counted until the region is asked.
#[must_use]
pub fn load(store: &ConfigStore) -> UsageState {
    store
        .read_daemon_file(USAGE_ANALYTICS_FILE)
        .ok()
        .flatten()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

fn save(store: &ConfigStore, state: &UsageState) -> anyhow::Result<()> {
    store.write_daemon_file(USAGE_ANALYTICS_FILE, &serde_json::to_vec(state)?)
}

/// Delete the file: the counts not yet sent, the region answer and the
/// install id.
fn clear(store: &ConfigStore) {
    if matches!(store.read_daemon_file(USAGE_ANALYTICS_FILE), Ok(Some(_))) {
        let _ = store.remove_daemon_file(USAGE_ANALYTICS_FILE);
    }
}

/// Why a report was not taken.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordError {
    /// Not on the allow-list.
    UnknownEvent,
    /// The file could not be written.
    Unavailable,
}

/// One report from a shell. `Ok(true)` when counted, `Ok(false)` when
/// counting is off (dropped, not an error: a shell reports regardless).
pub fn record(
    store: &ConfigStore,
    share_usage_data: bool,
    event: &str,
    now: DateTime<Utc>,
) -> Result<bool, RecordError> {
    if !is_usage_event(event) {
        return Err(RecordError::UnknownEvent);
    }
    let mut state = load(store);
    if !state.counting(share_usage_data, now) {
        return Ok(false);
    }
    state.install_id();
    state.count(event, now);
    save(store, &state).map_err(|_| RecordError::Unavailable)?;
    Ok(true)
}

/// Fixed outcomes of a request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageClientError {
    EndpointInvalid,
    HostNotAllowed,
    /// The commons does not serve the route.
    NotFound,
    /// Refused, unreadable or unreachable.
    Unavailable,
}

/// An unauthenticated client for the two usage routes. Never attaches a
/// credential, never follows a redirect, reads a bounded body.
pub struct UsageClient {
    http: reqwest::Client,
    origin: Url,
    allowlist: HostAllowlist,
}

impl UsageClient {
    pub fn new(ingest_url: &str, allowed_hosts: Option<&str>) -> Result<Self, UsageClientError> {
        let origin = Url::parse(ingest_url).map_err(|_| UsageClientError::EndpointInvalid)?;
        if !crate::mission_catalog::origin_is_valid(&origin) {
            return Err(UsageClientError::EndpointInvalid);
        }
        let allowlist = allowlist_for(allowed_hosts);
        allowlist
            .check(&origin)
            .map_err(|_| UsageClientError::HostNotAllowed)?;
        let http = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|_| UsageClientError::Unavailable)?;
        Ok(Self {
            http,
            origin,
            allowlist,
        })
    }

    fn url(&self, path: &str) -> Result<Url, UsageClientError> {
        let mut url = self.origin.clone();
        url.set_path(path);
        url.set_query(None);
        url.set_fragment(None);
        self.allowlist
            .check(&url)
            .map_err(|_| UsageClientError::HostNotAllowed)?;
        Ok(url)
    }

    /// `GET /v1/public/usage-region`.
    pub async fn region(&self) -> Result<bool, UsageClientError> {
        let mut response = self
            .http
            .get(self.url(USAGE_REGION_PATH)?)
            .send()
            .await
            .map_err(|_| UsageClientError::Unavailable)?;
        match response.status() {
            StatusCode::NOT_FOUND => return Err(UsageClientError::NotFound),
            status if !status.is_success() => return Err(UsageClientError::Unavailable),
            _ => {}
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| UsageClientError::Unavailable)?
        {
            if body.len() + chunk.len() > USAGE_REGION_MAX_BYTES {
                return Err(UsageClientError::Unavailable);
            }
            body.extend_from_slice(&chunk);
        }
        let region: UsageRegion =
            serde_json::from_slice(&body).map_err(|_| UsageClientError::Unavailable)?;
        Ok(region.analytics_allowed)
    }

    /// `POST /v1/usage-events`. `Ok` only on a 2xx.
    pub async fn send(&self, batch: &UsageEventBatch) -> Result<(), UsageClientError> {
        let response = self
            .http
            .post(self.url(USAGE_EVENTS_PATH)?)
            .json(batch)
            .send()
            .await
            .map_err(|_| UsageClientError::Unavailable)?;
        match response.status() {
            StatusCode::NOT_FOUND => Err(UsageClientError::NotFound),
            status if status.is_success() => Ok(()),
            _ => Err(UsageClientError::Unavailable),
        }
    }
}

/// One pass: forget everything when off, ask the region when due, make the
/// install id when counting turns on, send finished weeks. Logs nothing: its
/// whole effect is the file.
pub async fn tick_with(
    store: &ConfigStore,
    share_usage_data: bool,
    client: Option<&UsageClient>,
    now: DateTime<Utc>,
) {
    if !share_usage_data {
        clear(store);
        return;
    }
    let mut state = load(store);
    let before = state.clone();
    if state.region_due(now) {
        let allowed = match client {
            Some(client) => client.region().await.ok(),
            None => None,
        };
        state.region = Some(match allowed {
            Some(allowed) => RegionCheck {
                allowed,
                next_check: now + REGION_RECHECK,
                valid_until: now + REGION_VALID_FOR,
            },
            // The ask failed: the old answer still runs out when it did, and
            // the region is asked again sooner.
            None => RegionCheck {
                allowed: state
                    .region
                    .is_some_and(|r| r.allowed && now < r.valid_until),
                next_check: now + REGION_RETRY,
                valid_until: state.region.map_or(now, |r| r.valid_until),
            },
        });
    }
    if state.counting(share_usage_data, now) {
        state.install_id();
        state.prune(now);
        if let (Some(client), Some((weeks, batch))) = (client, state.finished_batch(now)) {
            if client.send(&batch).await.is_ok() {
                for week in weeks {
                    state.weeks.remove(&week);
                }
            }
        }
    } else {
        // Not counting: nothing is kept, so nothing can be sent later.
        state.weeks.clear();
    }
    if state != before {
        let _ = save(store, &state);
    }
}

/// The daemon's pass, from its tick. Builds the client from the configured
/// ingest origin; without one, the region cannot be asked and nothing is
/// counted.
pub async fn tick(shared: &super::ipc::DaemonShared, now: DateTime<Utc>) {
    let share_usage_data = shared
        .settings
        .lock()
        .map(|s| s.share_usage_data)
        .unwrap_or(false);
    if !share_usage_data {
        tick_with(&shared.store, false, None, now).await;
        return;
    }
    // Most ticks: no region due, no finished week, and the file already as
    // the pass would leave it.
    let state = load(&shared.store);
    let settled = if state.counting(true, now) {
        state.install_id.is_some()
    } else {
        state.weeks.is_empty()
    };
    if !state.region_due(now) && state.finished_batch(now).is_none() && settled {
        return;
    }
    let client = match shared.store.load_config() {
        Ok(Some(cfg)) => crate::config::ingest_origin_url(&cfg.ingest_url, "/")
            .ok()
            .and_then(|origin| {
                UsageClient::new(origin.as_str(), cfg.allowed_hosts.as_deref()).ok()
            }),
        _ => None,
    };
    tick_with(&shared.store, true, client.as_ref(), now).await;
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::{Json, Router, http::StatusCode, routing::get, routing::post};

    use super::*;

    fn temp_store() -> (tempfile::TempDir, ConfigStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ConfigStore::open(dir.path().to_path_buf()).unwrap();
        (dir, store)
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn allowed_until(now: DateTime<Utc>) -> UsageState {
        UsageState {
            region: Some(RegionCheck {
                allowed: true,
                next_check: now + REGION_RECHECK,
                valid_until: now + REGION_VALID_FOR,
            }),
            install_id: None,
            weeks: BTreeMap::new(),
        }
    }

    #[test]
    fn counting_needs_the_setting_and_a_fresh_us_answer() {
        let now = at("2026-11-04T12:00:00Z");
        let state = allowed_until(now);
        assert!(state.counting(true, now));
        assert!(!state.counting(false, now), "setting off");
        assert!(
            !state.counting(true, now + REGION_VALID_FOR),
            "answer expired"
        );
        let mut refused = state.clone();
        refused.region.as_mut().unwrap().allowed = false;
        assert!(!refused.counting(true, now), "not US");
        assert!(!UsageState::default().counting(true, now), "never asked");
    }

    #[test]
    fn an_unlisted_event_is_refused_and_a_listed_one_counted_only_when_on() {
        let (_d, store) = temp_store();
        let now = at("2026-11-04T12:00:00Z");
        assert_eq!(
            record(&store, true, "opened_secret_file", now),
            Err(RecordError::UnknownEvent)
        );
        // Never asked the region: dropped, and no install id made.
        assert_eq!(record(&store, true, "tab_insights_opened", now), Ok(false));
        assert!(load(&store).install_id.is_none());
        save(&store, &allowed_until(now)).unwrap();
        assert_eq!(record(&store, true, "tab_insights_opened", now), Ok(true));
        assert_eq!(record(&store, true, "tab_insights_opened", now), Ok(true));
        assert_eq!(record(&store, false, "tab_insights_opened", now), Ok(false));
        let state = load(&store);
        assert_eq!(state.weeks["2026-W45"]["tab_insights_opened"], 2);
        assert_eq!(state.install_id.unwrap().get_version_num(), 4);
    }

    #[test]
    fn only_finished_weeks_are_sent_and_old_ones_dropped() {
        let now = at("2026-11-04T12:00:00Z"); // 2026-W45
        let mut state = allowed_until(now);
        state.install_id();
        state.count("setup_finished", now);
        for weeks_ago in 1..=12 {
            state.count("tab_home_opened", now - Duration::weeks(weeks_ago));
        }
        state.prune(now);
        assert_eq!(
            state.weeks.len(),
            RETAIN_WEEKS + 1,
            "kept 8 finished + current"
        );
        let (weeks, batch) = state.finished_batch(now).unwrap();
        assert!(!weeks.contains(&"2026-W45".to_owned()));
        assert_eq!(weeks.len(), RETAIN_WEEKS);
        assert_eq!(batch.install_id, state.install_id.unwrap());
        assert!(batch.validate().is_ok());
        assert!(batch.rows.iter().all(|r| r.week != "2026-W45"));
    }

    #[test]
    fn no_install_id_means_no_batch() {
        let now = at("2026-11-04T12:00:00Z");
        let mut state = allowed_until(now);
        state.count("setup_finished", now - Duration::weeks(1));
        assert!(state.finished_batch(now).is_none());
    }

    /// A commons that answers the region with `allowed` and records batches.
    async fn commons(
        allowed: bool,
        events: StatusCode,
    ) -> (String, Arc<Mutex<Vec<UsageEventBatch>>>) {
        let sent = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&sent);
        let app = Router::new()
            .route(
                USAGE_REGION_PATH,
                get(move || async move {
                    Json(UsageRegion {
                        analytics_allowed: allowed,
                    })
                }),
            )
            .route(
                USAGE_EVENTS_PATH,
                post(move |Json(batch): Json<UsageEventBatch>| {
                    let seen = Arc::clone(&seen);
                    async move {
                        seen.lock().unwrap().push(batch);
                        events
                    }
                }),
            );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        (format!("http://{addr}"), sent)
    }

    #[tokio::test]
    async fn a_us_answer_turns_counting_on_and_a_finished_week_is_sent_once() {
        let (_d, store) = temp_store();
        let (origin, sent) = commons(true, StatusCode::NO_CONTENT).await;
        let client = UsageClient::new(&origin, None).unwrap();
        let monday = at("2026-11-02T09:00:00Z"); // 2026-W45
        tick_with(&store, true, Some(&client), monday).await;
        let state = load(&store);
        assert!(state.counting(true, monday));
        let install_id = state.install_id.expect("made when counting turned on");
        record(&store, true, "tab_insights_opened", monday).unwrap();
        record(&store, true, "insights_refresh_clicked", monday).unwrap();

        // Same week: nothing sent.
        tick_with(&store, true, Some(&client), monday + Duration::hours(1)).await;
        assert!(sent.lock().unwrap().is_empty());

        // Next week: last week's totals go once, with the same install id,
        // and leave the file.
        let next = at("2026-11-09T09:00:00Z");
        tick_with(&store, true, Some(&client), next).await;
        tick_with(&store, true, Some(&client), next + Duration::hours(1)).await;
        let batches = sent.lock().unwrap().clone();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].install_id, install_id);
        let rows = &batches[0].rows;
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.week == "2026-W45" && r.count == 1));
        let state = load(&store);
        assert!(state.weeks.is_empty());
        assert_eq!(state.install_id, Some(install_id), "kept across weeks");
    }

    #[tokio::test]
    async fn a_failed_send_keeps_the_week_for_the_next_pass() {
        let (_d, store) = temp_store();
        let (origin, sent) = commons(true, StatusCode::SERVICE_UNAVAILABLE).await;
        let client = UsageClient::new(&origin, None).unwrap();
        let monday = at("2026-11-02T09:00:00Z");
        tick_with(&store, true, Some(&client), monday).await;
        record(&store, true, "setup_finished", monday).unwrap();
        tick_with(&store, true, Some(&client), at("2026-11-09T09:00:00Z")).await;
        assert_eq!(sent.lock().unwrap().len(), 1);
        assert_eq!(load(&store).weeks["2026-W45"]["setup_finished"], 1);
    }

    #[tokio::test]
    async fn a_non_us_answer_counts_nothing_and_makes_no_install_id() {
        let (_d, store) = temp_store();
        let (origin, sent) = commons(false, StatusCode::NO_CONTENT).await;
        let client = UsageClient::new(&origin, None).unwrap();
        let now = at("2026-11-04T12:00:00Z");
        tick_with(&store, true, Some(&client), now).await;
        let state = load(&store);
        assert!(!state.counting(true, now));
        assert!(state.install_id.is_none());
        assert_eq!(record(&store, true, "tab_home_opened", now), Ok(false));
        tick_with(&store, true, Some(&client), at("2026-11-16T12:00:00Z")).await;
        assert!(sent.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn turning_the_setting_off_deletes_counts_and_install_id_and_on_makes_a_new_one() {
        let (_d, store) = temp_store();
        let (origin, _sent) = commons(true, StatusCode::NO_CONTENT).await;
        let client = UsageClient::new(&origin, None).unwrap();
        let now = at("2026-11-04T12:00:00Z");
        tick_with(&store, true, Some(&client), now).await;
        record(&store, true, "tab_traces_opened", now).unwrap();
        let first = load(&store).install_id.unwrap();

        tick_with(&store, false, Some(&client), now).await;
        assert!(
            store
                .read_daemon_file(USAGE_ANALYTICS_FILE)
                .unwrap()
                .is_none()
        );

        tick_with(&store, true, Some(&client), now + Duration::minutes(1)).await;
        let state = load(&store);
        let second = state.install_id.unwrap();
        assert_ne!(first, second, "a new install id after off and on");
        assert!(state.weeks.is_empty(), "the old counts are gone");
    }

    #[tokio::test]
    async fn an_unreachable_commons_counts_nothing_new() {
        let (_d, store) = temp_store();
        let now = at("2026-11-04T12:00:00Z");
        // No client (no config): the ask fails, nothing is allowed.
        tick_with(&store, true, None, now).await;
        let state = load(&store);
        assert!(!state.counting(true, now));
        assert!(state.install_id.is_none());
        assert_eq!(state.region.unwrap().next_check, now + REGION_RETRY);
    }
}
