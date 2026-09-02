//! Talking to a witness: the nonce, the evidence, the collateral, and the one
//! function that sends a raw session.
//!
//! # This module cannot construct a `VerifiedWitness`
//!
//! [`witness_contribution`] is the only function in this crate that transmits
//! an unredacted session, and it takes a
//! [`&VerifiedWitness`](super::verify::VerifiedWitness). That type's fields
//! are private to `super::verify`, so nothing here can build one -- which is
//! what makes "verify, then send" a property of the types rather than of a
//! review. Nothing in this file may change that, and a `pub(crate)`
//! constructor over there would end it silently.

use std::sync::Arc;
use std::time::Duration;

use trace_commons_attestation::quote::{Collateral, parse_collateral};
use trace_commons_operator_client::host_allowlist::HostAllowlist;
use trace_commons_protocol::trace_contribution::{
    ConsentScope, RawTraceContribution, TraceAllowedUse, TraceContributionEnvelope,
};

use super::verify::VerifiedWitness;
use super::{WITNESS_NONCE_LEN, WitnessTrustError};
use crate::envelope::{MAX_ENVELOPE_BYTES, raw_contribution_size_ok};

/// The header the certificate travels in, on the witness response and on
/// `POST /v1/traces`. One spelling, so the client forwards what it received.
pub const WITNESS_CERTIFICATE_HEADER: &str = "x-trace-witness-certificate";
/// The header the signature travels in.
pub const WITNESS_SIGNATURE_HEADER: &str = "x-trace-witness-signature";

/// A contributor's attestation nonce: exactly 32 bytes, fresh per
/// verification.
///
/// The field is private and the production constructor is [`Self::fresh`].
/// **Never reused across submissions**: a reused nonce turns a replayed quote
/// into an accepted one for as long as the reuse lasts, and the reuse is
/// invisible at the response boundary because a replayed quote verifies
/// perfectly.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct WitnessNonce([u8; WITNESS_NONCE_LEN]);

impl WitnessNonce {
    /// 32 fresh bytes from the system CSPRNG.
    ///
    /// `ring::rand::SystemRandom`, already a dependency of this crate. `Err`
    /// when the system source is unavailable, which is a refusal: a nonce
    /// this client did not choose at random is not a nonce.
    pub fn fresh() -> Result<Self, WitnessTrustError> {
        use ring::rand::SecureRandom as _;
        let mut bytes = [0u8; WITNESS_NONCE_LEN];
        ring::rand::SystemRandom::new()
            .fill(&mut bytes)
            .map_err(|_| WitnessTrustError::WitnessAttestationUnavailable)?;
        Ok(Self(bytes))
    }

    /// Build from known bytes. `cfg(test)` only -- a production caller that
    /// could choose the nonce could choose a constant, which is the whole
    /// failure `fresh` exists to prevent.
    #[cfg(test)]
    pub fn from_bytes(bytes: [u8; WITNESS_NONCE_LEN]) -> Self {
        Self(bytes)
    }

    /// The bytes, for building report data and for the query parameter.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Bare lowercase hex, no `0x` prefix -- the encoding
    /// `/v1/attestation?nonce=` accepts and the only one it accepts.
    pub fn to_hex(self) -> String {
        hex::encode(self.0)
    }
}

impl std::fmt::Debug for WitnessNonce {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The nonce is this client's own and is not content; rendering it is
        // what makes a failing attestation readable.
        write!(formatter, "WitnessNonce({})", hex::encode(self.0))
    }
}

/// What `GET /v1/attestation` returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationEvidence {
    /// The raw quote as lowercase hex, no `0x` prefix.
    pub quote_hex: String,
    /// The address the witness says signs its certificates. **Advisory
    /// only**: the address that matters is the pinned one, and the quote's
    /// report data is what binds it. This is carried so a mismatch can be
    /// reported, never so it can be trusted.
    pub signing_address: String,
}

