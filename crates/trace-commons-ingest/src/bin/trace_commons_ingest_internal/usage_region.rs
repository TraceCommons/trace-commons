// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! `GET /v1/public/usage-region`: whether a client may send anonymous usage
//! counts, which it may only from the United States (DRAFT, NEEDS APPROVAL,
//! 2026-11-04).
//!
//! Unauthenticated and outside tenant context, like
//! `/v1/public/first-run-estimate`: a client asks before it sends anything,
//! and the answer is about the request, not about an account.
//!
//! The address is the one the trusted edge observed
//! (`client_ip_for_referral_mark`, the same hop and trust assumption as the
//! rate limiter). It is looked up in a MaxMind-format country database whose
//! path the operator sets in [`USAGE_REGION_MMDB_ENV`], and then dropped:
//! nothing about it is stored, and nothing about it or the answer is logged.
//!
//! Fail closed. No database configured, a database that will not open, an
//! address the edge did not attribute, a lookup error or a country that is not
//! `US`: every one answers `{"analytics_allowed": false}`, and a client sends
//! nothing.
//!
//! `POST /v1/usage-events` takes a client's weekly totals
//! ([`UsageEventBatch`]): unauthenticated and outside tenant context for the
//! same reason. The batch is refused whole unless the protocol's own
//! validation takes it, then stored through `Database::record_usage_events`
//! (V152: a non-tenant table only a guard-owned function writes). It does
//! not ask the region again (owner, 2026-11-04: too early); the client only
//! sends after a US answer. Logs carry fixed labels only, never the install
//! id, the address or a row.

use std::net::IpAddr;
use std::sync::{Arc, LazyLock};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use trace_commons_protocol::usage_analytics::{UsageEventBatch, UsageRegion};

use super::{
    ACCOUNT_RATE_LIMITER, AppState, api_error, client_ip_for_rate_limit,
    client_ip_for_referral_mark,
};

/// Path to a MaxMind-format (`.mmdb`) country database, such as GeoLite2
/// Country. Unset: every request answers `false`.
pub(super) const USAGE_REGION_MMDB_ENV: &str = "TRACE_COMMONS_USAGE_REGION_MMDB";

/// The one country that may send usage counts.
const ALLOWED_COUNTRY: &str = "US";

/// Requests per minute from one address. A client asks every few days.
const USAGE_REGION_PER_IP_LIMIT: u32 = 30;

/// Requests per minute from everyone: the real bound.
const USAGE_REGION_GLOBAL_LIMIT: u32 = 1200;

/// Where an address is. `Err` is any lookup failure; `Ok(None)` an address
/// the database does not place.
pub(super) trait CountryLookup: Send + Sync {
    fn country(&self, address: IpAddr) -> Result<Option<String>, ()>;
}

/// A MaxMind-format database read into memory once.
struct MmdbCountries(maxminddb::Reader<Vec<u8>>);

impl CountryLookup for MmdbCountries {
    fn country(&self, address: IpAddr) -> Result<Option<String>, ()> {
        let found = self.0.lookup(address).map_err(|_| ())?;
        found
            .decode_path::<String>(&[
                maxminddb::PathElement::Key("country"),
                maxminddb::PathElement::Key("iso_code"),
            ])
            .map_err(|_| ())
    }
}

/// Open the database at `path`. `None` when it will not open: the gate then
/// answers `false` for everyone.
pub(super) fn open_countries(path: &std::path::Path) -> Option<Box<dyn CountryLookup>> {
    maxminddb::Reader::open_readfile(path)
        .ok()
        .map(|reader| Box::new(MmdbCountries(reader)) as Box<dyn CountryLookup>)
}

/// The database configured for this process, opened on first use. A path
/// that will not open is reported once, by label, never by path.
static COUNTRIES: LazyLock<Option<Box<dyn CountryLookup>>> = LazyLock::new(|| {
    let path = std::env::var_os(USAGE_REGION_MMDB_ENV)?;
    let opened = open_countries(std::path::Path::new(&path));
    if opened.is_none() {
        tracing::warn!(
            control = "usage_region_mmdb",
            "usage region database did not open; usage counts are off"
        );
    }
    opened
});

