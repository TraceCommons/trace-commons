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