/// The witnessed result: the envelope bytes as received, and the certificate
/// over them.
#[derive(Clone)]
pub struct WitnessedEnvelope {
    /// The serialised envelope, byte for byte as it came off the wire.
    /// Nothing may deserialise, re-serialise, re-order, pretty-print or
    /// append to these before they reach `POST /v1/traces`.
    pub envelope_bytes: Vec<u8>,
    /// The certificate as compact JSON, exactly as the header carried it.
    pub certificate_json: String,
    /// The signature, `0x`-prefixed hex.
    pub signature_hex: String,
}

impl std::fmt::Debug for WitnessedEnvelope {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WitnessedEnvelope")
            .field("envelope_bytes", &"<withheld>")
            .field("certificate_json", &"<withheld>")
            .field("signature_hex", &"<withheld>")
            .finish()
    }
}

/// The two reads a client makes before it trusts anything.
///
/// A trait so the ordering tests can drive a recording double, and so the
/// allowlist gate is observable as "nothing was contacted" rather than
/// inferred from a check existing.
#[async_trait::async_trait]
pub trait WitnessTransport: Send + Sync {
    /// `GET /v1/attestation?nonce=<hex>`.
    async fn attestation(
        &self,
        nonce: &WitnessNonce,
    ) -> Result<AttestationEvidence, WitnessTrustError>;

    /// `POST /v1/attestation-collateral` against **ingest**, not the witness.
    async fn collateral(&self, quote: &[u8]) -> Result<Collateral, WitnessTrustError>;

    /// `POST /v1/witness` with the raw contribution.
    ///
    /// Takes `&VerifiedWitness` rather than a URL, and that is the whole
    /// point: an implementation cannot be called without one, and only
    /// `super::verify` can make one.
    async fn witness(
        &self,
        witness: &VerifiedWitness,
        body: &[u8],
    ) -> Result<WitnessedEnvelope, WitnessTrustError>;
}

/// The consent grant a claim carried, applied by the witness *inside* the
/// certified bytes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GrantedConsent {
    pub scopes: Vec<ConsentScope>,
    pub uses: Vec<TraceAllowedUse>,
}

/// Send a raw session to a witness that has been verified, and check what
/// comes back.
///
/// # Why the client verifies a certificate it is only forwarding
///
/// It is the only party holding both the input and the returned artifact. A
/// witness that returned an artifact its own certificate does not cover is
/// **undetectable on the server**, which would check that certificate against
/// the bytes it holds, find them consistent, and never have seen what was
/// sent. So both halves are checked here: the signature recovers to the
/// **pinned** address, and the digest matches the returned envelope bytes *as
/// received on the wire* -- never a re-serialisation of a parsed envelope,
/// which would compare the certificate against bytes nobody will ever send.
pub async fn witness_contribution(
    transport: &dyn WitnessTransport,
    witness: &VerifiedWitness,
    raw: RawTraceContribution,
    granted: &GrantedConsent,
) -> Result<WitnessedEnvelope, WitnessTrustError> {
    // Refused locally, before anything is offered. The client already refuses
    // raw contributions above this bound in `raw_contribution_size_ok`; what
    // is new here is naming the refusal on this path, where the cost of
    // finding out late is that the session was already transmitted.
    raw_contribution_size_ok(&raw).map_err(|_| WitnessTrustError::WitnessPayloadTooLarge)?;

    let body = serde_json::to_vec(&serde_json::json!({
        "raw_contribution": raw,
        "granted_scopes": granted.scopes,
        "granted_uses": granted.uses,
    }))
    .map_err(|_| WitnessTrustError::WitnessResponseMalformed)?;

    let response = transport.witness(witness, &body).await?;

    // The envelope must still be submittable. Checked before the certificate
    // so an oversized artifact is reported as oversized rather than as a
    // certificate problem.
    if response.envelope_bytes.len() > MAX_ENVELOPE_BYTES {
        return Err(WitnessTrustError::WitnessPayloadTooLarge);
    }

    verify_certificate(&response, witness.signing_address())?;
    Ok(response)
}

