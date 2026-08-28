// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Slice 2 passkey scaffolding: the WebAuthn relying-party instance and the
//! in-process ceremony store.
//!
//! This module owns the construction of the `webauthn_rs::Webauthn` relying party
//! from a validated [`WebauthnConfig`], plus the short-lived store that holds
//! in-flight ceremony state (registration / discoverable authentication) between
//! the challenge response and its verification.
//!
//! ## Single-instance limitation
//!
//! [`CeremonyStore`] keeps ceremony state in process memory ([`Mutex`] +
//! [`HashMap`]). It is therefore correct ONLY for a single-process deployment: a
//! challenge issued by one process cannot be completed by another, and state is
//! lost on restart. The pilot runs a single ingest process, so this is acceptable
//! for Slice 2. A multi-process or horizontally-scaled deployment MUST replace
//! this with a shared store (e.g. a short-TTL row keyed by ceremony id). The store
//! is instantiated and threaded via `AppState` (NOT a global static) so it is
//! injectable and unit-testable.
//!
//! The register/login/manage ceremonies and handlers that consume this scaffolding
//! land in later Slice 2 tasks; this module deliberately stops at construction.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use base64::Engine;
use webauthn_rs::prelude::{CredentialID, DiscoverableAuthentication, PasskeyRegistration, Url};
use webauthn_rs::{Webauthn, WebauthnBuilder};

use crate::config::WebauthnConfig;

/// Canonical string encoding for a WebAuthn `CredentialID` used as the stable
/// `credential_id` key in the `account_webauthn_credential` table.
///
/// The `CredentialID` is an opaque byte string. We pin ONE encoding — URL-safe
/// base64, no padding — so that every code path that handles a credential id
/// agrees byte-for-byte: enrollment stores this form, `exclude_credentials`
/// decodes back from it (Task 5), and the login/assertion path (Task 6) must
/// produce the SAME string from the asserted credential id to look up the row.
/// Centralizing the encoding here keeps those paths from drifting. The credential
/// id is a PUBLIC identifier (it is sent in the clear in every assertion), not a
/// secret, so storing/returning this string is safe.
pub fn credential_id_to_string(cred_id: &CredentialID) -> String {
    // `CredentialID` (== `HumanBinaryData`) derefs / `AsRef`s to the raw bytes.
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(cred_id.as_ref())
}

/// Inverse of [`credential_id_to_string`]: decode the canonical URL-safe-base64
/// (no pad) string back into a `CredentialID`. Used to rebuild
/// `exclude_credentials` from the account's stored credential ids during
/// registration. A malformed string surfaces as an `Err` rather than silently
/// producing a wrong id.
pub fn credential_id_from_string(s: &str) -> anyhow::Result<CredentialID> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(s)?;
    Ok(CredentialID::from(bytes))
}

/// Time-to-live for a stored ceremony. WebAuthn ceremonies are interactive and
/// short; three minutes is comfortably longer than a user takes to tap an
/// authenticator while still bounding how long stale challenge state survives.
pub const CEREMONY_TTL: Duration = Duration::from_secs(3 * 60);

/// Build the relying-party `Webauthn` instance from a validated [`WebauthnConfig`].
///
/// The `rp_origin` is parsed into a [`Url`]; an invalid origin (or an rp_id the
/// builder rejects) surfaces as an `Err`, so a misconfigured relying party fails
/// at startup rather than producing broken ceremonies. Uses the webauthn-rs 0.5
/// builder: `WebauthnBuilder::new(rp_id, &rp_origin)?.rp_name(rp_name).build()`.
pub fn build_webauthn(cfg: &WebauthnConfig) -> anyhow::Result<Webauthn> {
    let rp_origin = Url::parse(&cfg.rp_origin)?;
    let webauthn = WebauthnBuilder::new(&cfg.rp_id, &rp_origin)?
        .rp_name(&cfg.rp_name)
        .build()?;
    Ok(webauthn)
}

/// In-flight WebAuthn ceremony state held between challenge issuance and
/// verification.
///
/// The two variants carry the webauthn-rs 0.5 server-side state types that the
/// later Slice 2 tasks produce: [`PasskeyRegistration`] from the registration
/// challenge (Task 5) and [`DiscoverableAuthentication`] from the discoverable
/// login challenge (Task 6). Neither is constructible outside a real ceremony, so
/// unit tests exercise [`CeremonyStore`] over a stand-in type via its generic
/// parameter rather than constructing these variants directly.
pub enum CeremonyState {
    /// Pending passkey registration (consumed by `finish_passkey_registration`).
    Registration(PasskeyRegistration),
    /// Pending discoverable authentication (consumed by
    /// `finish_discoverable_authentication`).
    DiscoverableAuthentication(DiscoverableAuthentication),
    /// Pending NEAR sign-in (Slice 3a): the server-issued NEP-413 challenge
    /// nonce and the `recipient` the signed message must bind to. Issued by the
    /// NEAR login-begin handler and consumed by login-finish in later Slice 3a
    /// tasks. Unlike the WebAuthn variants this carries plain owned data, so it
    /// is directly constructible (and round-trippable through `CeremonyStore`).
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
/// Generic over the stored value `S` purely for testability: production code uses
/// `CeremonyStore<CeremonyState>`, while unit tests instantiate it over a simple
/// stand-in (the real [`CeremonyState`] variants are not constructible without a
/// live ceremony). See the module-level single-instance limitation.
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
    fn build_webauthn_succeeds_for_valid_config() {
        let cfg = WebauthnConfig {
            rp_id: "tracecommons.ai".to_string(),
            rp_origin: "https://app.tracecommons.ai".to_string(),
            rp_name: "TraceCommons".to_string(),
        };
        assert!(build_webauthn(&cfg).is_ok());
    }

    #[test]
    fn build_webauthn_errors_for_invalid_origin() {
        let cfg = WebauthnConfig {
            rp_id: "tracecommons.ai".to_string(),
            rp_origin: "not a url".to_string(),
            rp_name: "TraceCommons".to_string(),
        };
        assert!(build_webauthn(&cfg).is_err());
    }

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
    fn credential_id_round_trips_through_canonical_string() {
        // Arbitrary bytes including a high byte and a zero, to catch any
        // encoding that mangles non-ASCII or trailing nulls.
        let raw = vec![0x00u8, 0x01, 0x7f, 0x80, 0xff, 0x10, 0x20, 0x30];
        let cred_id = CredentialID::from(raw.clone());
        let encoded = credential_id_to_string(&cred_id);
        // No padding, URL-safe alphabet only.
        assert!(!encoded.contains('='));
        assert!(!encoded.contains('+') && !encoded.contains('/'));
        let decoded = credential_id_from_string(&encoded).expect("decodes");
        assert_eq!(decoded.as_ref(), raw.as_slice());
        // And the re-encoding is stable (canonical).
        assert_eq!(credential_id_to_string(&decoded), encoded);
    }

    #[test]
    fn store_round_trips_near_challenge_variant() {
        // The generic CeremonyStore must carry the new NearChallenge variant just
        // like any other CeremonyState value: put -> take returns it, single-use.
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
            Some(_) => panic!("expected NearChallenge, got a different CeremonyState"),
            None => panic!("expected NearChallenge, got None"),
        }
        // Single-use.
        assert!(store.take("near-1").is_none());
    }

    #[test]
    fn credential_id_from_string_rejects_malformed() {
        // '!' is outside the base64url alphabet.
        assert!(credential_id_from_string("not valid base64!").is_err());
    }
}
