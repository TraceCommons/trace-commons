// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The enclave seam: the witness's signing key, and its nonce-bound quote.
//!
//! Two things a contributor needs before it will send raw bytes to a witness:
//! the address that will sign the certificate, and hardware evidence that the
//! address belongs to a program the contributor pinned. This module produces
//! both from a dstack guest agent, and it binds them to each other.
//!
//! # The binding is the whole point
//!
//! A quote over a fixed report body is a replay: an operator who once ran the
//! honest image can serve that quote forever from a machine running something
//! else. So the contributor chooses a 32-byte nonce, and
//! [`witness_report_data`] lays it into the 64 bytes the TDX module signs,
//! next to the signing address:
//!
//! ```text
//! 0        8                       28                          60      64
//! | "tcwitns1" | signing address (20) | contributor nonce (32) | zeros |
//! ```
//!
//! Literal, not hashed. A verifier reads its own nonce out of the quote it was
//! handed and compares 20 bytes against the address
//! `verify_witness_certificate` recovered, with no digest construction to
//! reimplement and get subtly wrong. The eight-byte tag is domain separation:
//! dstack will happily quote report data composed by some other surface of the
//! same app (`Worker.GetAttestationForAppKey` uses a `dip1::` prefix of its
//! own), and a quote issued for another purpose must not read as a witness
//! quote.
//!
//! [`DstackEnclave::attestation_quote`] does not trust the agent to have done
//! this. It parses the quote it gets back and refuses unless the report data
//! in the signed body is the report data it asked for, and unless MRTD still
//! matches what this process measured at boot. Without that check the binding
//! would be an intention rather than a property.
//!
//! # Why `GetKey` and not `Sign`
//!
//! The guest agent has a `Sign` method, and it is the wrong one. It returns 64
//! bytes, `r || s`, with no recovery byte -- and
//! [`crate::redaction_witness::verification`] recovers the signer from an
//! EIP-191 signature, which requires the 65-byte `r || s || v` form. A witness
//! that signed with the agent would emit certificates its own verifier cannot
//! check.
//!
//! So this module calls `GetKey(path = "vms", purpose = "signing", algorithm =
//! "secp256k1")` -- the same derivation `Sign` uses internally -- takes the raw
//! 32-byte private scalar it returns, and produces the recoverable signature
//! in-process with `k256`. One signing scheme, one place it is implemented,
//! and no socket round trip per signature.
//!
//! # What this key is not
//!
//! It is not isolated. dstack derives it as
//! `HKDF-SHA256(salt = "RATLS", ikm = app root secret, info = path)`, and the
//! `algorithm` argument does not enter the derivation at all: the secp256k1
//! and ed25519 forms of `path = "vms"` are the **same 32 bytes**, and the
//! deprecated `Tappd.DeriveKey` serves that same secret as a P-256 key on a
//! different socket. dstack's own documentation states it plainly --
//! compromise of one is compromise of all. Nothing here may be written as
//! though a purpose string bought separation.
//!
//! `GetKey` also returns a two-link chain -- the app root key signing the key
//! claim, and the KMS root key signing the app root public key -- which binds
//! this address to the app id without any quote at all. It is captured
//! ([`DstackEnclave::signing_key_chain`]) and deliberately unused: verifying
//! it means trusting a KMS root, and reading that root from the KMS you are
//! checking proves nothing. It is here so a later slice that acquires the root
//! out of band does not have to re-plumb the call.
//!
//! # What is pinned, and what an upgrade does
//!
//! [`Enclave::measurement`] reports **MRTD and MRCONFIGID**, and nothing else.
//!
//! - Not RTMR3: it carries a per-deployment random `instance-id`, so two
//!   instances of byte-identical code report different values and no pin could
//!   admit both.
//! - Not RTMR0: it hashes SMBIOS tables, which change when the VM is resized.
//!   Growing the machine would break every pin.
//!
//! MRTD is the boot-time measurement of the VM image. MRCONFIGID is the
//! configuration the image was launched with; on a live NEAR AI deployment
//! captured 2026-09-02 it is **config-id v1** -- the byte `01`, the compose
//! hash, then fifteen zero bytes. v2 additionally commits to the app id and
//! the key-provider identity. Either form pins the compose hash, which is the
//! code identity; **do not claim app-id binding for a v1 deployment**, because
//! v1 does not carry one.
//!
//! The signing address and the measurement are pinned **separately**, and the
//! reason is operational: the key derives from the stable app id, not from the
//! measurement, so an image upgrade changes MRTD and keeps the address. That
//! makes an upgrade a re-allowlisting of one measurement rather than a
//! fleet-wide identity break.
//!
//! That guarantee holds for an image upgrade **and not for a surface
//! migration**. dstack's v1 guest API derives different key material by
//! design, with no compatibility mode: moving off `/v0/` would change the
//! signing address for every deployment at once. This module therefore pins
//! the surface explicitly in [`GET_KEY_PATH`] and [`GET_QUOTE_PATH`], and an
//! operator reading the runbook needs to know that changing them is a key
//! rotation, not a version bump.

use super::{Enclave, SeamUnavailable, Signer};
use async_trait::async_trait;
use k256::ecdsa::SigningKey;
use sha3::{Digest, Keccak256};
use zeroize::Zeroizing;

/// The guest agent's unix socket. Not network-reachable, by design: the
/// witness proxies it.
pub const DSTACK_SOCKET_PATH: &str = "/var/run/dstack.sock";

/// The pinned `GetKey` surface. Changing the `v0` here changes the derived
/// key, and therefore the signing address of every deployment.
pub const GET_KEY_PATH: &str = "/v0/GetKey";

/// The pinned `GetQuote` surface. See [`GET_KEY_PATH`] on why the version is
/// spelled out rather than left to the agent's unversioned alias.
pub const GET_QUOTE_PATH: &str = "/v0/GetQuote";

/// `GetKey`'s HKDF info string. `Sign` uses this same value internally, which
/// is what makes the in-process signature and an agent signature the same key.
pub const SIGNING_KEY_PATH: &str = "vms";

/// Echoed into the `GetKey` chain claim. It does **not** enter the derivation;
/// see the module docs on what this key is not.
pub const SIGNING_KEY_PURPOSE: &str = "signing";

/// The curve asked for. Also does not enter the derivation.
pub const SIGNING_KEY_ALGORITHM: &str = "secp256k1";

