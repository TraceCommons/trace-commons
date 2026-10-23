//! The signed NEAR AI pin-set format a staging build takes its measurement
//! sets from, and its verifier.
//!
//! A production client pins the measurement sets ingest publishes, unsigned,
//! at `/v1/contributors/me/near-ai-measurements`. A staging build does not
//! take them from ingest at all: it fetches a document signed by a key the
//! project holds off the staging host, and this module is what decides
//! whether that document may be pinned. A staging host that is compromised
//! can then serve a stale document, never a forged one.
//!
//! # The document
//!
//! ```json
//! {
//!   "payload": "<base64url of the payload bytes>",
//!   "signature": "<base64url Ed25519 signature>",
//!   "key_id": "<first 16 hex of SHA-256 of the public key>"
//! }
//! ```
//!
//! The payload is signed as exact bytes, so no canonical-JSON rule is
//! needed: the signature covers [`SIGNING_PREFIX`] followed by the decoded
//! payload bytes, which keeps the key from signing anything else usefully.
//! Both base64url fields are the unpadded alphabet of RFC 4648 section 5;
//! padding, the standard alphabet and non-zero trailing bits are refused.
//!
//! The payload, once the signature has passed:
//!
//! ```json
//! {
//!   "format": "trace-commons-near-ai-pins/v2",
//!   "channel": "staging",
//!   "gateway_host": "<the staging gateway host>",
//!   "sequence": 42,
//!   "issued_at": "2026-10-21T18:00:00Z",
//!   "expires_at": "2026-10-28T18:00:00Z",
//!   "sets": ["mrtd=<96 hex>,rtmr0=<96 hex>,rtmr1=<96 hex>,rtmr2=<96 hex>,mrconfigid=<96 hex>"],
//!   "allowlist": {"rtmr3": {...}, "compose_actions": {"compose_manager_images": [...], "compose_files": [...]}}
//! }
//! ```
//!
//! Each set is [`ExpectedMeasurements`] syntax, and any one of them matching
//! a quote is a pass, as with the unsigned document. Every set must name at
//! least [`REQUIRED_REGISTERS`] (owner decision S16): RTMR3 is not pinned but
//! replayed, and the replay checks the `compose-hash` event against
//! MRCONFIGID, which only means something when MRCONFIGID is pinned.
//! Timestamps are exactly `YYYY-MM-DDTHH:MM:SSZ`.
//!
//! `allowlist` is what the RTMR3 replay and the compose-manager record are
//! checked against ([`crate::staging_replay::Allowlist`]). It is inside the
//! signed payload, so the same key, sequence and expiry cover it and the
//! sets. It is why this is format v2, with its own signing prefix: a v1
//! document carries no allowlist, and this verifier refuses one
//! ([`PinRefusal::V1Document`]) rather than read it as a document with an
//! empty allowlist.
//!
//! # Failing closed
//!
//! [`verify`] returns the sets only when every check passes, and otherwise a
//! [`PinRefusal`], which pins nothing. There is no partial result: one set
//! that does not parse refuses the whole document. The checks run in the
//! order that keeps unsigned bytes from being interpreted: the document's
//! shape, the key id, the signature, and only then the payload.
//!
//! Two rows of the failure table are not here, because they need state this
//! function does not hold: a fetch that fails, and the sequence check (a
//! lower sequence than one already accepted, or the same sequence with other
//! bytes). The caller keeps the highest accepted sequence and compares it
//! with [`VerifiedPins::sequence`] and [`VerifiedPins::payload_sha256`].
//!
//! [`verify`] is the only entry point: the types that decode the document
//! and the payload are private to this crate, so nothing outside it can act
//! on payload bytes whose signature has not verified.
//!
//! # What the caller owns
//!
//! [`verify`] sets no limit on the document's size. The caller caps the
//! response body it reads before passing the bytes here.
//!
//! A JSON object that repeats a key is read with the last value winning, as
//! `serde_json` does. That cannot be used to change what a payload says
//! without the signing key, because the signature covers the exact bytes,
//! but the signing tool must not emit a repeated key, or another parser of
//! the same signed bytes could read a different value.
//!
//! Neither decoded type prints its bytes under `{:?}`: the outer document
//! shows lengths and the payload's SHA-256, the payload shows its numbers
//! and the number of sets, so formatting one into a log line keeps the
//! hash-only rule.

use std::fmt;

use ring::signature::{ED25519, UnparsedPublicKey};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use crate::measurements::{ExpectedMeasurements, MeasurementField};
use crate::staging_replay::Allowlist;

/// The payload's `format` value this verifier reads: v2, which carries the
/// RTMR3 and compose-action allowlist.
pub const FORMAT: &str = "trace-commons-near-ai-pins/v2";

/// The domain-separation prefix the signature covers ahead of the payload
/// bytes. Its own, per format, so a v1 signature never verifies as v2.
pub const SIGNING_PREFIX: &[u8] = b"trace-commons-near-ai-pins/v2\n";

/// The v1 format, which carried no allowlist. Never accepted; named only so
/// a v1 document is refused with its own label ([`PinRefusal::V1Document`]).
pub const FORMAT_V1: &str = "trace-commons-near-ai-pins/v1";

/// The v1 signing prefix. Only ever used to recognise a v1 document after
/// the v2 signature check failed; nothing signed under it is read.
pub const SIGNING_PREFIX_V1: &[u8] = b"trace-commons-near-ai-pins/v1\n";

