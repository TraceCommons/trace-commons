// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The transport the attestation drill talks to, and its live implementation.
//!
//! Everything above this module is pure: [`super::AttestationReport`] parses,
//! [`super::quote`] verifies, [`super::measurements`] pins, and
//! [`super::receipt`] checks a signature. This module is the only part that
//! goes on the network, and it exists as a trait so the drill can be driven
//! end to end from a stub in tests.
//!
//! Four calls, in the order the drill makes them:
//!
//! 1. [`AttestationClient::fetch_report`] -- `GET {base}/attestation/report`.
//!    Deliberately **without** `include_tls_fingerprint`. In that default mode
//!    the quote's `report_data[0..20]` is the raw signing address and
//!    `[20..32]` are zero; with the flag set it is instead
//!    `SHA256(signing_address || spki_hash)` across all 32 bytes. NEAR AI's
//!    own README documents only the latter. The drill asserts the zero
//!    padding so that if this call ever grows the flag, the binding check
//!    fails loudly instead of quietly comparing an address against half a
//!    hash.
//! 2. [`AttestationClient::fetch_collateral`] -- Intel DCAP collateral for
//!    that quote. See the note on the cargo feature below.
//! 3. [`AttestationClient::complete`] -- one minimal chat completion. The
//!    caller supplies the request bytes and this method must put **those
//!    bytes** on the wire: the receipt binds `SHA256(request_body_as_sent)`
//!    and `SHA256(response_body_as_received)`,
//!    so a re-serialization here surfaces later as
//!    [`super::receipt::ReceiptError::RequestHashMismatch`], which reads as
//!    tampering rather than as the caller bug it would be.
//! 4. [`AttestationClient::fetch_receipt`] -- `GET {base}/signature/{chat_id}`.
//!
//! **Collateral fetching is behind the `near-attestation-collateral` cargo
//! feature, off by default.** `dcap-qvl`'s `report` feature is its collateral
//! client, and enabling it pulls a second async HTTP stack (reqwest 0.13)
//! into every build; nothing else in this workspace compiles that today. With
//! the feature off, [`HttpAttestationClient::fetch_collateral`] refuses with a
//! named missing control rather than silently degrading, so a default build
//! runs the drill and gets a red result explaining exactly what to rebuild.
//! Fetching the PCS endpoints by hand instead is not an option worth taking:
//! FMSPC extraction from the PCK leaf and CRL handling are precisely the
//! fiddly parts `dcap-qvl` already gets right.
//!
//! Errors here are hash-only. `dcap-qvl` error chains and reqwest errors both
//! quote URLs, and this crate's logs and audit rows never carry one.

use std::fmt;
use std::time::Duration;

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use super::AttestationReport;
use super::quote::Collateral;
use super::receipt::{ReceiptAlgo, ReceiptPayload};

/// Missing-control name when no NEAR AI base URL is configured.
pub const BASE_URL_CONTROL: &str = "near_ai_base_url";

/// Missing-control name when no NEAR AI API key is configured.
pub const API_KEY_CONTROL: &str = "near_ai_api_key";

/// Missing-control name when no NEAR AI model is configured.
pub const MODEL_CONTROL: &str = "near_ai_model";

/// Missing-control name when the binary was built without the collateral
/// client, so no Intel collateral can be fetched.
pub const COLLATERAL_CLIENT_CONTROL: &str = "near_ai_attestation_collateral_client";

/// Intel's own Provisioning Certification Service.
///
/// The default deliberately points at Intel rather than at a caching PCCS
/// mirror: the collateral is what the quote is verified against, and the
/// shorter the trust path to Intel the better. Operators running an on-prem
/// PCCS override it.
pub const INTEL_PCS_URL: &str = "https://api.trustedservices.intel.com";

/// Which of the four calls a failure came from.
///
/// Carried on every error so evidence can name the step without the drill
/// having to infer it from ordering.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttestationStep {
    Report,
    Collateral,
    Completion,
    Receipt,
}

impl AttestationStep {
    pub fn as_str(self) -> &'static str {
        match self {
            AttestationStep::Report => "report",
            AttestationStep::Collateral => "collateral",
            AttestationStep::Completion => "completion",
            AttestationStep::Receipt => "receipt",
        }
    }
}

