// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The judgement claim, verified offline (spec section 3, "The account
//! credential").
//!
//! The upload-claim issuer mints an EdDSA JWT with
//! `aud = "trace-commons-insights-judgement"`, a lifetime of at most 15
//! minutes and an `account_ref` claim, at `POST /v1/insights-judgement/claim`
//! (`trace_upload_claim_issuer::judgement_claim`). The
//! service verifies it against the issuer's Ed25519 public keys from its
//! measured compose environment (`TRACE_COMMONS_JUDGEMENT_ISSUER_PUBLIC_KEYS`)
//! and makes no call to ingest.
//!
//! # Pooled tenants, checked here too
//!
//! `account_ref` is the account's tenant id for a per-account tenant, and
//! the issuer refuses to mint for an account in a pooled tenant. The service
//! does not take that on trust: a per-account tenant id has a fixed shape
//! (`near-` or `nearai-` and 64 lowercase hex, drawn at random by
//! `near_account_identity`), and an `account_ref` of any other shape is
//! refused as [`JudgementRefusal::AccountIneligible`]. Both ends fail
//! closed, so a pooled tenant id that reached a claim by mistake still
//! cannot share one cap across many people.
//!
//! # Times
//!
//! `jsonwebtoken` checks neither "issued at most 15 minutes before expiry"
//! nor "expires at most 15 minutes from now", and checks expiry against the
//! system clock. So its time checks are off and this module checks every
//! bound against an injected `now`, each with the 60 s clock-skew allowance
//! between the issuer and the CVM (spec section 3, "Lifetime and clock
//! skew"):
//!
//! - `exp - iat <= 900`: the lifetime itself, both issuer-signed, no skew;
//! - `now <= exp + 60`;
//! - `exp - now <= 900 + 60`;
//! - `iat <= now + 60`.
//!
//! The issuer clamps a judgement claim to `900 - 60 = 840` seconds (slice
//! 4), so a claim verifies whichever way the two clocks differ by up to a
//! minute. The verified `iat` is what the limiter's month follows.

use base64::Engine as _;
use jsonwebtoken::{Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use trace_commons_protocol::insights_judgement::JudgementRefusal;

/// The audience of a judgement claim. Distinct from the upload audience, so
/// an upload claim is never accepted here and a judgement claim never by
/// ingest.
pub const JUDGEMENT_CLAIM_AUDIENCE: &str = "trace-commons-insights-judgement";
/// The longest a judgement claim may live, and the furthest ahead of `now`
/// its expiry may be.
pub const JUDGEMENT_CLAIM_MAX_LIFETIME_SECS: i64 = 15 * 60;
/// The clock skew allowed between the issuer and the CVM, on each bound
/// that compares a claim time with `now`.
pub const JUDGEMENT_CLAIM_CLOCK_SKEW_SECS: i64 = 60;
/// Environment variable carrying the issuer's Ed25519 public keys.
pub const ISSUER_PUBLIC_KEYS_ENV: &str = "TRACE_COMMONS_JUDGEMENT_ISSUER_PUBLIC_KEYS";

/// The issuer's public keys. More than one during a rotation overlap.
pub struct IssuerKeys {
    keys: Vec<DecodingKey>,
}

impl std::fmt::Debug for IssuerKeys {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "IssuerKeys({} keys)", self.keys.len())
    }
}

/// The issuer key list did not parse. Carries nothing from the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("judgement_issuer_public_keys_malformed")]
pub struct IssuerKeysMalformed;

/// Parse `TRACE_COMMONS_JUDGEMENT_ISSUER_PUBLIC_KEYS`: comma-separated raw
/// Ed25519 public keys, 64 hex each, no prefix. Whitespace around an element
/// is trimmed. An empty set, or any element that is not a key, refuses, so
/// a service configured with nothing never appears configured.
pub fn parse_issuer_public_keys(spec: &str) -> Result<IssuerKeys, IssuerKeysMalformed> {
    let mut keys = Vec::new();
    for element in spec.split(',') {
        let element = element.trim();
        if element.is_empty() {
            continue;
        }
        let raw: [u8; 32] = hex::decode(element)
            .ok()
            .and_then(|bytes| bytes.try_into().ok())
            .ok_or(IssuerKeysMalformed)?;
        // `from_ed_components` takes the raw key as unpadded base64url.
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw);
        keys.push(DecodingKey::from_ed_components(&encoded).map_err(|_| IssuerKeysMalformed)?);
    }
    if keys.is_empty() {
        return Err(IssuerKeysMalformed);
    }
    Ok(IssuerKeys { keys })
}