/// Domain separation for witness report data. Eight bytes, so the address
/// lands on a readable offset.
pub const WITNESS_QUOTE_DOMAIN: &[u8; 8] = b"tcwitns1";

/// The nonce a contributor supplies. Exactly 32 bytes: shorter is a weaker
/// freshness claim than the caller thinks it is making, and this module will
/// not silently accept one.
pub const WITNESS_NONCE_LEN: usize = 32;

/// TDX signs exactly this many bytes of caller-supplied report data.
pub const REPORT_DATA_LEN: usize = 64;

/// Where the signing address sits in the report data.
const ADDRESS_AT: usize = 8;

/// Where the contributor's nonce sits in the report data.
const NONCE_AT: usize = 28;

/// Raw bytes of an Ethereum-style address.
const ADDRESS_LEN: usize = 20;

/// Offsets into a TDX quote, measured from the start of the quote. The 48-byte
/// v4 header precedes a TD report body whose layout is fixed by the TDX module
/// specification. Used only to *construct* test quotes; the implementation
/// parses with `dcap_qvl` rather than trusting these.
#[cfg(test)]
mod quote_offsets {
    pub(super) const MR_TD: usize = 48 + 136;
    pub(super) const MR_CONFIG_ID: usize = 48 + 184;
    pub(super) const RT_MR0: usize = 48 + 328;
    pub(super) const RT_MR3: usize = 48 + 472;
    pub(super) const REPORT_DATA: usize = 48 + 520;
}

/// Why the enclave seam refused.
///
/// Label-only, like [`super::WitnessError`]: these reach operational surfaces,
/// and a quote request carries a contributor-chosen nonce, so no variant may
/// echo a value. Callers branch on the variant.
///
/// There is no variant that means "quote unavailable, proceeding". A caller
/// that cannot get a bound quote has no evidence, and no evidence is a
/// refusal.
#[derive(Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum EnclaveError {
    /// The nonce was not exactly [`WITNESS_NONCE_LEN`] bytes.
    #[error("the supplied nonce is not the expected length")]
    NonceLength,
    /// The report data offered was longer than TDX will sign. dstack treats
    /// this as an error and so does this module: a truncated report body drops
    /// exactly the caller's nonce.
    #[error("the report data offered is longer than the platform will sign")]
    ReportDataTooLong,
    /// A signing address was not `0x` followed by 40 hex characters.
    #[error("the signing address is malformed")]
    SigningAddressMalformed,
    /// The guest agent socket could not be reached or the exchange failed.
    #[error("the guest agent could not be reached")]
    Transport,
    /// The guest agent answered, and refused.
    #[error("the guest agent refused the request with HTTP {status}")]
    AgentRefused {
        /// The status line's code, carried because "refused" alone is not
        /// diagnosable: a 404 means the method or its surface is wrong, a 400
        /// means the arguments are, and a 403 means the caller is. Those need
        /// different fixes and the operator cannot tell them apart from the
        /// message otherwise. A status code is not contributor data and does
        /// not fall under the hash-only rule.
        status: u16,
    },
    /// The guest agent's answer did not have the shape this client expects.
    #[error("the guest agent's response could not be read")]
    MalformedResponse,
    /// The 32 bytes the agent derived are not a valid secp256k1 scalar.
    #[error("the derived signing key is unusable")]
    SigningKeyUnusable,
    /// The bytes returned did not parse as a TDX quote.
    #[error("the quote could not be parsed")]
    QuoteUnparsable,
    /// The quote parsed, and is not a TD report. As of dstack 0.6.0 `GetQuote`
    /// is Intel TDX only, so this means the platform is not what the pin
    /// describes.
    #[error("the quote does not carry a TD report")]
    QuoteNotTdx,
    /// The signed report data is not the report data that was asked for. This
    /// is the replay guard: a quote that does not carry the caller's nonce
    /// proves nothing about now.
    #[error("the quote does not carry the report data it was asked for")]
    ReportDataMismatch,
    /// The quote reports a different MRTD than this process measured at boot.
    /// Either the agent is proxying someone else's quote or the platform
    /// changed underneath a running witness; both are refusals.
    #[error("the quote reports a different measurement than this witness booted with")]
    MeasurementChanged,
    /// The signature could not be produced.
    #[error("the witness could not sign")]
    SigningFailed,
}

impl std::fmt::Debug for EnclaveError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// What `GetKey` returned.
///
/// The scalar is [`Zeroizing`] so a transport buffer is not the thing that
/// keeps the app secret alive; [`SigningKey`] zeroizes itself thereafter.
pub struct DerivedSigningKey {
    /// The 32 derived bytes: a big-endian secp256k1 private scalar.
    pub scalar: Zeroizing<[u8; 32]>,
    /// The two-link chain to the KMS root, each element 65 bytes. Captured,
    /// not verified; see the module docs.
    pub chain: Vec<Vec<u8>>,
}

/// The transport to a dstack guest agent.
///
/// Separated from [`DstackEnclave`] so the binding logic -- which is the part
/// worth testing -- runs against a substitute rather than a socket. The two
/// methods are the only two calls this service makes.
#[async_trait]
pub trait GuestAgent: Send + Sync {
    /// `GetKey(path = "vms", purpose = "signing", algorithm = "secp256k1")`.
    async fn signing_key(&self) -> Result<DerivedSigningKey, EnclaveError>;

    /// `GetQuote(report_data)`, returning the **raw** quote bytes.
    ///
    /// Raw deliberately: dstack 0.5.9 rewired the v1 `VersionedAttestation`
    /// envelope to msgpack, and this workspace has no decoder for it. The raw
    /// quote is what `dcap_qvl` verifies anyway.
    async fn quote(&self, report_data: &[u8; REPORT_DATA_LEN]) -> Result<Vec<u8>, EnclaveError>;
}

