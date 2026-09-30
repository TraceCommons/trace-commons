//! HTTP client for the upload-claim issuer: enrollment and claim-minting.
//!
//! Every request is checked against the configured [`HostAllowlist`] before
//! it leaves the process. Non-2xx responses never echo the response body —
//! only the `{"error": "<label>"}` label is surfaced, or the HTTP status if
//! no label parses.

use anyhow::{Context, Result, anyhow};
use chrono::{DateTime, Duration, Utc};
use serde::Deserialize;
use trace_commons_operator_client::host_allowlist::HostAllowlist;
use trace_commons_protocol::onboarding::{
    TraceInstanceEnrollRequest, TraceOnboardRequest, TraceOnboardResponse,
};

use crate::identity::{
    SignedClaimRequest, TRACE_DEVICE_KEY_ID_HEADER, TRACE_DEVICE_SIGNATURE_HEADER,
};

/// A minted upload-claim bearer token.
#[derive(Debug, Clone)]
pub struct ClaimToken {
    pub access_token: String,
    pub expires_at: DateTime<Utc>,
    /// Consent scopes the issuer granted, echoed from the response. Empty
    /// means an older issuer that does not echo this field.
    pub consent_scopes: Vec<String>,
    /// Allowed uses the issuer granted, echoed from the response. Empty
    /// means an older issuer that does not echo this field.
    pub allowed_uses: Vec<String>,
}

impl ClaimToken {
    /// True if the token has at least 60 seconds of validity left from `now`.
    pub fn is_fresh(&self, now: DateTime<Utc>) -> bool {
        now + Duration::seconds(60) < self.expires_at
    }
}

#[derive(Deserialize)]
struct ClaimTokenResponse {
    access_token: String,
    expires_at: DateTime<Utc>,
    #[serde(default)]
    consent_scopes: Vec<String>,
    #[serde(default)]
    allowed_uses: Vec<String>,
}

#[derive(Deserialize)]
struct ErrorLabel {
    error: String,
}

/// HTTP client for the enroll and upload-claim-mint endpoints.
pub struct IssuerClient {
    http: reqwest::Client,
    allowlist: HostAllowlist,
}

impl IssuerClient {
    /// Construct a client with a 30-second request timeout.
    pub fn new(allowlist: HostAllowlist) -> Result<Self> {
        let http = reqwest::Client::builder()
            // Service endpoints never delegate credentials or trust through redirects.
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .context("building issuer HTTP client")?;
        Ok(Self { http, allowlist })
    }

    /// Enroll this device with the issuer, exchanging an instance-signed
    /// grant for tenant/audience/ingest details.
    pub async fn enroll(
        &self,
        issuer_url: &str,
        req: &TraceInstanceEnrollRequest,
    ) -> Result<TraceOnboardResponse> {
        let url = format!("{issuer_url}/v1/enroll");
        let parsed = reqwest::Url::parse(&url).with_context(|| format!("parsing {url}"))?;
        self.allowlist.check(&parsed)?;

        let response = self
            .http
            .post(parsed)
            .json(req)
            .send()
            .await
            .with_context(|| format!("sending enroll request to {url}"))?;

        if !response.status().is_success() {
            return Err(error_from_response(response, "enroll refused").await);
        }

        response
            .json::<TraceOnboardResponse>()
            .await
            .context("parsing enroll response")
    }

    /// Redeem an invite code, registering this device and returning the
    /// tenant/audience/ingest details needed to write `contributor.json`.
    ///
    /// NOT idempotent: every success spends one use of the invite, including
    /// re-registering a key that is already enrolled. Call it once.
    pub async fn onboard(
        &self,
        issuer_url: &str,
        req: &TraceOnboardRequest,
    ) -> Result<TraceOnboardResponse> {
        let url = format!("{issuer_url}/v1/onboard");
        let parsed = reqwest::Url::parse(&url).with_context(|| format!("parsing {url}"))?;
        self.allowlist.check(&parsed)?;

        let response = self
            .http
            .post(parsed)
            .json(req)
            .send()
            .await
            .with_context(|| format!("sending onboard request to {url}"))?;

        if !response.status().is_success() {
            return Err(error_from_response(response, "onboard refused").await);
        }

        response
            .json::<TraceOnboardResponse>()
            .await
            .context("parsing onboard response")
    }