impl fmt::Display for AttestationStep {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Why a call to the NEAR AI endpoint did not produce what the drill asked
/// for.
///
/// No variant carries a URL, a body, a header or a key. `detail_hash` is a
/// truncated SHA-256 of the underlying message, so two operators seeing the
/// same hash are looking at the same failure without the message reaching a
/// log line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AttestationClientError {
    /// A control the call needs is not configured, or was not compiled in.
    #[error("{step} step refused: missing control {control}")]
    MissingControl {
        step: AttestationStep,
        control: &'static str,
    },
    /// The request did not complete.
    #[error("{step} step could not reach the endpoint (detail {detail_hash})")]
    Transport {
        step: AttestationStep,
        detail_hash: String,
    },
    /// The endpoint answered with a non-success status.
    #[error("{step} step was answered with HTTP {status}")]
    HttpStatus { step: AttestationStep, status: u16 },
    /// The response arrived but was not the shape the drill needs.
    #[error("{step} step response was not the expected shape (detail {detail_hash})")]
    MalformedResponse {
        step: AttestationStep,
        detail_hash: String,
    },
}

impl AttestationClientError {
    /// Which call failed.
    pub fn step(&self) -> AttestationStep {
        match self {
            Self::MissingControl { step, .. }
            | Self::Transport { step, .. }
            | Self::HttpStatus { step, .. }
            | Self::MalformedResponse { step, .. } => *step,
        }
    }

    /// A stable, hash-only label for evidence. Never the message.
    pub fn code(&self) -> &'static str {
        match self {
            Self::MissingControl { .. } => "missing_control",
            Self::Transport { .. } => "transport",
            Self::HttpStatus { .. } => "http_status",
            Self::MalformedResponse { .. } => "malformed_response",
        }
    }

    /// The missing control's name, when that is the failure.
    pub fn missing_control(&self) -> Option<&'static str> {
        match self {
            Self::MissingControl { control, .. } => Some(control),
            _ => None,
        }
    }
}

/// A truncated digest of an error message, safe to log.
fn detail_hash(message: &str) -> String {
    hex::encode(&Sha256::digest(message.as_bytes())[..8])
}

/// What one minimal completion yielded.
///
/// `response_body` is what the receipt's second hash covers: the **entire raw
/// response body**, byte for byte, not the assistant message content read out
/// of it. That was settled against a captured live triple, not reasoned about
/// -- see `crates/trace-commons-server/tests/near_ai_live_receipt.rs`. The
/// drill never streams, because for a streaming response the body is a series
/// of SSE frames that cannot be reproduced from parsed deltas.
///
/// Nothing may re-serialize this from a parsed form, for the same reason the
/// request body is kept verbatim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompletionOutcome {
    /// The completion's `id`, which addresses its receipt.
    pub chat_id: String,
    /// The whole response body, exactly as received.
    pub response_body: String,
}

/// The four calls the drill makes against a NEAR AI inference endpoint.
#[async_trait]
pub trait AttestationClient: Send + Sync {
    /// The model id the drill should name in its request body, so the bytes
    /// signed by the receipt and the bytes we hash are the same bytes.
    fn model(&self) -> &str;

    /// Fetch the attestation report for `nonce` (64 lowercase hex chars).
    async fn fetch_report(&self, nonce: &str) -> Result<AttestationReport, AttestationClientError>;

    /// Fetch Intel DCAP collateral for `quote`.
    async fn fetch_collateral(&self, quote: &[u8]) -> Result<Collateral, AttestationClientError>;

    /// POST `request_body` verbatim as a chat completion.
    async fn complete(
        &self,
        request_body: &[u8],
    ) -> Result<CompletionOutcome, AttestationClientError>;

    /// Fetch the receipt for a completion id.
    async fn fetch_receipt(&self, chat_id: &str) -> Result<ReceiptPayload, AttestationClientError>;
}

/// The live client.
///
/// `base_url` is the endpoint's `/v1` root with no trailing slash, e.g.
/// `https://qwen3-6-35b.completions.near.ai/v1` -- the same value
/// `TRACE_COMMONS_NEAR_AI_BASE_URL` already carries for the scorer.
pub struct HttpAttestationClient {
    http: reqwest::Client,
    base_url: String,
    model: String,
    api_key: SecretString,
    #[cfg_attr(
        not(feature = "near-attestation-collateral"),
        expect(
            dead_code,
            reason = "read only by the collateral client, which the \
                      near-attestation-collateral feature compiles in"
        )
    )]
    pccs_url: String,
}

