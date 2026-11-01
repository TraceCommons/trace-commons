// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use base64::Engine;
use rand::RngCore;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;

/// Durable pseudonymous account id (locally-owned UUID).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccountId(uuid::Uuid);
impl AccountId {
    pub fn from_uuid(u: uuid::Uuid) -> Self {
        Self(u)
    }
    pub fn as_uuid(&self) -> uuid::Uuid {
        self.0
    }
}

/// The ONLY ownership-bearing principal set for the account read surface.
/// Producible only within this module (and by `AccountCtx` later), never
/// convertible into the `&TenantAuth` shape the legacy
/// `visible_submission_records` helper wants.
#[derive(Debug, Clone)]
pub struct AccountPrincipalSet(BTreeSet<String>);
impl AccountPrincipalSet {
    /// The ONLY way to mint a principal set, and `pub(crate)` so only this crate
    /// can call it. The single sanctioned mint site is
    /// `Database::expand_account_principals`, which feeds it the result of the
    /// active-membership expansion (`unlinked_at IS NULL`, Hardening A). The bins
    /// receive an already-built `AccountPrincipalSet` from that method and cannot
    /// construct one themselves: an ownership-bearing set is producible only
    /// inside the lib (Hardening C). The type's consumers (`AccountCtx`, the
    /// account visibility predicate) cannot be coerced into the legacy
    /// `&TenantAuth` surface.
    // Intentionally named `from_iter` (the slice's chosen vocabulary) without
    // implementing `std::iter::FromIterator`: a `FromIterator` impl would make
    // the set constructible by anything in scope via `.collect()`, eroding the
    // "only the lib mints a set" guarantee. Local allow only; not a CI
    // allow-list change.
    #[allow(clippy::should_implement_trait)]
    pub(crate) fn from_iter<I: IntoIterator<Item = String>>(it: I) -> Self {
        Self(it.into_iter().collect())
    }

    /// TEST-SUPPORT mint. The production `from_iter` is `pub(crate)` and so is
    /// unreachable from the binary crate; this helper exists ONLY so the binary
    /// crate's `#[cfg(test)]` unit tests can build a set to exercise the
    /// binary-local account visibility predicate. It cannot be `#[cfg(test)]`-
    /// gated because the lib is compiled WITHOUT its own test cfg when the bin
    /// crate's tests are built, so a cfg-gated item would be invisible there;
    /// and a Cargo feature would not be enabled under the repo's pinned plain
    /// `cargo test --no-run` gate. It is therefore `pub` but `#[doc(hidden)]`
    /// and loudly named: production bin code constructs an `AccountPrincipalSet`
    /// ONLY via `Database::expand_account_principals` (the single sanctioned
    /// mint site, Hardening A/C); calling this from non-test code would be an
    /// obvious review red flag. Grep-guarded by convention, not the type system.
    #[doc(hidden)]
    #[allow(clippy::should_implement_trait)]
    pub fn from_iter_for_test_only<I: IntoIterator<Item = String>>(it: I) -> Self {
        Self::from_iter(it)
    }
    pub fn contains(&self, principal_ref: &str) -> bool {
        self.0.contains(principal_ref)
    }
    pub fn to_vec(&self) -> Vec<String> {
        self.0.iter().cloned().collect()
    }
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Actor/audit ref for the cookie path. Reserved-prefix literal, NOT hashed,
/// so it is structurally incapable of equalling any `principal_<sha>` ref.
pub fn account_actor_ref(account: &AccountId) -> String {
    format!("account-actor:{}", account.as_uuid())
}

/// Which credential the `AccountCtx` resolver accepted. The cookie path is a
/// low-privilege contributor surface and carries NO review/admin capability by
/// construction (`AccountCtx` exposes only `account_id` + `principal_set`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccountAuthMethod {
    /// Browser session cookie (`__Host-tc_account_session`) — contributor-only.
    SessionCookie,
    /// Device bearer token resolved to its linked account.
    DeviceBearer,
    /// Native-app bearer token (`tcn1_...`) minted by the loopback sign-in
    /// flow and backed by a real `trace_sessions` row (`client_kind =
    /// 'native'`). It is a SESSION, not a device credential: it expires, it
    /// rotates on use, and `revoke-all` kills it. It is deliberately WEAK for
    /// the strong-authenticator gate — a native token can read and withdraw,
    /// never change authenticators or redirect the NEAR wallet payout, except
    /// for the first `NEAR_AI_LOGIN_FRESH_SECS` after a NEAR AI sign-in minted
    /// it (`AccountCtx::fresh_near_ai_login`). It
    /// may link a payout rail's recipient (the NEAR AI org): the owner chose
    /// the signed-in session as that authority on 2026-10-23, behind the
    /// server's own ownership check with NEAR AI (payout spec section 5).
    NativeToken,
}

