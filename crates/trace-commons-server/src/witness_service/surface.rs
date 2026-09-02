// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The narrowed capability the HTTP surface is given.
//!
//! [`super::witness`] takes three seams as trait objects; a handler that held
//! them would hold more authority than serving two routes needs. In
//! particular it would hold an [`Enclave`], and
//! [`Enclave::attestation_quote`] takes **arbitrary** report data -- so a
//! handler could compose report data itself, get it wrong, and serve a quote
//! that carries no caller nonce. That failure is a replay, and it looks
//! exactly like a success at the response boundary.
//!
//! [`Enclave::nonce_bound_quote`] exists to prevent it, but a method nobody is
//! *required* to use is a convention, and this repository keeps finding
//! conventions at the bottom of its defects.
//!
//! So this module holds the seams behind private fields and publishes exactly
//! two operations: [`WitnessService::witness`] and
//! [`WitnessService::attest`]. Neither returns the enclave, nothing derefs to
//! it, and [`super::http`] is a *different* module -- so Rust's privacy rules,
//! not a comment, are what make the unbound call unwritable there. The second
//! half of the guard is [`ContributorNonce`]: `attest` cannot be called with
//! anything but 32 bytes that were parsed from hex, so there is no path by
//! which a short, padded, truncated or hashed nonce reaches `report_data`.
//!
//! This is the shape `WitnessCertificate` uses at the other end of the
//! service, where a digest can only come from a `CorrespondenceProof`.
//!
//! [`Enclave`]: super::Enclave
//! [`Enclave::attestation_quote`]: super::Enclave::attestation_quote
//! [`Enclave::nonce_bound_quote`]: super::Enclave::nonce_bound_quote

use std::sync::Arc;

use super::enclave::WITNESS_NONCE_LEN;
use super::{
    Enclave, SeamUnavailable, Signer, TranscriptRedactor, WitnessError, WitnessRequest,
    WitnessResponse, witness,
};

/// A contributor's attestation nonce: exactly [`WITNESS_NONCE_LEN`] bytes,
/// and no way to make one that is not.
///
/// The field is private and there is no constructor but [`Self::parse_hex`].
/// That is the point of the type rather than an accident of style: the only
/// interesting way for `/v1/attestation` to fail is to serve a quote bound to
/// bytes that are not the ones the caller chose, and every route to that
/// failure -- padding a short nonce, truncating a long one, hashing a
/// malformed one into the right length -- begins with a value that is not
/// already 32 bytes. A `&[u8]` parameter would leave all of them open.
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct ContributorNonce([u8; WITNESS_NONCE_LEN]);

impl ContributorNonce {
    /// Parse exactly `2 * WITNESS_NONCE_LEN` hex characters.
    ///
    /// Bare hex only. dstack's `GetQuote` accepts an optional `0x` prefix and
    /// right-pads anything shorter than 64 bytes, and both of those are
    /// leniencies this surface declines: one encoding means a contributor and
    /// a witness cannot disagree about which bytes were bound, and padding is
    /// precisely how a nonce that was never sent ends up inside a quote that
    /// appears to answer for it.
    ///
    /// Case-insensitive, because `hex::decode` accepts both and the two
    /// spellings are the same 32 bytes.
    pub fn parse_hex(value: &str) -> Result<Self, NonceMalformed> {
        if value.len() != WITNESS_NONCE_LEN * 2 {
            return Err(NonceMalformed);
        }
        let decoded = hex::decode(value).map_err(|_| NonceMalformed)?;
        let bytes = decoded.try_into().map_err(|_| NonceMalformed)?;
        Ok(Self(bytes))
    }

    /// The bytes, for [`WitnessService::attest`] and nothing else.
    ///
    /// `pub(super)` on purpose. If the HTTP module could read these it could
    /// hand them to something that is not `nonce_bound_quote`, and the type
    /// would be back to being a convention.
    pub(super) fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}

impl std::fmt::Debug for ContributorNonce {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The nonce is the contributor's, not content, and rendering it is
        // what makes a failing attestation test readable. Hand-written rather
        // than derived so that gaining a content-bearing field would be a
        // visible decision here.
        write!(formatter, "ContributorNonce({})", hex::encode(self.0))
    }
}

/// The nonce was not exactly 32 bytes of bare hex.
///
/// One variant, carrying nothing. "Too short", "not hex" and "`0x`-prefixed"
/// are the same instruction to the caller -- send 64 hex characters -- and
/// three labels would only tell a prober which of its guesses was closest.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the nonce is not 32 bytes of hex")]
pub struct NonceMalformed;

/// The enclave could not produce a quote.
///
/// Deliberately not a [`WitnessError`] variant. `WitnessError` answers "why
/// was nothing certified"; every one of its variants means a certificate was
/// refused, and an attestation failure certifies nothing either way. Folding
/// the two would let a surface report a quote failure with a name that reads
/// as a redaction failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the enclave could not produce a nonce-bound quote")]
pub struct AttestationError;

