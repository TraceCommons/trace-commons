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
//! Every failure mode is a [`LoginIntrospectionFailure`], and the caller
//! refuses provisioning on either variant. There is no arm that admits on a
//! failed introspection and no cached positive: the answer is used once, to
//! derive an anchor, and then the token is gone.
//!
//! The variants say only whether NEAR AI *answered*:
//!
//! - [`LoginIntrospectionFailure::Refused`] -- NEAR AI answered and the answer
//!   names no usable subject: a `4xx` other than `408`, `421`, `425` or `429`,
//!   an unparseable `2xx` body, a subject that is missing or malformed. This
//!   is the fact about the token, and it stays one label for every cause.
//! - [`LoginIntrospectionFailure::Unavailable`] -- NEAR AI did not answer:
//!   unreachable, slow, a body that stops arriving or runs past the cap, a
//!   `5xx`, `429`, `408`, `421` or `425`. This is a fact about NEAR AI and the
//!   path to it, the same for every token at that moment, so telling it apart
//!   says nothing about whose token this is. A caller uses it to say "try
//!   again later" rather than "refused".
//! - [`LoginIntrospectionFailure::ClientUnbuildable`] -- this process could
//!   not build a client to ask. A local fault, the same on every request.
//!
//! # The token is a bearer secret in transit
//!
//! It reaches exactly one place: the `Authorization` header of the outbound
//! request. It is never logged, never stored, never placed in an error, and
//! never returned. [`LoginIntrospectionFailure`] carries a control name and no
//! value, so no `Display`, `Debug` or `?err` formatting of it can leak the
//! token -- the same hash-only rule the rest of this crate follows.

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use std::time::Duration;

/// The URL `GET`-ed to learn whose token this is.
///
/// **`/users/me`, not `/me`.** `cloud-api` defines the route as `/me` and then
/// nests that router under `/users`, so the served path is `{base}/users/me`.
/// Reading the route definition alone gives the wrong answer, which is how
/// `/v1/me` shipped -- reported on #851.
///
/// Named rather than inlined so the test can assert the real constructed URL
/// rather than a formatter against a fake base.
fn introspection_url(base_url: &str) -> String {
    format!("{}/users/me", base_url.trim_end_matches('/'))
}

/// Why an introspection established no subject. Labels only.
///
/// `Refused` is one label for every cause on the token's side, on purpose:
/// those causes are all "NEAR AI answered and named no one we can anchor",
/// and distinguishing them would describe the token to whoever holds it.
/// `Unavailable` is kept apart because it is not about the token at all: NEAR
/// AI, or the path to it, did not answer. Both refuse provisioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum LoginIntrospectionFailure {
    /// NEAR AI answered, and the answer names no usable subject.
    #[error("near_ai_login_unverified")]
    Refused,
    /// NEAR AI did not answer: unreachable, timed out, a body that stopped
    /// arriving or ran past [`MAX_INTROSPECTION_BODY_BYTES`], a `5xx`, `429`,
    /// `408`, `421` or `425`.
    #[error("near_ai_login_unavailable")]
    Unavailable,
    /// This process could not build an HTTP client to ask NEAR AI (the TLS
    /// backend or root store). A local fault, the same on every request,
    /// kept apart so the caller logs it as one rather than as NEAR AI down.
    #[error("near_ai_login_client_unbuildable")]
    ClientUnbuildable,
}

/// The most of a `GET /users/me` body this reads. The response carries
/// `organizations` and `workspaces` arrays whose size we do not control, and
/// a cap that refused a heavy but genuine answer would turn into a refused
/// sign-in, so it is generous: a few hundred bytes are used, and 256 KiB is
/// far above any profile while still bounding what one finish can buffer.
pub const MAX_INTROSPECTION_BODY_BYTES: usize = 256 * 1024;

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

