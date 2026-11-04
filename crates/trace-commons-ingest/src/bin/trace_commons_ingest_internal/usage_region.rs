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

use std::net::IpAddr;
use std::sync::LazyLock;

use axum::Json;
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::IntoResponse;
use trace_commons_protocol::usage_analytics::UsageRegion;

use super::{
    ACCOUNT_RATE_LIMITER, api_error, client_ip_for_rate_limit, client_ip_for_referral_mark,
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

/// Whether the request in `headers` may send usage counts. Shared with the
/// events route, which checks again before it takes a batch.
pub(super) fn request_is_allowed(headers: &HeaderMap) -> bool {
    analytics_allowed(COUNTRIES.as_deref(), client_ip_for_referral_mark(headers))
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
