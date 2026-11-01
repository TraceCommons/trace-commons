// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Server-observed network marks for the referral program
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, sections
//! 4 and 7, "Client is untrusted").
//!
//! The contributor client is open source, so nothing it sends is abuse
//! evidence (owner decision, 2026-10-29). What the server observes itself
//! is: the address the trusted edge saw a request come from. A mark is
//!
//! ```text
//! "sha256:" || hex(HMAC-SHA256(key, DOMAIN || canonical prefix))
//! ```
//!
//! over the network prefix of that address, IPv4 /24 or IPv6 /48 (an
//! IPv4-mapped IPv6 address is its IPv4 address). The key is the operator
//! secret `TRACE_COMMONS_REFERRAL_NETWORK_MARK_KEY` (base64, at least 32
//! bytes). It must be keyed: there are only 2^24 IPv4 /24s, so an unkeyed
//! hash of one is reversed by enumeration.
//!
//! An address that is not a public unicast address (loopback, private,
//! link-local, unique-local, multicast, unspecified, broadcast) gives no
//! prefix: a misconfigured edge that reports its own neighbour for every
//! caller must not make every pair of accounts look like one network.
//!
//! A shared mark between a referee and their referrer holds the referral
//! stream (`referral_same_network`). It never counts toward a code's
//! suspension: colleagues referring each other from one office network are
//! the main legitimate case. Marks are kept for the earning period plus
//! [`REFERRAL_NETWORK_MARK_GRACE_DAYS`].
//!
//! Nothing here logs. A prefix and a mark are never written to a log line,
//! an audit row or a response.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use base64::{Engine, engine::general_purpose::STANDARD};
use ring::hmac;
use zeroize::Zeroizing;

/// The operator secret the marks are keyed under, base64.
pub const REFERRAL_NETWORK_MARK_KEY_ENV: &str = "TRACE_COMMONS_REFERRAL_NETWORK_MARK_KEY";

/// HMAC accepts any key length; only this stops a short key a database
/// reader could search alongside the 2^24 /24s.
pub const REFERRAL_NETWORK_MARK_KEY_MIN_BYTES: usize = 32;

/// Marks are kept this many days past the earning period.
pub const REFERRAL_NETWORK_MARK_GRACE_DAYS: u32 = 30;

/// A refreshed mark's `last_seen_at` moves at most this often, so a busy
/// account's reads do not write on every request.
pub const REFERRAL_NETWORK_MARK_REFRESH_MINUTES: i64 = 60;

const MARK_DOMAIN: &[u8] = b"trace_commons.referral_network_mark.v1\n";

/// How long a mark is kept: the earning period plus the grace.
pub fn network_mark_retention_days(earn_days: u32) -> u32 {
    earn_days.saturating_add(REFERRAL_NETWORK_MARK_GRACE_DAYS)
}

/// Why the key was refused. The label names the control, never the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReferralNetworkMarkKeyError {
    /// The referral program or its bonus is configured and the key is not.
    #[error("referral_network_mark_key_missing")]
    Missing,
    /// The key is present but not base64 of at least 32 bytes.
    #[error("referral_network_mark_key_invalid")]
    Invalid,
}

impl ReferralNetworkMarkKeyError {
    pub fn label(self) -> &'static str {
        match self {
            Self::Missing => "referral_network_mark_key_missing",
            Self::Invalid => "referral_network_mark_key_invalid",
        }
    }

    /// The setting's name, safe to log beside the label.
    pub fn setting(self) -> &'static str {
        REFERRAL_NETWORK_MARK_KEY_ENV
    }
}

/// The prepared key. The raw bytes are dropped once it is built, and
/// nothing returns them; `Debug` prints nothing of it.
pub struct ReferralNetworkMarkKey {
    key: hmac::Key,
}

impl std::fmt::Debug for ReferralNetworkMarkKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReferralNetworkMarkKey")
            .finish_non_exhaustive()
    }
}