    /// Mint an upload-claim bearer token, sending the pre-signed body
    /// verbatim (never re-serialized) so the device signature stays valid.
    pub async fn mint_claim(
        &self,
        issuer_url: &str,
        signed: &SignedClaimRequest,
    ) -> Result<ClaimToken> {
        let url = format!("{issuer_url}/v1/trace-upload-claim");
        let parsed = reqwest::Url::parse(&url).with_context(|| format!("parsing {url}"))?;
        self.allowlist.check(&parsed)?;

        let response = self
            .http
            .post(parsed)
            .header("content-type", "application/json")
            .header(TRACE_DEVICE_KEY_ID_HEADER, &signed.device_key_id)
            .header(TRACE_DEVICE_SIGNATURE_HEADER, &signed.signature_b64)
            .body(signed.body.clone())
            .send()
            .await
            .with_context(|| format!("sending claim request to {url}"))?;

        if !response.status().is_success() {
            return Err(error_from_response(response, "claim refused").await);
        }

        let parsed: ClaimTokenResponse = response.json().await.context("parsing claim response")?;
        Ok(ClaimToken {
            access_token: parsed.access_token,
            expires_at: parsed.expires_at,
            consent_scopes: parsed.consent_scopes,
            allowed_uses: parsed.allowed_uses,
        })
    }
}

impl IssuerClient {
    /// Ask the issuer whether an invite is usable, who issued it and what it
    /// pays, WITHOUT joining: unlike [`Self::onboard`] this spends no use and
    /// registers nothing. The code goes in the body, never the URL. A refused
    /// invite is `Ok` with `valid == false` and a `reason_label`; `Err` is a
    /// transport failure or an issuer refusal such as its rate limit
    /// (`invite_lookup_rate_limited`) or an issuer that predates the route.
    ///
    /// Do NOT call this per keystroke. Validate the code's format on the
    /// client first and call once, on an explicit "Look up" action: the
    /// issuer's lookup budget is small and shared, every call counts against
    /// it, so partial codes burn it and the user is throttled just as the
    /// code completes.
    ///
    /// Clients MUST present the returned `credit_range` as "estimated credit
    /// per accepted trace, not yet settled", never as a payment promise. The
    /// range is operator-set and not enforced by the ledger.
    pub async fn lookup_invite(
        &self,
        issuer_url: &str,
        invite_code: &str,
    ) -> Result<trace_commons_protocol::invite_lookup::InviteLookupResponse> {
        use trace_commons_protocol::invite_lookup::{
            INVITE_LOOKUP_PATH, INVITE_LOOKUP_REQUEST_SCHEMA_VERSION, InviteLookupRequest,
            InviteLookupResponse,
        };
        let url = format!("{}{INVITE_LOOKUP_PATH}", issuer_url.trim_end_matches('/'));
        let parsed = reqwest::Url::parse(&url).context("parsing the issuer URL")?;
        self.allowlist.check(&parsed)?;
        let request = InviteLookupRequest {
            schema_version: INVITE_LOOKUP_REQUEST_SCHEMA_VERSION.to_string(),
            invite_code: invite_code.trim().to_string(),
        };
        let response = self
            .http
            .post(parsed)
            .json(&request)
            .send()
            .await
            .context("sending the invite lookup request")?;
        if !response.status().is_success() {
            return Err(error_from_response(response, "invite lookup refused").await);
        }
        response
            .json::<InviteLookupResponse>()
            .await
            .context("parsing the invite lookup response")
    }
}

/// What the issuer said about this device's invite.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InviteSubjectAnswer {
    /// The device's own invite subject hash.
    Hash(String),
    /// An issuer older than the route (404 or 405): the caller asks the
    /// contributor for the invite instead.
    Unsupported,
    /// A named refusal from the issuer, by its label.
    Refused(String),
}

fn is_subject_hash(value: &str) -> bool {
    value.strip_prefix("sha256:").is_some_and(|hex| {
        hex.len() == 64 && hex.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    })
}

