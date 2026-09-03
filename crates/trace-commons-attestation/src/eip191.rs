//! EIP-191 `personal_sign` signer recovery.
//!
//! Its own module, outside the `receipt` feature, because it is **not** a
//! NEAR AI concern. Two things in this project are signed this way and only
//! one of them is an inference receipt:
//!
//! - a NEAR AI receipt, verified by [`crate::receipt`];
//! - a redaction-witness certificate, which a contributor client verifies
//!   before forwarding to ingest -- the client is the only party holding both
//!   what it sent and what came back, so it is the only party that can catch
//!   a witness returning a certificate signed by some other key.
//!
//! An earlier arrangement had these functions inside `receipt.rs` and gated
//! the whole module, on the reasoning that `k256` and `sha3` were
//! receipt-only. They are not. Gating them would have forced the witness
//! client either to enable a "receipt" feature it makes no receipts with, or
//! to skip the signature check -- and the second of those is a weaker
//! security property bought for nineteen transitive crates. The split is
//! here so the crate is honest about which half a caller uses, not so it can
//! claim a saving it does not make: `k256` and `sha3` are unconditional
//! dependencies and every consumer of this crate pays for them.
//!
//! [`crate::address::decode_address`] is the one genuinely cheap piece and
//! lives apart from both.

use k256::ecdsa::{RecoveryId, Signature, VerifyingKey};
use sha3::{Digest as _, Keccak256};

/// Why a signature did not recover.
///
/// Three variants, because a caller does three different things about them. A
/// signature that is *not 65 bytes of hex* and one that is well formed but
/// recovers to nobody are different failures: the first is a malformed
/// response, the second is a forgery or a corrupted message.
///
/// Nothing here carries the signature or the message. `Debug` delegates to
/// `Display` so that `tracing::warn!(?err)` cannot render what `%err` is
/// guarded against.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum Eip191Error {
    /// Not 65 bytes of hex, or the first 64 bytes are not a signature.
    #[error("the signature is not 65 bytes of hex")]
    SignatureMalformed,
    /// The 65th byte is neither 0/1 nor 27/28.
    #[error("signature recovery byte {v} is neither 0/1 nor 27/28")]
    RecoveryIdUnsupported { v: u8 },
    /// No public key recovers from this signature over this digest.
    #[error("no signer recovers from the signature")]
    SignatureUnrecoverable,
}

