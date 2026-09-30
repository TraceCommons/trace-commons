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
//! - [`PerSourceCreationCap`], the per-client-IP cap on successful creations
//!   in a rolling 24 hours. The per-minute limits alone would let a handful of
//!   addresses fill the ceiling in minutes; this makes filling it take many
//!   addresses and days.
//!
//! Every log line here is a fixed label. No count, no identifier.

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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

/// The environment variable holding the per-IP daily creation cap.
pub const NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY_ENV: &str =
    "TRACE_COMMONS_NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY";

/// Successful native passkey creations one client IP may make in
/// [`NATIVE_PASSKEY_CREATION_CAP_WINDOW`] when the operator sets nothing.
pub const DEFAULT_NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY: u32 = 10;

/// The rolling window the per-IP creation cap counts over.
pub const NATIVE_PASSKEY_CREATION_CAP_WINDOW: Duration = Duration::from_secs(24 * 60 * 60);

/// The most distinct sources the cap tracks at once. Only a SUCCESSFUL
/// creation adds a source, and successful creations are themselves bounded by
/// the unbound-account ceiling, so this is a backstop far above what the
/// ceiling admits in a day. At the bound, after expired entries are dropped,
/// a new source is refused (fail closed) rather than an old one forgotten.
pub const NATIVE_PASSKEY_CREATION_CAP_MAX_SOURCES: usize = 100_000;

/// A per-source cap on successful native passkey creations in a rolling
/// window.
///
/// **In process, by design.** Every other limiter on these routes is in
/// process, and this one shares their single-instance assumption. Keeping it
/// out of PostgreSQL means nothing derived from a client IP is ever written
/// anywhere: no migration, no stored keyed hash, no key to provision or
/// rotate. The cost is that a restart clears it, granting each address at most
/// one more cap's worth; restarts are operator actions, and the unbound-account
/// ceiling, which is in the database, stays the hard bound on rows.
///
/// Sources are held as a salted SHA-256 of the client IP, with a salt drawn
/// from the OS RNG per process, so the map never holds an address and the
/// memory per source is fixed whatever the caller sent. Memory is bounded by
/// [`NATIVE_PASSKEY_CREATION_CAP_MAX_SOURCES`] sources of at most `limit`
/// instants each.
///
/// A creation takes a [`CreationReservation`] BEFORE its database write and
/// commits it only on success; a reservation dropped uncommitted gives its
/// slot back. Concurrent finishes from one source therefore cannot overshoot.
/// A poisoned lock refuses.
#[derive(Debug)]
pub struct PerSourceCreationCap {
    limit: u32,
    window: Duration,
    max_sources: usize,
    salt: [u8; 32],
    sources: Mutex<HashMap<[u8; 32], VecDeque<Instant>>>,
}

impl PerSourceCreationCap {
    /// A cap of `limit` successful creations per source per window. `0`
    /// admits none.
    pub fn with_limit(limit: u32) -> Self {
        Self::with_bounds(
            limit,
            NATIVE_PASSKEY_CREATION_CAP_WINDOW,
            NATIVE_PASSKEY_CREATION_CAP_MAX_SOURCES,
        )
    }

    /// As [`Self::with_limit`] with an explicit window and source bound.
    pub fn with_bounds(limit: u32, window: Duration, max_sources: usize) -> Self {
        use rand::RngCore;
        let mut salt = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut salt);
        Self {
            limit,
            window,
            max_sources,
            salt,
            sources: Mutex::new(HashMap::new()),
        }
    }

    /// Parse the configured value. Unset or blank is
    /// [`DEFAULT_NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY`]; a non-negative
    /// integer is the limit; anything else is an error, so a typo fails
    /// startup instead of silently lifting (or tightening) the cap.
    pub fn parse(value: Option<&str>) -> Result<Self, String> {
        let Some(raw) = value.map(str::trim).filter(|raw| !raw.is_empty()) else {
            return Ok(Self::with_limit(
                DEFAULT_NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY,
            ));
        };
        raw.parse::<u32>().map(Self::with_limit).map_err(|_| {
            format!("{NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY_ENV} must be a non-negative integer")
        })
    }

    /// Read [`NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY_ENV`].
    pub fn from_env() -> Result<Self, String> {
        Self::parse(
            std::env::var(NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY_ENV)
                .ok()
                .as_deref(),
        )
    }

    /// The configured per-source limit.
    pub fn limit(&self) -> u32 {
        self.limit
    }

    fn key(&self, source: &str) -> [u8; 32] {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(self.salt);
        hasher.update(source.as_bytes());
        hasher.finalize().into()
    }

    /// Take one slot for `source` at `now`, or `None` when the source has
    /// already made `limit` creations within the window ending at `now` (or
    /// the source table is full, or the lock is poisoned). Slots that were
    /// taken but not committed count until they are dropped.
    pub fn try_reserve(&self, source: &str, now: Instant) -> Option<CreationReservation<'_>> {
        let key = self.key(source);
        let mut sources = self.sources.lock().ok()?;
        let window = self.window;
        let live = |at: &Instant| now.saturating_duration_since(*at) < window;
        if !sources.contains_key(&key) && sources.len() >= self.max_sources {
            sources.retain(|_, times| {
                times.retain(live);
                !times.is_empty()
            });
            if sources.len() >= self.max_sources {
                return None;
            }
        }
        let times = sources.entry(key).or_default();
        times.retain(live);
        if times.len() >= self.limit as usize {
            if times.is_empty() {
                sources.remove(&key);
            }
            return None;
        }
        times.push_back(now);
        Some(CreationReservation {
            cap: self,
            key,
            at: now,
            committed: false,
        })
    }

    /// How many sources are tracked now. For tests and diagnostics; no
    /// source is identifiable from it.
    pub fn tracked_sources(&self) -> usize {
        self.sources.lock().map_or(0, |sources| sources.len())
    }

    fn release(&self, key: &[u8; 32], at: Instant) {
        let Ok(mut sources) = self.sources.lock() else {
            return;
        };
        if let Some(times) = sources.get_mut(key) {
            if let Some(index) = times.iter().position(|taken| *taken == at) {
                times.remove(index);
            }
            if times.is_empty() {
                sources.remove(key);
            }
        }
    }
}

