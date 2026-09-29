// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Native passkey creation and sign-in (Z2 native passkey identity, slice S2):
//! the pieces that are not HTTP handlers.
//!
//! - [`UnboundAccountCeiling`], the operator's cap on how many passkey-origin
//!   accounts may sit `unbound` at once. Creation is unauthenticated and the
//!   attestation is `none`, so any caller may be a script; the ceiling bounds
//!   the rows such a caller can make. **Unset means creation is disabled**: a
//!   deployment opts in with a number (the pilot runs 5000, an operator
//!   setting, not a default here).
//! - [`normalize_passkey_label`], the bound on the one client-supplied string
//!   creation accepts.
//!
//! Every log line here is a fixed label. No count, no identifier.

use std::sync::atomic::{AtomicBool, Ordering};

/// The environment variable holding the unbound-account ceiling.
pub const UNBOUND_PASSKEY_ACCOUNT_CEILING_ENV: &str =
    "TRACE_COMMONS_UNBOUND_PASSKEY_ACCOUNT_CEILING";

/// The label logged once each time the unbound count reaches the ceiling.
/// Operators alert on it.
pub const UNBOUND_ACCOUNT_CEILING_REACHED: &str = "unbound_account_ceiling_reached";

/// Longest passkey label accepted at `create/start`, in characters.
pub const NATIVE_PASSKEY_LABEL_MAX_CHARS: usize = 64;

/// The unbound-account ceiling and its once-per-crossing log latch.
#[derive(Debug)]
pub struct UnboundAccountCeiling {
    limit: Option<i64>,
    /// True while the last observed count was at or over the limit, so the
    /// crossing is logged once rather than on every refused request.
    reached: AtomicBool,
}

/// The ceiling's answer for one observed count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CeilingCheck {
    /// Below the ceiling: creation may proceed.
    Open,
    /// At or over the ceiling, or no ceiling configured: refuse.
    Closed,
}

impl UnboundAccountCeiling {
    /// No ceiling configured: passkey creation is disabled.
    pub fn disabled() -> Self {
        Self {
            limit: None,
            reached: AtomicBool::new(false),
        }
    }

    /// A ceiling of `limit` unbound accounts. `0` admits none.
    pub fn with_limit(limit: i64) -> Self {
        Self {
            limit: Some(limit.max(0)),
            reached: AtomicBool::new(false),
        }
    }

    /// Parse the configured value. Unset or blank is `disabled`; a
    /// non-negative integer is a limit; anything else is an error, so a typo
    /// fails startup instead of silently disabling (or enabling) creation.
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        let Some(raw) = value.map(str::trim).filter(|raw| !raw.is_empty()) else {
            return Ok(Self::disabled());
        };
        match raw.parse::<i64>() {
            Ok(limit) if limit >= 0 => Ok(Self::with_limit(limit)),
            _ => Err(format!(
                "{UNBOUND_PASSKEY_ACCOUNT_CEILING_ENV} must be a non-negative integer"
            )),
        }
    }

    /// Read [`UNBOUND_PASSKEY_ACCOUNT_CEILING_ENV`].
    pub fn from_env() -> Result<Self, String> {
        Self::parse(
            std::env::var(UNBOUND_PASSKEY_ACCOUNT_CEILING_ENV)
                .ok()
                .as_deref(),
        )
    }

    /// The configured limit, `None` when creation is disabled.
    pub fn limit(&self) -> Option<i64> {
        self.limit
    }

    /// Compare an observed unbound count with the ceiling. Logs
    /// [`UNBOUND_ACCOUNT_CEILING_REACHED`] only on the transition from below
    /// to at-or-over, and re-arms when a later count is below again.
    pub fn check(&self, unbound: i64) -> CeilingCheck {
        let Some(limit) = self.limit else {
            return CeilingCheck::Closed;
        };
        if unbound >= limit {
            self.note_reached();
            CeilingCheck::Closed
        } else {
            self.reached.store(false, Ordering::Relaxed);
            CeilingCheck::Open
        }
    }

    /// Record that the ceiling was found reached (by [`Self::check`], or by
    /// the re-check inside the creation transaction). Returns true when this
    /// call is the crossing, i.e. the one that logged.
    pub fn note_reached(&self) -> bool {
        let crossed = !self.reached.swap(true, Ordering::Relaxed);
        if crossed {
            tracing::warn!(
                target: "trace_commons::passkey",
                label = UNBOUND_ACCOUNT_CEILING_REACHED,
                "{UNBOUND_ACCOUNT_CEILING_REACHED}"
            );
        }
        crossed
    }
}