impl ReferralNetworkMarkKey {
    /// From raw key bytes, at least [`REFERRAL_NETWORK_MARK_KEY_MIN_BYTES`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, ReferralNetworkMarkKeyError> {
        if bytes.len() < REFERRAL_NETWORK_MARK_KEY_MIN_BYTES {
            return Err(ReferralNetworkMarkKeyError::Invalid);
        }
        Ok(Self {
            key: hmac::Key::new(hmac::HMAC_SHA256, bytes),
        })
    }

    /// From the operator's value. `required` is whether referrals are
    /// configured (a program start, or a bonus mode other than
    /// `disabled`): then an absent or empty key refuses the start, fail
    /// closed. A present value is checked whatever `required` says, so a
    /// typo is found before the program opens.
    pub fn from_env_value(
        raw: Option<&str>,
        required: bool,
    ) -> Result<Option<Self>, ReferralNetworkMarkKeyError> {
        let Some(raw) = raw.filter(|raw| !raw.is_empty()) else {
            return if required {
                Err(ReferralNetworkMarkKeyError::Missing)
            } else {
                Ok(None)
            };
        };
        if raw.trim() != raw {
            return Err(ReferralNetworkMarkKeyError::Invalid);
        }
        let bytes = Zeroizing::new(
            STANDARD
                .decode(raw)
                .map_err(|_| ReferralNetworkMarkKeyError::Invalid)?,
        );
        Self::from_bytes(&bytes).map(Some)
    }

    /// [`Self::from_env_value`] over the process environment. A value that
    /// is not Unicode is malformed, never unset.
    pub fn from_env(required: bool) -> Result<Option<Self>, ReferralNetworkMarkKeyError> {
        match std::env::var(REFERRAL_NETWORK_MARK_KEY_ENV) {
            Ok(value) => Self::from_env_value(Some(&value), required),
            Err(std::env::VarError::NotPresent) => Self::from_env_value(None, required),
            Err(std::env::VarError::NotUnicode(_)) => Err(ReferralNetworkMarkKeyError::Invalid),
        }
    }

    /// The mark of a canonical prefix (from [`canonical_network_prefix`]).
    pub fn mark(&self, canonical_prefix: &str) -> String {
        let mut context = hmac::Context::with_key(&self.key);
        context.update(MARK_DOMAIN);
        context.update(canonical_prefix.as_bytes());
        format!("sha256:{}", hex::encode(context.sign().as_ref()))
    }

    /// The mark of the network `address` is in, or `None` when it is not a
    /// public unicast address.
    pub fn mark_for_address(&self, address: IpAddr) -> Option<String> {
        canonical_network_prefix(address).map(|prefix| self.mark(&prefix))
    }
}

fn public_v4(address: Ipv4Addr) -> bool {
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_private()
        || address.is_link_local()
        || address.is_broadcast()
        || address.is_multicast()
        || address.octets()[0] == 0)
}

fn public_v6(address: Ipv6Addr) -> bool {
    let first = address.segments()[0];
    !(address.is_unspecified()
        || address.is_loopback()
        || address.is_multicast()
        // fc00::/7, unique local.
        || (first & 0xfe00) == 0xfc00
        // fe80::/10, link local.
        || (first & 0xffc0) == 0xfe80)
}

/// The canonical, domain-tagged network prefix of `address`: `v4:a.b.c.0/24`
/// or `v6:<first 48 bits>::/48`. An IPv4-mapped IPv6 address is its IPv4
/// address. `None` for an address that is not public unicast.
pub fn canonical_network_prefix(address: IpAddr) -> Option<String> {
    let v4 = |address: Ipv4Addr| {
        public_v4(address).then(|| {
            let [a, b, c, _] = address.octets();
            format!("v4:{a}.{b}.{c}.0/24")
        })
    };
    match address {
        IpAddr::V4(address) => v4(address),
        IpAddr::V6(address) => match address.to_ipv4_mapped() {
            Some(mapped) => v4(mapped),
            None => public_v6(address).then(|| {
                let s = address.segments();
                let prefix = Ipv6Addr::new(s[0], s[1], s[2], 0, 0, 0, 0, 0);
                format!("v6:{prefix}/48")
            }),
        },
    }
}

