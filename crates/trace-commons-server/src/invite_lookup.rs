// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Decision logic for the non-redeeming invite lookup
//! (`POST /v1/invite/lookup`, #1118 Z1).
//!
//! Pure: no I/O. The issuer route reads the invite row and hands it here, so
//! what a lookup may disclose is decided in exactly one place. The answer
//! carries the public issuer name and the credit range and nothing else: never
//! `policy_label`, `issued_by_label`, `note_label`, `max_uses` or the use
//! count.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Mutex;
use std::time::Instant;

use axum::http::{HeaderMap, HeaderName};
use chrono::{DateTime, Utc};
use trace_commons_protocol::invite_lookup::{
    CreditRange, INVITE_LOOKUP_REASON_EXHAUSTED, INVITE_LOOKUP_REASON_EXPIRED,
    INVITE_LOOKUP_REASON_NOT_FOUND, INVITE_LOOKUP_REASON_REVOKED, InviteLookupResponse,
};

use crate::db::postgres::InvitePeek;

/// Classify an invite for a lookup. Revocation outranks expiry, and expiry
/// outranks exhaustion, so a caller is told the most durable reason.
pub fn classify_invite(peek: Option<&InvitePeek>, now: DateTime<Utc>) -> InviteLookupResponse {
    let Some(peek) = peek else {
        return InviteLookupResponse::invalid(INVITE_LOOKUP_REASON_NOT_FOUND);
    };
    let entry = &peek.entry;
    if entry.revoked_at.is_some() {
        return InviteLookupResponse::invalid(INVITE_LOOKUP_REASON_REVOKED);
    }
    if entry.expires_at.is_some_and(|exp| exp <= now) {
        return InviteLookupResponse::invalid(INVITE_LOOKUP_REASON_EXPIRED);
    }
    if peek.consumed_uses >= entry.max_uses {
        return InviteLookupResponse::invalid(INVITE_LOOKUP_REASON_EXHAUSTED);
    }
    InviteLookupResponse {
        valid: true,
        issuer_display_name: entry.issuer_display_name.clone(),
        credit_range: entry
            .credit_range
            .map(|(min, max)| CreditRange::points(min, max)),
        reason_label: None,
    }
}

/// Bucket shared by lookups whose trusted client header is absent or does not
/// hold an IP address.
pub const UNATTRIBUTED_CLIENT_KEY: &str = "unattributed";

/// Bucket shared by newcomers while the per-client table is full.
pub const OVERFLOW_CLIENT_KEY: &str = "overflow";

/// Most clients the per-client lookup limiter tracks at once. Past this a new
/// client shares [`OVERFLOW_CLIENT_KEY`], so a caller rotating addresses
/// cannot grow the table without bound.
pub const MAX_TRACKED_LOOKUP_CLIENTS: usize = 16_384;

/// Validate the operator's `TRACE_COMMONS_INVITE_LOOKUP_CLIENT_IP_HEADER`.
/// Unset or blank means per-client keying is off; a value that is not a
/// header name refuses startup rather than silently leaving it off.
pub fn parse_client_ip_header(raw: Option<&str>) -> anyhow::Result<Option<HeaderName>> {
    let Some(raw) = raw.map(str::trim).filter(|raw| !raw.is_empty()) else {
        return Ok(None);
    };
    HeaderName::from_bytes(raw.to_ascii_lowercase().as_bytes())
        .map(Some)
        .map_err(|_| {
            anyhow::anyhow!("TRACE_COMMONS_INVITE_LOOKUP_CLIENT_IP_HEADER is not a header name")
        })
}