/// Bound the optional label a caller gives its new passkey.
///
/// Blank is absent. A label longer than [`NATIVE_PASSKEY_LABEL_MAX_CHARS`]
/// characters, or holding a control character, is refused (`Err`) rather than
/// trimmed, so what the authenticator stores is exactly what was asked for.
/// The label becomes the WebAuthn user name the macOS sheet and keychain show
/// on the contributor's own device; it never reaches a log or an audit row.
pub fn normalize_passkey_label(label: Option<&str>) -> Result<Option<String>, InvalidPasskeyLabel> {
    let Some(label) = label.map(str::trim).filter(|label| !label.is_empty()) else {
        return Ok(None);
    };
    if label.chars().count() > NATIVE_PASSKEY_LABEL_MAX_CHARS || label.chars().any(char::is_control)
    {
        return Err(InvalidPasskeyLabel);
    }
    Ok(Some(label.to_string()))
}

/// A passkey label over the length bound or holding a control character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidPasskeyLabel;

impl std::fmt::Display for InvalidPasskeyLabel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid_passkey_label")
    }
}

impl std::error::Error for InvalidPasskeyLabel {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_or_blank_disables_creation() {
        for value in [None, Some(""), Some("   ")] {
            let ceiling = UnboundAccountCeiling::parse(value).expect("parses");
            assert_eq!(ceiling.limit(), None, "{value:?}");
            assert_eq!(ceiling.check(0), CeilingCheck::Closed, "{value:?}");
        }
    }

    #[test]
    fn a_malformed_value_is_an_error_not_a_default() {
        for value in ["five", "-1", "5000.0", "5_000", "0x10"] {
            assert!(
                UnboundAccountCeiling::parse(Some(value)).is_err(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn the_ceiling_admits_below_and_refuses_at_and_over() {
        let ceiling = UnboundAccountCeiling::parse(Some(" 5000 ")).expect("parses");
        assert_eq!(ceiling.limit(), Some(5000));
        assert_eq!(ceiling.check(4999), CeilingCheck::Open);
        assert_eq!(ceiling.check(5000), CeilingCheck::Closed);
        assert_eq!(ceiling.check(5001), CeilingCheck::Closed);
        let none = UnboundAccountCeiling::with_limit(0);
        assert_eq!(none.check(0), CeilingCheck::Closed);
    }

    #[test]
    fn the_crossing_is_noted_once_and_re_arms_below() {
        let ceiling = UnboundAccountCeiling::with_limit(2);
        assert_eq!(ceiling.check(1), CeilingCheck::Open);
        assert!(ceiling.note_reached(), "first crossing logs");
        assert!(!ceiling.note_reached(), "still reached: no second log");
        assert_eq!(ceiling.check(3), CeilingCheck::Closed);
        assert!(!ceiling.note_reached());
        assert_eq!(ceiling.check(1), CeilingCheck::Open);
        assert!(ceiling.note_reached(), "a new crossing logs again");
    }

    #[test]
    fn labels_are_bounded_not_trimmed_to_fit() {
        assert_eq!(normalize_passkey_label(None), Ok(None));
        assert_eq!(normalize_passkey_label(Some("  ")), Ok(None));
        assert_eq!(
            normalize_passkey_label(Some(" My trace passkey ")),
            Ok(Some("My trace passkey".to_string()))
        );
        let longest = "é".repeat(NATIVE_PASSKEY_LABEL_MAX_CHARS);
        assert_eq!(
            normalize_passkey_label(Some(&longest)),
            Ok(Some(longest.clone()))
        );
        assert_eq!(
            normalize_passkey_label(Some(&format!("{longest}x"))),
            Err(InvalidPasskeyLabel)
        );
        assert_eq!(
            normalize_passkey_label(Some("a\nb")),
            Err(InvalidPasskeyLabel)
        );
        assert_eq!(
            normalize_passkey_label(Some("a\u{7f}b")),
            Err(InvalidPasskeyLabel)
        );
    }
}