/// Check a certificate against the bytes that came back with it.
///
/// Split out so it is testable without a transport, and so the two checks it
/// makes are visible in one place.
fn verify_certificate(
    response: &WitnessedEnvelope,
    pinned_address: &str,
) -> Result<(), WitnessTrustError> {
    use sha2::{Digest as _, Sha256};

    let certificate: serde_json::Value = serde_json::from_str(&response.certificate_json)
        .map_err(|_| WitnessTrustError::WitnessResponseMalformed)?;
    let claimed = certificate
        .get("redacted_sha256")
        .and_then(serde_json::Value::as_str)
        .ok_or(WitnessTrustError::WitnessResponseMalformed)?;

    // Over the bytes as received. Not over a re-serialisation of a parsed
    // envelope, which would compare the certificate against bytes nobody will
    // ever send.
    let actual = hex::encode(Sha256::digest(&response.envelope_bytes));
    if !claimed.eq_ignore_ascii_case(&actual) {
        return Err(WitnessTrustError::WitnessCertificateMismatched);
    }

    // The signature is over the certificate's canonical signing bytes, which
    // the server derives from these same fields. The client checks recovery
    // against the PINNED address, never against an address the witness
    // reported -- a witness that could name the address its signature
    // recovers to could sign with any key at all.
    let signing_bytes = certificate_signing_bytes(&certificate)
        .ok_or(WitnessTrustError::WitnessResponseMalformed)?;
    let recovered = trace_commons_attestation::eip191::recover_eip191_signer(
        &signing_bytes,
        &response.signature_hex,
    )
    .map_err(|_| WitnessTrustError::WitnessCertificateUnverified)?;
    let expected = trace_commons_attestation::address::decode_address(pinned_address)
        .ok_or(WitnessTrustError::WitnessCertificateUnverified)?;
    if recovered != expected {
        return Err(WitnessTrustError::WitnessCertificateUnverified);
    }
    Ok(())
}

/// Rebuild the certificate's signing preimage from its wire fields.
///
/// **Length-prefixed, never JSON.** The server's `WitnessCertificate` has
/// deliberately no `Serialize`, precisely so that no JSON form of it can drift
/// into being treated as the signing preimage: `serde_json`'s map ordering is
/// not guaranteed, and `serde_json/preserve_order` -- which `dcap-qvl` enables
/// in this crate's graph -- has already moved digests in this workspace once.
/// The length prefixes are what make the encoding injective; concatenating
/// fields directly would let content shift across a boundary without changing
/// the bytes.
///
/// This is a second implementation of an encoding whose first implementation
/// is AGPL and unreachable from here. That duplication is a real cost and the
/// reason `a_certificate_this_client_accepts_is_one_the_server_issued` exists:
/// it drives the server's own witness through a fixture and requires this
/// function to verify what that produced, so the two cannot drift silently.
fn certificate_signing_bytes(certificate: &serde_json::Value) -> Option<Vec<u8>> {
    const SIGNING_DOMAIN: &[u8] = b"trace_commons.redaction_witness_certificate.v1\n";

    let digest = certificate.get("redacted_sha256")?.as_str()?;
    let verdict = certificate.get("residual_risk_verdict")?.as_str()?;
    let policy = certificate.get("redaction_policy_version")?.as_str()?;
    let measurement = certificate.get("witness_measurement")?.as_str()?;
    let timestamp = certificate.get("timestamp")?.as_i64()?;

    // The verdict is a fixed-width tag, not its label. These values are
    // assigned permanently on the server side; changing one would let a
    // Medium certificate re-verify as Low.
    let verdict_tag = match verdict {
        "low" => 1u8,
        "medium" => 2,
        "high" => 3,
        _ => return None,
    };

    let mut bytes = Vec::new();
    bytes.extend_from_slice(SIGNING_DOMAIN);
    for field in [digest, policy, measurement] {
        bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
        bytes.extend_from_slice(field.as_bytes());
    }
    bytes.push(verdict_tag);
    bytes.extend_from_slice(&timestamp.to_be_bytes());
    Some(bytes)
}

/// The HTTP implementation.
pub struct HttpWitnessTransport {
    http: reqwest::Client,
    witness_url: String,
    collateral_url: String,
    allowlist: Arc<HostAllowlist>,
}

