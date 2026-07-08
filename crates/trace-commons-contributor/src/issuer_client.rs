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
use trace_commons_protocol::onboarding::{TraceInstanceEnrollRequest, TraceOnboardResponse};

use crate::identity::{
    SignedClaimRequest, TRACE_DEVICE_KEY_ID_HEADER, TRACE_DEVICE_SIGNATURE_HEADER,
};

/// A minted upload-claim bearer token.
#[derive(Debug, Clone)]
pub struct ClaimToken {
    pub access_token: String,
    pub expires_at: DateTime<Utc>,
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
        })
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
}