/// The whole decision: true only for an attributed address the database
/// places in the United States.
pub(super) fn analytics_allowed(
    lookup: Option<&dyn CountryLookup>,
    address: Option<IpAddr>,
) -> bool {
    let (Some(lookup), Some(address)) = (lookup, address) else {
        return false;
    };
    matches!(lookup.country(address), Ok(Some(country)) if country == ALLOWED_COUNTRY)
}

/// Whether the request in `headers` may send usage counts.
fn request_is_allowed(headers: &HeaderMap) -> bool {
    analytics_allowed(COUNTRIES.as_deref(), client_ip_for_referral_mark(headers))
}

/// Batches per minute from one address. A client sends about once a week.
const USAGE_EVENTS_PER_IP_LIMIT: u32 = 30;

/// Batches per minute from everyone.
const USAGE_EVENTS_GLOBAL_LIMIT: u32 = 1200;

/// `POST /v1/usage-events`: 204 when stored, 400 with a fixed label when the
/// batch is refused, 503 when there is no database or the write failed.
pub(super) async fn usage_events_handler(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Bytes,
) -> axum::response::Response {
    let client_ip = client_ip_for_rate_limit(&headers);
    if !ACCOUNT_RATE_LIMITER.check(
        &format!("usage-events-ip:{client_ip}"),
        USAGE_EVENTS_PER_IP_LIMIT,
    ) || !ACCOUNT_RATE_LIMITER.check_global("usage-events-global", USAGE_EVENTS_GLOBAL_LIMIT)
    {
        return api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }
    let batch = match parse_batch(&body) {
        Ok(batch) => batch,
        Err(label) => return api_error(StatusCode::BAD_REQUEST, label).into_response(),
    };
    let Some(db) = state.db_mirror.as_ref() else {
        return api_error(StatusCode::SERVICE_UNAVAILABLE, "usage_events_unavailable")
            .into_response();
    };
    match db.record_usage_events(&batch).await {
        Ok(_) => StatusCode::NO_CONTENT.into_response(),
        Err(_) => {
            tracing::warn!(
                reason = "usage_events_store_failed",
                "usage events not stored"
            );
            api_error(StatusCode::SERVICE_UNAVAILABLE, "usage_events_unavailable").into_response()
        }
    }
}

/// The batch, or the fixed label it was refused with. Unknown fields,
/// a non-random install id, an unlisted event or a bound broken: refused
/// whole.
pub(super) fn parse_batch(body: &[u8]) -> Result<UsageEventBatch, &'static str> {
    let batch: UsageEventBatch =
        serde_json::from_slice(body).map_err(|_| "usage-batch-malformed")?;
    batch.validate().map_err(|e| match e {
        trace_commons_protocol::usage_analytics::UsageBatchError::TooLarge => {
            "usage-batch-too-large"
        }
        trace_commons_protocol::usage_analytics::UsageBatchError::InstallIdInvalid => {
            "usage-install-id-invalid"
        }
        trace_commons_protocol::usage_analytics::UsageBatchError::UnknownEvent => {
            "usage-event-unknown"
        }
        trace_commons_protocol::usage_analytics::UsageBatchError::CountInvalid => {
            "usage-count-invalid"
        }
        trace_commons_protocol::usage_analytics::UsageBatchError::WeekInvalid => {
            "usage-week-invalid"
        }
    })?;
    Ok(batch)
}