/// One slot of a [`PerSourceCreationCap`], held across a creation. Dropped
/// without [`Self::commit`], it gives the slot back.
#[derive(Debug)]
#[must_use = "an uncommitted reservation releases its slot when dropped"]
pub struct CreationReservation<'a> {
    cap: &'a PerSourceCreationCap,
    key: [u8; 32],
    at: Instant,
    committed: bool,
}

impl CreationReservation<'_> {
    /// The creation succeeded: the slot stays taken for the window.
    pub fn commit(mut self) {
        self.committed = true;
    }
}

impl Drop for CreationReservation<'_> {
    fn drop(&mut self) {
        if !self.committed {
            self.cap.release(&self.key, self.at);
        }
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

    #[test]
    fn the_creation_cap_defaults_to_ten_and_refuses_typos() {
        for value in [None, Some(""), Some("  ")] {
            let cap = PerSourceCreationCap::parse(value).expect("parses");
            assert_eq!(cap.limit(), DEFAULT_NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY);
        }
        assert_eq!(DEFAULT_NATIVE_PASSKEY_CREATIONS_PER_IP_PER_DAY, 10);
        assert_eq!(PerSourceCreationCap::parse(Some(" 3 ")).unwrap().limit(), 3);
        for value in ["ten", "-1", "2.5", "1_0"] {
            assert!(
                PerSourceCreationCap::parse(Some(value)).is_err(),
                "{value:?}"
            );
        }
    }

    #[test]
    fn the_creation_cap_trips_at_n_plus_one_for_one_source_only() {
        let cap = PerSourceCreationCap::with_limit(10);
        let now = Instant::now();
        for n in 1..=10 {
            cap.try_reserve("203.0.113.7", now)
                .unwrap_or_else(|| panic!("creation {n} is within the cap"))
                .commit();
        }
        assert!(
            cap.try_reserve("203.0.113.7", now).is_none(),
            "creation 11 from the same source is refused"
        );
        assert!(
            cap.try_reserve("198.51.100.9", now).is_some(),
            "another source is unaffected"
        );
    }

    #[test]
    fn the_creation_cap_window_rolls() {
        let window = Duration::from_secs(100);
        let cap = PerSourceCreationCap::with_bounds(2, window, 16);
        let t0 = Instant::now();
        cap.try_reserve("a", t0).expect("first").commit();
        cap.try_reserve("a", t0 + Duration::from_secs(50))
            .expect("second")
            .commit();
        assert!(
            cap.try_reserve("a", t0 + Duration::from_secs(99)).is_none(),
            "both still inside the window"
        );
        // The first creation leaves the window; exactly one slot frees.
        cap.try_reserve("a", t0 + Duration::from_secs(100))
            .expect("the oldest has rolled out")
            .commit();
        assert!(
            cap.try_reserve("a", t0 + Duration::from_secs(120))
                .is_none(),
            "the second and third are still inside"
        );
    }

    #[test]
    fn an_uncommitted_reservation_gives_its_slot_back() {
        let cap = PerSourceCreationCap::with_limit(1);
        let now = Instant::now();
        let held = cap.try_reserve("a", now).expect("first");
        assert!(
            cap.try_reserve("a", now).is_none(),
            "a held slot counts, so concurrent finishes cannot overshoot"
        );
        drop(held);
        assert_eq!(cap.tracked_sources(), 0, "a released source is forgotten");
        cap.try_reserve("a", now)
            .expect("the slot came back")
            .commit();
        assert!(cap.try_reserve("a", now).is_none());
    }

    #[test]
    fn the_source_table_is_bounded_and_fails_closed() {
        let window = Duration::from_secs(100);
        let cap = PerSourceCreationCap::with_bounds(5, window, 2);
        let t0 = Instant::now();
        cap.try_reserve("a", t0).expect("a").commit();
        cap.try_reserve("b", t0).expect("b").commit();
        assert!(
            cap.try_reserve("c", t0).is_none(),
            "full: a new source is refused"
        );
        cap.try_reserve("a", t0)
            .expect("a known source still counts")
            .commit();
        // Once the old entries expire they are dropped to make room.
        cap.try_reserve("c", t0 + window)
            .expect("expired sources are collected")
            .commit();
        assert_eq!(cap.tracked_sources(), 1);
    }

    #[test]
    fn a_zero_cap_admits_none() {
        let cap = PerSourceCreationCap::with_limit(0);
        assert!(cap.try_reserve("a", Instant::now()).is_none());
        assert_eq!(cap.tracked_sources(), 0);
    }
}