/// The registers every v2 set must name (S16). More are allowed.
pub const REQUIRED_REGISTERS: [MeasurementField; 5] = [
    MeasurementField::Mrtd,
    MeasurementField::Rtmr0,
    MeasurementField::Rtmr1,
    MeasurementField::Rtmr2,
    MeasurementField::MrConfigId,
];

/// The payload's `channel` value this verifier accepts.
pub const CHANNEL: &str = "staging";

/// How far in the future `issued_at` may be, to absorb clock skew between
/// the signing machine and the device. Expiry gets no such allowance.
pub const ISSUE_SKEW_SECS: u64 = 5 * 60;

/// The longest a signed set may live, from `issued_at` to `expires_at`.
pub const MAX_LIFETIME_SECS: u64 = 14 * 24 * 60 * 60;

/// The number of hex characters of the public key's SHA-256 that make its
/// key id.
const KEY_ID_HEX_LEN: usize = 16;

/// Why a signed pin-set document was refused. Every refusal pins nothing.
///
/// Each variant is a label and nothing more: no host name, key, measurement
/// or document byte is carried, so the label can go into a status reply or a
/// log line as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinRefusal {
    /// The document or the signed payload is not the v2 shape: not JSON, a
    /// field missing, extra or of the wrong type, bad base64url, or a
    /// timestamp not in the one accepted form.
    Malformed,
    /// `key_id` is not the id of the compiled-in key.
    UnknownKey,
    /// The Ed25519 signature does not verify over the prefixed payload,
    /// including when the compiled-in key is not a valid Ed25519 public key.
    BadSignature,
    /// The payload's `format` is not [`FORMAT`].
    WrongFormat,
    /// The payload's `channel` is not [`CHANNEL`].
    WrongChannel,
    /// The payload's `gateway_host` is not the build's gateway host.
    WrongGatewayHost,
    /// `now` is at or past `expires_at`.
    Expired,
    /// `issued_at` is more than [`ISSUE_SKEW_SECS`] after `now`.
    IssuedInFuture,
    /// `expires_at` is not after `issued_at`, or more than
    /// [`MAX_LIFETIME_SECS`] after it.
    LifetimeInvalid,
    /// A set is not [`ExpectedMeasurements`] syntax.
    SetUnparseable,
    /// The payload lists no sets.
    NoSets,
    /// A v1 document, signed by the compiled-in key under the v1 prefix. A
    /// replay-aware build refuses it: it carries no allowlist.
    V1Document,
    /// A set does not name every one of [`REQUIRED_REGISTERS`].
    SetMissingRegister,
}

impl PinRefusal {
    /// A stable label for status replies and logs.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            PinRefusal::Malformed => "pins_malformed",
            PinRefusal::UnknownKey => "pins_unknown_key",
            PinRefusal::BadSignature => "pins_bad_signature",
            PinRefusal::WrongFormat => "pins_wrong_format",
            PinRefusal::WrongChannel => "pins_wrong_channel",
            PinRefusal::WrongGatewayHost => "pins_wrong_gateway_host",
            PinRefusal::Expired => "pins_expired",
            PinRefusal::IssuedInFuture => "pins_issued_in_future",
            PinRefusal::LifetimeInvalid => "pins_lifetime_invalid",
            PinRefusal::SetUnparseable => "pins_set_unparseable",
            PinRefusal::NoSets => "pins_no_sets",
            PinRefusal::V1Document => "pins_v1_document",
            PinRefusal::SetMissingRegister => "pins_set_missing_register",
        }
    }
}

impl fmt::Display for PinRefusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

impl std::error::Error for PinRefusal {}

/// The outer document, decoded but not yet verified. Private to the crate:
/// [`verify`] is the only supported entry point.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SignedPinSet {
    /// The exact payload bytes the signature covers (after the prefix).
    pub(crate) payload: Vec<u8>,
    /// The Ed25519 signature.
    pub(crate) signature: Vec<u8>,
    /// The signing key's id, as the document states it.
    pub(crate) key_id: String,
}

impl fmt::Debug for SignedPinSet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SignedPinSet")
            .field("payload_len", &self.payload.len())
            .field(
                "payload_sha256",
                &hex::encode(Sha256::digest(&self.payload)),
            )
            .field("signature_len", &self.signature.len())
            .field("key_id_len", &self.key_id.len())
            .finish()
    }
}

impl SignedPinSet {
    /// Decode the outer document. Every field is required and no other is
    /// allowed.
    pub(crate) fn parse(document: &[u8]) -> Result<Self, PinRefusal> {
        let object = json_object(document)?;
        exact_fields(&object, &["payload", "signature", "key_id"])?;
        Ok(SignedPinSet {
            payload: base64url_decode(string_field(&object, "payload")?)?,
            signature: base64url_decode(string_field(&object, "signature")?)?,
            key_id: string_field(&object, "key_id")?.to_owned(),
        })
    }
}

/// The signed payload, parsed but not yet checked against the build.
/// Private to the crate: it is only ever parsed from bytes whose signature
/// [`verify`] has checked.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct PinPayload {
    pub(crate) format: String,
    pub(crate) channel: String,
    pub(crate) gateway_host: String,
    pub(crate) sequence: u64,
    /// Seconds since the Unix epoch.
    pub(crate) issued_at: u64,
    /// Seconds since the Unix epoch.
    pub(crate) expires_at: u64,
    /// The sets as written, not yet parsed.
    pub(crate) sets: Vec<String>,
    /// The allowlist, parsed.
    pub(crate) allowlist: Allowlist,
}