/// Resolved account context guarding the `/v1/account/*` read surface.
///
/// Producible only by the resolver (`resolve_account_ctx`) — it is the sole
/// other mint site for an `AccountPrincipalSet` besides this module's own code.
/// It deliberately exposes ONLY the account id, the active-membership principal
/// set, the accepted auth method, the tenant, and an actor/audit ref. It carries
/// no role/privilege field, so neither path can be treated as reviewer/admin:
/// the cookie path is structurally incapable of escalating.
#[derive(Debug, Clone)]
pub struct AccountCtx {
    pub account_id: AccountId,
    pub principal_set: AccountPrincipalSet,
    pub auth_method: AccountAuthMethod,
    pub tenant_id: String,
    /// Device `principal_ref` (bearer path) or `account-actor:{id}` (cookie path).
    pub actor_ref: String,
    /// The id of the credential that authenticated THIS session. A NEAR login
    /// populates it with the authenticating NEAR public key. `None` for
    /// device-link cookie sessions and for the bearer path. Used solely to flag
    /// the currently-authenticating key in the NEAR identity list
    /// (`this_session`); it is a public id, never a secret.
    pub auth_credential_id: Option<String>,
    /// Strength label for this session, used by the strong-authenticator gate.
    /// Cookie path: the validated session row's `client_kind` (`'near'` strong,
    /// every other value weak). Bearer path: always `'device'` (a device
    /// bearer is NOT a strong authenticator for the gate). A public label, never
    /// key material.
    pub client_kind: String,
    /// Whether a NEAR AI sign-in minted THIS session within the last
    /// `NEAR_AI_LOGIN_FRESH_SECS`. Read only by the strong-authenticator gate
    /// (`may_change_authenticators`); `false` on every path but a native
    /// token from a recent NEAR AI sign-in.
    pub fresh_near_ai_login: bool,
    /// The sha256 `token_hash` of the session credential THIS request
    /// presented (cookie or native token), set by the resolver. Used only to
    /// spare the caller's own session when a device revoke ends the account's
    /// other sessions. A hash, never the secret; `None` when no session was
    /// presented.
    pub session_token_hash: Option<String>,
}

impl AccountCtx {
    /// True iff THIS session was minted by a NEAR wallet assertion. Only a
    /// strong session may freely change authenticators; any other session is
    /// gated unless the account has zero strong authenticators (the
    /// bootstrapping carve-out, enforced by the caller). A `'passkey'` session
    /// minted before passkeys were removed is weak.
    pub fn is_strong_session(&self) -> bool {
        self.client_kind == "near"
    }

    /// True iff THIS session may change authenticators, the payout target or
    /// merge accounts whatever the account holds: a NEAR wallet session, or a
    /// session a NEAR AI sign-in minted within `NEAR_AI_LOGIN_FRESH_SECS`.
    /// Any other session is refused unless the account has no strong
    /// authenticator at all (the bootstrapping carve-out, enforced by the
    /// caller).
    pub fn may_change_authenticators(&self) -> bool {
        self.is_strong_session() || self.fresh_near_ai_login
    }
}

/// How long a session a NEAR AI sign-in minted counts as that sign-in, for the
/// strong-authenticator gate: ten minutes. Past it the session is as weak as
/// any other native session, and changing an authenticator, the payout target
/// or merging accounts takes a new NEAR AI sign-in.
pub const NEAR_AI_LOGIN_FRESH_SECS: i64 = 10 * 60;

/// 160-bit CSPRNG login code, URL-safe base64 (unpadded).
pub fn generate_login_code() -> String {
    let mut bytes = [0u8; 20];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// 160-bit CSPRNG session secret (same entropy source as the login code).
pub fn generate_session_secret() -> String {
    generate_login_code()
}

/// sha256:<lowercase-hex> of the raw secret. Store ONLY this, never the raw secret.
pub fn hash_secret(raw: &str) -> String {
    let digest = Sha256::digest(raw.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_principal_set_membership() {
        let set = AccountPrincipalSet::from_iter(["principal_abc".to_string()]);
        assert!(set.contains("principal_abc"));
        assert!(!set.contains("principal_xyz"));
    }
    #[test]
    fn account_actor_ref_is_not_sha_shaped() {
        let actor = account_actor_ref(&AccountId::from_uuid(uuid::Uuid::nil()));
        assert!(actor.starts_with("account-actor:"));
        assert!(!actor.starts_with("principal_"));
    }
    #[test]
    fn generated_code_is_high_entropy_and_url_safe() {
        let a = generate_login_code();
        let b = generate_login_code();
        assert_ne!(a, b);
        assert!(a.len() >= 27); // >=160 bits base64url, unpadded
        assert!(
            a.bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        );
    }
    /// Only a NEAR wallet session is strong. A `'passkey'` session minted
    /// before passkeys were removed is as weak as a native or device-link one.
    #[test]
    fn only_a_near_session_is_strong() {
        let ctx = |client_kind: &str| AccountCtx {
            account_id: AccountId::from_uuid(uuid::Uuid::nil()),
            principal_set: AccountPrincipalSet::from_iter_for_test_only(Vec::<String>::new()),
            auth_method: AccountAuthMethod::SessionCookie,
            tenant_id: "t".to_string(),
            actor_ref: "account-actor:t".to_string(),
            auth_credential_id: None,
            client_kind: client_kind.to_string(),
            fresh_near_ai_login: false,
            session_token_hash: None,
        };
        assert!(ctx("near").is_strong_session());
        for weak in ["passkey", "native", "web", "device"] {
            assert!(!ctx(weak).is_strong_session(), "{weak}");
        }
    }
    #[test]
    fn hash_is_sha256_prefixed_shape() {
        let h = hash_secret("abc");
        assert!(h.starts_with("sha256:"));
        assert_eq!(h.len(), "sha256:".len() + 64);
    }
}
