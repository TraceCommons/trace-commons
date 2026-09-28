// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! An account's binding state (Z2, native passkey identity, slice S1).
//!
//! A passkey-origin account is created `unbound` and becomes `bound` in the
//! one transaction that attaches its near.ai identity. Until then the account
//! authenticates normally but reaches only a short allowlist of account routes
//! (the unbound gate in the ingest binary).
//!
//! **Absence of a binding row means legacy, not unbound.** Every account that
//! existed before this feature, and every account created by a path other than
//! passkey creation, has no row and is never gated. That makes the one
//! dangerous failure a passkey-origin account that somehow has no row: it
//! would read as legacy and skip the gate. The invariant that prevents it is
//! that a passkey-origin account row and its binding row are written in the
//! same transaction, by `PgBackend::insert_passkey_origin_account`, and by
//! nothing else. Do not insert a passkey-origin `trace_accounts` row any
//! other way.
//!
//! Every label here is a fixed, public string. None carries an identifier.

use std::fmt;

/// The binding state an authenticated account session carries, as read from
/// `trace_account_bindings` in the same query that validates the session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountBindingState {
    /// No binding row: an account that predates this feature or was created by
    /// a path other than passkey creation. Never gated.
    Legacy,
    /// A passkey-origin account that has not attached a near.ai identity. Gated.
    Unbound,
    /// A passkey-origin account whose near.ai identity is attached. Not gated.
    Bound,
    /// A passkey-origin account closed by the existing-account branch of bind.
    /// Gated: a closed account is not a bound one.
    Closed,
}

/// A stored `state` value this build does not know. The database CHECK makes
/// it unreachable today; if it is ever reached, the session is refused rather
/// than guessed at.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownBindingState;

impl fmt::Display for UnknownBindingState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("account_binding_state_unknown")
    }
}

impl std::error::Error for UnknownBindingState {}

impl AccountBindingState {
    /// Map the joined `trace_account_bindings.state` column. `None` (no row) is
    /// [`AccountBindingState::Legacy`]; an unrecognised value is an error.
    pub fn from_stored(state: Option<&str>) -> Result<Self, UnknownBindingState> {
        match state {
            None => Ok(Self::Legacy),
            Some("unbound") => Ok(Self::Unbound),
            Some("bound") => Ok(Self::Bound),
            Some("closed") => Ok(Self::Closed),
            Some(_) => Err(UnknownBindingState),
        }
    }

    /// The public label `GET /v1/account/binding` answers with.
    pub fn label(self) -> &'static str {
        match self {
            Self::Legacy => "legacy",
            Self::Unbound => "unbound",
            Self::Bound => "bound",
            Self::Closed => "closed",
        }
    }

    /// True when the session may reach only the unbound allowlist. Only
    /// `Legacy` and `Bound` are ungated; anything else is refused by default.
    pub fn is_gated(self) -> bool {
        !matches!(self, Self::Legacy | Self::Bound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_absent_row_is_legacy_and_never_gated() {
        let state = AccountBindingState::from_stored(None).expect("absence is legacy");
        assert_eq!(state, AccountBindingState::Legacy);
        assert!(!state.is_gated());
        assert_eq!(state.label(), "legacy");
    }

    #[test]
    fn stored_states_map_and_gate() {
        let cases = [
            ("unbound", AccountBindingState::Unbound, true),
            ("bound", AccountBindingState::Bound, false),
            ("closed", AccountBindingState::Closed, true),
        ];
        for (stored, expected, gated) in cases {
            let state = AccountBindingState::from_stored(Some(stored)).expect("known state");
            assert_eq!(state, expected);
            assert_eq!(state.is_gated(), gated, "{stored}");
            assert_eq!(state.label(), stored);
        }
    }

    #[test]
    fn an_unknown_stored_state_is_an_error_not_a_guess() {
        for stored in ["", "UNBOUND", "binding", "legacy"] {
            assert_eq!(
                AccountBindingState::from_stored(Some(stored)),
                Err(UnknownBindingState),
                "{stored:?}"
            );
        }
    }
}