impl fmt::Debug for PinPayload {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PinPayload")
            .field("format_len", &self.format.len())
            .field("channel_len", &self.channel.len())
            .field("gateway_host_len", &self.gateway_host.len())
            .field("sequence", &self.sequence)
            .field("issued_at", &self.issued_at)
            .field("expires_at", &self.expires_at)
            .field("set_count", &self.sets.len())
            .field(
                "compose_manager_image_count",
                &self.allowlist.compose_actions.compose_manager_images.len(),
            )
            .field(
                "compose_file_count",
                &self.allowlist.compose_actions.compose_files.len(),
            )
            .finish()
    }
}

impl PinPayload {
    /// Parse the payload bytes. Every field is required and no other is
    /// allowed. Only call this on bytes whose signature has verified.
    pub(crate) fn parse(payload: &[u8]) -> Result<Self, PinRefusal> {
        let object = json_object(payload)?;
        exact_fields(
            &object,
            &[
                "format",
                "channel",
                "gateway_host",
                "sequence",
                "issued_at",
                "expires_at",
                "sets",
                "allowlist",
            ],
        )?;
        let sequence = object
            .get("sequence")
            .and_then(Value::as_u64)
            .ok_or(PinRefusal::Malformed)?;
        let sets = object
            .get("sets")
            .and_then(Value::as_array)
            .ok_or(PinRefusal::Malformed)?
            .iter()
            .map(|set| set.as_str().map(str::to_owned).ok_or(PinRefusal::Malformed))
            .collect::<Result<Vec<_>, _>>()?;
        let allowlist = Allowlist::from_json(object.get("allowlist").ok_or(PinRefusal::Malformed)?)
            .map_err(|_| PinRefusal::Malformed)?;
        Ok(PinPayload {
            format: string_field(&object, "format")?.to_owned(),
            channel: string_field(&object, "channel")?.to_owned(),
            gateway_host: string_field(&object, "gateway_host")?.to_owned(),
            sequence,
            issued_at: parse_utc(string_field(&object, "issued_at")?)
                .ok_or(PinRefusal::Malformed)?,
            expires_at: parse_utc(string_field(&object, "expires_at")?)
                .ok_or(PinRefusal::Malformed)?,
            sets,
            allowlist,
        })
    }
}

/// A document that passed every check [`verify`] makes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedPins {
    /// The signed sequence, for the caller's rollback check.
    pub sequence: u64,
    /// Seconds since the Unix epoch.
    pub issued_at: u64,
    /// Seconds since the Unix epoch. The sets must not be used at or after
    /// this instant, however recently they were fetched.
    pub expires_at: u64,
    /// The measurement sets, every one parsed. Never empty. Each names at
    /// least [`REQUIRED_REGISTERS`].
    pub sets: Vec<ExpectedMeasurements>,
    /// What every model entry's RTMR3 replay and compose-manager record are
    /// checked against before its key is trusted.
    pub allowlist: Allowlist,
    /// SHA-256 of the exact payload bytes, so the caller can refuse the same
    /// sequence carrying other bytes without keeping the document.
    pub payload_sha256: [u8; 32],
}

/// The key id of an Ed25519 public key: the first 16 hex characters of the
/// SHA-256 of its raw bytes.
#[must_use]
pub fn key_id(public_key: &[u8]) -> String {
    let mut id = hex::encode(Sha256::digest(public_key));
    id.truncate(KEY_ID_HEX_LEN);
    id
}

/// The exact bytes the signature covers: [`SIGNING_PREFIX`] followed by the
/// payload.
#[must_use]
pub fn signing_input(payload: &[u8]) -> Vec<u8> {
    let mut input = Vec::with_capacity(SIGNING_PREFIX.len() + payload.len());
    input.extend_from_slice(SIGNING_PREFIX);
    input.extend_from_slice(payload);
    input
}

/// Verify a signed pin-set document against the build's public key, gateway
/// host and the current time (seconds since the Unix epoch).
///
/// Returns the sets and the allowlist only if the key id matches, the
/// signature verifies, the payload is the v2 format for the staging channel
/// and this gateway host, the lifetime is positive and at most 14 days,
/// `issued_at <= now + 5 min`, `now < expires_at`, and every set parses and
/// names [`REQUIRED_REGISTERS`], checked in that order. The sequence is not
/// checked here; see the module documentation.
///
/// A document whose v2 signature does not verify, but whose signature does
/// verify under [`SIGNING_PREFIX_V1`], is a v1 document from the compiled-in
/// key and is refused as [`PinRefusal::V1Document`]; its payload is not read.
pub fn verify(
    document: &[u8],
    public_key: &[u8],
    now: u64,
    gateway_host: &str,
) -> Result<VerifiedPins, PinRefusal> {
    let signed = SignedPinSet::parse(document)?;
    if signed.key_id != key_id(public_key) {
        return Err(PinRefusal::UnknownKey);
    }
    let key = UnparsedPublicKey::new(&ED25519, public_key);
    if key
        .verify(&signing_input(&signed.payload), &signed.signature)
        .is_err()
    {
        let mut v1 = SIGNING_PREFIX_V1.to_vec();
        v1.extend_from_slice(&signed.payload);
        return Err(if key.verify(&v1, &signed.signature).is_ok() {
            PinRefusal::V1Document
        } else {
            PinRefusal::BadSignature
        });
    }

    let payload = PinPayload::parse(&signed.payload)?;
    if payload.format != FORMAT {
        return Err(PinRefusal::WrongFormat);
    }
    if payload.channel != CHANNEL {
        return Err(PinRefusal::WrongChannel);
    }
    if payload.gateway_host != gateway_host {
        return Err(PinRefusal::WrongGatewayHost);
    }
    // The document's own lifetime first, so a document that could never be
    // valid reports that, whatever the clock says.
    if payload.expires_at <= payload.issued_at
        || payload.expires_at - payload.issued_at > MAX_LIFETIME_SECS
    {
        return Err(PinRefusal::LifetimeInvalid);
    }
    if payload.issued_at > now.saturating_add(ISSUE_SKEW_SECS) {
        return Err(PinRefusal::IssuedInFuture);
    }
    if now >= payload.expires_at {
        return Err(PinRefusal::Expired);
    }
    if payload.sets.is_empty() {
        return Err(PinRefusal::NoSets);
    }
    let sets = payload
        .sets
        .iter()
        .map(
            |set| match ExpectedMeasurements::from_env_value(Some(set)) {
                Ok(Some(parsed)) => Ok(parsed),
                Ok(None) | Err(_) => Err(PinRefusal::SetUnparseable),
            },
        )
        .collect::<Result<Vec<_>, _>>()?;
    if sets.iter().any(|set| {
        let named = set.pinned_fields();
        REQUIRED_REGISTERS
            .iter()
            .any(|field| !named.contains(field))
    }) {
        return Err(PinRefusal::SetMissingRegister);
    }

    Ok(VerifiedPins {
        sequence: payload.sequence,
        issued_at: payload.issued_at,
        expires_at: payload.expires_at,
        sets,
        allowlist: payload.allowlist,
        payload_sha256: Sha256::digest(&signed.payload).into(),
    })
}