impl HttpAttestationClient {
    /// Build a client. `timeout` bounds every one of the four calls.
    pub fn new(
        base_url: impl Into<String>,
        model: impl Into<String>,
        api_key: SecretString,
        pccs_url: impl Into<String>,
        timeout: Duration,
    ) -> Result<Self, AttestationClientError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|e| AttestationClientError::Transport {
                step: AttestationStep::Report,
                detail_hash: detail_hash(&e.to_string()),
            })?;
        Ok(Self {
            http,
            base_url: base_url.into().trim_end_matches('/').to_string(),
            model: model.into(),
            api_key,
            pccs_url: pccs_url.into(),
        })
    }
}

/// The completion response, as much of it as the drill reads.
///
/// Only `id`, which addresses the receipt. Deliberately nothing else: the
/// hashed material is the raw body, and modelling `choices[0].message.content`
/// here previously made the drill depend on a shape the live service does not
/// always produce -- a thinking model returns `content: null` with the text in
/// `reasoning_content`, which would not even deserialize.
#[derive(Debug, Deserialize)]
struct ChatCompletionResponse {
    id: String,
}

/// The receipt response.
#[derive(Debug, Deserialize)]
struct SignatureResponse {
    text: String,
    signature: String,
    signing_address: String,
}

#[async_trait]
impl AttestationClient for HttpAttestationClient {
    fn model(&self) -> &str {
        &self.model
    }

    async fn fetch_report(&self, nonce: &str) -> Result<AttestationReport, AttestationClientError> {
        let step = AttestationStep::Report;
        // No `include_tls_fingerprint` and no `signing_address`: see the
        // module docs. The report endpoint is unauthenticated, but the key is
        // sent anyway so a deployment whose endpoint requires it works too.
        let response = self
            .http
            .get(format!("{}/attestation/report", self.base_url))
            .query(&[
                ("model", self.model.as_str()),
                ("nonce", nonce),
                ("signing_algo", "ecdsa"),
            ])
            .bearer_auth(self.api_key.expose_secret())
            .send()
            .await
            .map_err(|e| AttestationClientError::Transport {
                step,
                detail_hash: detail_hash(&e.to_string()),
            })?;
        let body = success_body(step, response).await?;
        AttestationReport::from_json(&body).map_err(|e| AttestationClientError::MalformedResponse {
            step,
            detail_hash: detail_hash(&format!("{e:#}")),
        })
    }

    #[cfg(feature = "near-attestation-collateral")]
    async fn fetch_collateral(&self, quote: &[u8]) -> Result<Collateral, AttestationClientError> {
        let step = AttestationStep::Collateral;
        let client = dcap_qvl::collateral::CollateralClient::with_default_http(&self.pccs_url)
            .map_err(|e| AttestationClientError::Transport {
                step,
                detail_hash: detail_hash(&format!("{e:#}")),
            })?;
        client
            .fetch(quote)
            .await
            .map_err(|e| AttestationClientError::Transport {
                step,
                detail_hash: detail_hash(&format!("{e:#}")),
            })
    }

    #[cfg(not(feature = "near-attestation-collateral"))]
    async fn fetch_collateral(&self, _quote: &[u8]) -> Result<Collateral, AttestationClientError> {
        Err(AttestationClientError::MissingControl {
            step: AttestationStep::Collateral,
            control: COLLATERAL_CLIENT_CONTROL,
        })
    }

    async fn complete(
        &self,
        request_body: &[u8],
    ) -> Result<CompletionOutcome, AttestationClientError> {
        let step = AttestationStep::Completion;
        // `.body(Vec<u8>)`, not `.json(&value)`: the receipt binds
        // SHA-256 over the bytes actually sent.
        let response = self
            .http
            .post(format!("{}/chat/completions", self.base_url))
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .bearer_auth(self.api_key.expose_secret())
            .body(request_body.to_vec())
            .send()
            .await
            .map_err(|e| AttestationClientError::Transport {
                step,
                detail_hash: detail_hash(&e.to_string()),
            })?;
        let body = success_body(step, response).await?;
        let parsed: ChatCompletionResponse =
            serde_json::from_str(&body).map_err(|e| AttestationClientError::MalformedResponse {
                step,
                detail_hash: detail_hash(&e.to_string()),
            })?;
        // `body`, not a re-serialization of `parsed`: the receipt binds
        // SHA-256 over the bytes the service sent.
        Ok(CompletionOutcome {
            chat_id: parsed.id,
            response_body: body,
        })
    }