/// Lay a signing address and a contributor nonce into the 64 bytes TDX signs.
///
/// See the module docs for the layout and why it is literal rather than
/// hashed. The trailing four bytes are zero, which is also what the platform
/// would pad them to.
pub fn witness_report_data(
    signing_address: &str,
    nonce: &[u8],
) -> Result<[u8; REPORT_DATA_LEN], EnclaveError> {
    if nonce.len() != WITNESS_NONCE_LEN {
        return Err(EnclaveError::NonceLength);
    }
    let address = decode_address(signing_address)?;

    let mut report_data = [0u8; REPORT_DATA_LEN];
    report_data[..WITNESS_QUOTE_DOMAIN.len()].copy_from_slice(WITNESS_QUOTE_DOMAIN);
    report_data[ADDRESS_AT..ADDRESS_AT + ADDRESS_LEN].copy_from_slice(&address);
    report_data[NONCE_AT..NONCE_AT + WITNESS_NONCE_LEN].copy_from_slice(nonce);
    Ok(report_data)
}

/// `0x` + 40 hex characters, and nothing else. Case-insensitive: this compares
/// bytes, and a checksummed address and its lowercase form are the same
/// address.
fn decode_address(signing_address: &str) -> Result<[u8; ADDRESS_LEN], EnclaveError> {
    let body = signing_address
        .strip_prefix("0x")
        .ok_or(EnclaveError::SigningAddressMalformed)?;
    let bytes = hex::decode(body).map_err(|_| EnclaveError::SigningAddressMalformed)?;
    bytes
        .try_into()
        .map_err(|_| EnclaveError::SigningAddressMalformed)
}

/// The measurement fields this witness pins, read out of a signed quote.
#[derive(Clone, Copy, PartialEq, Eq)]
struct QuoteFacts {
    mr_td: [u8; 48],
    mr_config_id: [u8; 48],
    report_data: [u8; REPORT_DATA_LEN],
}

impl QuoteFacts {
    /// Parse with `dcap_qvl` -- the same crate that verifies quotes elsewhere
    /// in this workspace -- rather than reading fixed offsets, so a TD 1.5
    /// report is handled and a malformed quote is a refusal rather than a
    /// misread slice.
    ///
    /// This parses; it does not verify. Signature and TCB verification is
    /// `trace_commons_attestation`'s job and needs collateral this process
    /// does not fetch. What the parse is trusted for here is narrow and
    /// sound: the agent is on a local socket inside the same VM, and the
    /// checks built on these facts are "is this the report data I just asked
    /// for" and "is this still the MRTD I booted with", both of which catch a
    /// substituted quote regardless of whether its signature is good.
    fn parse(quote: &[u8]) -> Result<Self, EnclaveError> {
        let parsed =
            dcap_qvl::quote::Quote::parse(quote).map_err(|_| EnclaveError::QuoteUnparsable)?;
        let report = match parsed.report {
            dcap_qvl::quote::Report::TD10(report) => report,
            dcap_qvl::quote::Report::TD15(report) => report.base,
            dcap_qvl::quote::Report::SgxEnclave(_) => return Err(EnclaveError::QuoteNotTdx),
        };
        Ok(Self {
            mr_td: report.mr_td,
            mr_config_id: report.mr_config_id,
            report_data: report.report_data,
        })
    }

    /// The string an operator pins and a certificate reports.
    ///
    /// Two named, `+`-joined hex fields rather than a bare digest, so an
    /// operator diffing two pins can see *which* half moved: MRTD alone means
    /// a new image, MRCONFIGID alone means the same image relaunched with a
    /// different compose.
    fn measurement_label(&self) -> String {
        format!(
            "mrtd:{}+mrconfigid:{}",
            hex::encode(self.mr_td),
            hex::encode(self.mr_config_id)
        )
    }
}

/// A witness backed by a dstack guest agent.
///
/// Holds all three of the seam's answers -- the signing key, the address, and
/// the measurement -- because all three are fixed for the life of the VM and
/// none of them should cost a socket round trip per request.
///
/// The measurement is captured **once, at [`connect`](Self::connect)**, from a
/// quote taken at startup. MRTD cannot change without a reboot, so a
/// per-request read would buy nothing; capturing it at boot means a witness
/// that cannot name itself fails to start rather than failing mid-request,
/// which is the difference between an operator seeing it and a contributor
/// seeing it.
pub struct DstackEnclave {
    agent: Box<dyn GuestAgent>,
    signing_key: SigningKey,
    signing_address: String,
    measurement: String,
    boot_mr_td: [u8; 48],
    key_chain: Vec<Vec<u8>>,
}

impl DstackEnclave {
    /// Fetch the signing key and the boot measurement. Two socket calls, once.
    pub async fn connect(agent: Box<dyn GuestAgent>) -> Result<Self, EnclaveError> {
        let derived = agent.signing_key().await?;
        let signing_key = SigningKey::from_slice(derived.scalar.as_slice())
            .map_err(|_| EnclaveError::SigningKeyUnusable)?;
        let signing_address = address_of(&signing_key);

        // A nonce of zeros: this quote is evidence of nothing to anybody, and
        // is not served to callers. It exists so the boot measurement is read
        // out of a signed TD report rather than out of `Info`, whose
        // `tcb_info` reports MRTD but has no MRCONFIGID field at all -- the
        // only place MRCONFIGID exists as a measured value is the report body.
        let boot_report_data = witness_report_data(&signing_address, &[0u8; WITNESS_NONCE_LEN])?;
        let boot_quote = agent.quote(&boot_report_data).await?;
        let facts = QuoteFacts::parse(&boot_quote)?;
        if facts.report_data != boot_report_data {
            return Err(EnclaveError::ReportDataMismatch);
        }

        Ok(Self {
            agent,
            signing_key,
            signing_address,
            measurement: facts.measurement_label(),
            boot_mr_td: facts.mr_td,
            key_chain: derived.chain,
        })
    }

    /// The two-link `GetKey` chain, captured and unverified. See module docs.
    pub fn signing_key_chain(&self) -> &[Vec<u8>] {
        &self.key_chain
    }
}

/// keccak256 of the uncompressed public key without its `0x04` tag, last 20
/// bytes -- the derivation `recover_eip191_signer` inverts.
fn address_of(signing_key: &SigningKey) -> String {
    let point = signing_key.verifying_key().to_encoded_point(false);
    let digest = Keccak256::digest(&point.as_bytes()[1..]);
    format!("0x{}", hex::encode(&digest[12..]))
}