/// What `/v1/attestation` returns.
///
/// The quote and the address it names, and nothing else. Notably **not** the
/// measurement label: the measurement a contributor pins has to be read out of
/// the quote they verified, and serving a convenient copy beside it invites
/// pinning the copy -- which any witness could type, attested by nothing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttestationEvidence {
    /// The raw quote bytes as lowercase hex, no `0x` prefix.
    pub quote_hex: String,
    /// The address that signs this witness's certificates, as the quote's
    /// report data binds it.
    pub signing_address: String,
}

/// The witness, as the HTTP surface is allowed to see it.
pub struct WitnessService {
    redactor: Arc<dyn TranscriptRedactor>,
    signer: Arc<dyn Signer>,
    enclave: Arc<dyn Enclave>,
    max_request_bytes: usize,
}

impl WitnessService {
    /// Assemble the service from its three seams and the request bound.
    ///
    /// The bound is a constructor parameter rather than a constant because the
    /// witness receives **raw** transcripts: the redacted-envelope cap is
    /// 16 MiB, the measured raw-to-envelope ratio on this pilot is about
    /// 3.4:1, and 7% of real sessions already exceed the cap before that
    /// multiplier. There is no single right number, so the deployment picks
    /// one and the surface refuses over it by name.
    pub fn new(
        redactor: Arc<dyn TranscriptRedactor>,
        signer: Arc<dyn Signer>,
        enclave: Arc<dyn Enclave>,
        max_request_bytes: usize,
    ) -> Self {
        Self {
            redactor,
            signer,
            enclave,
            max_request_bytes,
        }
    }

    /// The largest request body this witness will read, in bytes.
    pub fn max_request_bytes(&self) -> usize {
        self.max_request_bytes
    }

    /// Redact, judge and certify. Thin over [`super::witness`].
    pub async fn witness(&self, request: WitnessRequest) -> Result<WitnessResponse, WitnessError> {
        witness(
            request,
            self.redactor.as_ref(),
            self.signer.as_ref(),
            self.enclave.as_ref(),
        )
        .await
    }

    /// A quote bound to `nonce` and to this witness's signing address.
    ///
    /// The single call site of [`Enclave::nonce_bound_quote`] in this crate,
    /// and the only way any caller outside this module obtains a quote.
    ///
    /// [`Enclave::nonce_bound_quote`]: super::Enclave::nonce_bound_quote
    pub async fn attest(
        &self,
        nonce: &ContributorNonce,
    ) -> Result<AttestationEvidence, AttestationError> {
        let quote = self
            .enclave
            .nonce_bound_quote(nonce.as_bytes())
            .await
            .map_err(|SeamUnavailable| AttestationError)?;
        Ok(AttestationEvidence {
            quote_hex: hex::encode(quote),
            signing_address: self.enclave.signing_address().to_string(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_nonce_is_thirty_two_bytes_of_bare_hex_and_nothing_else() {
        let good = hex::encode([0x5au8; WITNESS_NONCE_LEN]);
        assert_eq!(
            ContributorNonce::parse_hex(&good)
                .expect("64 hex characters")
                .as_bytes(),
            &[0x5au8; WITNESS_NONCE_LEN]
        );
        // Uppercase is the same 32 bytes, not a second encoding.
        assert_eq!(
            ContributorNonce::parse_hex(&good.to_uppercase()).expect("uppercase hex"),
            ContributorNonce::parse_hex(&good).expect("lowercase hex")
        );

        let rejected = [
            ("empty", String::new()),
            ("one byte short", hex::encode([0xaau8; 31])),
            ("one byte long", hex::encode([0xaau8; 33])),
            ("odd length", "a".repeat(63)),
            ("not hex", "z".repeat(64)),
            ("0x prefixed", format!("0x{}", hex::encode([0xaau8; 31]))),
            (
                "0x prefixed full length",
                format!("0x{}", hex::encode([0xaau8; WITNESS_NONCE_LEN])),
            ),
            ("trailing space", format!("{} ", &good[..63])),
        ];
        // Collected, not asserted in the loop: a short-circuiting assertion
        // would let the first accepted case hide every one after it.
        let accepted: Vec<&str> = rejected
            .iter()
            .filter(|(_, value)| ContributorNonce::parse_hex(value).is_ok())
            .map(|(label, _)| *label)
            .collect();
        assert!(
            accepted.is_empty(),
            "malformed nonces accepted: {accepted:?}"
        );
    }

    #[test]
    fn debug_renders_the_nonce_and_the_error_carries_nothing() {
        let nonce = ContributorNonce::parse_hex(&hex::encode([0x01u8; WITNESS_NONCE_LEN]))
            .expect("well formed");
        assert!(format!("{nonce:?}").contains(&hex::encode([0x01u8; WITNESS_NONCE_LEN])));
        assert_eq!(
            format!("{NonceMalformed:?}"),
            "NonceMalformed",
            "the error gained a field, and that field will reach a log"
        );
    }
}