    async fn fetch_receipt(&self, chat_id: &str) -> Result<ReceiptPayload, AttestationClientError> {
        let step = AttestationStep::Receipt;
        let response = self
            .http
            .get(format!(
                "{}/signature/{}",
                self.base_url,
                urlencode_path_segment(chat_id)
            ))
            .query(&[("model", self.model.as_str()), ("signing_algo", "ecdsa")])
            .bearer_auth(self.api_key.expose_secret())
            .send()
            .await
            .map_err(|e| AttestationClientError::Transport {
                step,
                detail_hash: detail_hash(&e.to_string()),
            })?;
        let body = success_body(step, response).await?;
        let parsed: SignatureResponse =
            serde_json::from_str(&body).map_err(|e| AttestationClientError::MalformedResponse {
                step,
                detail_hash: detail_hash(&e.to_string()),
            })?;
        Ok(ReceiptPayload {
            text: parsed.text,
            signature: parsed.signature,
            signing_address: parsed.signing_address,
            signing_algo: ReceiptAlgo::Ecdsa,
        })
    }
}

/// Read a response body, turning a non-success status into an error that
/// names the status and nothing else.
async fn success_body(
    step: AttestationStep,
    response: reqwest::Response,
) -> Result<String, AttestationClientError> {
    let status = response.status();
    if !status.is_success() {
        return Err(AttestationClientError::HttpStatus {
            step,
            status: status.as_u16(),
        });
    }
    response
        .text()
        .await
        .map_err(|e| AttestationClientError::Transport {
            step,
            detail_hash: detail_hash(&e.to_string()),
        })
}

/// Percent-encode the characters that would let a completion id escape its
/// path segment. The id is provider-supplied, so it is not trusted to be a
/// bare token.
fn urlencode_path_segment(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_completion_id_cannot_escape_its_path_segment() {
        // A provider that ever returned an id containing a slash or a query
        // separator would otherwise let it address a different endpoint.
        assert_eq!(
            urlencode_path_segment("../attestation/report?x=1"),
            "..%2Fattestation%2Freport%3Fx%3D1"
        );
        assert_eq!(
            urlencode_path_segment("chatcmpl-abc_123.4~x"),
            "chatcmpl-abc_123.4~x"
        );
    }

    #[test]
    fn error_codes_name_the_condition_and_the_step() {
        let err = AttestationClientError::MissingControl {
            step: AttestationStep::Collateral,
            control: COLLATERAL_CLIENT_CONTROL,
        };
        assert_eq!(err.code(), "missing_control");
        assert_eq!(err.step(), AttestationStep::Collateral);
        assert_eq!(err.missing_control(), Some(COLLATERAL_CLIENT_CONTROL));
        let http = AttestationClientError::HttpStatus {
            step: AttestationStep::Receipt,
            status: 401,
        };
        assert_eq!(http.code(), "http_status");
        assert_eq!(http.missing_control(), None);
    }

    /// The refusal above is constructed by hand, so it proves the rendering
    /// and nothing about the build. This one calls the real client and is
    /// therefore the only thing that proves the feature gate is wired: with
    /// `near-attestation-collateral` off, `fetch_collateral` must refuse by
    /// this exact control name, before any network call. With the feature on
    /// the method reaches Intel, so this test cannot exist in that build --
    /// hence the `cfg`, and hence the CI job that builds the other side.
    #[cfg(not(feature = "near-attestation-collateral"))]
    #[tokio::test]
    async fn without_the_collateral_feature_the_client_refuses_by_name() {
        let client = HttpAttestationClient::new(
            "https://invalid.test/v1",
            "model",
            SecretString::from("unused"),
            "https://invalid.test",
            Duration::from_secs(1),
        )
        .expect("client builds");
        let err = client
            .fetch_collateral(b"not a quote")
            .await
            .expect_err("a build without the collateral client cannot fetch collateral");
        assert_eq!(
            err,
            AttestationClientError::MissingControl {
                step: AttestationStep::Collateral,
                control: COLLATERAL_CLIENT_CONTROL,
            }
        );
        assert_eq!(
            err.missing_control(),
            Some("near_ai_attestation_collateral_client")
        );
    }

    #[test]
    fn error_messages_carry_no_url_or_body() {
        // The messages are what reach an operator. `detail_hash` is the only
        // channel for the underlying text, and it is a digest.
        let err = AttestationClientError::Transport {
            step: AttestationStep::Report,
            detail_hash: detail_hash("https://secret.example/v1/attestation/report failed"),
        };
        let rendered = err.to_string();
        assert!(!rendered.contains("https"));
        assert!(!rendered.contains("secret.example"));
        assert!(rendered.contains("report step"));
    }

    #[test]
    fn detail_hash_is_a_truncated_digest_not_the_message() {
        let hashed = detail_hash("token sk-live-abcdef");
        assert_eq!(hashed.len(), 16);
        assert!(!hashed.contains("sk-"));
    }
}
