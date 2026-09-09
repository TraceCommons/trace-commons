// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Introspecting a NEAR AI login, so a contributor can anchor invite-free
//! admission on the account their receipts already come from (#836).
//!
//! # What this establishes, and what it does not
//!
//! It establishes **who**: the stable subject id behind a short-lived access
//! token, asked of NEAR AI rather than taken from the client. A
//! client-asserted account id would be worthless here -- an attacker runs a
//! modified client and asserts a fresh one per submission, which is the sybil
//! problem the anchor exists to bound.
//!
//! It does **not** establish *freshly, for us*. A token this server did not
//! participate in minting is replayable across commonses and over time, so
//! nothing here is a substitute for the ceremony nonce the provisioning route
//! binds alongside it. Introspection answers identity; the ceremony answers
//! liveness. Neither answers the other.
//!
//! # Fail closed
//!
//! Every failure mode -- unreachable, slow, non-200, unparseable, a subject
//! that is missing or malformed -- is [`LoginIntrospectionRefused`], and the
//! caller refuses provisioning. There is no arm that admits on a failed
//! introspection and no cached positive: the answer is used once, to derive an
//! anchor, and then the token is gone.
//!
//! # The token is a bearer secret in transit
//!
//! It reaches exactly one place: the `Authorization` header of the outbound
//! request. It is never logged, never stored, never placed in an error, and
//! never returned. [`LoginIntrospectionRefused`] carries a control name and no
//! value, so no `Display`, `Debug` or `?err` formatting of it can leak the
//! token -- the same hash-only rule the rest of this crate follows.

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::time::Duration;

/// The label a refused introspection reports. One label for every cause, on
/// purpose: the causes are all "we could not establish who this is", and
/// distinguishing them for the caller would describe our dependency's state to
/// someone who may be probing it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("near_ai_login_unverified")]
pub struct LoginIntrospectionRefused;

/// What `GET /me` tells us, and **nothing else**.
///
/// The real response also carries `email`, `username`, `organizations` and
/// `workspaces`. None of them appears here, and that is a control rather than
/// an omission: `serde` drops unknown fields, so those values never become a
/// value in this process at all. There is nothing to accidentally log, store
/// or return, because there is nothing to log.
///
/// **Do not add a field to this struct without deciding it is anchorable.**
/// In particular there must never be a `#[serde(flatten)]` map or a
/// `serde_json::Value` catch-all here; either would reintroduce the whole
/// response through the back door.
#[derive(Deserialize)]
struct IntrospectedUser {
    /// The stable subject. This is the only thing anchored.
    id: String,
    /// How the contributor authenticated to NEAR AI. Absent on a response that
    /// does not carry it, which is [`AuthProvider::UNKNOWN`] rather than a
    /// refusal -- it is recorded for later trust tiering, not relied on now.
    #[serde(default)]
    auth_provider: Option<String>,
}

/// A verified login, reduced to the two things worth keeping.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedNearAiLogin {
    subject_id: String,
    auth_provider: String,
}

impl VerifiedNearAiLogin {
    /// The subject to anchor. Only `NearAccountIdentity::login_index_label`
    /// should consume this: it is an account identifier, and the anchor is the
    /// blind index over it, never the identifier itself.
    pub fn subject_id(&self) -> &str {
        &self.subject_id
    }

    /// The recorded provider label, already normalised to the stored shape.
    pub fn auth_provider(&self) -> &str {
        &self.auth_provider
    }
}

/// The label stored when NEAR AI names no provider, or names one outside the
/// shape the column accepts.
pub const UNKNOWN_AUTH_PROVIDER: &str = "unknown";

/// Normalise a provider label to what `trace_near_account_anchors.auth_provider`
/// will accept.
///
/// Constrained by shape rather than an enumerated list: this repo has not
/// observed NEAR AI's full vocabulary, and a guessed enumeration would refuse a
/// legitimate provider the day they add one. Anything outside the shape becomes
/// `unknown`, so a third party's free text never reaches the column -- the
/// database enforces the same rule, and this is what keeps a write from
/// failing on it.
fn normalise_auth_provider(raw: Option<&str>) -> String {
    let candidate = raw.unwrap_or_default().trim().to_ascii_lowercase();
    let acceptable = !candidate.is_empty()
        && candidate.len() <= 32
        && candidate
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
    if acceptable {
        candidate
    } else {
        UNKNOWN_AUTH_PROVIDER.to_string()
    }
}