/// What a non-success answer from NEAR AI means. A `5xx`, a `429`, or a
/// status that says the request was not handled (`408` Request Timeout,
/// `421` Misdirected Request, `425` Too Early, as a fronting proxy or CDN
/// answers) is NEAR AI not answering the question, the same for every token
/// at that moment; any other status is its answer about this token.
fn failure_for_status(status: reqwest::StatusCode) -> LoginIntrospectionFailure {
    use reqwest::StatusCode;
    if status.is_server_error()
        || matches!(
            status,
            StatusCode::TOO_MANY_REQUESTS
                | StatusCode::REQUEST_TIMEOUT
                | StatusCode::MISDIRECTED_REQUEST
                | StatusCode::TOO_EARLY
        )
    {
        LoginIntrospectionFailure::Unavailable
    } else {
        LoginIntrospectionFailure::Refused
    }
}

/// Ask NEAR AI whose token this is.
///
/// `base_url` is the API root -- the `/v1` prefix, **not** including
/// `/users` -- so the constructed path is `{base}/users/me`. The caller
/// supplies it so a deployment can point at a different one and so tests can
/// point at a local server. `timeout`
/// bounds the whole request: a hung dependency must not hang a request holding
/// a database connection.
pub async fn introspect_login(
    base_url: &str,
    token: &SecretString,
    timeout: Duration,
) -> Result<VerifiedNearAiLogin, LoginIntrospectionFailure> {
    let url = introspection_url(base_url);
    // A client that cannot be built (the TLS backend or root store) fails
    // the same way on every request: a local fault, never an answer about
    // this token.
    let http = reqwest::Client::builder()
        .timeout(timeout)
        .build()
        .map_err(|_| LoginIntrospectionFailure::ClientUnbuildable)?;
    // The one place the token appears. `bearer_auth` rather than a formatted
    // string so it is never a `String` this function owns and could log.
    let response = http
        .get(url)
        .bearer_auth(token.expose_secret())
        .send()
        .await
        .map_err(|_| LoginIntrospectionFailure::Unavailable)?;
    let status = response.status();
    if !status.is_success() {
        return Err(failure_for_status(status));
    }
    // Read, then parse, as two steps: a body that stops arriving (the
    // timeout running out, a reset after the headers) is NEAR AI not
    // answering; a body that arrived and does not parse is its answer.
    //
    // Every read error is the first kind. reqwest has no content
    // decompression enabled in this workspace, so a read error cannot be a
    // corrupt compressed body; `a_compressed_body_is_never_decompressed`
    // pins that, because a workspace crate turning on `gzip` would make a
    // corrupt compressed answer read as NEAR AI not answering.
    //
    // Bounded: a body past the cap is not an answer we can read, so it is
    // `Unavailable`, which only the token's holder ever sees.
    if response
        .content_length()
        .is_some_and(|length| length > MAX_INTROSPECTION_BODY_BYTES as u64)
    {
        return Err(LoginIntrospectionFailure::Unavailable);
    }
    let mut response = response;
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| LoginIntrospectionFailure::Unavailable)?
    {
        if body.len() + chunk.len() > MAX_INTROSPECTION_BODY_BYTES {
            return Err(LoginIntrospectionFailure::Unavailable);
        }
        body.extend_from_slice(&chunk);
    }
    let user: IntrospectedUser =
        serde_json::from_slice(&body).map_err(|_| LoginIntrospectionFailure::Refused)?;
    // A subject that is empty, or long enough to be something other than an
    // identifier, is not one we will anchor. Bounded before it reaches the
    // HMAC so a hostile response cannot make us hash something unbounded.
    let subject_id = user.id.trim().to_string();
    if subject_id.is_empty() || subject_id.len() > 256 {
        return Err(LoginIntrospectionFailure::Refused);
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

    /// The constructed URL is the one `cloud-api` actually serves.
    ///
    /// Pinned against the **real** literal rather than against a formatter
    /// with a fake base, because the defect was that the real path was wrong:
    /// a test composing `"http://test/" + "/me"` would have passed throughout.
    ///
    /// `cloud-api` defines the route as `/me` and nests the router under
    /// `/users`, so reading the route definition gives `/v1/me` and the served
    /// path is `/v1/users/me`. That gap is why this assertion names the whole
    /// URL and not a suffix.
    #[test]
    fn the_introspection_url_is_the_one_cloud_api_serves() {
        // The production base, spelled here so a change to either side has to
        // meet this assertion.
        let base = "https://cloud-api.near.ai/v1";
        assert_eq!(
            introspection_url(base),
            "https://cloud-api.near.ai/v1/users/me"
        );
        // A trailing slash on the base must not double the separator.
        assert_eq!(
            introspection_url("https://cloud-api.near.ai/v1/"),
            "https://cloud-api.near.ai/v1/users/me"
        );
        // And the path we shipped is not the path we serve, kept as an
        // assertion so a regression to it fails here rather than at runtime.
        assert_ne!(introspection_url(base), format!("{base}/me"));
    }

    /// The refusal names a control and never a value.
    #[test]
    fn the_refusal_carries_no_token_and_no_identity() {
        let refused = LoginIntrospectionFailure::Refused;
        assert_eq!(refused.to_string(), "near_ai_login_unverified");
        assert_eq!(format!("{refused:?}"), "Refused");
        let unavailable = LoginIntrospectionFailure::Unavailable;
        assert_eq!(unavailable.to_string(), "near_ai_login_unavailable");
        assert_eq!(format!("{unavailable:?}"), "Unavailable");
        let unbuildable = LoginIntrospectionFailure::ClientUnbuildable;
        assert_eq!(unbuildable.to_string(), "near_ai_login_client_unbuildable");
    }

    /// Only NEAR AI not handling the request is `Unavailable`. Every answer it gives
    /// about the token, whatever the status, stays the one refusal.
    #[test]
    fn only_near_ai_not_handling_the_request_reads_as_unavailable() {
        use reqwest::StatusCode;
        for status in [
            StatusCode::INTERNAL_SERVER_ERROR,
            StatusCode::BAD_GATEWAY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::GATEWAY_TIMEOUT,
            StatusCode::TOO_MANY_REQUESTS,
            StatusCode::REQUEST_TIMEOUT,
            StatusCode::MISDIRECTED_REQUEST,
            StatusCode::TOO_EARLY,
        ] {
            assert_eq!(
                failure_for_status(status),
                LoginIntrospectionFailure::Unavailable,
                "{status}"
            );
        }
        for status in [
            StatusCode::BAD_REQUEST,
            StatusCode::UNAUTHORIZED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::GONE,
            StatusCode::MOVED_PERMANENTLY,
            StatusCode::IM_A_TEAPOT,
        ] {
            assert_eq!(
                failure_for_status(status),
                LoginIntrospectionFailure::Refused,
                "{status}"
            );
        }
    }

    /// A NEAR AI that is not listening is `Unavailable`, and one that answers
    /// `401` is `Refused`: the two cases the provisioning route tells apart.
    #[tokio::test]
    async fn an_unreachable_near_ai_is_unavailable_and_a_401_is_refused() {
        let token = SecretString::from("fixture-token".to_string());
        let closed = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            drop(listener);
            base
        };
        assert_eq!(
            introspect_login(&closed, &token, Duration::from_secs(5)).await,
            Err(LoginIntrospectionFailure::Unavailable)
        );

        for (status, expected) in [
            (
                axum::http::StatusCode::UNAUTHORIZED,
                LoginIntrospectionFailure::Refused,
            ),
            (
                axum::http::StatusCode::SERVICE_UNAVAILABLE,
                LoginIntrospectionFailure::Unavailable,
            ),
        ] {
            let router = axum::Router::new().route(
                "/users/me",
                axum::routing::get(move || async move { status }),
            );
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
            assert_eq!(
                introspect_login(&base, &token, Duration::from_secs(5)).await,
                Err(expected),
                "{status}"
            );
        }
    }

    /// A `200` whose body stops arriving is NEAR AI not answering, not a
    /// refusal: the connection drops partway through the body, or the body
    /// stalls past the timeout. A complete body that does not parse is the
    /// refusal.
    #[tokio::test]
    async fn a_body_that_stops_arriving_is_unavailable_and_a_bad_one_is_refused() {
        use tokio::io::AsyncWriteExt as _;
        /// A one-shot server that writes `head` and then either closes or
        /// holds the connection open.
        async fn raw(head: &'static str, hold: bool) -> String {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 4096];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut request).await;
                socket.write_all(head.as_bytes()).await.unwrap();
                if hold {
                    tokio::time::sleep(Duration::from_secs(30)).await;
                }
            });
            base
        }
        let token = SecretString::from("fixture-token".to_string());
        let short = Duration::from_millis(500);

        let truncated = raw(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n{\"id\"",
            false,
        )
        .await;
        assert_eq!(
            introspect_login(&truncated, &token, short).await,
            Err(LoginIntrospectionFailure::Unavailable),
            "a body cut off after the headers"
        );

        let stalled = raw(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 100\r\n\r\n{\"id\"",
            true,
        )
        .await;
        assert_eq!(
            introspect_login(&stalled, &token, short).await,
            Err(LoginIntrospectionFailure::Unavailable),
            "a body that stalls past the timeout"
        );

        let garbled = raw(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 9\r\nconnection: close\r\n\r\nnot json!",
            false,
        )
        .await;
        assert_eq!(
            introspect_login(&garbled, &token, short).await,
            Err(LoginIntrospectionFailure::Refused),
            "a complete body that does not parse"
        );

        // Past the cap, declared or streamed: not an answer we can read.
        let declared = raw(
            "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: 999999999\r\n\r\n{",
            true,
        )
        .await;
        assert_eq!(
            introspect_login(&declared, &token, short).await,
            Err(LoginIntrospectionFailure::Unavailable),
            "a declared length past the cap"
        );
        let streamed = {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0u8; 4096];
                let _ = tokio::io::AsyncReadExt::read(&mut socket, &mut request).await;
                socket
                    .write_all(b"HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ntransfer-encoding: chunked\r\n\r\n")
                    .await
                    .unwrap();
                let chunk = vec![b' '; 64 * 1024];
                for _ in 0..8 {
                    let head = format!("{:x}\r\n", chunk.len());
                    if socket.write_all(head.as_bytes()).await.is_err()
                        || socket.write_all(&chunk).await.is_err()
                        || socket.write_all(b"\r\n").await.is_err()
                    {
                        return;
                    }
                }
                let _ = socket.write_all(b"0\r\n\r\n").await;
            });
            base
        };
        assert_eq!(
            introspect_login(&streamed, &token, Duration::from_secs(5)).await,
            Err(LoginIntrospectionFailure::Unavailable),
            "a chunked body that runs past the cap"
        );
    }

    /// Pins that reqwest decompresses nothing here. A body labelled gzip that
    /// is not gzip reaches the JSON parse as raw bytes and is refused. If a
    /// crate in the workspace enabled reqwest's `gzip`, the decoder would
    /// fail first, the read error would become `Unavailable`, and this test
    /// would fail -- the cue to revisit the read-error mapping above.
    #[tokio::test]
    async fn a_compressed_body_is_never_decompressed() {
        let router = axum::Router::new().route(
            "/users/me",
            axum::routing::get(|| async {
                (
                    [
                        (axum::http::header::CONTENT_ENCODING, "gzip"),
                        (axum::http::header::CONTENT_TYPE, "application/json"),
                    ],
                    "this is not gzip and not json",
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        assert_eq!(
            introspect_login(
                &base,
                &SecretString::from("fixture-token".to_string()),
                Duration::from_secs(5)
            )
            .await,
            Err(LoginIntrospectionFailure::Refused)
        );
    }
}