impl HttpWitnessTransport {
    /// Build a transport. `collateral_url` is the **ingest** base URL, not
    /// the witness: collateral comes from ingest, which already has a PCCS
    /// and the rustls provider its client needs.
    pub fn new(
        witness_url: impl Into<String>,
        collateral_url: impl Into<String>,
        allowlist: Arc<HostAllowlist>,
        timeout: Duration,
    ) -> Result<Self, WitnessTrustError> {
        let http = reqwest::Client::builder()
            .timeout(timeout)
            .build()
            .map_err(|_| WitnessTrustError::WitnessAttestationUnavailable)?;
        Ok(Self {
            http,
            witness_url: witness_url.into(),
            collateral_url: collateral_url.into(),
            allowlist,
        })
    }

    /// The allowlist gate, applied **before** a request is built.
    ///
    /// The same `HostAllowlist` `issuer_url` and `ingest_url` pass. A host
    /// outside it is refused with nothing contacted, which the ordering tests
    /// assert directly rather than inferring from this check existing.
    fn allowed(&self, url: &str) -> Result<url::Url, WitnessTrustError> {
        let parsed = url::Url::parse(url).map_err(|_| WitnessTrustError::WitnessHostNotAllowed)?;
        self.allowlist
            .check(&parsed)
            .map_err(|_| WitnessTrustError::WitnessHostNotAllowed)?;
        Ok(parsed)
    }
}

#[async_trait::async_trait]
impl WitnessTransport for HttpWitnessTransport {
    async fn attestation(
        &self,
        nonce: &WitnessNonce,
    ) -> Result<AttestationEvidence, WitnessTrustError> {
        let base = self.allowed(&self.witness_url)?;
        let url = base
            .join("/v1/attestation")
            .map_err(|_| WitnessTrustError::WitnessHostNotAllowed)?;
        let response = self
            .http
            .get(url)
            .query(&[("nonce", nonce.to_hex())])
            .send()
            .await
            .map_err(|_| WitnessTrustError::WitnessAttestationUnavailable)?;
        if !response.status().is_success() {
            return Err(WitnessTrustError::WitnessAttestationUnavailable);
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|_| WitnessTrustError::WitnessAttestationUnavailable)?;
        Ok(AttestationEvidence {
            quote_hex: body
                .get("quote_hex")
                .and_then(serde_json::Value::as_str)
                .ok_or(WitnessTrustError::WitnessAttestationUnavailable)?
                .to_string(),
            signing_address: body
                .get("signing_address")
                .and_then(serde_json::Value::as_str)
                .ok_or(WitnessTrustError::WitnessAttestationUnavailable)?
                .to_string(),
        })
    }

    async fn collateral(&self, quote: &[u8]) -> Result<Collateral, WitnessTrustError> {
        let base = self.allowed(&self.collateral_url)?;
        let url = base
            .join("/v1/attestation-collateral")
            .map_err(|_| WitnessTrustError::WitnessHostNotAllowed)?;
        let response = self
            .http
            .post(url)
            .json(&serde_json::json!({ "quote_hex": hex::encode(quote) }))
            .send()
            .await
            .map_err(|_| WitnessTrustError::WitnessCollateralUnavailable)?;
        if !response.status().is_success() {
            return Err(WitnessTrustError::WitnessCollateralUnavailable);
        }
        let body = response
            .text()
            .await
            .map_err(|_| WitnessTrustError::WitnessCollateralUnavailable)?;
        parse_collateral(&body).map_err(|_| WitnessTrustError::WitnessCollateralUnavailable)
    }

