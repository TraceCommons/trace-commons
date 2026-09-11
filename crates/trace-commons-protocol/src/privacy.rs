//! Shared privacy validation for text that may leave the contributor's device.

use std::sync::OnceLock;

use sha2::{Digest, Sha256};

use crate::trace_contribution::DeterministicTraceRedactor;

/// Stable failure returned when outbound text contains sensitive material.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OutboundTextValidationError {
    SensitiveText,
}

/// Reject text that would disclose a credential, private identity, or wallet secret.
///
/// Product surfaces map this neutral error into their own validation errors. The
/// detector remains shared so every outbound boundary enforces the same rules.
pub fn validate_outbound_text(value: &str) -> Result<(), OutboundTextValidationError> {
    if contains_near_private_key(value) || resembles_wallet_recovery_phrase(value) {
        return Err(OutboundTextValidationError::SensitiveText);
    }
    let redactor = DeterministicTraceRedactor::deterministic_only(Vec::new());
    let (redacted, report) = redactor.redact_text(value);
    if redacted != value || report.blocked_secret_detected {
        return Err(OutboundTextValidationError::SensitiveText);
    }
    Ok(())
}

fn contains_near_private_key(value: &str) -> bool {
    // NEAR serializes an Ed25519 keypair from 64 bytes and a secp256k1 secret
    // scalar from 32 bytes. Canonical Base58 preserves leading zero bytes, so
    // each encoded form stays within these disjoint character bounds.
    const FORMATS: [(&[u8], usize, usize); 2] = [(b"ed25519:", 64, 88), (b"secp256k1:", 32, 44)];
    let bytes = value.as_bytes();
    FORMATS.iter().any(|&(prefix, min_chars, max_chars)| {
        bytes
            .windows(prefix.len())
            .enumerate()
            .any(|(start, candidate)| {
                if !candidate.eq_ignore_ascii_case(prefix) {
                    return false;
                }
                let encoded_chars = bytes[start + prefix.len()..]
                    .iter()
                    .take(max_chars.saturating_add(1))
                    .take_while(|byte| {
                        byte.is_ascii_alphanumeric() && !matches!(**byte, b'0' | b'O' | b'I' | b'l')
                    })
                    .count();
                (min_chars..=max_chars).contains(&encoded_chars)
            })
    })
}

fn resembles_wallet_recovery_phrase(value: &str) -> bool {
    let lowered = value.to_ascii_lowercase();
    if [
        "seed phrase",
        "recovery phrase",
        "wallet mnemonic",
        "secret phrase",
    ]
    .iter()
    .any(|cue| lowered.contains(cue))
    {
        return true;
    }
    let words = value
        .split(|character: char| !character.is_ascii_alphabetic())
        .map(str::to_ascii_lowercase)
        .filter(|word| !word.is_empty())
        .collect::<Vec<_>>();
    [12, 15, 18, 21, 24]
        .into_iter()
        .any(|word_count| words.windows(word_count).any(is_valid_bip39_mnemonic))
}

fn is_valid_bip39_mnemonic(words: &[String]) -> bool {
    // Official BIP-0039 English list, pinned from bitcoin/bips commit
    // 620871a7a442e276a058b487cd8743775fb499a4 (MIT licensed).
    static WORDS: OnceLock<Vec<&'static str>> = OnceLock::new();
    let wordlist = WORDS.get_or_init(|| include_str!("bip39_english.txt").lines().collect());
    let Some(indices) = words
        .iter()
        .map(|word| {
            wordlist
                .binary_search_by(|candidate| candidate.cmp(&word.as_str()))
                .ok()
        })
        .collect::<Option<Vec<_>>>()
    else {
        return false;
    };

    let total_bits = indices.len() * 11;
    let entropy_bits = total_bits * 32 / 33;
    let checksum_bits = total_bits - entropy_bits;
    let mut entropy = vec![0u8; entropy_bits / 8];
    for bit_index in 0..entropy_bits {
        let word_index = bit_index / 11;
        let within_word = bit_index % 11;
        let bit = (indices[word_index] >> (10 - within_word)) & 1;
        entropy[bit_index / 8] |= (bit as u8) << (7 - (bit_index % 8));
    }
    let digest = Sha256::digest(&entropy);
    (0..checksum_bits).all(|offset| {
        let mnemonic_bit_index = entropy_bits + offset;
        let word_index = mnemonic_bit_index / 11;
        let within_word = mnemonic_bit_index % 11;
        let actual = (indices[word_index] >> (10 - within_word)) & 1;
        let expected = (digest[offset / 8] >> (7 - (offset % 8))) & 1;
        actual as u8 == expected
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_near_private_keys_case_insensitively_at_valid_lengths() {
        for prefix in ["ed25519:", "ED25519:", "Ed25519:"] {
            for encoded_chars in [64, 88] {
                assert_eq!(
                    validate_outbound_text(&format!("{prefix}{}", "A".repeat(encoded_chars))),
                    Err(OutboundTextValidationError::SensitiveText)
                );
            }
        }
        for prefix in ["secp256k1:", "SECP256K1:", "Secp256K1:"] {
            for encoded_chars in [32, 44] {
                assert_eq!(
                    validate_outbound_text(&format!("{prefix}{}", "A".repeat(encoded_chars))),
                    Err(OutboundTextValidationError::SensitiveText)
                );
            }
        }
    }

    #[test]
    fn near_private_key_length_boundaries_do_not_overmatch() {
        for value in [
            format!("ed25519:{}", "A".repeat(44)),
            format!("ed25519:{}", "A".repeat(63)),
            format!("ed25519:{}", "A".repeat(89)),
            format!("secp256k1:{}", "A".repeat(31)),
            format!("secp256k1:{}", "A".repeat(45)),
            format!("secp256k1:{}", "A".repeat(64)),
        ] {
            assert!(!contains_near_private_key(&value));
        }
    }

    #[test]
    fn rejects_valid_mnemonics_and_recovery_cues_without_overmatching_prose() {
        assert_eq!(
            validate_outbound_text(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about"
            ),
            Err(OutboundTextValidationError::SensitiveText)
        );
        assert_eq!(
            validate_outbound_text("Never paste a recovery phrase into exported text."),
            Err(OutboundTextValidationError::SensitiveText)
        );
        assert_eq!(
            validate_outbound_text(
                "Review the output carefully and publish only the evidence that supports the result."
            ),
            Ok(())
        );
        assert_eq!(
            validate_outbound_text(
                "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon"
            ),
            Ok(())
        );
    }

    #[test]
    fn rejects_credentials_and_private_identity() {
        let access_key = ["AK", "IAIOSFODNN7EXAMPLE"].concat();
        assert_eq!(
            validate_outbound_text(&format!("Use token {access_key} to authenticate.")),
            Err(OutboundTextValidationError::SensitiveText)
        );
        assert_eq!(
            validate_outbound_text("Send the result to private.person@example.com"),
            Err(OutboundTextValidationError::SensitiveText)
        );
    }
}
