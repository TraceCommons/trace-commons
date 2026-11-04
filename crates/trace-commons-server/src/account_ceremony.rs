// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The in-process ceremony store for challenge/response sign-in flows.
//!
//! A ceremony is the short-lived state held between a challenge response and
//! its verification: today the NEAR wallet sign-in challenge
//! ([`CeremonyState::NearChallenge`]) and, through the generic parameter, the
//! NEAR legion and celestine sloth claim challenges and the native loopback
//! sign-in flows.
//!
//! ## Single-instance limitation
//!
//! [`CeremonyStore`] keeps ceremony state in process memory ([`Mutex`] +
//! [`HashMap`]). It is therefore correct ONLY for a single-process deployment: a
//! challenge issued by one process cannot be completed by another, and state is
//! lost on restart. The pilot runs a single ingest process, so this is
//! acceptable. A multi-process or horizontally-scaled deployment MUST replace
//! this with a shared store (e.g. a short-TTL row keyed by ceremony id). The store
//! is instantiated and threaded via `AppState` (NOT a global static) so it is
//! injectable and unit-testable.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Time-to-live for a stored ceremony. Ceremonies are interactive and short;
/// three minutes is comfortably longer than a user takes to approve a sign-in
/// while still bounding how long stale challenge state survives.
pub const CEREMONY_TTL: Duration = Duration::from_secs(3 * 60);

/// In-flight ceremony state held between challenge issuance and verification.
pub enum CeremonyState {
    /// Pending NEAR sign-in (Slice 3a): the server-issued NEP-413 challenge
    /// nonce and the `recipient` the signed message must bind to. Issued by the
    /// NEAR login-begin handler and consumed by login-finish.
    NearChallenge {
        /// 32-byte server nonce the wallet signs over.
        nonce: [u8; 32],
        /// NEP-413 `recipient` the signed message must bind to.
        recipient: String,
    },
}

/// Generate a fresh, high-entropy ceremony id.
///
/// Reuses the Slice 1 CSPRNG (`account_session::generate_login_code`, 160-bit
/// OsRng, URL-safe base64) so ceremony ids are unguessable and url/cookie-safe.
pub fn new_ceremony_id() -> String {
    crate::account_session::generate_login_code()
}

/// Single-use, TTL-bounded in-process store for in-flight ceremony state.
///
/// Generic over the stored value `S`: the NEAR sign-in uses
/// `CeremonyStore<CeremonyState>`, the claim surfaces store their own challenge
/// types, and unit tests use a simple stand-in. See the module-level
/// single-instance limitation.
pub struct CeremonyStore<S = CeremonyState> {
    entries: Mutex<HashMap<String, (S, Instant)>>,
    ttl: Duration,
}

impl<S> Default for CeremonyStore<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S> CeremonyStore<S> {
    /// Create an empty store with the default [`CEREMONY_TTL`].
    pub fn new() -> Self {
        Self::with_ttl(CEREMONY_TTL)
    }

    /// Create an empty store with an explicit TTL (used by tests).
    pub fn with_ttl(ttl: Duration) -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
            ttl,
        }
    }

    /// Store `state` under `id`, stamped with the current time. Opportunistically
    /// drops already-expired entries while the lock is held.
    pub fn put(&self, id: String, state: S) {
        let now = Instant::now();
        let mut guard = self.entries.lock().expect("ceremony store mutex poisoned");
        let ttl = self.ttl;
        guard.retain(|_, (_, inserted)| now.duration_since(*inserted) < ttl);
        guard.insert(id, (state, now));
    }

    /// Remove and return the state for `id` if present and not expired.
    ///
    /// Single-use: the entry is removed on the first successful `take`, so a
    /// second `take` of the same id returns `None`. An entry older than the TTL is
    /// dropped and treated as absent. Also opportunistically GCs other expired
    /// entries while the lock is held.
    pub fn take(&self, id: &str) -> Option<S> {
        let now = Instant::now();
        let mut guard = self.entries.lock().expect("ceremony store mutex poisoned");
        let ttl = self.ttl;
        // Opportunistic GC of unrelated expired entries.
        guard.retain(|_, (_, inserted)| now.duration_since(*inserted) < ttl);
        let (state, inserted) = guard.remove(id)?;
        // Belt-and-suspenders: the `retain` above already GC'd every expired entry,
        // so a survivor here is necessarily fresh. This second check guards a future
        // refactor that drops the opportunistic `retain` — the single-use TTL bound
        // then still holds.
        if now.duration_since(inserted) >= ttl {
            return None;
        }
        Some(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_ceremony_id_is_high_entropy_and_unique() {
        let a = new_ceremony_id();
        let b = new_ceremony_id();
        assert_ne!(a, b);
        assert!(a.len() >= 20, "ceremony id should be high-entropy");
    }

    #[test]
    fn store_put_then_take_returns_value() {
        let store: CeremonyStore<u32> = CeremonyStore::new();
        store.put("id-1".to_string(), 42);
        assert_eq!(store.take("id-1"), Some(42));
    }

    #[test]
    fn store_take_is_single_use() {
        let store: CeremonyStore<u32> = CeremonyStore::new();
        store.put("id-1".to_string(), 42);
        assert_eq!(store.take("id-1"), Some(42));
        assert_eq!(store.take("id-1"), None);
    }

    #[test]
    fn store_drops_expired_entries() {
        let store: CeremonyStore<u32> = CeremonyStore::with_ttl(Duration::ZERO);
        store.put("id-1".to_string(), 42);
        assert_eq!(store.take("id-1"), None);
    }

    #[test]
    fn store_missing_id_returns_none() {
        let store: CeremonyStore<u32> = CeremonyStore::new();
        assert_eq!(store.take("absent"), None);
    }

    #[test]
    fn store_round_trips_near_challenge_variant() {
        let store: CeremonyStore<CeremonyState> = CeremonyStore::new();
        let nonce = [7u8; 32];
        let recipient = "app.tracecommons.ai".to_string();
        store.put(
            "near-1".to_string(),
            CeremonyState::NearChallenge {
                nonce,
                recipient: recipient.clone(),
            },
        );
        match store.take("near-1") {
            Some(CeremonyState::NearChallenge {
                nonce: got_nonce,
                recipient: got_recipient,
            }) => {
                assert_eq!(got_nonce, nonce);
                assert_eq!(got_recipient, recipient);
            }
            None => panic!("expected NearChallenge, got None"),
        }
        // Single-use.
        assert!(store.take("near-1").is_none());
    }
}