/// An account whose judgement claim verified. The raw `account_ref` lives
/// only in request-scoped memory: `Debug` never shows it, and nothing logs
/// it.
pub struct VerifiedAccount {
    account_ref: String,
    iat: i64,
}

impl VerifiedAccount {
    /// The claim's issuer-signed `iat`: the clock the limiter's month
    /// follows (spec section 6), within 60 s of the service clock.
    pub fn iat(&self) -> i64 {
        self.iat
    }

    /// The account reference the limiter keys on (through its keyed digest,
    /// never directly).
    pub fn account_ref(&self) -> &str {
        &self.account_ref
    }
}

impl std::fmt::Debug for VerifiedAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("VerifiedAccount(..)")
    }
}

#[derive(Deserialize)]
struct JudgementClaims {
    exp: i64,
    iat: i64,
    #[serde(default)]
    account_ref: Option<String>,
}

/// Whether `account_ref` is a per-account tenant id: `near-` or `nearai-`
/// followed by 64 lowercase hex.
pub fn is_per_account_tenant_id(account_ref: &str) -> bool {
    let suffix = account_ref
        .strip_prefix("nearai-")
        .or_else(|| account_ref.strip_prefix("near-"));
    suffix.is_some_and(|hex| {
        hex.len() == 64
            && hex
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    })
}