impl Signer for DstackEnclave {
    fn sign_eip191(&self, message: &[u8]) -> Result<String, SeamUnavailable> {
        let mut hasher = Keccak256::new();
        hasher.update(b"\x19Ethereum Signed Message:\n");
        hasher.update(message.len().to_string().as_bytes());
        hasher.update(message);
        let digest: [u8; 32] = hasher.finalize().into();
        let (signature, recovery_id) = self
            .signing_key
            .sign_prehash_recoverable(&digest)
            .map_err(|_| SeamUnavailable)?;
        let mut raw = signature.to_bytes().to_vec();
        // 27/28, not 0/1: the offset EIP-191 recovery expects.
        raw.push(recovery_id.to_byte() + 27);
        Ok(format!("0x{}", hex::encode(raw)))
    }
}

#[async_trait]
impl Enclave for DstackEnclave {
    fn signing_address(&self) -> &str {
        &self.signing_address
    }

    async fn measurement(&self) -> Result<String, SeamUnavailable> {
        Ok(self.measurement.clone())
    }

    async fn attestation_quote(&self, report_data: &[u8]) -> Result<Vec<u8>, SeamUnavailable> {
        self.bound_quote(report_data)
            .await
            .map_err(|_| SeamUnavailable)
    }
}

impl DstackEnclave {
    /// The checked quote path, with the error kept.
    ///
    /// [`Enclave::attestation_quote`] flattens this to [`SeamUnavailable`]
    /// because [`super::witness`] decides which refusal an operator sees; this
    /// exists so the checks are testable by variant.
    pub async fn bound_quote(&self, report_data: &[u8]) -> Result<Vec<u8>, EnclaveError> {
        if report_data.len() > REPORT_DATA_LEN {
            return Err(EnclaveError::ReportDataTooLong);
        }
        // Right-padding, which is what the platform does with a short report
        // body anyway. Doing it here means the comparison below is against
        // the exact bytes the TDX module signed.
        let mut padded = [0u8; REPORT_DATA_LEN];
        padded[..report_data.len()].copy_from_slice(report_data);

        let quote = self.agent.quote(&padded).await?;
        let facts = QuoteFacts::parse(&quote)?;

        // The replay guard. An agent that returned a stored quote, or one
        // taken for a different caller, fails here.
        if facts.report_data != padded {
            return Err(EnclaveError::ReportDataMismatch);
        }
        // And a quote from somewhere else entirely fails here, even if it
        // somehow carried the right report data.
        if facts.mr_td != self.boot_mr_td {
            return Err(EnclaveError::MeasurementChanged);
        }
        Ok(quote)
    }
}

/// The dstack guest agent over its unix socket.
///
/// Hand-rolled rather than reached through an HTTP client crate: nothing in
/// this workspace speaks HTTP over a unix socket, `reqwest` cannot, and the
/// exchange is one `GET` with no body. Two requests over one connection each,
/// `Connection: close`.
///
/// Byte-valued fields come back as hex strings in the agent's JSON. That is
/// what dstack's guest API documents, and it is the one thing here that has
/// not been exercised against a live agent -- if it is ever wrong, `connect`
/// fails at boot with [`EnclaveError::MalformedResponse`] rather than a
/// witness running with a key it misread.
#[cfg(unix)]
pub struct DstackSocketAgent {
    socket_path: std::path::PathBuf,
}

#[cfg(unix)]
impl DstackSocketAgent {
    /// Point at [`DSTACK_SOCKET_PATH`].
    pub fn new() -> Self {
        Self::at(DSTACK_SOCKET_PATH)
    }

    /// Point at a socket somewhere else. Used by the transport test, and by an
    /// operator running the agent outside its default path.
    pub fn at(socket_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            socket_path: socket_path.into(),
        }
    }

    /// One `GET`, one JSON object back.
    async fn get(&self, target: &str) -> Result<serde_json::Value, EnclaveError> {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let mut stream = tokio::net::UnixStream::connect(&self.socket_path)
            .await
            .map_err(|_| EnclaveError::Transport)?;
        let request = format!(
            "GET {target} HTTP/1.1\r\nHost: localhost\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
        );
        stream
            .write_all(request.as_bytes())
            .await
            .map_err(|_| EnclaveError::Transport)?;
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .map_err(|_| EnclaveError::Transport)?;
        let body = http_body(&response)?;
        serde_json::from_slice(body).map_err(|_| EnclaveError::MalformedResponse)
    }
}

#[cfg(unix)]
impl Default for DstackSocketAgent {
    fn default() -> Self {
        Self::new()
    }
}

/// Split an HTTP/1.1 response, insisting on a 200 and decoding a chunked body
/// if there is one.
#[cfg(unix)]
fn http_body(response: &[u8]) -> Result<&[u8], EnclaveError> {
    let split = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(EnclaveError::MalformedResponse)?;
    let head =
        std::str::from_utf8(&response[..split]).map_err(|_| EnclaveError::MalformedResponse)?;
    let status = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .ok_or(EnclaveError::MalformedResponse)?;
    if status != "200" {
        // The agent answered and said no. Distinct from a socket that was not
        // there, because the two mean different things to an operator -- and
        // carrying the code, because "refused" alone does not say whether the
        // method, the arguments or the caller is what it objected to.
        return Err(EnclaveError::AgentRefused {
            // A status line this side of the check is ASCII digits or the
            // response was malformed; 0 records "unparseable" rather than
            // inventing a code.
            status: status.parse::<u16>().unwrap_or(0),
        });
    }
    let body = &response[split + 4..];
    if head
        .to_ascii_lowercase()
        .contains("transfer-encoding: chunked")
    {
        return chunked_body_bounds(body).map(|(start, end)| &body[start..end]);
    }
    Ok(body)
}

/// The extent of the first chunk of a chunked body.
///
/// dstack's answers are a single JSON object, so one chunk is what a
/// hyper-based server produces for them. A body that arrives in more than one
/// chunk is not stitched -- it is refused, because a silently truncated JSON
/// object would be read as a malformed response anyway and this way the reason
/// is the honest one.
#[cfg(unix)]
fn chunked_body_bounds(body: &[u8]) -> Result<(usize, usize), EnclaveError> {
    let line_end = body
        .windows(2)
        .position(|window| window == b"\r\n")
        .ok_or(EnclaveError::MalformedResponse)?;
    let size_line =
        std::str::from_utf8(&body[..line_end]).map_err(|_| EnclaveError::MalformedResponse)?;
    let size = usize::from_str_radix(size_line.split(';').next().unwrap_or("").trim(), 16)
        .map_err(|_| EnclaveError::MalformedResponse)?;
    let start = line_end + 2;
    let end = start
        .checked_add(size)
        .filter(|end| *end <= body.len())
        .ok_or(EnclaveError::MalformedResponse)?;
    Ok((start, end))
}