/// Ask NEAR AI whose token this is.
///
/// `base_url` is the API root; the caller supplies it so a deployment can point
/// at a different one and so tests can point at a local server. `timeout`
/// bounds the whole request: a hung dependency must not hang a request holding
/// a database connection.
pub async fn introspect_login(
    base_url: &str,
    token: &SecretString,
    timeout: Duration,
) -> Result<VerifiedNearAiLogin, LoginIntrospectionRefused> {
    let url = format!("{}/me", base_url.trim_end_matches('/'));
    let http = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|_| LoginIntrospectionRefused)?;
    // The one place the token appears. `bearer_auth` rather than a formatted
    // string so it is never a `String` this function owns and could log.
    let response = http
        .get(url)
        .bearer_auth(token.expose_secret())
        .send()
        .await
        .map_err(|_| LoginIntrospectionRefused)?;
    if !response.status().is_success() {
        return Err(LoginIntrospectionRefused);
    }
    let user: IntrospectedUser = response
        .json()
        .await
        .map_err(|_| LoginIntrospectionRefused)?;
    // A subject that is empty, or long enough to be something other than an
    // identifier, is not one we will anchor. Bounded before it reaches the
    // HMAC so a hostile response cannot make us hash something unbounded.
    let subject_id = user.id.trim().to_string();
    if subject_id.is_empty() || subject_id.len() > 256 {
        return Err(LoginIntrospectionRefused);
    }
    Ok(VerifiedNearAiLogin {
        subject_id,
        auth_provider: normalise_auth_provider(user.auth_provider.as_deref()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_provider_label_outside_the_stored_shape_becomes_unknown() {
        assert_eq!(normalise_auth_provider(Some("github")), "github");
        assert_eq!(normalise_auth_provider(Some("  GitHub  ")), "github");
        assert_eq!(normalise_auth_provider(Some("near_wallet")), "near_wallet");
        for rejected in [
            None,
            Some(""),
            Some("   "),
            // The column's own CHECK would refuse each of these, so a write
            // carrying one would fail rather than record `unknown`.
            Some("has space"),
            Some("has-hyphen"),
            Some("Hostile\nInjection"),
            Some("a@b.example"),
        ] {
            assert_eq!(
                normalise_auth_provider(rejected),
                UNKNOWN_AUTH_PROVIDER,
                "{rejected:?}"
            );
        }
        assert_eq!(
            normalise_auth_provider(Some(&"a".repeat(32))),
            "a".repeat(32)
        );
        assert_eq!(
            normalise_auth_provider(Some(&"a".repeat(33))),
            UNKNOWN_AUTH_PROVIDER,
            "past the column's bound"
        );
    }

    /// The response's personal fields are not merely unused, they are
    /// unrepresentable: `IntrospectedUser` has nowhere to put them.
    #[test]
    fn the_introspection_response_cannot_carry_an_email_or_a_username() {
        let body = serde_json::json!({
            "id": "user_123",
            "email": "someone@example.com",
            "username": "someone",
            "organizations": [{"id": "org_1"}],
            "workspaces": [{"id": "ws_1"}],
            "auth_provider": "github",
        });
        let user: IntrospectedUser = serde_json::from_value(body).expect("parses");
        assert_eq!(user.id, "user_123");
        assert_eq!(user.auth_provider.as_deref(), Some("github"));

        // The guard that matters is structural, so assert on the source: a
        // `flatten` or a `Value` catch-all here would silently readmit every
        // field this struct exists to drop.
        let source = include_str!("near_ai_login.rs");
        let declaration = source
            .split_once("struct IntrospectedUser {")
            .expect("the struct is declared here")
            .1
            .split_once('}')
            .expect("and closed")
            .0;
        for forbidden in ["flatten", "Value", "HashMap", "BTreeMap"] {
            assert!(
                !declaration.contains(forbidden),
                "IntrospectedUser gained a {forbidden}, which readmits the whole response"
            );
        }
        for personal in ["email", "username", "organizations", "workspaces"] {
            assert!(
                !declaration.contains(personal),
                "IntrospectedUser gained a field for {personal}"
            );
        }
    }

    /// The refusal names a control and never a value.
    #[test]
    fn the_refusal_carries_no_token_and_no_identity() {
        let refused = LoginIntrospectionRefused;
        assert_eq!(refused.to_string(), "near_ai_login_unverified");
        assert_eq!(format!("{refused:?}"), "LoginIntrospectionRefused");
    }
}