fn json_object(bytes: &[u8]) -> Result<Map<String, Value>, PinRefusal> {
    match serde_json::from_slice(bytes) {
        Ok(Value::Object(object)) => Ok(object),
        _ => Err(PinRefusal::Malformed),
    }
}

/// Refuse a missing field or one the format does not define.
fn exact_fields(object: &Map<String, Value>, fields: &[&str]) -> Result<(), PinRefusal> {
    if object.len() == fields.len() && fields.iter().all(|f| object.contains_key(*f)) {
        Ok(())
    } else {
        Err(PinRefusal::Malformed)
    }
}

fn string_field<'a>(object: &'a Map<String, Value>, field: &str) -> Result<&'a str, PinRefusal> {
    object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(PinRefusal::Malformed)
}

/// Decode unpadded base64url (RFC 4648 section 5). Padding, characters
/// outside the alphabet, an impossible length and non-zero trailing bits are
/// all refused, so each byte string has exactly one accepted encoding.
fn base64url_decode(text: &str) -> Result<Vec<u8>, PinRefusal> {
    fn value(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'-' => Some(62),
            b'_' => Some(63),
            _ => None,
        }
    }
    let bytes = text.as_bytes();
    if bytes.len() % 4 == 1 {
        return Err(PinRefusal::Malformed);
    }
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut n = 0u32;
        for (i, &c) in chunk.iter().enumerate() {
            n |= value(c).ok_or(PinRefusal::Malformed)? << (18 - 6 * i);
        }
        // A chunk of k characters carries k - 1 bytes; the bits below them
        // must be zero (a full chunk has none).
        let produced = chunk.len() - 1;
        if n & ((1u32 << (24 - 8 * produced)) - 1) != 0 {
            return Err(PinRefusal::Malformed);
        }
        out.extend_from_slice(&n.to_be_bytes()[1..=produced]);
    }
    Ok(out)
}