    async fn witness(
        &self,
        witness: &VerifiedWitness,
        body: &[u8],
    ) -> Result<WitnessedEnvelope, WitnessTrustError> {
        let base = self.allowed(witness.url())?;
        let url = base
            .join("/v1/witness")
            .map_err(|_| WitnessTrustError::WitnessHostNotAllowed)?;
        let response = self
            .http
            .post(url)
            .header(reqwest::header::CONTENT_TYPE, "application/json")
            .body(body.to_vec())
            .send()
            .await
            .map_err(|_| WitnessTrustError::WitnessAttestationUnavailable)?;
        if !response.status().is_success() {
            return Err(WitnessTrustError::WitnessResponseMalformed);
        }
        let headers = response.headers().clone();
        let read = |name: &str| {
            headers
                .get(name)
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
                .ok_or(WitnessTrustError::WitnessResponseMalformed)
        };
        let certificate_json = read(WITNESS_CERTIFICATE_HEADER)?;
        let signature_hex = read(WITNESS_SIGNATURE_HEADER)?;
        // `bytes()`, never `json()`: the certificate covers these exact bytes
        // and a parse-then-reserialise here would break the digest before the
        // client ever checked it.
        let envelope_bytes = response
            .bytes()
            .await
            .map_err(|_| WitnessTrustError::WitnessResponseMalformed)?
            .to_vec();
        Ok(WitnessedEnvelope {
            envelope_bytes,
            certificate_json,
            signature_hex,
        })
    }
}