/// The per-client throttle key for one lookup, read from the header the
/// operator named as written by their trusted proxy.
///
/// The rightmost hop of the last header line is used: a proxy that appends
/// (`X-Forwarded-For`) puts its own observation last, and anything to its
/// left came from the caller. IPv6 callers are keyed by /64, the smallest
/// block one subscriber usually controls. A missing or non-IP value falls
/// into [`UNATTRIBUTED_CLIENT_KEY`], which is throttled like any one client.
///
/// Only meaningful when the issuer is reachable solely through that proxy
/// and the proxy overwrites or appends the header; see
/// `docs/operator/pilot-allowlist.md`.
pub fn lookup_client_key(headers: &HeaderMap, header: &HeaderName) -> String {
    let hop = headers
        .get_all(header)
        .iter()
        .next_back()
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit(',').next())
        .map(str::trim)
        .and_then(|hop| hop.parse::<IpAddr>().ok());
    match hop {
        Some(IpAddr::V4(v4)) => v4.to_string(),
        Some(IpAddr::V6(v6)) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                let prefix = Ipv6Addr::new(s[0], s[1], s[2], s[3], 0, 0, 0, 0);
                format!("{prefix}/64")
            }
        },
        None => UNATTRIBUTED_CLIENT_KEY.to_string(),
    }
}

struct ClientBucket {
    tokens: f64,
    last: Instant,
}

/// Token buckets per lookup client, bounded to a fixed number of clients.
/// Capacity and refill match `InstanceRateLimiter`: `rate_per_min` tokens,
/// refilled evenly over a minute.
pub struct LookupClientRateLimiter {
    state: Mutex<ClientTable>,
    max_clients: usize,
}

struct ClientTable {
    buckets: HashMap<String, ClientBucket>,
    last_prune: Option<Instant>,
}

impl LookupClientRateLimiter {
    pub fn new(max_clients: usize) -> Self {
        Self {
            state: Mutex::new(ClientTable {
                buckets: HashMap::new(),
                last_prune: None,
            }),
            max_clients,
        }
    }