impl IssuerClient {
    /// Ask the issuer which invite this device was onboarded with. Signed by
    /// the device key over the exact body, as a claim request is; see
    /// `trace_commons_protocol::device_invite_subject`.
    pub async fn device_invite_subject(
        &self,
        issuer_url: &str,
        tenant_id: &str,
        device: &crate::identity::DeviceIdentity,
    ) -> Result<InviteSubjectAnswer> {
        use trace_commons_protocol::device_invite_subject::{
            DEVICE_INVITE_SUBJECT_PATH, DEVICE_INVITE_SUBJECT_REQUEST_SCHEMA_VERSION,
            DeviceInviteSubjectRequest, DeviceInviteSubjectResponse,
        };
        let url = format!(
            "{}{DEVICE_INVITE_SUBJECT_PATH}",
            issuer_url.trim_end_matches('/')
        );
        let parsed = reqwest::Url::parse(&url).context("parsing the issuer URL")?;
        self.allowlist.check(&parsed)?;
        let body = serde_json::to_vec(&DeviceInviteSubjectRequest {
            schema_version: DEVICE_INVITE_SUBJECT_REQUEST_SCHEMA_VERSION.to_string(),
            tenant_id: tenant_id.to_string(),
            device_key_id: device.device_key_id.clone(),
            issued_at: Utc::now().timestamp(),
        })
        .context("serializing the invite subject request")?;
        let signature = device.sign_b64(&body);
        let response = self
            .http
            .post(parsed)
            .header("content-type", "application/json")
            .header(TRACE_DEVICE_KEY_ID_HEADER, &device.device_key_id)
            .header(TRACE_DEVICE_SIGNATURE_HEADER, signature)
            .body(body)
            .send()
            .await
            .context("sending the invite subject request")?;
        let status = response.status();
        if matches!(
            status,
            reqwest::StatusCode::NOT_FOUND | reqwest::StatusCode::METHOD_NOT_ALLOWED
        ) {
            return Ok(InviteSubjectAnswer::Unsupported);
        }
        if !status.is_success() {
            return Ok(match response.json::<ErrorLabel>().await {
                Ok(ErrorLabel { error }) => InviteSubjectAnswer::Refused(error),
                Err(_) => InviteSubjectAnswer::Refused(status.as_u16().to_string()),
            });
        }
        let answer: DeviceInviteSubjectResponse = response
            .json()
            .await
            .context("parsing the invite subject response")?;
        if !is_subject_hash(&answer.invite_subject_hash) {
            anyhow::bail!("invite subject response malformed");
        }
        Ok(InviteSubjectAnswer::Hash(answer.invite_subject_hash))
    }
}

/// This device's invite subject hash, for the legacy invite migration.
/// `Ok(None)` when the issuer predates the route, so the caller asks the
/// contributor for their invite. Refusals are fixed migration labels.
pub async fn fetch_device_invite_subject(
    cfg: &crate::config::ContributorConfig,
    device: &crate::identity::DeviceIdentity,
) -> Result<Option<String>> {
    let client = IssuerClient::new(crate::config::allowlist_for(cfg.allowed_hosts.as_deref()))
        .map_err(|_| anyhow!("legacy_migration_unavailable"))?;
    match client
        .device_invite_subject(&cfg.issuer_url, &cfg.tenant_id, device)
        .await
    {
        Ok(InviteSubjectAnswer::Hash(hash)) => Ok(Some(hash)),
        Ok(InviteSubjectAnswer::Unsupported) => Ok(None),
        Ok(InviteSubjectAnswer::Refused(label)) => Err(anyhow!(match label.as_str() {
            "device_key_not_registered" | "device_key_revoked" | "device_not_invite_onboarded" => {
                "legacy_migration_device_not_eligible"
            }
            _ => "legacy_migration_unavailable",
        })),
        Err(_) => Err(anyhow!("legacy_migration_unavailable")),
    }
}