/// A hex string, with or without `0x`, as bytes.
#[cfg(unix)]
fn hex_field(value: &serde_json::Value, field: &str) -> Result<Vec<u8>, EnclaveError> {
    let text = value
        .get(field)
        .and_then(serde_json::Value::as_str)
        .ok_or(EnclaveError::MalformedResponse)?;
    hex::decode(text.strip_prefix("0x").unwrap_or(text))
        .map_err(|_| EnclaveError::MalformedResponse)
}

#[cfg(unix)]
#[async_trait]
impl GuestAgent for DstackSocketAgent {
    async fn signing_key(&self) -> Result<DerivedSigningKey, EnclaveError> {
        let response = self
            .get(&format!(
                "{GET_KEY_PATH}?path={SIGNING_KEY_PATH}&purpose={SIGNING_KEY_PURPOSE}&algorithm={SIGNING_KEY_ALGORITHM}"
            ))
            .await?;
        let key = hex_field(&response, "key")?;
        let scalar: [u8; 32] = key
            .as_slice()
            .try_into()
            .map_err(|_| EnclaveError::MalformedResponse)?;
        let chain = response
            .get("signature_chain")
            .and_then(serde_json::Value::as_array)
            .map(|links| {
                links
                    .iter()
                    .filter_map(serde_json::Value::as_str)
                    .filter_map(|link| hex::decode(link.strip_prefix("0x").unwrap_or(link)).ok())
                    .collect()
            })
            .unwrap_or_default();
        Ok(DerivedSigningKey {
            scalar: Zeroizing::new(scalar),
            chain,
        })
    }

    async fn quote(&self, report_data: &[u8; REPORT_DATA_LEN]) -> Result<Vec<u8>, EnclaveError> {
        let response = self
            .get(&format!(
                "{GET_QUOTE_PATH}?report_data=0x{}",
                hex::encode(report_data)
            ))
            .await?;
        hex_field(&response, "quote")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_attestation::receipt::recover_eip191_signer;

    /// A real TDX quote, borrowed from the NEAR AI attestation fixture. Tests
    /// splice their own fields into it rather than inventing a quote, so the
    /// parser is exercised against bytes a real TDX module produced --
    /// including the auth-data tail, which a hand-built body would omit and
    /// which `Quote::parse` reads.
    const REAL_REPORT: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_report.json"
    );

    /// secp256k1 scalar 1. Its address is a published test vector, so
    /// `the_signing_address_comes_from_the_agents_derived_key` compares
    /// against ground truth from outside this file rather than against the
    /// same derivation spelled twice.
    const SCALAR_ONE: [u8; 32] = {
        let mut scalar = [0u8; 32];
        scalar[31] = 1;
        scalar
    };
    const ADDRESS_OF_SCALAR_ONE: &str = "0x7e5f4552091a69125d5dfcb7b8c2659029395bdf";

    const STUB_MR_TD: [u8; 48] = [0xa1; 48];
    const STUB_RT_MR0: [u8; 48] = [0xc3; 48];
    const STUB_RT_MR3: [u8; 48] = [0xd4; 48];

    /// Shaped like a real config-id v1: the version byte, then what would be
    /// the compose hash, then zero padding.
    fn stub_mr_config_id() -> [u8; 48] {
        let mut value = [0u8; 48];
        value[0] = 0x01;
        for (index, byte) in value.iter_mut().enumerate().take(33).skip(1) {
            *byte = 0xb0 + (index as u8 % 16);
        }
        value
    }

    /// A nonce whose every byte differs, so a test cannot pass against a
    /// rotated, reversed or partially-copied nonce -- which `[0x5a; 32]` would.
    fn distinct_nonce(seed: u8) -> [u8; WITNESS_NONCE_LEN] {
        let mut nonce = [0u8; WITNESS_NONCE_LEN];
        for (index, byte) in nonce.iter_mut().enumerate() {
            *byte = seed
                .wrapping_add((index as u8).wrapping_mul(7))
                .wrapping_add(3);
        }
        nonce
    }

    fn real_quote_bytes() -> Vec<u8> {
        let value: serde_json::Value = serde_json::from_str(REAL_REPORT).expect("fixture parses");
        let hex_quote = value["intel_quote"].as_str().expect("fixture has a quote");
        hex::decode(hex_quote.strip_prefix("0x").unwrap_or(hex_quote)).expect("quote is hex")
    }

    /// The fixture quote with these fields spliced in.
    ///
    /// The offsets are asserted rather than assumed: `splice_offsets_are_the_
    /// fields_they_claim` parses a spliced quote and requires every field back
    /// out, so a wrong offset is a red test rather than a silent misread.
    fn quote_with(
        report_data: &[u8; REPORT_DATA_LEN],
        mr_td: &[u8; 48],
        mr_config_id: &[u8; 48],
    ) -> Vec<u8> {
        let mut quote = real_quote_bytes();
        quote[quote_offsets::MR_TD..quote_offsets::MR_TD + 48].copy_from_slice(mr_td);
        quote[quote_offsets::MR_CONFIG_ID..quote_offsets::MR_CONFIG_ID + 48]
            .copy_from_slice(mr_config_id);
        quote[quote_offsets::RT_MR0..quote_offsets::RT_MR0 + 48].copy_from_slice(&STUB_RT_MR0);
        quote[quote_offsets::RT_MR3..quote_offsets::RT_MR3 + 48].copy_from_slice(&STUB_RT_MR3);
        quote[quote_offsets::REPORT_DATA..quote_offsets::REPORT_DATA + REPORT_DATA_LEN]
            .copy_from_slice(report_data);
        quote
    }

    /// What the stub does on every quote call *after* the one `connect` makes.
    /// Boot always gets an honest quote; a substitution that applied at boot
    /// would fail construction and never reach the check under test.
    #[derive(Default)]
    struct AfterBoot {
        fail: bool,
        report_data: Option<[u8; REPORT_DATA_LEN]>,
        mr_td: Option<[u8; 48]>,
    }

    struct StubAgent {
        scalar: [u8; 32],
        mr_td: [u8; 48],
        mr_config_id: [u8; 48],
        after_boot: AfterBoot,
        asked: std::sync::Mutex<Vec<[u8; REPORT_DATA_LEN]>>,
    }