/// Verify a judgement claim at `now` (unix seconds).
///
/// [`JudgementRefusal::CredentialRefused`] for anything wrong with the token
/// itself: not EdDSA, no configured key verifies it, wrong audience,
/// expired or issued in the future by more than the skew, a lifetime over
/// 15 minutes, or no
/// `account_ref`. [`JudgementRefusal::AccountIneligible`] for a valid claim
/// whose `account_ref` is not a per-account tenant.
pub fn verify_judgement_claim(
    keys: &IssuerKeys,
    token: &str,
    now: i64,
) -> Result<VerifiedAccount, JudgementRefusal> {
    let mut validation = Validation::new(Algorithm::EdDSA);
    validation.set_audience(&[JUDGEMENT_CLAIM_AUDIENCE]);
    validation.set_required_spec_claims(&["aud", "exp", "iat"]);
    // Checked below against `now`, with the 15-minute bounds jsonwebtoken
    // does not know about.
    validation.validate_exp = false;
    validation.validate_nbf = false;
    validation.leeway = 0;

    let claims = keys
        .keys
        .iter()
        .find_map(|key| jsonwebtoken::decode::<JudgementClaims>(token, key, &validation).ok())
        .ok_or(JudgementRefusal::CredentialRefused)?
        .claims;

    let skew = JUDGEMENT_CLAIM_CLOCK_SKEW_SECS;
    let lifetime_ok = now <= claims.exp.saturating_add(skew)
        && claims.iat <= now.saturating_add(skew)
        && claims.exp.saturating_sub(claims.iat) <= JUDGEMENT_CLAIM_MAX_LIFETIME_SECS
        && claims.exp.saturating_sub(now) <= JUDGEMENT_CLAIM_MAX_LIFETIME_SECS + skew;
    if !lifetime_ok {
        return Err(JudgementRefusal::CredentialRefused);
    }
    let account_ref = claims
        .account_ref
        .ok_or(JudgementRefusal::CredentialRefused)?;
    if !is_per_account_tenant_id(&account_ref) {
        return Err(JudgementRefusal::AccountIneligible);
    }
    Ok(VerifiedAccount {
        account_ref,
        iat: claims.iat,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use jsonwebtoken::{EncodingKey, Header};
    use ring::signature::{Ed25519KeyPair, KeyPair};

    pub(crate) const NOW: i64 = 1_792_540_800;
    pub(crate) const ACCOUNT: &str =
        "near-0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

    /// A test issuer: a fresh Ed25519 key, never used by a deployment.
    pub(crate) struct TestIssuer {
        encoding: EncodingKey,
        public_hex: String,
    }

    impl TestIssuer {
        pub(crate) fn new() -> Self {
            let rng = ring::rand::SystemRandom::new();
            let pkcs8 = Ed25519KeyPair::generate_pkcs8(&rng).expect("generate");
            let pair = Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("parse");
            Self {
                encoding: EncodingKey::from_ed_der(pkcs8.as_ref()),
                public_hex: hex::encode(pair.public_key().as_ref()),
            }
        }

        pub(crate) fn public_hex(&self) -> &str {
            &self.public_hex
        }

        pub(crate) fn keys(&self) -> IssuerKeys {
            parse_issuer_public_keys(&self.public_hex).expect("one key")
        }

        pub(crate) fn sign(&self, claims: serde_json::Value) -> String {
            jsonwebtoken::encode(&Header::new(Algorithm::EdDSA), &claims, &self.encoding)
                .expect("sign")
        }

        pub(crate) fn claim_for(&self, account_ref: &str, iat: i64, exp: i64) -> String {
            self.sign(serde_json::json!({
                "iss": "trace-commons-upload-claim-issuer",
                "aud": JUDGEMENT_CLAIM_AUDIENCE,
                "sub": "device-ref",
                "account_ref": account_ref,
                "iat": iat,
                "exp": exp,
                "jti": "j-1",
            }))
        }

        pub(crate) fn valid_claim(&self) -> String {
            self.claim_for(ACCOUNT, NOW - 10, NOW + 600)
        }
    }

    #[test]
    fn a_valid_claim_verifies_and_yields_the_account() {
        let issuer = TestIssuer::new();
        let account =
            verify_judgement_claim(&issuer.keys(), &issuer.valid_claim(), NOW).expect("verifies");
        assert_eq!(account.account_ref(), ACCOUNT);
        assert!(!format!("{account:?}").contains("near-"));
    }

    #[test]
    fn a_nearai_tenant_is_per_account_too() {
        let issuer = TestIssuer::new();
        let account_ref = format!("nearai-{}", "a".repeat(64));
        let token = issuer.claim_for(&account_ref, NOW, NOW + 60);
        assert_eq!(
            verify_judgement_claim(&issuer.keys(), &token, NOW)
                .expect("verifies")
                .account_ref(),
            account_ref
        );
    }

    #[test]
    fn the_upload_audience_is_refused() {
        let issuer = TestIssuer::new();
        let token = issuer.sign(serde_json::json!({
            "aud": "trace-commons-ingest",
            "account_ref": ACCOUNT,
            "iat": NOW,
            "exp": NOW + 60,
        }));
        assert_eq!(
            verify_judgement_claim(&issuer.keys(), &token, NOW).unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
    }

    #[test]
    fn an_expired_claim_is_refused_past_the_skew() {
        let issuer = TestIssuer::new();
        for exp in [NOW - 61, NOW - 600] {
            let token = issuer.claim_for(ACCOUNT, exp - 60, exp);
            assert_eq!(
                verify_judgement_claim(&issuer.keys(), &token, NOW).unwrap_err(),
                JudgementRefusal::CredentialRefused
            );
        }
        // Up to 60 s past expiry is clock skew, not expiry.
        for exp in [NOW, NOW - 60] {
            let token = issuer.claim_for(ACCOUNT, exp - 60, exp);
            assert!(verify_judgement_claim(&issuer.keys(), &token, NOW).is_ok());
        }
    }

    /// The issuer clamps a judgement claim to 840 s (slice 4). A service
    /// clock 30 s either side of the issuer's accepts it, and so does one
    /// for a claim at the old 900 s clamp.
    #[test]
    fn a_claim_verifies_at_issuer_time_minus_and_plus_thirty_seconds() {
        let issuer = TestIssuer::new();
        let issued = NOW;
        for lifetime in [840, 900] {
            let token = issuer.claim_for(ACCOUNT, issued, issued + lifetime);
            for service_now in [issued - 30, issued + 30] {
                let account = verify_judgement_claim(&issuer.keys(), &token, service_now)
                    .unwrap_or_else(|e| panic!("lifetime {lifetime} at {service_now}: {e:?}"));
                assert_eq!(account.iat(), issued);
            }
        }
        // The edges: iat 60 s ahead with expiry 960 s ahead is accepted, one
        // more second of iat is not.
        let token = issuer.claim_for(ACCOUNT, NOW + 60, NOW + 960);
        assert!(verify_judgement_claim(&issuer.keys(), &token, NOW).is_ok());
        let token = issuer.claim_for(ACCOUNT, NOW + 61, NOW + 961);
        assert_eq!(
            verify_judgement_claim(&issuer.keys(), &token, NOW).unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
    }

    #[test]
    fn a_lifetime_over_fifteen_minutes_is_refused() {
        let issuer = TestIssuer::new();
        // Issued long ago, still unexpired: lifetime too long.
        let token = issuer.claim_for(ACCOUNT, NOW - 3600, NOW + 60);
        assert_eq!(
            verify_judgement_claim(&issuer.keys(), &token, NOW).unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
        // One second over fifteen minutes, wherever now falls.
        let token = issuer.claim_for(ACCOUNT, NOW, NOW + 901);
        assert_eq!(
            verify_judgement_claim(&issuer.keys(), &token, NOW).unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
        // Exactly fifteen minutes is allowed.
        let token = issuer.claim_for(ACCOUNT, NOW, NOW + 900);
        assert!(verify_judgement_claim(&issuer.keys(), &token, NOW).is_ok());
    }

    #[test]
    fn a_claim_issued_in_the_future_is_refused() {
        let issuer = TestIssuer::new();
        let token = issuer.claim_for(ACCOUNT, NOW + 120, NOW + 300);
        assert_eq!(
            verify_judgement_claim(&issuer.keys(), &token, NOW).unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let issuer = TestIssuer::new();
        let other = TestIssuer::new();
        assert_eq!(
            verify_judgement_claim(&other.keys(), &issuer.valid_claim(), NOW).unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
    }

    #[test]
    fn either_key_verifies_during_a_rotation_overlap() {
        let old = TestIssuer::new();
        let new = TestIssuer::new();
        let keys = parse_issuer_public_keys(&format!("{}, {}", old.public_hex(), new.public_hex()))
            .expect("two keys");
        assert!(verify_judgement_claim(&keys, &old.valid_claim(), NOW).is_ok());
        assert!(verify_judgement_claim(&keys, &new.valid_claim(), NOW).is_ok());
    }

    #[test]
    fn a_pooled_tenant_is_ineligible() {
        let issuer = TestIssuer::new();
        for pooled in [
            "devfolio-hackathon",
            "tenant-a",
            "near-short",
            // Uppercase hex is not the shape the server draws.
            &format!("near-{}", "A".repeat(64)),
            &format!("nearx-{}", "a".repeat(64)),
        ] {
            let token = issuer.claim_for(pooled, NOW, NOW + 60);
            assert_eq!(
                verify_judgement_claim(&issuer.keys(), &token, NOW).unwrap_err(),
                JudgementRefusal::AccountIneligible,
                "{pooled}"
            );
        }
    }

    #[test]
    fn a_claim_without_account_ref_is_refused() {
        let issuer = TestIssuer::new();
        let token = issuer.sign(serde_json::json!({
            "aud": JUDGEMENT_CLAIM_AUDIENCE,
            "iat": NOW,
            "exp": NOW + 60,
        }));
        assert_eq!(
            verify_judgement_claim(&issuer.keys(), &token, NOW).unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
    }

    #[test]
    fn a_non_eddsa_token_is_refused() {
        let issuer = TestIssuer::new();
        let token = jsonwebtoken::encode(
            &Header::new(Algorithm::HS256),
            &serde_json::json!({
                "aud": JUDGEMENT_CLAIM_AUDIENCE,
                "account_ref": ACCOUNT,
                "iat": NOW,
                "exp": NOW + 60,
            }),
            &EncodingKey::from_secret(issuer.public_hex().as_bytes()),
        )
        .expect("sign");
        assert_eq!(
            verify_judgement_claim(&issuer.keys(), &token, NOW).unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
        assert_eq!(
            verify_judgement_claim(&issuer.keys(), "not a jwt", NOW).unwrap_err(),
            JudgementRefusal::CredentialRefused
        );
    }

    #[test]
    fn issuer_key_lists_parse_closed() {
        let key = "ab".repeat(32);
        assert!(parse_issuer_public_keys(&key).is_ok());
        assert!(parse_issuer_public_keys(&format!(" {key} ,\n{} ", "cd".repeat(32))).is_ok());
        for bad in [
            "",
            " , ",
            "abcd",
            &format!("0x{key}"),
            &format!("{key},zz"),
            &"g".repeat(64),
        ] {
            assert_eq!(
                parse_issuer_public_keys(bad).unwrap_err(),
                IssuerKeysMalformed,
                "{bad:?}"
            );
        }
    }
}