    /// Take one token from `client`'s bucket. False when it is empty or the
    /// rate is zero.
    pub fn try_acquire(&self, client: &str, rate_per_min: u32, now: Instant) -> bool {
        if rate_per_min == 0 {
            return false;
        }
        let cap = f64::from(rate_per_min);
        let refill_per_sec = cap / 60.0;
        let mut table = self.state.lock().expect("LookupClientRateLimiter poisoned");
        let mut key = client;
        if !table.buckets.contains_key(key) && table.buckets.len() >= self.max_clients {
            // Forget clients whose bucket has fully refilled: a fresh bucket
            // is indistinguishable from theirs. At most once a second, so a
            // flood of new keys does not rescan the table per request.
            let due = table
                .last_prune
                .is_none_or(|last| now.saturating_duration_since(last).as_secs() >= 1);
            if due {
                table.last_prune = Some(now);
                table.buckets.retain(|_, bucket| {
                    let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
                    bucket.tokens + elapsed * refill_per_sec < cap
                });
            }
            if table.buckets.len() >= self.max_clients {
                key = OVERFLOW_CLIENT_KEY;
            }
        }
        let bucket = table
            .buckets
            .entry(key.to_string())
            .or_insert(ClientBucket {
                tokens: cap,
                last: now,
            });
        let elapsed = now.saturating_duration_since(bucket.last).as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_per_sec).min(cap);
        bucket.last = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Clients currently tracked, overflow bucket included.
    pub fn tracked(&self) -> usize {
        self.state
            .lock()
            .expect("LookupClientRateLimiter poisoned")
            .buckets
            .len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trace_invite_registry::{InviteEntry, InviteTenantMode};
    use chrono::Duration;
    use std::time::Duration as StdDuration;

    fn peek(consumed: u32) -> InvitePeek {
        InvitePeek {
            entry: InviteEntry {
                invite_subject_hash: format!("sha256:{}", "a".repeat(64)),
                policy_label: "SECRET-POLICY".to_string(),
                tenant_mode: InviteTenantMode::Derived,
                fixed_tenant_id: None,
                tenant_template_id: Some("tmpl".to_string()),
                policy_version: "v1".to_string(),
                allowed_consent_scopes: Vec::new(),
                allowed_uses: Vec::new(),
                max_uses: 3,
                expires_at: None,
                issuance_source: "operator".to_string(),
                issued_by_label: Some("SECRET-ISSUER".to_string()),
                credential_binding_hash: None,
                note_label: Some("SECRET-NOTE".to_string()),
                issuer_display_name: Some("Trace Commons Pilot".to_string()),
                credit_range: Some((2, 6)),
                revoked_at: None,
            },
            consumed_uses: consumed,
        }
    }

    #[test]
    fn a_live_invite_names_the_issuer_and_range() {
        let response = classify_invite(Some(&peek(1)), Utc::now());
        assert!(response.valid);
        assert_eq!(
            response.issuer_display_name.as_deref(),
            Some("Trace Commons Pilot")
        );
        assert_eq!(response.credit_range, Some(CreditRange::points(2, 6)));
        assert_eq!(response.reason_label, None);
    }

    #[test]
    fn a_live_invite_without_public_fields_is_valid_and_bare() {
        let mut p = peek(0);
        p.entry.issuer_display_name = None;
        p.entry.credit_range = None;
        let response = classify_invite(Some(&p), Utc::now());
        assert!(response.valid);
        assert_eq!(response.issuer_display_name, None);
        assert_eq!(response.credit_range, None);
    }

    #[test]
    fn refusals_are_labelled() {
        let now = Utc::now();
        assert_eq!(
            classify_invite(None, now).reason_label.as_deref(),
            Some("not_found")
        );
        let mut revoked = peek(0);
        revoked.entry.revoked_at = Some(now);
        assert_eq!(
            classify_invite(Some(&revoked), now).reason_label.as_deref(),
            Some("revoked")
        );
        let mut expired = peek(0);
        expired.entry.expires_at = Some(now - Duration::seconds(1));
        assert_eq!(
            classify_invite(Some(&expired), now).reason_label.as_deref(),
            Some("expired")
        );
        assert_eq!(
            classify_invite(Some(&peek(3)), now).reason_label.as_deref(),
            Some("exhausted")
        );
        // Exactly at the boundary the last use is still available.
        assert!(classify_invite(Some(&peek(2)), now).valid);
    }

    #[test]
    fn revoked_outranks_expired_outranks_exhausted() {
        let now = Utc::now();
        let mut p = peek(3);
        p.entry.expires_at = Some(now - Duration::seconds(1));
        assert_eq!(
            classify_invite(Some(&p), now).reason_label.as_deref(),
            Some("expired")
        );
        p.entry.revoked_at = Some(now);
        assert_eq!(
            classify_invite(Some(&p), now).reason_label.as_deref(),
            Some("revoked")
        );
    }

    fn headers_with(values: &[&str]) -> HeaderMap {
        let mut headers = HeaderMap::new();
        for value in values {
            headers.append("x-forwarded-for", value.parse().unwrap());
        }
        headers
    }

    #[test]
    fn the_client_key_is_the_hop_the_nearest_proxy_wrote() {
        let name = HeaderName::from_static("x-forwarded-for");
        assert_eq!(
            lookup_client_key(&headers_with(&["203.0.113.7"]), &name),
            "203.0.113.7"
        );
        // A client-supplied prefix does not move the key: the trusted proxy
        // appends, so only the rightmost hop is its own observation.
        assert_eq!(
            lookup_client_key(&headers_with(&["198.51.100.1, 203.0.113.7"]), &name),
            "203.0.113.7"
        );
        assert_eq!(
            lookup_client_key(&headers_with(&["198.51.100.1", "203.0.113.7"]), &name),
            "203.0.113.7"
        );
    }

    #[test]
    fn ipv6_clients_are_keyed_by_their_slash_64() {
        let name = HeaderName::from_static("x-forwarded-for");
        let a = lookup_client_key(&headers_with(&["2001:db8:1:2:aaaa::1"]), &name);
        let b = lookup_client_key(&headers_with(&["2001:db8:1:2:ffff::9"]), &name);
        let other = lookup_client_key(&headers_with(&["2001:db8:1:3::1"]), &name);
        assert_eq!(a, b, "one /64 is one client");
        assert_ne!(a, other);
        assert_eq!(
            lookup_client_key(&headers_with(&["::ffff:203.0.113.7"]), &name),
            "203.0.113.7"
        );
    }

    #[test]
    fn a_missing_or_unparseable_header_shares_one_bucket() {
        let name = HeaderName::from_static("x-forwarded-for");
        assert_eq!(
            lookup_client_key(&HeaderMap::new(), &name),
            UNATTRIBUTED_CLIENT_KEY
        );
        for junk in ["", "unknown", "203.0.113.7:443", "not an ip"] {
            assert_eq!(
                lookup_client_key(&headers_with(&[junk]), &name),
                UNATTRIBUTED_CLIENT_KEY,
                "{junk:?}"
            );
        }
    }

    #[test]
    fn the_client_header_is_opt_in_and_validated() {
        assert_eq!(parse_client_ip_header(None).unwrap(), None);
        assert_eq!(parse_client_ip_header(Some("  ")).unwrap(), None);
        assert_eq!(
            parse_client_ip_header(Some("CF-Connecting-IP")).unwrap(),
            Some(HeaderName::from_static("cf-connecting-ip"))
        );
        assert!(parse_client_ip_header(Some("bad header")).is_err());
    }

    #[test]
    fn each_client_has_its_own_bucket() {
        let limiter = LookupClientRateLimiter::new(8);
        let t0 = Instant::now();
        assert!(limiter.try_acquire("a", 2, t0));
        assert!(limiter.try_acquire("a", 2, t0));
        assert!(!limiter.try_acquire("a", 2, t0));
        assert!(limiter.try_acquire("b", 2, t0));
        assert!(!limiter.try_acquire("c", 0, t0), "a zero rate refuses");
        // Refill: 2/min is one token every 30 seconds.
        assert!(limiter.try_acquire("a", 2, t0 + StdDuration::from_secs(31)));
    }

    #[test]
    fn tracked_clients_stay_bounded() {
        let limiter = LookupClientRateLimiter::new(4);
        let t0 = Instant::now();
        for i in 0..4 {
            assert!(limiter.try_acquire(&format!("c{i}"), 1, t0));
        }
        // Full, and every tracked client is mid-window: a newcomer shares
        // the overflow bucket instead of growing the map.
        assert!(limiter.try_acquire("new-1", 1, t0));
        assert!(!limiter.try_acquire("new-2", 1, t0));
        assert!(limiter.tracked() <= 5, "{}", limiter.tracked());
        // Once the tracked clients have fully refilled they are forgettable
        // and a newcomer gets a bucket of its own again.
        let later = t0 + StdDuration::from_secs(61);
        assert!(limiter.try_acquire("new-3", 1, later));
        assert!(limiter.try_acquire("new-4", 1, later));
        assert!(limiter.tracked() <= 5, "{}", limiter.tracked());
        for i in 0..10_000 {
            limiter.try_acquire(&format!("flood{i}"), 1, later);
        }
        assert!(limiter.tracked() <= 5, "{}", limiter.tracked());
    }

    #[test]
    fn no_response_leaks_operator_only_fields() {
        let now = Utc::now();
        let mut revoked = peek(0);
        revoked.entry.revoked_at = Some(now);
        for p in [peek(0), peek(3), revoked] {
            let body = serde_json::to_string(&classify_invite(Some(&p), now)).unwrap();
            for secret in ["SECRET-POLICY", "SECRET-ISSUER", "SECRET-NOTE", "max_uses"] {
                assert!(!body.contains(secret), "{secret} leaked in {body}");
            }
        }
    }
}