    impl StubAgent {
        fn new() -> Self {
            Self {
                scalar: SCALAR_ONE,
                mr_td: STUB_MR_TD,
                mr_config_id: stub_mr_config_id(),
                after_boot: AfterBoot::default(),
                asked: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn after_boot(mut self, after_boot: AfterBoot) -> Self {
            self.after_boot = after_boot;
            self
        }
    }

    #[async_trait]
    impl GuestAgent for StubAgent {
        async fn signing_key(&self) -> Result<DerivedSigningKey, EnclaveError> {
            Ok(DerivedSigningKey {
                scalar: Zeroizing::new(self.scalar),
                chain: vec![vec![0x01; 65], vec![0x02; 65]],
            })
        }

        async fn quote(
            &self,
            report_data: &[u8; REPORT_DATA_LEN],
        ) -> Result<Vec<u8>, EnclaveError> {
            let boot = {
                let mut asked = self
                    .asked
                    .lock()
                    .expect("no test panics while holding this");
                asked.push(*report_data);
                asked.len() == 1
            };
            if boot {
                return Ok(quote_with(report_data, &self.mr_td, &self.mr_config_id));
            }
            if self.after_boot.fail {
                return Err(EnclaveError::Transport);
            }
            let signed = self.after_boot.report_data.unwrap_or(*report_data);
            let mr_td = self.after_boot.mr_td.unwrap_or(self.mr_td);
            Ok(quote_with(&signed, &mr_td, &self.mr_config_id))
        }
    }

    async fn connected() -> DstackEnclave {
        DstackEnclave::connect(Box::new(StubAgent::new()))
            .await
            .expect("the stub agent answers both calls")
    }

    #[test]
    fn splice_offsets_are_the_fields_they_claim() {
        // The one test that validates the test helper. Every distinctive
        // value goes in and must come back out of a real parse; a wrong
        // offset shows up here rather than as a confusing failure elsewhere.
        let report_data = [0x77u8; REPORT_DATA_LEN];
        let facts = QuoteFacts::parse(&quote_with(&report_data, &STUB_MR_TD, &stub_mr_config_id()))
            .expect("a spliced real quote still parses");
        assert_eq!(facts.report_data, report_data);
        assert_eq!(facts.mr_td, STUB_MR_TD);
        assert_eq!(facts.mr_config_id, stub_mr_config_id());
    }

    #[test]
    fn the_nonce_reaches_the_report_data() {
        let nonce = distinct_nonce(0x40);
        let report_data =
            witness_report_data(ADDRESS_OF_SCALAR_ONE, &nonce).expect("well-formed inputs");

        assert_eq!(&report_data[NONCE_AT..NONCE_AT + WITNESS_NONCE_LEN], &nonce);
        assert_eq!(
            &report_data[..WITNESS_QUOTE_DOMAIN.len()],
            WITNESS_QUOTE_DOMAIN
        );
        assert_eq!(&report_data[NONCE_AT + WITNESS_NONCE_LEN..], &[0u8; 4]);

        // Positive control. Without it a `witness_report_data` that ignored
        // its nonce and happened to leave zeros there would still pass the
        // assertion above for a zero nonce, and the whole freshness claim
        // would rest on a test that cannot fail.
        let other = witness_report_data(ADDRESS_OF_SCALAR_ONE, &distinct_nonce(0x91))
            .expect("well-formed inputs");
        assert_ne!(report_data, other);
        assert_ne!(
            &report_data[NONCE_AT..NONCE_AT + WITNESS_NONCE_LEN],
            &other[NONCE_AT..NONCE_AT + WITNESS_NONCE_LEN]
        );
    }

    #[test]
    fn the_signing_address_is_bound_alongside_the_nonce() {
        let nonce = distinct_nonce(0x22);
        let report_data =
            witness_report_data(ADDRESS_OF_SCALAR_ONE, &nonce).expect("well-formed inputs");
        let expected = hex::decode(&ADDRESS_OF_SCALAR_ONE[2..]).expect("the vector is hex");
        assert_eq!(
            &report_data[ADDRESS_AT..ADDRESS_AT + ADDRESS_LEN],
            &expected[..]
        );

        // The same nonce under a different address must not produce the same
        // 64 bytes, or the address half of the binding is decorative.
        let elsewhere = witness_report_data("0x00112233445566778899aabbccddeeff00112233", &nonce)
            .expect("well-formed inputs");
        assert_ne!(report_data, elsewhere);
        assert_eq!(
            &report_data[NONCE_AT..NONCE_AT + WITNESS_NONCE_LEN],
            &elsewhere[NONCE_AT..NONCE_AT + WITNESS_NONCE_LEN],
            "only the address should have moved"
        );
    }

    #[test]
    fn a_nonce_of_the_wrong_length_is_refused() {
        let lengths = [0usize, 1, WITNESS_NONCE_LEN - 1, WITNESS_NONCE_LEN + 1, 64];
        assert_eq!(lengths.len(), 5, "the loop below must not be vacuous");
        for length in lengths {
            assert_eq!(
                witness_report_data(ADDRESS_OF_SCALAR_ONE, &vec![0x11u8; length]),
                Err(EnclaveError::NonceLength),
                "a {length}-byte nonce was accepted"
            );
        }
    }

    #[test]
    fn a_malformed_signing_address_is_refused() {
        let addresses = [
            "7e5f4552091a69125d5dfcb7b8c2659029395bdf",     // no 0x
            "0x7e5f4552091a69125d5dfcb7b8c2659029395b",     // 19 bytes
            "0x7e5f4552091a69125d5dfcb7b8c2659029395bdfaa", // 21 bytes
            "0xzz5f4552091a69125d5dfcb7b8c2659029395bdf",   // not hex
            "0x",
            "",
        ];
        assert_eq!(addresses.len(), 6, "the loop below must not be vacuous");
        for address in addresses {
            assert_eq!(
                witness_report_data(address, &distinct_nonce(1)),
                Err(EnclaveError::SigningAddressMalformed),
                "{address:?} was accepted as an address"
            );
        }
    }

    #[tokio::test]
    async fn the_signing_address_comes_from_the_agents_derived_key() {
        let enclave = connected().await;
        assert_eq!(enclave.signing_address(), ADDRESS_OF_SCALAR_ONE);
    }

    #[tokio::test]
    async fn a_witness_signature_recovers_to_the_reported_signing_address() {
        // Ground truth from outside: the recovery this workspace already uses
        // for NEAR AI receipts, which is the same code path
        // `verify_witness_certificate` relies on. A 64-byte agent signature
        // could not be checked here at all, which is why this module derives
        // the key and signs in process.
        let enclave = connected().await;
        let message = b"witness certificate signing bytes";
        let signature = enclave.sign_eip191(message).expect("the stub key signs");
        let recovered = recover_eip191_signer(message, &signature).expect("65 bytes, v = 27/28");
        assert_eq!(
            format!("0x{}", hex::encode(recovered)),
            enclave.signing_address()
        );
    }

    #[tokio::test]
    async fn the_measurement_names_mrtd_and_mrconfigid_and_nothing_else() {
        let enclave = connected().await;
        let measurement = enclave.measurement().await.expect("captured at connect");

        assert_eq!(
            measurement,
            format!(
                "mrtd:{}+mrconfigid:{}",
                hex::encode(STUB_MR_TD),
                hex::encode(stub_mr_config_id())
            )
        );
        // The decision that RTMR0 and RTMR3 are excluded is the whole reason
        // this pin can admit two instances of one image, so it is asserted
        // rather than left to the format string.
        assert!(!measurement.contains(&hex::encode(STUB_RT_MR0)));
        assert!(!measurement.contains(&hex::encode(STUB_RT_MR3)));
    }

    #[tokio::test]
    async fn the_quote_carries_the_nonce_the_caller_supplied() {
        let enclave = connected().await;
        let nonce = distinct_nonce(0x5c);

        let quote = (&enclave as &dyn Enclave)
            .nonce_bound_quote(&nonce)
            .await
            .expect("the stub quotes what it is asked");

        // Read out of the signed TD report body, not out of the request: what
        // makes this a freshness proof is that the nonce is inside the thing
        // the platform signed.
        let facts = QuoteFacts::parse(&quote).expect("a real quote shape");
        assert_eq!(
            &facts.report_data[NONCE_AT..NONCE_AT + WITNESS_NONCE_LEN],
            &nonce
        );
        assert_eq!(
            &facts.report_data[ADDRESS_AT..ADDRESS_AT + ADDRESS_LEN],
            &hex::decode(&enclave.signing_address()[2..]).expect("hex")[..]
        );
        assert_eq!(
            facts.report_data,
            witness_report_data(enclave.signing_address(), &nonce).expect("well-formed")
        );
    }

    #[tokio::test]
    async fn a_quote_that_does_not_carry_the_nonce_is_refused() {
        // The replay: an agent serving a quote taken for somebody else. It is
        // a well-formed, parseable, correctly-measured quote, and it must
        // still be refused, because the only thing wrong with it is the one
        // thing that matters.
        let stale =
            witness_report_data(ADDRESS_OF_SCALAR_ONE, &distinct_nonce(0xee)).expect("well-formed");
        let enclave = DstackEnclave::connect(Box::new(StubAgent::new().after_boot(AfterBoot {
            report_data: Some(stale),
            ..AfterBoot::default()
        })))
        .await
        .expect("boot is honest");

        let report_data =
            witness_report_data(ADDRESS_OF_SCALAR_ONE, &distinct_nonce(0x01)).expect("well-formed");
        assert_eq!(
            enclave.bound_quote(&report_data).await,
            Err(EnclaveError::ReportDataMismatch)
        );
    }

    #[tokio::test]
    async fn a_quote_reporting_a_different_measurement_is_refused() {
        let enclave = DstackEnclave::connect(Box::new(StubAgent::new().after_boot(AfterBoot {
            mr_td: Some([0xfe; 48]),
            ..AfterBoot::default()
        })))
        .await
        .expect("boot is honest");

        let report_data =
            witness_report_data(ADDRESS_OF_SCALAR_ONE, &distinct_nonce(0x02)).expect("well-formed");
        assert_eq!(
            enclave.bound_quote(&report_data).await,
            Err(EnclaveError::MeasurementChanged)
        );
    }

    #[tokio::test]
    async fn a_failing_quote_request_is_a_named_error_not_an_empty_quote() {
        let enclave = DstackEnclave::connect(Box::new(StubAgent::new().after_boot(AfterBoot {
            fail: true,
            ..AfterBoot::default()
        })))
        .await
        .expect("boot is honest");

        let report_data =
            witness_report_data(ADDRESS_OF_SCALAR_ONE, &distinct_nonce(0x03)).expect("well-formed");
        assert_eq!(
            enclave.bound_quote(&report_data).await,
            Err(EnclaveError::Transport)
        );
        // And through the trait, where the variant is flattened: still a
        // refusal, and specifically not `Ok(vec![])`, which is the shape a
        // caller would mistake for evidence.
        assert_eq!(
            (&enclave as &dyn Enclave)
                .nonce_bound_quote(&distinct_nonce(0x03))
                .await,
            Err(SeamUnavailable)
        );
    }

    #[tokio::test]
    async fn report_data_the_platform_will_not_sign_is_refused_rather_than_truncated() {
        let enclave = connected().await;
        assert_eq!(
            enclave.bound_quote(&[0x11u8; REPORT_DATA_LEN + 1]).await,
            Err(EnclaveError::ReportDataTooLong)
        );
        // dstack errors on over-length report data rather than truncating, and
        // so does this: a truncation would drop exactly the caller's nonce and
        // return a quote that verifies as something else.
        assert_eq!(
            (&enclave as &dyn Enclave)
                .nonce_bound_quote(&[0x11u8; WITNESS_NONCE_LEN + 1])
                .await,
            Err(SeamUnavailable)
        );
    }

    #[tokio::test]
    async fn short_report_data_is_right_padded_the_way_the_platform_would() {
        let enclave = connected().await;
        let quote = enclave
            .bound_quote(&[0x42u8; 8])
            .await
            .expect("eight bytes is under the cap");
        let facts = QuoteFacts::parse(&quote).expect("a real quote shape");
        let mut expected = [0u8; REPORT_DATA_LEN];
        expected[..8].copy_from_slice(&[0x42u8; 8]);
        assert_eq!(facts.report_data, expected);
    }

    #[tokio::test]
    async fn the_key_chain_is_captured_for_a_later_slice() {
        let enclave = connected().await;
        assert_eq!(enclave.signing_key_chain().len(), 2);
        // Length only. Nothing in this module verifies the chain, and a test
        // that checked its contents would read as though something did.
    }

    #[test]
    fn no_refusal_echoes_the_nonce_or_the_address() {
        let variants = [
            EnclaveError::NonceLength,
            EnclaveError::ReportDataTooLong,
            EnclaveError::SigningAddressMalformed,
            EnclaveError::Transport,
            EnclaveError::AgentRefused { status: 500 },
            EnclaveError::MalformedResponse,
            EnclaveError::SigningKeyUnusable,
            EnclaveError::QuoteUnparsable,
            EnclaveError::QuoteNotTdx,
            EnclaveError::ReportDataMismatch,
            EnclaveError::MeasurementChanged,
            EnclaveError::SigningFailed,
        ];
        // Pinned so a new variant has to be added here rather than skipping
        // the check silently.
        assert_eq!(variants.len(), 12);
        // The property is that no refusal echoes the nonce or the signing
        // address. This used to be enforced as "no ASCII digit anywhere",
        // which is a proxy rather than the property -- and it is the reason
        // AgentRefused could not carry the status code that says whether the
        // agent objected to the method, the arguments or the caller. A
        // deployment spent a long time undiagnosable behind that.
        //
        // Checked against the real shapes instead: a nonce is 32 bytes and an
        // address 20, so both render as long hex runs. Anything that long is
        // refused; a three-digit status is not, and cannot be either of them.
        const LONGEST_SAFE_RUN: usize = 8;
        for variant in variants {
            let rendered = format!("{variant} {variant:?}");
            assert!(!rendered.contains("0x"), "{rendered}");
            let mut run = 0usize;
            for character in rendered.chars() {
                run = if character.is_ascii_hexdigit() {
                    run + 1
                } else {
                    0
                };
                assert!(
                    run <= LONGEST_SAFE_RUN,
                    "a hex run this long can carry a nonce or an address: {rendered}"
                );
            }
        }
    }

    /// Serve one canned HTTP response over a unix socket and hand back the
    /// request bytes the client sent, so the request line itself is asserted
    /// rather than assumed.
    #[cfg(unix)]
    async fn serve_response(
        response: String,
    ) -> (
        tempfile::TempDir,
        std::path::PathBuf,
        tokio::task::JoinHandle<String>,
    ) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        let dir = tempfile::tempdir().expect("a temp dir");
        let path = dir.path().join("dstack.sock");
        let listener = tokio::net::UnixListener::bind(&path).expect("bind");
        let handle = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.expect("accept");
            let mut buffer = [0u8; 2048];
            let read = stream.read(&mut buffer).await.expect("read");
            stream.write_all(response.as_bytes()).await.expect("write");
            stream.shutdown().await.expect("shutdown");
            String::from_utf8_lossy(&buffer[..read]).into_owned()
        });
        (dir, path, handle)
    }

    #[cfg(unix)]
    mod socket {
        use super::super::*;
        use super::{SCALAR_ONE, serve_response};

        #[tokio::test]
        async fn the_quote_request_names_the_pinned_surface_and_hex_report_data() {
            let (_dir, path, handle) = serve_response(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\n\r\n{\"quote\":\"0xdeadbeef\"}"
                    .to_string(),
            )
            .await;
            let agent = DstackSocketAgent::at(&path);
            let report_data = [0x3cu8; REPORT_DATA_LEN];
            let quote = agent.quote(&report_data).await.expect("the canned answer");
            assert_eq!(quote, vec![0xde, 0xad, 0xbe, 0xef]);

            let request = handle.await.expect("the server task");
            assert!(
                request.starts_with(&format!(
                    "GET {GET_QUOTE_PATH}?report_data=0x{} HTTP/1.1",
                    hex::encode(report_data)
                )),
                "{request}"
            );
        }

        #[tokio::test]
        async fn the_key_request_names_the_pinned_path_purpose_and_algorithm() {
            let (_dir, path, handle) = serve_response(format!(
                "HTTP/1.1 200 OK\r\n\r\n{{\"key\":\"{}\",\"signature_chain\":[\"0x{}\",\"0x{}\"]}}",
                hex::encode(SCALAR_ONE),
                hex::encode([0x01u8; 65]),
                hex::encode([0x02u8; 65])
            ))
            .await;
            let agent = DstackSocketAgent::at(&path);
            let derived = agent.signing_key().await.expect("the canned answer");
            assert_eq!(*derived.scalar, SCALAR_ONE);
            assert_eq!(derived.chain.len(), 2);

            let request = handle.await.expect("the server task");
            assert!(request.contains(GET_KEY_PATH), "{request}");
            assert!(
                request.contains(&format!("path={SIGNING_KEY_PATH}")),
                "{request}"
            );
            assert!(
                request.contains(&format!("purpose={SIGNING_KEY_PURPOSE}")),
                "{request}"
            );
            assert!(
                request.contains(&format!("algorithm={SIGNING_KEY_ALGORITHM}")),
                "{request}"
            );
        }

        #[tokio::test]
        async fn a_chunked_answer_is_decoded() {
            let (_dir, path, _handle) = serve_response(
                "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n16\r\n{\"quote\":\"0xdeadbeef\"}\r\n0\r\n\r\n"
                    .to_string(),
            )
            .await;
            let quote = DstackSocketAgent::at(&path)
                .quote(&[0u8; REPORT_DATA_LEN])
                .await
                .expect("chunked is decoded");
            assert_eq!(quote, vec![0xde, 0xad, 0xbe, 0xef]);
        }

        #[tokio::test]
        async fn an_agent_that_says_no_is_distinguished_from_a_socket_that_is_not_there() {
            let (_dir, path, _handle) =
                serve_response("HTTP/1.1 500 Internal Server Error\r\n\r\n{}".to_string()).await;
            assert_eq!(
                DstackSocketAgent::at(&path)
                    .quote(&[0u8; REPORT_DATA_LEN])
                    .await,
                Err(EnclaveError::AgentRefused { status: 500 })
            );
            assert_eq!(
                DstackSocketAgent::at("/nonexistent/dstack.sock")
                    .quote(&[0u8; REPORT_DATA_LEN])
                    .await,
                Err(EnclaveError::Transport)
            );
        }
    }
}