/// [`canonical_network_prefix`] of an address as text, `None` for anything
/// that does not parse.
pub fn canonical_network_prefix_of(hop: &str) -> Option<String> {
    hop.trim()
        .parse::<IpAddr>()
        .ok()
        .and_then(canonical_network_prefix)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(byte: u8) -> ReferralNetworkMarkKey {
        ReferralNetworkMarkKey::from_bytes(&[byte; 32]).unwrap()
    }

    #[test]
    fn ipv4_is_its_slash_24() {
        assert_eq!(
            canonical_network_prefix_of("203.0.113.77").as_deref(),
            Some("v4:203.0.113.0/24")
        );
        assert_eq!(
            canonical_network_prefix_of("203.0.113.1"),
            canonical_network_prefix_of("203.0.113.254"),
        );
        assert_ne!(
            canonical_network_prefix_of("203.0.113.1"),
            canonical_network_prefix_of("203.0.114.1"),
        );
        assert_eq!(
            canonical_network_prefix_of(" 198.51.100.9 ").as_deref(),
            Some("v4:198.51.100.0/24")
        );
    }

    #[test]
    fn ipv6_is_its_slash_48() {
        assert_eq!(
            canonical_network_prefix_of("2001:db8:1234:5678::1").as_deref(),
            Some("v6:2001:db8:1234::/48")
        );
        assert_eq!(
            canonical_network_prefix_of("2001:db8:1234:ffff:1:2:3:4"),
            canonical_network_prefix_of("2001:db8:1234::9"),
            "addresses in one /48 share a prefix"
        );
        assert_ne!(
            canonical_network_prefix_of("2001:db8:1234::1"),
            canonical_network_prefix_of("2001:db8:1235::1"),
        );
    }

    #[test]
    fn a_v4_mapped_v6_address_is_its_v4_prefix() {
        assert_eq!(
            canonical_network_prefix_of("::ffff:203.0.113.77").as_deref(),
            Some("v4:203.0.113.0/24")
        );
        let key = key(1);
        assert_eq!(
            key.mark_for_address("::ffff:203.0.113.77".parse().unwrap()),
            key.mark_for_address("203.0.113.5".parse().unwrap()),
        );
    }

    #[test]
    fn garbage_and_non_public_addresses_give_none() {
        for hop in [
            "",
            "unattributed",
            "203.0.113",
            "203.0.113.0/24",
            "999.1.1.1",
            "2001:db8::zz",
            "localhost",
            "127.0.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "169.254.1.1",
            "255.255.255.255",
            "224.0.0.1",
            "::",
            "::1",
            "fe80::1",
            "fd12:3456::1",
            "ff02::1",
            "::ffff:10.0.0.1",
            "::ffff:127.0.0.1",
        ] {
            assert_eq!(canonical_network_prefix_of(hop), None, "{hop:?}");
        }
    }

    #[test]
    fn marks_are_deterministic_shaped_and_separated_by_key() {
        let one = key(1);
        let prefix = canonical_network_prefix_of("203.0.113.77").unwrap();
        let mark = one.mark(&prefix);
        assert_eq!(mark, key(1).mark(&prefix), "same key, same mark");
        assert_eq!(mark.len(), "sha256:".len() + 64);
        assert!(mark.starts_with("sha256:"));
        assert!(
            mark["sha256:".len()..]
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit() && !byte.is_ascii_uppercase())
        );
        assert_ne!(mark, key(2).mark(&prefix), "another key, another mark");
        assert_ne!(
            mark,
            one.mark(&canonical_network_prefix_of("203.0.114.77").unwrap()),
            "another network, another mark"
        );
        // Keyed and domain-separated: not the bare digest of the prefix.
        use sha2::{Digest, Sha256};
        assert_ne!(
            mark,
            format!("sha256:{:x}", Sha256::digest(prefix.as_bytes()))
        );
        assert!(!mark.contains("203"));
    }

    #[test]
    fn the_key_is_fail_closed_when_referrals_are_configured() {
        let good = STANDARD.encode([7u8; 32]);
        assert!(
            ReferralNetworkMarkKey::from_env_value(None, false)
                .unwrap()
                .is_none()
        );
        assert!(
            ReferralNetworkMarkKey::from_env_value(Some(""), false)
                .unwrap()
                .is_none()
        );
        assert_eq!(
            ReferralNetworkMarkKey::from_env_value(None, true).unwrap_err(),
            ReferralNetworkMarkKeyError::Missing
        );
        assert_eq!(
            ReferralNetworkMarkKey::from_env_value(Some(""), true).unwrap_err(),
            ReferralNetworkMarkKeyError::Missing
        );
        for bad in [
            "not base64!!".to_owned(),
            STANDARD.encode([7u8; 31]),
            format!(" {good}"),
        ] {
            for required in [false, true] {
                assert_eq!(
                    ReferralNetworkMarkKey::from_env_value(Some(&bad), required).unwrap_err(),
                    ReferralNetworkMarkKeyError::Invalid,
                    "{bad:?}"
                );
            }
        }
        assert!(
            ReferralNetworkMarkKey::from_env_value(Some(&good), true)
                .unwrap()
                .is_some()
        );
        assert_eq!(
            ReferralNetworkMarkKeyError::Missing.label(),
            "referral_network_mark_key_missing"
        );
        assert_eq!(
            ReferralNetworkMarkKeyError::Invalid.setting(),
            REFERRAL_NETWORK_MARK_KEY_ENV
        );
    }

    #[test]
    fn the_key_prints_nothing() {
        let printed = format!("{:?}", key(9));
        assert_eq!(printed, "ReferralNetworkMarkKey { .. }");
    }

    #[test]
    fn retention_is_the_earning_period_plus_thirty_days() {
        assert_eq!(network_mark_retention_days(92), 122);
        assert_eq!(network_mark_retention_days(u32::MAX), u32::MAX);
    }
}