/// `GET /v1/public/usage-region`.
pub(super) async fn usage_region_handler(headers: HeaderMap) -> axum::response::Response {
    let client_ip = client_ip_for_rate_limit(&headers);
    if !ACCOUNT_RATE_LIMITER.check(
        &format!("usage-region-ip:{client_ip}"),
        USAGE_REGION_PER_IP_LIMIT,
    ) || !ACCOUNT_RATE_LIMITER.check_global("usage-region-global", USAGE_REGION_GLOBAL_LIMIT)
    {
        return api_error(StatusCode::TOO_MANY_REQUESTS, "rate limited").into_response();
    }
    let body = UsageRegion {
        analytics_allowed: request_is_allowed(&headers),
    };
    let mut response = Json(body).into_response();
    let headers = response.headers_mut();
    // Per address: never cached by a shared cache.
    headers.insert(
        axum::http::header::CACHE_CONTROL,
        HeaderValue::from_static("private, no-store"),
    );
    headers.insert(
        axum::http::header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(Result<Option<&'static str>, ()>);

    impl CountryLookup for Fixed {
        fn country(&self, _address: IpAddr) -> Result<Option<String>, ()> {
            self.0.map(|c| c.map(str::to_owned))
        }
    }

    fn address() -> Option<IpAddr> {
        Some("203.0.113.7".parse().unwrap())
    }

    #[test]
    fn only_a_us_address_is_allowed() {
        assert!(analytics_allowed(Some(&Fixed(Ok(Some("US")))), address()));
        for refused in [
            Ok(Some("DE")),
            Ok(Some("us")),
            Ok(Some("GB")),
            Ok(None),
            Err(()),
        ] {
            assert!(
                !analytics_allowed(Some(&Fixed(refused)), address()),
                "{refused:?}"
            );
        }
    }

    #[test]
    fn no_database_or_no_address_is_not_allowed() {
        assert!(!analytics_allowed(None, address()));
        assert!(!analytics_allowed(Some(&Fixed(Ok(Some("US")))), None));
    }

    #[test]
    fn a_database_that_will_not_open_is_no_database() {
        let dir = tempfile::tempdir().unwrap();
        assert!(open_countries(&dir.path().join("missing.mmdb")).is_none());
        let junk = dir.path().join("junk.mmdb");
        std::fs::write(&junk, b"not a maxmind database").unwrap();
        assert!(open_countries(&junk).is_none());
    }

    #[test]
    fn a_batch_is_taken_whole_or_refused_with_a_label() {
        let id = uuid::Uuid::new_v4();
        let good = serde_json::json!({
            "install_id": id,
            "rows": [{"event": "tab_insights_opened", "count": 3, "week": "2026-W45"}]
        });
        assert_eq!(
            parse_batch(good.to_string().as_bytes()).unwrap().install_id,
            id
        );
        for (body, label) in [
            (
                serde_json::json!({"install_id": id, "rows": [], "ip": "1.2.3.4"}),
                "usage-batch-malformed",
            ),
            (serde_json::json!({"rows": []}), "usage-batch-malformed"),
            (
                serde_json::json!({"install_id": uuid::Uuid::nil(), "rows": []}),
                "usage-install-id-invalid",
            ),
            (
                serde_json::json!({"install_id": id, "rows": [{"event": "read_file", "count": 1, "week": "2026-W45"}]}),
                "usage-event-unknown",
            ),
            (
                serde_json::json!({"install_id": id, "rows": [{"event": "setup_finished", "count": 0, "week": "2026-W45"}]}),
                "usage-count-invalid",
            ),
            (
                serde_json::json!({"install_id": id, "rows": [{"event": "setup_finished", "count": 1, "week": "45"}]}),
                "usage-week-invalid",
            ),
        ] {
            assert_eq!(
                parse_batch(body.to_string().as_bytes()).unwrap_err(),
                label,
                "{body}"
            );
        }
        assert_eq!(
            parse_batch(b"not json").unwrap_err(),
            "usage-batch-malformed"
        );
    }

    #[tokio::test]
    async fn an_unattributed_request_is_told_no() {
        // No X-Forwarded-For: the edge attributed nothing, so whatever the
        // database says, the answer is false.
        let response = usage_region_handler(HeaderMap::new()).await;
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024)
            .await
            .unwrap();
        let body: UsageRegion = serde_json::from_slice(&bytes).unwrap();
        assert!(!body.analytics_allowed);
    }
}
