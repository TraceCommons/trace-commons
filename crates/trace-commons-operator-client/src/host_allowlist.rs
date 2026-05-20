//! Defense-in-depth host allowlist.
//!
//! Operator binaries that talk to trace-commons-server endpoints may also be
//! invoked against staging or testnet deployments. To avoid an operator
//! accidentally pointing a production-credentialed binary at an attacker-
//! controlled URL (e.g. via a poisoned env var), the [`Client`] enforces
//! that the target endpoint's host is on an explicit allowlist before any
//! request leaves the process.
//!
//! The allowlist is sourced from:
//!   1. The `--allowed-hosts <CSV>` CLI flag, or
//!   2. The `TRACE_COMMONS_ALLOWED_HOSTS` env var, or
//!   3. `None`, in which case host enforcement is OFF (legacy behavior).
//!
//! When the allowlist is set, host matching is exact (case-insensitive ASCII).
//! Wildcards and CIDRs are not supported — we want this list reviewable at a
//! glance.
//!
//! [`Client`]: crate::Client

use std::collections::BTreeSet;

use crate::error::{Error, Result};

#[derive(Debug, Clone, Default)]
pub struct HostAllowlist {
    hosts: Option<BTreeSet<String>>,
}

impl HostAllowlist {
    /// Allowlist that lets every host through. Equivalent to legacy behavior.
    pub fn permissive() -> Self {
        Self { hosts: None }
    }

    /// Construct from a CSV string like `"api.example,staging.example"`.
    /// Empty entries are dropped. If the resulting set is empty, returns
    /// a permissive allowlist (callers can detect via [`Self::is_enforcing`]).
    pub fn from_csv(csv: &str) -> Self {
        let hosts: BTreeSet<String> = csv
            .split(',')
            .map(|h| h.trim().to_ascii_lowercase())
            .filter(|h| !h.is_empty())
            .collect();
        if hosts.is_empty() {
            Self::permissive()
        } else {
            Self { hosts: Some(hosts) }
        }
    }

    /// Read the allowlist from the `TRACE_COMMONS_ALLOWED_HOSTS` env var.
    /// Returns a permissive allowlist if the var is unset or empty.
    pub fn from_env() -> Self {
        match std::env::var("TRACE_COMMONS_ALLOWED_HOSTS") {
            Ok(csv) => Self::from_csv(&csv),
            Err(_) => Self::permissive(),
        }
    }

    pub fn is_enforcing(&self) -> bool {
        self.hosts.is_some()
    }

    /// Confirm that a target URL's host is on the allowlist. No-op when the
    /// allowlist is permissive.
    pub fn check(&self, url: &url::Url) -> Result<()> {
        let Some(allowed) = &self.hosts else {
            return Ok(());
        };
        let host = url
            .host_str()
            .ok_or_else(|| Error::InvalidEndpoint {
                endpoint: url.to_string(),
                source: url::ParseError::EmptyHost,
            })?
            .to_ascii_lowercase();
        if allowed.contains(&host) {
            Ok(())
        } else {
            Err(Error::HostNotAllowed {
                host,
                allowed: allowed.iter().cloned().collect::<Vec<_>>().join(","),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> url::Url {
        url::Url::parse(s).expect("valid url in test")
    }

    #[test]
    fn permissive_allowlist_lets_anything_through() {
        let allowlist = HostAllowlist::permissive();
        assert!(!allowlist.is_enforcing());
        allowlist
            .check(&url("https://evil.example/v1/traces"))
            .unwrap();
    }

    #[test]
    fn csv_allowlist_enforces_exact_match() {
        let allowlist = HostAllowlist::from_csv("api.example,staging.example");
        assert!(allowlist.is_enforcing());
        allowlist
            .check(&url("https://api.example/v1/traces"))
            .unwrap();
        allowlist
            .check(&url("https://staging.example/v1/traces"))
            .unwrap();
    }

    #[test]
    fn csv_allowlist_rejects_unlisted_host() {
        let allowlist = HostAllowlist::from_csv("api.example");
        let err = allowlist
            .check(&url("https://evil.example/v1/traces"))
            .unwrap_err();
        assert_eq!(err.kind(), "host-not-allowed");
    }

    #[test]
    fn csv_allowlist_is_case_insensitive() {
        let allowlist = HostAllowlist::from_csv("API.Example");
        allowlist
            .check(&url("https://api.example/v1/traces"))
            .unwrap();
    }

    #[test]
    fn empty_csv_yields_permissive_allowlist() {
        let allowlist = HostAllowlist::from_csv("");
        assert!(!allowlist.is_enforcing());
    }

    #[test]
    fn csv_drops_empty_and_whitespace_entries() {
        let allowlist = HostAllowlist::from_csv(" , api.example , ,");
        assert!(allowlist.is_enforcing());
        allowlist
            .check(&url("https://api.example/v1/traces"))
            .unwrap();
    }

    #[test]
    fn subdomain_is_not_a_wildcard_match() {
        let allowlist = HostAllowlist::from_csv("api.example");
        let err = allowlist
            .check(&url("https://sub.api.example/v1/traces"))
            .unwrap_err();
        assert_eq!(err.kind(), "host-not-allowed");
    }
}