/// Build an error from a non-2xx response, surfacing only the `{"error":
/// <label>}` label (or the HTTP status if no label parses). Never echoes
/// the raw response body.
async fn error_from_response(response: reqwest::Response, prefix: &str) -> anyhow::Error {
    let status = response.status();
    match response.json::<ErrorLabel>().await {
        Ok(ErrorLabel { error }) => anyhow!("{prefix}: {error}"),
        Err(_) => anyhow!("{prefix}: {status}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{Json, Router, routing::post};

    async fn spawn(router: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn lookup_invite_posts_the_code_in_the_body_and_parses_the_answer() {
        let router = Router::new().route(
            "/v1/invite/lookup",
            post(|uri: axum::http::Uri, body: String| async move {
                assert_eq!(uri.query(), None, "the code must never ride in the URL");
                let sent: serde_json::Value = serde_json::from_str(&body).unwrap();
                assert_eq!(sent["invite_code"], "ABCDEFGHJKLMNPQR");
                assert_eq!(
                    sent["schema_version"],
                    "trace_commons.invite_lookup_request.v1"
                );
                Json(serde_json::json!({
                    "valid": true,
                    "issuer_display_name": "Trace Commons Pilot",
                    "credit_range": {"min": 2, "max": 6, "unit": "points_per_accepted_trace"},
                }))
            }),
        );
        let base = spawn(router).await;
        let client = IssuerClient::new(
            trace_commons_operator_client::host_allowlist::HostAllowlist::permissive(),
        )
        .unwrap();
        let answer = client
            .lookup_invite(&base, "  ABCDEFGHJKLMNPQR ")
            .await
            .unwrap();
        assert!(answer.valid);
        assert_eq!(
            answer.issuer_display_name.as_deref(),
            Some("Trace Commons Pilot")
        );
        let range = answer.credit_range.unwrap();
        assert_eq!((range.min, range.max), (2, 6));
    }

    #[tokio::test]
    async fn lookup_invite_surfaces_a_refusal_label_and_a_rate_limit() {
        let router = Router::new().route(
            "/v1/invite/lookup",
            post(|body: String| async move {
                if body.contains("LIMITEDLIMITEDLI") {
                    (
                        axum::http::StatusCode::TOO_MANY_REQUESTS,
                        Json(serde_json::json!({"error": "invite_lookup_rate_limited"})),
                    )
                } else {
                    (
                        axum::http::StatusCode::OK,
                        Json(serde_json::json!({"valid": false, "reason_label": "expired"})),
                    )
                }
            }),
        );
        let base = spawn(router).await;
        let client = IssuerClient::new(
            trace_commons_operator_client::host_allowlist::HostAllowlist::permissive(),
        )
        .unwrap();
        let answer = client
            .lookup_invite(&base, "ABCDEFGHJKLMNPQR")
            .await
            .unwrap();
        assert!(!answer.valid);
        assert_eq!(answer.reason_label.as_deref(), Some("expired"));
        let error = client
            .lookup_invite(&base, "LIMITEDLIMITEDLI")
            .await
            .unwrap_err();
        assert!(error.to_string().contains("invite_lookup_rate_limited"));
        assert!(!error.to_string().contains("LIMITEDLIMITEDLI"));
    }

    #[tokio::test]
    async fn mint_claim_sends_signed_body_verbatim_and_parses_token() {
        let router = Router::new().route(
            "/v1/trace-upload-claim",
            post(|headers: axum::http::HeaderMap, body: String| async move {
                assert_eq!(body, r#"{"k":"v"}"#);
                assert_eq!(headers.get("x-trace-device-key-id").unwrap(), "sha256:ab");
                assert_eq!(headers.get("x-trace-device-signature").unwrap(), "c2ln");
                Json(serde_json::json!({
                    "access_token": "jwt-token",
                    "token_type": "Bearer",
                    "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                    "expires_in": 300,
                    "consent_scopes": ["debugging_evaluation"],
                    "allowed_uses": ["debugging"],
                }))
            }),
        );
        let base = spawn(router).await;
        let client = IssuerClient::new(
            trace_commons_operator_client::host_allowlist::HostAllowlist::permissive(),
        )
        .unwrap();
        let signed = crate::identity::SignedClaimRequest {
            body: r#"{"k":"v"}"#.into(),
            device_key_id: "sha256:ab".into(),
            signature_b64: "c2ln".into(),
        };
        let token = client.mint_claim(&base, &signed).await.unwrap();
        assert_eq!(token.access_token, "jwt-token");
        assert!(token.is_fresh(chrono::Utc::now()));
        assert_eq!(
            token.consent_scopes,
            vec!["debugging_evaluation".to_string()]
        );
        assert_eq!(token.allowed_uses, vec!["debugging".to_string()]);
    }

    #[tokio::test]
    async fn mint_claim_response_without_echo_fields_yields_empty_vecs() {
        let router = Router::new().route(
            "/v1/trace-upload-claim",
            post(
                |_headers: axum::http::HeaderMap, _body: String| async move {
                    Json(serde_json::json!({
                        "access_token": "jwt-token",
                        "token_type": "Bearer",
                        "expires_at": chrono::Utc::now() + chrono::Duration::seconds(300),
                        "expires_in": 300,
                    }))
                },
            ),
        );
        let base = spawn(router).await;
        let client = IssuerClient::new(
            trace_commons_operator_client::host_allowlist::HostAllowlist::permissive(),
        )
        .unwrap();
        let signed = crate::identity::SignedClaimRequest {
            body: r#"{"k":"v"}"#.into(),
            device_key_id: "sha256:ab".into(),
            signature_b64: "c2ln".into(),
        };
        let token = client.mint_claim(&base, &signed).await.unwrap();
        assert!(token.consent_scopes.is_empty());
        assert!(token.allowed_uses.is_empty());
    }

    #[tokio::test]
    async fn enroll_error_label_is_surfaced_without_body() {
        let router = Router::new().route(
            "/v1/enroll",
            post(|| async {
                (
                    axum::http::StatusCode::FORBIDDEN,
                    Json(serde_json::json!({"error": "EnrollNotAuthorized"})),
                )
            }),
        );
        let base = spawn(router).await;
        let client = IssuerClient::new(
            trace_commons_operator_client::host_allowlist::HostAllowlist::permissive(),
        )
        .unwrap();
        let doc = ring::signature::Ed25519KeyPair::generate_pkcs8(&ring::rand::SystemRandom::new())
            .unwrap();
        let grant = crate::identity::mint_grant(
            doc.as_ref(),
            &base,
            "instance-1",
            "alice",
            "aud",
            "sha256:ab",
            300,
            chrono::Utc::now(),
        )
        .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let store = crate::config::ConfigStore::open(dir.path().to_path_buf()).unwrap();
        let device = crate::identity::DeviceIdentity::load_or_generate(&store).unwrap();
        // Force a matching device_key_id so we reach the HTTP call.
        let grant2 = crate::identity::mint_grant(
            doc.as_ref(),
            &base,
            "instance-1",
            "alice",
            "aud",
            &device.device_key_id,
            300,
            chrono::Utc::now(),
        )
        .unwrap();
        let req = crate::identity::build_enroll_request(&grant2, &device).unwrap();
        let err = client.enroll(&base, &req).await.unwrap_err();
        assert!(err.to_string().contains("EnrollNotAuthorized"));
        let _ = grant;
    }
    #[tokio::test]
    async fn signed_claim_requests_never_follow_redirects() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let count = hits.clone();
        let target = spawn(Router::new().route(
            "/redirected",
            post(move || {
                let count = count.clone();
                async move {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    Json(serde_json::json!({}))
                }
            }),
        ))
        .await;
        let origin = spawn(Router::new().route(
            "/v1/trace-upload-claim",
            post(move || {
                let target = target.clone();
                async move {
                    (
                        axum::http::StatusCode::TEMPORARY_REDIRECT,
                        [(axum::http::header::LOCATION, format!("{target}/redirected"))],
                    )
                }
            }),
        ))
        .await;
        let client = IssuerClient::new(HostAllowlist::permissive()).unwrap();
        let signed = SignedClaimRequest {
            body: "{}".into(),
            device_key_id: "device".into(),
            signature_b64: "signature".into(),
        };
        assert!(client.mint_claim(&origin, &signed).await.is_err());
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
    }
}