impl std::fmt::Debug for Eip191Error {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// The EIP-191 `personal_sign` digest of `message`.
///
/// The length in the preamble is the **byte** length rendered as decimal
/// ASCII, not the character count. For any message outside ASCII the two
/// differ, and a verifier that used the character count would recover a
/// different -- and therefore rejected -- signer.
pub(crate) fn eip191_digest(message: &[u8]) -> [u8; 32] {
    let mut hasher = Keccak256::new();
    hasher.update(b"\x19Ethereum Signed Message:\n");
    hasher.update(message.len().to_string().as_bytes());
    hasher.update(message);
    hasher.finalize().into()
}

/// Recover the 20-byte Ethereum address that produced `signature_hex` over
/// `message` under EIP-191.
///
/// The recovery byte is accepted in both encodings, 27/28 (Ethereum) and 0/1
/// (raw ECDSA). A verifier that handled only one would reject valid
/// signatures from a signer using the other, and that failure would only show
/// up in production against data that cannot be replayed.
pub fn recover_eip191_signer(message: &[u8], signature_hex: &str) -> Result<[u8; 20], Eip191Error> {
    let raw = hex::decode(strip_0x(signature_hex)).map_err(|_| Eip191Error::SignatureMalformed)?;
    if raw.len() != 65 {
        return Err(Eip191Error::SignatureMalformed);
    }
    let v = raw[64];
    let recovery_byte = match v {
        0 | 1 => v,
        27 | 28 => v - 27,
        other => return Err(Eip191Error::RecoveryIdUnsupported { v: other }),
    };
    let recovery_id =
        RecoveryId::from_byte(recovery_byte).ok_or(Eip191Error::RecoveryIdUnsupported { v })?;
    let signature =
        Signature::from_slice(&raw[..64]).map_err(|_| Eip191Error::SignatureMalformed)?;

    let digest = eip191_digest(message);
    let key = VerifyingKey::recover_from_prehash(&digest, &signature, recovery_id)
        .map_err(|_| Eip191Error::SignatureUnrecoverable)?;

    Ok(address_of(&key))
}

/// The Ethereum address of a secp256k1 public key: the last 20 bytes of the
/// keccak256 of the uncompressed encoding with its `0x04` tag removed.
pub(crate) fn address_of(key: &VerifyingKey) -> [u8; 20] {
    let point = key.to_encoded_point(false);
    let digest = Keccak256::digest(&point.as_bytes()[1..]);
    let mut address = [0u8; 20];
    address.copy_from_slice(&digest[12..]);
    address
}

pub(crate) fn strip_0x(s: &str) -> &str {
    s.strip_prefix("0x")
        .or_else(|| s.strip_prefix("0X"))
        .unwrap_or(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k256::ecdsa::SigningKey;

    /// A fixed key, never generated: a random key makes a failure
    /// unreproducible, and this suite is about exact bytes.
    fn signing_key() -> SigningKey {
        SigningKey::from_slice(&Keccak256::digest(b"eip191-module-test-seed"))
            .expect("the seed is a valid scalar")
    }

    fn sign(message: &[u8], recovery_offset: u8) -> String {
        let key = signing_key();
        let digest = eip191_digest(message);
        let (signature, recovery_id) = key
            .sign_prehash_recoverable(&digest)
            .expect("the digest is 32 bytes");
        let mut raw = signature.to_bytes().to_vec();
        raw.push(recovery_id.to_byte() + recovery_offset);
        format!("0x{}", hex::encode(raw))
    }

    fn expected_address() -> [u8; 20] {
        address_of(signing_key().verifying_key())
    }

    #[test]
    fn a_signature_recovers_the_signer_in_both_recovery_encodings() {
        let message = b"trace_commons.redaction_witness_certificate.v1";
        // 27/28, the Ethereum spelling.
        assert_eq!(
            recover_eip191_signer(message, &sign(message, 27)).expect("recovers"),
            expected_address()
        );
        // 0/1, the raw ECDSA spelling. Same signer, not a second identity.
        assert_eq!(
            recover_eip191_signer(message, &sign(message, 0)).expect("recovers"),
            expected_address()
        );
    }

    #[test]
    fn a_signature_over_other_bytes_recovers_someone_else() {
        // The assertion that says recovery is actually reading the message.
        // Without it, a verifier that ignored `message` entirely would pass
        // the test above.
        let signature = sign(b"one message", 27);
        let recovered = recover_eip191_signer(b"another message", &signature);
        match recovered {
            Ok(address) => assert_ne!(
                address,
                expected_address(),
                "recovery ignored the message it was given"
            ),
            Err(Eip191Error::SignatureUnrecoverable) => {}
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }

    #[test]
    fn the_digest_counts_bytes_and_not_characters() {
        // A message whose byte length differs from its character count. A
        // verifier using `chars().count()` builds a different preamble and
        // recovers a different address, so this is what pins the choice.
        let message = "sold for 10 \u{20ac} to caf\u{e9}".as_bytes();
        assert_ne!(
            message.len(),
            "sold for 10 \u{20ac} to caf\u{e9}".chars().count()
        );
        assert_eq!(
            recover_eip191_signer(message, &sign(message, 27)).expect("recovers"),
            expected_address()
        );
    }

    #[test]
    fn malformed_signatures_are_refused_by_specific_name() {
        let message = b"anything";
        assert_eq!(
            recover_eip191_signer(message, "not hex").unwrap_err(),
            Eip191Error::SignatureMalformed
        );
        assert_eq!(
            recover_eip191_signer(message, "0xabcd").unwrap_err(),
            Eip191Error::SignatureMalformed,
            "a 2-byte signature is not 65 bytes"
        );

        // A well-formed 65-byte signature whose recovery byte is neither
        // encoding. Built from a real signature so the only thing wrong with
        // it is the byte under test.
        let mut raw = hex::decode(strip_0x(&sign(message, 27))).unwrap();
        raw[64] = 42;
        assert_eq!(
            recover_eip191_signer(message, &hex::encode(&raw)).unwrap_err(),
            Eip191Error::RecoveryIdUnsupported { v: 42 }
        );
    }

    #[test]
    fn errors_carry_neither_the_signature_nor_the_message() {
        const MARKER: &str = "zzq-eip191-marker-zzq";
        let err = recover_eip191_signer(MARKER.as_bytes(), MARKER).unwrap_err();
        for rendering in [format!("{err}"), format!("{err:?}")] {
            assert!(!rendering.contains(MARKER));
        }
    }
}