/// Parse exactly `YYYY-MM-DDTHH:MM:SSZ` to seconds since the Unix epoch.
/// Anything else, including a date before the epoch, is `None`.
fn parse_utc(text: &str) -> Option<u64> {
    let b = text.as_bytes();
    if b.len() != 20
        || b[4] != b'-'
        || b[7] != b'-'
        || b[10] != b'T'
        || b[13] != b':'
        || b[16] != b':'
        || b[19] != b'Z'
    {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u64> {
        let digits = &b[range];
        if !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        Some(digits.iter().fold(0, |n, d| n * 10 + u64::from(d - b'0')))
    };
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let month_days = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    if year < 1970
        || !(1..=12).contains(&month)
        || day == 0
        || day > month_days[(month - 1) as usize]
        || hour > 23
        || minute > 59
        || second > 59
    {
        return None;
    }
    let days_before_year: u64 = (1970..year)
        .map(|y| {
            if (y % 4 == 0 && y % 100 != 0) || y % 400 == 0 {
                366
            } else {
                365
            }
        })
        .sum();
    let days_before_month: u64 = month_days[..(month - 1) as usize].iter().sum();
    let days = days_before_year + days_before_month + day - 1;
    Some(days * 86_400 + hour * 3_600 + minute * 60 + second)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::staging_replay::Allowlist;
    use ring::rand::SystemRandom;
    use ring::signature::{Ed25519KeyPair, KeyPair};

    const GATEWAY: &str = "gateway.staging.example";
    /// 2026-10-21T18:00:00Z.
    const ISSUED: u64 = 1_792_605_600;
    const DAY: u64 = 86_400;

    fn key_pair() -> Ed25519KeyPair {
        let pkcs8 = Ed25519KeyPair::generate_pkcs8(&SystemRandom::new()).expect("generate");
        Ed25519KeyPair::from_pkcs8(pkcs8.as_ref()).expect("parse")
    }

    fn b64url(bytes: &[u8]) -> String {
        const ALPHABET: &[u8; 64] =
            b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let b = [
                chunk[0],
                *chunk.get(1).unwrap_or(&0),
                *chunk.get(2).unwrap_or(&0),
            ];
            let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
            for i in 0..=chunk.len() {
                out.push(ALPHABET[((n >> (18 - 6 * i)) & 63) as usize] as char);
            }
        }
        out
    }

    /// A set naming the S16 registers, as a v2 document must.
    fn set(fill: char) -> String {
        let v: String = std::iter::repeat_n(fill, 96).collect();
        format!("mrtd={v},mrconfigid={v},rtmr0={v},rtmr1={v},rtmr2={v}")
    }

    /// A synthetic allowlist, as the v2 payload carries it.
    fn allowlist_json() -> serde_json::Value {
        serde_json::json!({
            "rtmr3": {
                "app_id": "a1".repeat(20),
                "mr_kms": "3b".repeat(48),
                "os_image_hash": "05".repeat(32),
                "key_provider": "6b6d73",
                "markers": [{"name": "system-ready", "payload": "00"}],
            },
            "compose_actions": {
                "compose_manager_images": [format!("registry.example/compose-manager@sha256:{}", "ab".repeat(32))],
                "compose_files": [
                    {"file": "example/models.yaml", "file_sha256": "f1".repeat(32), "commit": "c1".repeat(20)},
                ],
            },
        })
    }

    fn payload_json(
        format: &str,
        channel: &str,
        gateway: &str,
        issued_at: &str,
        expires_at: &str,
        sets: &[String],
    ) -> Vec<u8> {
        serde_json::json!({
            "format": format,
            "channel": channel,
            "gateway_host": gateway,
            "sequence": 42,
            "issued_at": issued_at,
            "expires_at": expires_at,
            "sets": sets,
            "allowlist": allowlist_json(),
        })
        .to_string()
        .into_bytes()
    }

    fn good_payload() -> Vec<u8> {
        payload_json(
            FORMAT,
            CHANNEL,
            GATEWAY,
            "2026-10-21T18:00:00Z",
            "2026-10-28T18:00:00Z",
            &[set('a'), set('b')],
        )
    }

    fn document(payload: &[u8], signature: &[u8], key_id: &str) -> Vec<u8> {
        serde_json::json!({
            "payload": b64url(payload),
            "signature": b64url(signature),
            "key_id": key_id,
        })
        .to_string()
        .into_bytes()
    }

    fn signed(key: &Ed25519KeyPair, payload: &[u8]) -> Vec<u8> {
        let signature = key.sign(&signing_input(payload));
        document(
            payload,
            signature.as_ref(),
            &key_id(key.public_key().as_ref()),
        )
    }

    fn check(key: &Ed25519KeyPair, doc: &[u8], now: u64) -> Result<VerifiedPins, PinRefusal> {
        verify(doc, key.public_key().as_ref(), now, GATEWAY)
    }

    #[test]
    fn a_well_signed_current_set_verifies() {
        let key = key_pair();
        let payload = good_payload();
        let pins = check(&key, &signed(&key, &payload), ISSUED + DAY).expect("verifies");
        assert_eq!(pins.sequence, 42);
        assert_eq!(pins.issued_at, ISSUED);
        assert_eq!(pins.expires_at, ISSUED + 7 * DAY);
        assert_eq!(pins.sets.len(), 2);
        assert_eq!(pins.sets[0].to_pin_string(), set('a'));
        let digest: [u8; 32] = Sha256::digest(&payload).into();
        assert_eq!(pins.payload_sha256, digest);
        assert_eq!(
            pins.allowlist,
            Allowlist::from_json(&allowlist_json()).unwrap(),
            "the signed allowlist comes back parsed"
        );
    }

    /// A v1 document: the v1 format, the v1 prefix, no allowlist.
    fn v1_document(key: &Ed25519KeyPair) -> Vec<u8> {
        let v: String = std::iter::repeat_n('a', 96).collect();
        let payload = serde_json::json!({
            "format": FORMAT_V1,
            "channel": CHANNEL,
            "gateway_host": GATEWAY,
            "sequence": 42,
            "issued_at": "2026-10-21T18:00:00Z",
            "expires_at": "2026-10-28T18:00:00Z",
            "sets": [format!("mrtd={v},rtmr0={v},rtmr1={v},rtmr2={v},rtmr3={v}")],
        })
        .to_string()
        .into_bytes();
        let mut input = SIGNING_PREFIX_V1.to_vec();
        input.extend_from_slice(&payload);
        let signature = key.sign(&input);
        document(
            &payload,
            signature.as_ref(),
            &key_id(key.public_key().as_ref()),
        )
    }

    #[test]
    fn a_v1_document_is_refused_by_a_replay_aware_verifier() {
        let key = key_pair();
        // A genuine v1 document, signed by our key under the v1 prefix: it
        // carries no allowlist, so it cannot pass as one with an empty
        // allowlist. Refused with its own label, and nothing pinned.
        assert_eq!(
            check(&key, &v1_document(&key), ISSUED + DAY).unwrap_err(),
            PinRefusal::V1Document
        );
        // The v1 format string under the v2 prefix is a wrong format.
        let payload = payload_json(
            FORMAT_V1,
            CHANNEL,
            GATEWAY,
            "2026-10-21T18:00:00Z",
            "2026-10-28T18:00:00Z",
            &[set('a')],
        );
        assert_eq!(
            check(&key, &signed(&key, &payload), ISSUED).unwrap_err(),
            PinRefusal::WrongFormat
        );
        // A v1 document signed by another key is that key's problem first.
        assert_eq!(
            check(&key, &v1_document(&key_pair()), ISSUED).unwrap_err(),
            PinRefusal::UnknownKey
        );
        assert_eq!(FORMAT, "trace-commons-near-ai-pins/v2");
        assert_eq!(FORMAT_V1, "trace-commons-near-ai-pins/v1");
        assert_eq!(SIGNING_PREFIX_V1, b"trace-commons-near-ai-pins/v1\n");
    }

    #[test]
    fn the_payload_must_carry_a_well_formed_allowlist() {
        let key = key_pair();
        let mut missing: serde_json::Value = serde_json::from_slice(&good_payload()).unwrap();
        missing.as_object_mut().unwrap().remove("allowlist");
        let mut malformed = missing.clone();
        malformed["allowlist"] = serde_json::json!({"rtmr3": {}, "compose_actions": {}});
        for value in [missing, malformed] {
            assert_eq!(
                check(&key, &signed(&key, value.to_string().as_bytes()), ISSUED).unwrap_err(),
                PinRefusal::Malformed,
                "{value}"
            );
        }
        // The placeholder (empty values, empty lists) is well formed: it is
        // the replay that refuses every entry checked against it.
        let mut placeholder: serde_json::Value = serde_json::from_slice(&good_payload()).unwrap();
        placeholder["allowlist"] = serde_json::json!({
            "rtmr3": {"app_id": "", "mr_kms": "", "os_image_hash": "", "key_provider": "", "markers": []},
            "compose_actions": {"compose_manager_images": [], "compose_files": []},
        });
        let pins = check(
            &key,
            &signed(&key, placeholder.to_string().as_bytes()),
            ISSUED,
        )
        .expect("the placeholder verifies");
        assert!(pins.allowlist.compose_actions.is_empty());
    }

    #[test]
    fn a_set_without_the_s16_registers_is_refused() {
        // The replay checks the compose-hash event against MRCONFIGID and
        // trusts RTMR0-2 and MRTD as pinned: a set that leaves any of them
        // out would make those checks vacuous.
        let key = key_pair();
        let v: String = std::iter::repeat_n('a', 96).collect();
        for bad in [
            format!("mrtd={v},rtmr0={v},rtmr1={v},rtmr2={v},rtmr3={v}"),
            format!("mrtd={v},rtmr0={v},rtmr1={v},mrconfigid={v}"),
            format!("rtmr0={v},rtmr1={v},rtmr2={v},mrconfigid={v}"),
        ] {
            let payload = payload_json(
                FORMAT,
                CHANNEL,
                GATEWAY,
                "2026-10-21T18:00:00Z",
                "2026-10-28T18:00:00Z",
                &[set('b'), bad.clone()],
            );
            assert_eq!(
                check(&key, &signed(&key, &payload), ISSUED).unwrap_err(),
                PinRefusal::SetMissingRegister,
                "{bad}"
            );
        }
        // More than the S16 registers is allowed: rtmr3 as well.
        let payload = payload_json(
            FORMAT,
            CHANNEL,
            GATEWAY,
            "2026-10-21T18:00:00Z",
            "2026-10-28T18:00:00Z",
            &[format!(
                "mrtd={v},rtmr0={v},rtmr1={v},rtmr2={v},rtmr3={v},mrconfigid={v}"
            )],
        );
        assert!(check(&key, &signed(&key, &payload), ISSUED).is_ok());
    }

    #[test]
    fn the_key_id_is_the_first_sixteen_hex_of_the_key_digest() {
        let key = key_pair();
        let full = hex::encode(Sha256::digest(key.public_key().as_ref()));
        assert_eq!(key_id(key.public_key().as_ref()), full[..16]);
    }

    #[test]
    fn a_signature_by_another_key_is_refused() {
        let key = key_pair();
        let other = key_pair();
        let payload = good_payload();
        // Signed by `other`, claiming the expected key's id.
        let signature = other.sign(&signing_input(&payload));
        let doc = document(
            &payload,
            signature.as_ref(),
            &key_id(key.public_key().as_ref()),
        );
        assert_eq!(
            check(&key, &doc, ISSUED).unwrap_err(),
            PinRefusal::BadSignature
        );
        // A tampered payload under a good signature is the same failure.
        let mut tampered = payload.clone();
        tampered[0] = b' ';
        let good = key.sign(&signing_input(&payload));
        let doc = document(&tampered, good.as_ref(), &key_id(key.public_key().as_ref()));
        assert_eq!(
            check(&key, &doc, ISSUED).unwrap_err(),
            PinRefusal::BadSignature
        );
    }

    #[test]
    fn an_unknown_key_id_is_refused() {
        let key = key_pair();
        let other = key_pair();
        let payload = good_payload();
        // Honestly signed by another key: its own id, not ours.
        assert_eq!(
            check(&key, &signed(&other, &payload), ISSUED).unwrap_err(),
            PinRefusal::UnknownKey
        );
        // Our signature, but the id field altered.
        let signature = key.sign(&signing_input(&payload));
        let doc = document(&payload, signature.as_ref(), "0000000000000000");
        assert_eq!(
            check(&key, &doc, ISSUED).unwrap_err(),
            PinRefusal::UnknownKey
        );
    }

    #[test]
    fn a_key_that_is_not_an_ed25519_key_refuses_every_document() {
        let key = key_pair();
        let doc = signed(&key, &good_payload());
        let short = &key.public_key().as_ref()[..31];
        // The id does not match the short key: refused before the signature.
        let refusal = verify(&doc, short, ISSUED, GATEWAY).unwrap_err();
        assert_eq!(refusal, PinRefusal::UnknownKey);
        // The id is the short key's own, which anyone can compute for a key
        // compiled into a public build: only the signature check stands in
        // the way, and it must refuse a key that is not an Ed25519 key.
        let payload = good_payload();
        let signature = key.sign(&signing_input(&payload));
        let doc = document(&payload, signature.as_ref(), &key_id(short));
        let refusal = verify(&doc, short, ISSUED, GATEWAY).unwrap_err();
        assert_eq!(refusal, PinRefusal::BadSignature);
    }

    #[test]
    fn the_signature_covers_the_domain_prefix() {
        let key = key_pair();
        let payload = good_payload();
        let bare = key.sign(&payload);
        let doc = document(&payload, bare.as_ref(), &key_id(key.public_key().as_ref()));
        assert_eq!(
            check(&key, &doc, ISSUED).unwrap_err(),
            PinRefusal::BadSignature
        );
        assert_eq!(SIGNING_PREFIX, b"trace-commons-near-ai-pins/v2\n");
        assert_eq!(
            signing_input(b"xy"),
            b"trace-commons-near-ai-pins/v2\nxy".to_vec()
        );
    }

    #[test]
    fn a_wrong_channel_format_or_gateway_host_is_refused() {
        let key = key_pair();
        let (i, e) = ("2026-10-21T18:00:00Z", "2026-10-28T18:00:00Z");
        let sets = [set('a')];
        let cases = [
            (
                payload_json(
                    "trace-commons-near-ai-pins/v3",
                    CHANNEL,
                    GATEWAY,
                    i,
                    e,
                    &sets,
                ),
                PinRefusal::WrongFormat,
            ),
            (
                payload_json(FORMAT, "production", GATEWAY, i, e, &sets),
                PinRefusal::WrongChannel,
            ),
            (
                payload_json(FORMAT, CHANNEL, "gateway.example", i, e, &sets),
                PinRefusal::WrongGatewayHost,
            ),
        ];
        for (payload, want) in cases {
            assert_eq!(
                check(&key, &signed(&key, &payload), ISSUED).unwrap_err(),
                want
            );
        }
    }

    #[test]
    fn an_expired_or_future_set_is_refused() {
        let key = key_pair();
        let doc = signed(&key, &good_payload());
        let expires = ISSUED + 7 * DAY;
        // Issue edge: up to five minutes of clock skew is allowed, no more.
        assert!(check(&key, &doc, ISSUED - ISSUE_SKEW_SECS).is_ok());
        assert_eq!(
            check(&key, &doc, ISSUED - ISSUE_SKEW_SECS - 1).unwrap_err(),
            PinRefusal::IssuedInFuture
        );
        // Expiry edge: strict, as the spec states (`now < expires_at`).
        assert!(check(&key, &doc, expires - 1).is_ok());
        assert_eq!(check(&key, &doc, expires).unwrap_err(), PinRefusal::Expired);
        assert_eq!(
            check(&key, &doc, expires + DAY).unwrap_err(),
            PinRefusal::Expired
        );
    }

    #[test]
    fn a_lifetime_over_fourteen_days_is_refused() {
        let key = key_pair();
        let sets = [set('a')];
        let exactly = payload_json(
            FORMAT,
            CHANNEL,
            GATEWAY,
            "2026-10-21T18:00:00Z",
            "2026-11-04T18:00:00Z",
            &sets,
        );
        assert!(check(&key, &signed(&key, &exactly), ISSUED).is_ok());
        let over = payload_json(
            FORMAT,
            CHANNEL,
            GATEWAY,
            "2026-10-21T18:00:00Z",
            "2026-11-04T18:00:01Z",
            &sets,
        );
        assert_eq!(
            check(&key, &signed(&key, &over), ISSUED).unwrap_err(),
            PinRefusal::LifetimeInvalid
        );
        // Expiring before it was issued is no lifetime at all.
        let backwards = payload_json(
            FORMAT,
            CHANNEL,
            GATEWAY,
            "2026-10-21T18:00:00Z",
            "2026-10-21T17:59:00Z",
            &sets,
        );
        // The lifetime is checked before the clock, so a backwards document
        // reports its own label at any time, not `pins_expired`.
        for now in [ISSUED - 120, ISSUED, ISSUED + DAY] {
            assert_eq!(
                check(&key, &signed(&key, &backwards), now).unwrap_err(),
                PinRefusal::LifetimeInvalid,
                "{now}"
            );
        }
        // Expiring the instant it is issued is no lifetime either, even
        // inside the issue skew window, where the clock checks would pass.
        let zero = payload_json(
            FORMAT,
            CHANNEL,
            GATEWAY,
            "2026-10-21T18:00:00Z",
            "2026-10-21T18:00:00Z",
            &sets,
        );
        assert_eq!(
            check(&key, &signed(&key, &zero), ISSUED - 60).unwrap_err(),
            PinRefusal::LifetimeInvalid
        );
    }

    #[test]
    fn debug_output_carries_no_document_bytes() {
        let key = key_pair();
        let payload = good_payload();
        let doc = signed(&key, &payload);
        let outer = SignedPinSet::parse(&doc).expect("parses");
        let inner = PinPayload::parse(&payload).expect("parses");
        let printed = format!("{outer:?} {inner:?}");
        for secret in [
            "a".repeat(96),
            "b".repeat(96),
            GATEWAY.to_string(),
            hex::encode(&outer.signature),
            format!("{:?}", outer.signature),
            format!("{:?}", outer.payload),
        ] {
            assert!(!printed.contains(&secret), "{printed}");
        }
        // What it does print is lengths and the payload digest.
        assert!(
            printed.contains(&hex::encode(Sha256::digest(&payload))),
            "{printed}"
        );
    }

    #[test]
    fn one_unparseable_set_refuses_the_document() {
        let key = key_pair();
        let (i, e) = ("2026-10-21T18:00:00Z", "2026-10-28T18:00:00Z");
        for bad in [
            "mrtd=00".to_string(),
            "compose_hash=".to_string() + &"0".repeat(96),
            String::new(),
            "not a set".to_string(),
        ] {
            let payload = payload_json(FORMAT, CHANNEL, GATEWAY, i, e, &[set('a'), bad]);
            assert_eq!(
                check(&key, &signed(&key, &payload), ISSUED).unwrap_err(),
                PinRefusal::SetUnparseable
            );
        }
    }

    #[test]
    fn an_empty_set_list_is_refused() {
        let key = key_pair();
        let payload = payload_json(
            FORMAT,
            CHANNEL,
            GATEWAY,
            "2026-10-21T18:00:00Z",
            "2026-10-28T18:00:00Z",
            &[],
        );
        assert_eq!(
            check(&key, &signed(&key, &payload), ISSUED).unwrap_err(),
            PinRefusal::NoSets
        );
    }

    #[test]
    fn a_malformed_document_or_payload_is_refused() {
        let key = key_pair();
        let id = key_id(key.public_key().as_ref());
        let payload = good_payload();
        let sig = b64url(key.sign(&signing_input(&payload)).as_ref());
        let p = b64url(&payload);
        for doc in [
            "not json".to_string(),
            "[]".to_string(),
            serde_json::json!({"payload": p, "signature": sig}).to_string(),
            serde_json::json!({"payload": p, "signature": sig, "key_id": id, "extra": 1})
                .to_string(),
            serde_json::json!({"payload": p + "=", "signature": sig, "key_id": id}).to_string(),
            serde_json::json!({"payload": "a+b/", "signature": sig, "key_id": id}).to_string(),
            // Non-zero trailing bits: a second encoding of the same byte.
            serde_json::json!({"payload": "AB", "signature": sig, "key_id": id}).to_string(),
            serde_json::json!({"payload": "AAAAA", "signature": sig, "key_id": id}).to_string(),
            serde_json::json!({"payload": b64url(&payload), "signature": 7, "key_id": id})
                .to_string(),
        ] {
            assert_eq!(
                check(&key, doc.as_bytes(), ISSUED).unwrap_err(),
                PinRefusal::Malformed,
                "{doc}"
            );
        }
        // A signed payload that is not the v1 shape.
        let mut extra: serde_json::Value = serde_json::from_slice(&good_payload()).unwrap();
        extra["note"] = serde_json::json!("x");
        let mut no_seq = extra.clone();
        no_seq.as_object_mut().unwrap().remove("note");
        no_seq["sequence"] = serde_json::json!(-1);
        let mut bad_time = no_seq.clone();
        bad_time["sequence"] = serde_json::json!(1);
        bad_time["issued_at"] = serde_json::json!("2026-10-21T18:00:00+00:00");
        for value in [extra, no_seq, bad_time] {
            let payload = value.to_string().into_bytes();
            assert_eq!(
                check(&key, &signed(&key, &payload), ISSUED).unwrap_err(),
                PinRefusal::Malformed,
                "{value}"
            );
        }
        assert_eq!(
            check(&key, &signed(&key, b"not json"), ISSUED).unwrap_err(),
            PinRefusal::Malformed
        );
    }

    #[test]
    fn base64url_decodes_every_length_it_can_encode() {
        let bytes: Vec<u8> = (0..=255).collect();
        for len in 0..40 {
            let slice = &bytes[200 - len..200];
            assert_eq!(
                base64url_decode(&b64url(slice)),
                Ok(slice.to_vec()),
                "{len}"
            );
        }
    }

    #[test]
    fn timestamps_parse_only_in_the_one_utc_form() {
        assert_eq!(parse_utc("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_utc("2026-10-21T18:00:00Z"), Some(ISSUED));
        assert_eq!(parse_utc("2028-02-29T00:00:00Z"), Some(1_835_395_200));
        for bad in [
            "2026-02-29T00:00:00Z",
            "2026-10-21T24:00:00Z",
            "2026-10-21T18:00:60Z",
            "2026-13-01T00:00:00Z",
            "2026-10-21 18:00:00Z",
            "2026-10-21T18:00:00.5Z",
            "2026-10-21T18:00:00z",
            "1969-12-31T23:59:59Z",
            "+026-10-21T18:00:00Z",
        ] {
            assert_eq!(parse_utc(bad), None, "{bad}");
        }
    }

    #[test]
    fn refusal_labels_are_stable_and_carry_no_document_bytes() {
        let all = [
            (PinRefusal::Malformed, "pins_malformed"),
            (PinRefusal::UnknownKey, "pins_unknown_key"),
            (PinRefusal::BadSignature, "pins_bad_signature"),
            (PinRefusal::WrongFormat, "pins_wrong_format"),
            (PinRefusal::WrongChannel, "pins_wrong_channel"),
            (PinRefusal::WrongGatewayHost, "pins_wrong_gateway_host"),
            (PinRefusal::Expired, "pins_expired"),
            (PinRefusal::IssuedInFuture, "pins_issued_in_future"),
            (PinRefusal::LifetimeInvalid, "pins_lifetime_invalid"),
            (PinRefusal::SetUnparseable, "pins_set_unparseable"),
            (PinRefusal::NoSets, "pins_no_sets"),
            (PinRefusal::V1Document, "pins_v1_document"),
            (PinRefusal::SetMissingRegister, "pins_set_missing_register"),
        ];
        for (refusal, label) in all {
            assert_eq!(refusal.label(), label);
            assert_eq!(refusal.to_string(), label);
        }
    }
}