/// Parse the bytes a witness returned, for a caller that needs the envelope
/// as a value.
///
/// Provided so that no caller is tempted to parse and then re-serialise: the
/// parsed value is for reading, and `envelope_bytes` remains the only thing
/// that is ever submitted.
pub fn parse_witnessed_envelope(
    response: &WitnessedEnvelope,
) -> Result<TraceContributionEnvelope, WitnessTrustError> {
    serde_json::from_slice(&response.envelope_bytes)
        .map_err(|_| WitnessTrustError::WitnessResponseMalformed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::extract::{Query, Request};
    use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
    use axum::response::{IntoResponse, Response};
    use axum::routing::{get, post};
    use std::collections::HashMap;
    use std::sync::Mutex;

    /// What a local witness saw.
    ///
    /// Assertions are about what reached the wire, not about what the client
    /// believes it sent -- which is the only way to state "nothing was
    /// contacted" as a fact rather than as an inference from a check
    /// existing.
    #[derive(Default)]
    struct Seen {
        /// Route names in the order they were requested.
        routes: Vec<String>,
        /// The `nonce` query parameter, as received.
        nonces: Vec<String>,
        /// Bodies posted to `/v1/witness`, as received.
        witness_bodies: Vec<Vec<u8>>,
        /// Bodies posted to the collateral route, as received.
        collateral_bodies: Vec<Vec<u8>>,
    }

    /// What a local witness should answer with. `None` on any field makes
    /// that route 503, which is how the unreachable-route tests are driven.
    #[derive(Default, Clone)]
    struct Answers {
        attestation: Option<serde_json::Value>,
        collateral: Option<String>,
        witness: Option<(String, String, Vec<u8>)>,
    }

    struct LocalWitness {
        base: String,
        seen: Arc<Mutex<Seen>>,
        _shutdown: tokio::sync::oneshot::Sender<()>,
    }

    impl LocalWitness {
        fn routes(&self) -> Vec<String> {
            self.seen.lock().unwrap().routes.clone()
        }
        fn nonces(&self) -> Vec<String> {
            self.seen.lock().unwrap().nonces.clone()
        }
        fn collateral_bodies(&self) -> Vec<Vec<u8>> {
            self.seen.lock().unwrap().collateral_bodies.clone()
        }
    }

    /// Spawn a witness on an ephemeral port.
    ///
    /// A real socket rather than a mock: the thing under test is the
    /// transport -- which URL it composes, which query parameter it sets, and
    /// whether a body reached the wire at all.
    async fn local_witness(answers: Answers) -> LocalWitness {
        let seen = Arc::new(Mutex::new(Seen::default()));

        let attestation_seen = seen.clone();
        let attestation = answers.attestation.clone();
        let collateral_seen = seen.clone();
        let collateral = answers.collateral.clone();
        let witness_seen = seen.clone();
        let witness = answers.witness.clone();

        let app = Router::new()
            .route(
                "/v1/attestation",
                get(move |Query(query): Query<HashMap<String, String>>| {
                    let seen = attestation_seen.clone();
                    let body = attestation.clone();
                    async move {
                        {
                            let mut seen = seen.lock().unwrap();
                            seen.routes.push("attestation".to_string());
                            if let Some(nonce) = query.get("nonce") {
                                seen.nonces.push(nonce.clone());
                            }
                        }
                        match body {
                            Some(body) => axum::Json(body).into_response(),
                            None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                        }
                    }
                }),
            )
            .route(
                "/v1/attestation-collateral",
                post(move |request: Request| {
                    let seen = collateral_seen.clone();
                    let body = collateral.clone();
                    async move {
                        let bytes = axum::body::to_bytes(request.into_body(), 1024 * 1024)
                            .await
                            .unwrap_or_default();
                        {
                            let mut seen = seen.lock().unwrap();
                            seen.routes.push("collateral".to_string());
                            seen.collateral_bodies.push(bytes.to_vec());
                        }
                        match body {
                            Some(body) => body.into_response(),
                            None => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                        }
                    }
                }),
            )
            .route(
                "/v1/witness",
                post(move |request: Request| {
                    let seen = witness_seen.clone();
                    let answer = witness.clone();
                    async move {
                        let bytes = axum::body::to_bytes(request.into_body(), 64 * 1024 * 1024)
                            .await
                            .unwrap_or_default();
                        {
                            let mut seen = seen.lock().unwrap();
                            seen.routes.push("witness".to_string());
                            seen.witness_bodies.push(bytes.to_vec());
                        }
                        witness_answer(answer)
                    }
                }),
            );

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let (tx, rx) = tokio::sync::oneshot::channel();
        tokio::spawn(async move {
            let _ = axum::serve(listener, app)
                .with_graceful_shutdown(async {
                    let _ = rx.await;
                })
                .await;
        });
        LocalWitness {
            base,
            seen,
            _shutdown: tx,
        }
    }

    fn witness_answer(answer: Option<(String, String, Vec<u8>)>) -> Response {
        let Some((certificate, signature, envelope)) = answer else {
            return StatusCode::SERVICE_UNAVAILABLE.into_response();
        };
        let mut headers = HeaderMap::new();
        headers.insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        headers.insert(
            WITNESS_CERTIFICATE_HEADER,
            HeaderValue::from_str(&certificate).expect("a test certificate header"),
        );
        headers.insert(
            WITNESS_SIGNATURE_HEADER,
            HeaderValue::from_str(&signature).expect("a test signature header"),
        );
        (headers, envelope).into_response()
    }

    fn transport_for(base: &str, allowlist: HostAllowlist) -> HttpWitnessTransport {
        HttpWitnessTransport::new(
            base.to_string(),
            base.to_string(),
            Arc::new(allowlist),
            Duration::from_secs(5),
        )
        .expect("the transport builds")
    }

    fn permissive() -> HostAllowlist {
        HostAllowlist::permissive()
    }

    #[tokio::test]
    async fn the_nonce_on_the_wire_is_the_one_we_will_check_against() {
        let server = local_witness(Answers {
            attestation: Some(serde_json::json!({
                "quote_hex": "00ff",
                "signing_address": "0x1111111111111111111111111111111111111111",
            })),
            ..Answers::default()
        })
        .await;
        let transport = transport_for(&server.base, permissive());

        let nonce = WitnessNonce::fresh().expect("the system CSPRNG is available");
        let evidence = transport.attestation(&nonce).await.expect("evidence");

        assert_eq!(server.nonces(), vec![nonce.to_hex()]);
        // Bare hex, no `0x`: the witness surface accepts that encoding and
        // only that one, and a prefixed nonce would be refused as malformed.
        assert!(!nonce.to_hex().starts_with("0x"));
        assert_eq!(nonce.to_hex().len(), WITNESS_NONCE_LEN * 2);
        assert_eq!(evidence.quote_hex, "00ff");
    }

    #[tokio::test]
    async fn two_verifications_never_reuse_a_nonce() {
        // A reused nonce turns a replayed quote into an accepted one for as
        // long as the reuse lasts, and the reuse is invisible at the response
        // boundary because a replayed quote verifies perfectly.
        let mut seen = std::collections::HashSet::new();
        for _ in 0..32 {
            let nonce = WitnessNonce::fresh().expect("the system CSPRNG is available");
            assert!(
                seen.insert(nonce.to_hex()),
                "WitnessNonce::fresh repeated a value"
            );
        }
    }

    #[tokio::test]
    async fn a_host_outside_the_allowlist_is_refused_before_any_request() {
        let server = local_witness(Answers {
            attestation: Some(serde_json::json!({
                "quote_hex": "00ff",
                "signing_address": "0x1111111111111111111111111111111111111111",
            })),
            ..Answers::default()
        })
        .await;
        // An allowlist naming somebody else. The transport still points at the
        // live server, so if the gate were applied after the request the
        // server would record it.
        let transport = transport_for(&server.base, HostAllowlist::from_csv("allowed.example"));

        let err = transport
            .attestation(&WitnessNonce::fresh().unwrap())
            .await
            .expect_err("a host outside the allowlist is refused");
        assert_eq!(err, WitnessTrustError::WitnessHostNotAllowed);
        assert!(
            server.routes().is_empty(),
            "a refused host was still contacted: {:?}",
            server.routes()
        );
    }

    #[tokio::test]
    async fn an_unreachable_attestation_route_refuses_by_name() {
        let server = local_witness(Answers::default()).await;
        let transport = transport_for(&server.base, permissive());
        assert_eq!(
            transport
                .attestation(&WitnessNonce::fresh().unwrap())
                .await
                .unwrap_err(),
            WitnessTrustError::WitnessAttestationUnavailable
        );
    }

    #[tokio::test]
    async fn an_attestation_response_missing_a_field_refuses_rather_than_defaulting() {
        // A response that parses as JSON but names no quote. Defaulting to an
        // empty quote here would send an empty string into `verify_quote`,
        // which would fail -- but under the wrong error, telling a contributor
        // the quote did not verify when the witness never sent one.
        let server = local_witness(Answers {
            attestation: Some(serde_json::json!({ "signing_address": "0x11" })),
            ..Answers::default()
        })
        .await;
        let transport = transport_for(&server.base, permissive());
        assert_eq!(
            transport
                .attestation(&WitnessNonce::fresh().unwrap())
                .await
                .unwrap_err(),
            WitnessTrustError::WitnessAttestationUnavailable
        );
    }

    #[tokio::test]
    async fn missing_collateral_refuses_rather_than_verifying_without_it() {
        let server = local_witness(Answers::default()).await;
        let transport = transport_for(&server.base, permissive());
        assert_eq!(
            transport.collateral(b"quote").await.unwrap_err(),
            WitnessTrustError::WitnessCollateralUnavailable
        );
    }

    #[tokio::test]
    async fn malformed_collateral_refuses_rather_than_being_used() {
        let server = local_witness(Answers {
            collateral: Some("not collateral".to_string()),
            ..Answers::default()
        })
        .await;
        let transport = transport_for(&server.base, permissive());
        assert_eq!(
            transport.collateral(b"quote").await.unwrap_err(),
            WitnessTrustError::WitnessCollateralUnavailable
        );
    }

    #[tokio::test]
    async fn the_collateral_request_names_the_quote_it_is_for() {
        let server = local_witness(Answers {
            collateral: Some("{}".to_string()),
            ..Answers::default()
        })
        .await;
        let transport = transport_for(&server.base, permissive());
        let _ = transport.collateral(b"\x01\x02\xab").await;

        assert_eq!(server.routes(), vec!["collateral".to_string()]);
        let sent: serde_json::Value =
            serde_json::from_slice(&server.collateral_bodies()[0]).expect("the body is JSON");
        // Collateral for the wrong quote verifies nothing, and the failure
        // would look like a bad quote rather than a bad request.
        assert_eq!(sent["quote_hex"], "0102ab");
    }

    #[tokio::test]
    async fn a_nonce_debug_renders_the_nonce_and_nothing_else() {
        let nonce = WitnessNonce::from_bytes([0x01u8; WITNESS_NONCE_LEN]);
        let rendered = format!("{nonce:?}");
        assert!(rendered.contains(&hex::encode([0x01u8; WITNESS_NONCE_LEN])));
        assert!(rendered.starts_with("WitnessNonce("));
    }
}
