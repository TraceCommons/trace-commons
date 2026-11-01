//! The report-data layouts Trace Commons enclaves bind into their TDX quotes,
//! and the client-side rebuild that checks them.
//!
//! Two layouts, one per service:
//!
//! ```text
//! witness:   | "tcwitns1" (8) | signing address (20)   | nonce (32) | zeros (4) |
//! judgement: | "tcjudge1" (8) | boot binding[0..24] (24) | nonce (32)            |
//!
//! boot binding = SHA-256("trace_commons.insights_judgement.boot_keys.v1\n"
//!                        || seal_pk (33) || signing_address (20))
//! ```
//!
//! The judgement service binds two per-boot public keys, a sealing key and a
//! response-signing address, so it hashes them into the 24 bytes between the
//! tag and the nonce rather than laying out one address. That is a different
//! rebuild, not just a different tag, which is why the layout -- with its
//! inputs -- is the parameter here and the tag follows from it.
//!
//! The tag is domain separation between the services: a redaction-witness
//! quote must never read as an Insights judgement-service quote, or the
//! reverse. A caller names the layout every time; there is no default.
//!
//! This lives in the permissive attestation crate so that the contributor
//! client and the hosted ingest drill can both verify a service's quote
//! without either depending on the other. The AGPL services that *produce*
//! these bytes may import it (permissive code flows into the AGPL crates);
//! the witness producer (`witness_service::enclave` in `trace-commons-server`)
//! keeps its own copy and pins it to the same literals in its own tests.

use sha2::{Digest, Sha256};

/// TDX report data is 64 bytes.
pub const REPORT_DATA_LEN: usize = 64;
/// Length of the domain tag at the front of the report data.
pub const DOMAIN_LEN: usize = 8;
/// Witness layout: offset of the 20-byte signing address.
pub const ADDRESS_AT: usize = 8;
/// Witness layout: offset of the client nonce.
pub const NONCE_AT: usize = 28;
/// The client nonce length. 32 bytes, fresh per verification.
pub const NONCE_LEN: usize = 32;

/// Judgement layout: offset of the truncated boot binding.
pub const JUDGEMENT_BINDING_AT: usize = 8;
/// Judgement layout: how many bytes of the boot binding are bound.
pub const JUDGEMENT_BINDING_LEN: usize = 24;
/// Judgement layout: offset of the client nonce, NEAR AI's own convention.
pub const JUDGEMENT_NONCE_AT: usize = 32;
/// A SEC1-compressed secp256k1 public key, the judgement sealing key.
pub const SEAL_PUBLIC_KEY_LEN: usize = 33;
/// The prefix hashed in front of the judgement service's per-boot keys.
pub const JUDGEMENT_BOOT_KEYS_LABEL: &[u8] = b"trace_commons.insights_judgement.boot_keys.v1\n";

/// The redaction witness's domain tag.
pub const WITNESS_QUOTE_DOMAIN: &[u8; DOMAIN_LEN] = b"tcwitns1";
/// The Insights judgement service's domain tag.
pub const JUDGEMENT_QUOTE_DOMAIN: &[u8; DOMAIN_LEN] = b"tcjudge1";

/// Which service's report data a quote must carry, with that layout's inputs.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ReportDataLayout<'a> {
    /// The redaction witness: `tcwitns1 | address | nonce | zeros`.
    Witness {
        /// The pinned signing address.
        signing_address: &'a [u8; 20],
    },
    /// The Insights judgement service: `tcjudge1 | boot binding | nonce`.
    /// Both keys are the per-boot public values the attestation route
    /// returned beside the quote.
    Judgement {
        /// The per-boot sealing key, SEC1 compressed.
        seal_public_key: &'a [u8; SEAL_PUBLIC_KEY_LEN],
        /// The per-boot response-signing address.
        signing_address: &'a [u8; 20],
    },
}

impl ReportDataLayout<'_> {
    fn domain(&self) -> &'static [u8; DOMAIN_LEN] {
        match self {
            Self::Witness { .. } => WITNESS_QUOTE_DOMAIN,
            Self::Judgement { .. } => JUDGEMENT_QUOTE_DOMAIN,
        }
    }

    fn nonce_at(&self) -> usize {
        match self {
            Self::Witness { .. } => NONCE_AT,
            Self::Judgement { .. } => JUDGEMENT_NONCE_AT,
        }
    }
}

impl std::fmt::Debug for ReportDataLayout<'_> {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The layout's name only; the keys are not rendered.
        formatter.write_str(match self {
            Self::Witness { .. } => "ReportDataLayout::Witness",
            Self::Judgement { .. } => "ReportDataLayout::Judgement",
        })
    }
}

/// SHA-256 over the judgement service's two per-boot public values. The
/// report data carries the first [`JUDGEMENT_BINDING_LEN`] bytes.
pub fn judgement_boot_binding(
    seal_public_key: &[u8; SEAL_PUBLIC_KEY_LEN],
    signing_address: &[u8; 20],
) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(JUDGEMENT_BOOT_KEYS_LABEL);
    hasher.update(seal_public_key);
    hasher.update(signing_address);
    hasher.finalize().into()
}

/// Why a quote's report data is not the one this exchange expects.
///
/// Two variants, because they mean different things to a contributor: a
/// quote that does not carry this exchange's nonce is a replay, and one that
/// does but names another service, another address, other keys or stray
/// bytes is somebody else's machine. `Debug` delegates to `Display`, so
/// neither form renders any bytes of the report data.
#[derive(Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReportDataError {
    /// The report data does not carry this exchange's nonce.
    #[error("the quote is not bound to this exchange's nonce")]
    Replayed,
    /// Another service's tag, or the nonce matches but the signer, keys or
    /// padding do not.
    #[error("the quote does not name the expected signer under this domain")]
    SignerUnexpected,
}

impl std::fmt::Debug for ReportDataError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// Lay a layout's inputs and a nonce into the 64 bytes TDX signs.
pub fn expected_report_data(
    layout: ReportDataLayout<'_>,
    nonce: &[u8; NONCE_LEN],
) -> [u8; REPORT_DATA_LEN] {
    let mut expected = [0u8; REPORT_DATA_LEN];
    expected[..DOMAIN_LEN].copy_from_slice(layout.domain());
    match layout {
        ReportDataLayout::Witness { signing_address } => {
            expected[ADDRESS_AT..ADDRESS_AT + signing_address.len()]
                .copy_from_slice(signing_address);
        }
        ReportDataLayout::Judgement {
            seal_public_key,
            signing_address,
        } => {
            let binding = judgement_boot_binding(seal_public_key, signing_address);
            expected[JUDGEMENT_BINDING_AT..JUDGEMENT_BINDING_AT + JUDGEMENT_BINDING_LEN]
                .copy_from_slice(&binding[..JUDGEMENT_BINDING_LEN]);
        }
    }
    let nonce_at = layout.nonce_at();
    expected[nonce_at..nonce_at + NONCE_LEN].copy_from_slice(nonce);
    expected
}

/// Check a quote's report data against a layout and a nonce.
///
/// Reconstructed whole and compared in one shot, not field by field: a
/// field-by-field check has to decide what to do about the bytes it did not
/// name, and the honest answer -- they must match the layout, zeros included
/// -- is one a reconstruction gets for free. A buffer of any length other
/// than 64 fails the comparison.
///
/// On a mismatch the error is decided in this order:
///
/// 1. Another service's domain tag is somebody else's machine
///    ([`ReportDataError::SignerUnexpected`]). The two layouts put the nonce
///    at different offsets, so reading the nonce first would call every
///    cross-service quote a replay, which invites a retry that can never
///    succeed.
/// 2. A quote that does not carry this exchange's nonce at this layout's
///    offset is a replay, even for the right machine.
/// 3. Anything else -- address, keys, padding, length -- is somebody else's
///    machine.
pub fn check_report_data(
    report_data: &[u8],
    layout: ReportDataLayout<'_>,
    nonce: &[u8; NONCE_LEN],
) -> Result<(), ReportDataError> {
    let expected = expected_report_data(layout, nonce);
    if report_data == expected.as_slice() {
        return Ok(());
    }
    if report_data.get(..DOMAIN_LEN) != Some(layout.domain().as_slice()) {
        return Err(ReportDataError::SignerUnexpected);
    }
    let nonce_at = layout.nonce_at();
    let carries_our_nonce =
        report_data.get(nonce_at..nonce_at + NONCE_LEN) == Some(nonce.as_slice());
    Err(if carries_our_nonce {
        ReportDataError::SignerUnexpected
    } else {
        ReportDataError::Replayed
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const ADDRESS: [u8; 20] = [0x11; 20];
    const OTHER_ADDRESS: [u8; 20] = [0x22; 20];
    const NONCE: [u8; NONCE_LEN] = [0x5a; NONCE_LEN];
    const OTHER_NONCE: [u8; NONCE_LEN] = [0xa5; NONCE_LEN];
    const SEAL_PK: [u8; SEAL_PUBLIC_KEY_LEN] = [0x02; SEAL_PUBLIC_KEY_LEN];
    const OTHER_SEAL_PK: [u8; SEAL_PUBLIC_KEY_LEN] = [0x03; SEAL_PUBLIC_KEY_LEN];

    fn witness(address: &[u8; 20]) -> ReportDataLayout<'_> {
        ReportDataLayout::Witness {
            signing_address: address,
        }
    }

    fn judgement<'a>(
        seal_public_key: &'a [u8; SEAL_PUBLIC_KEY_LEN],
        signing_address: &'a [u8; 20],
    ) -> ReportDataLayout<'a> {
        ReportDataLayout::Judgement {
            seal_public_key,
            signing_address,
        }
    }

    #[test]
    fn the_layout_is_pinned_to_literals() {
        assert_eq!(WITNESS_QUOTE_DOMAIN, b"tcwitns1");
        assert_eq!(JUDGEMENT_QUOTE_DOMAIN, b"tcjudge1");
        assert_eq!(DOMAIN_LEN, 8);
        assert_eq!(ADDRESS_AT, 8);
        assert_eq!(NONCE_AT, 28);
        assert_eq!(NONCE_LEN, 32);
        assert_eq!(REPORT_DATA_LEN, 64);
        assert_eq!(ADDRESS_AT, DOMAIN_LEN);
        assert_eq!(ADDRESS_AT + 20, NONCE_AT);
        // The judgement layout: tag, 24 bytes of the boot binding, nonce, and
        // nothing unaccounted for.
        assert_eq!(JUDGEMENT_BINDING_AT, 8);
        assert_eq!(JUDGEMENT_BINDING_LEN, 24);
        assert_eq!(JUDGEMENT_NONCE_AT, 32);
        assert_eq!(
            JUDGEMENT_BINDING_AT + JUDGEMENT_BINDING_LEN,
            JUDGEMENT_NONCE_AT
        );
        assert_eq!(JUDGEMENT_NONCE_AT + NONCE_LEN, REPORT_DATA_LEN);
        assert_eq!(SEAL_PUBLIC_KEY_LEN, 33);
        assert_eq!(
            JUDGEMENT_BOOT_KEYS_LABEL,
            b"trace_commons.insights_judgement.boot_keys.v1\n"
        );
    }

    #[test]
    fn the_witness_rebuild_lays_out_domain_address_nonce_and_zeros() {
        let built = expected_report_data(witness(&ADDRESS), &NONCE);
        assert_eq!(&built[..8], b"tcwitns1");
        assert_eq!(&built[8..28], &ADDRESS);
        assert_eq!(&built[28..60], &NONCE);
        assert_eq!(&built[60..], &[0u8; 4]);
    }

    /// Computed outside this crate, by Python's `hashlib` over the layout in
    /// design section 3 (seal_pk is the compressed public key of the scalar
    /// 0x11..11, the address is 0x33 x 20, the nonce 0x5a x 32). A vector
    /// produced by the code under test would only prove it agrees with itself.
    const FROZEN_SEAL_PK: &str =
        "034f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa";
    const FROZEN_BOOT_BINDING: &str =
        "bc11662bdc3a36fe80acbee938c5d72b398b7fedcdca3b724949672e32e604a7";
    const FROZEN_JUDGEMENT_REPORT_DATA: &str = "74636a7564676531bc11662bdc3a36fe80acbee938c5d72b398b7fedcdca3b725a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a5a";

    #[test]
    fn the_judgement_rebuild_matches_a_frozen_vector() {
        let seal_pk: [u8; SEAL_PUBLIC_KEY_LEN] =
            hex::decode(FROZEN_SEAL_PK).unwrap().try_into().unwrap();
        let address = [0x33u8; 20];
        assert_eq!(
            hex::encode(judgement_boot_binding(&seal_pk, &address)),
            FROZEN_BOOT_BINDING
        );
        let built = expected_report_data(judgement(&seal_pk, &address), &NONCE);
        assert_eq!(hex::encode(built), FROZEN_JUDGEMENT_REPORT_DATA);
        assert_eq!(&built[..8], b"tcjudge1");
        assert_eq!(&built[32..], &NONCE);
    }

    #[test]
    fn a_quote_bound_under_each_layout_passes_under_that_layout() {
        for layout in [witness(&ADDRESS), judgement(&SEAL_PK, &ADDRESS)] {
            let report = expected_report_data(layout, &NONCE);
            check_report_data(&report, layout, &NONCE)
                .expect("a quote bound under this layout is accepted under it");
        }
    }

    #[test]
    fn a_witness_quote_is_refused_as_a_judgement_quote_and_the_reverse() {
        // The tag is read first, so a quote from the other service is named
        // as somebody else's machine, not as a replay to retry.
        let witness_quote = expected_report_data(witness(&ADDRESS), &NONCE);
        assert_eq!(
            check_report_data(&witness_quote, judgement(&SEAL_PK, &ADDRESS), &NONCE),
            Err(ReportDataError::SignerUnexpected)
        );
        let judgement_quote = expected_report_data(judgement(&SEAL_PK, &ADDRESS), &NONCE);
        assert_eq!(
            check_report_data(&judgement_quote, witness(&ADDRESS), &NONCE),
            Err(ReportDataError::SignerUnexpected)
        );
    }

    #[test]
    fn another_nonce_is_a_replay_and_another_address_is_another_signer() {
        for (ours, theirs) in [
            (witness(&ADDRESS), witness(&OTHER_ADDRESS)),
            (
                judgement(&SEAL_PK, &ADDRESS),
                judgement(&SEAL_PK, &OTHER_ADDRESS),
            ),
        ] {
            let replayed = expected_report_data(ours, &OTHER_NONCE);
            assert_eq!(
                check_report_data(&replayed, ours, &NONCE),
                Err(ReportDataError::Replayed)
            );
            // A replayed quote for another machine is still a replay: the
            // nonce is read before the signer.
            let both = expected_report_data(theirs, &OTHER_NONCE);
            assert_eq!(
                check_report_data(&both, ours, &NONCE),
                Err(ReportDataError::Replayed)
            );
            let stranger = expected_report_data(theirs, &NONCE);
            assert_eq!(
                check_report_data(&stranger, ours, &NONCE),
                Err(ReportDataError::SignerUnexpected)
            );
        }
    }

    #[test]
    fn a_judgement_quote_over_another_seal_key_is_another_signer() {
        // Same address, same nonce, another sealing key: a substituted key is
        // exactly what the boot binding exists to catch.
        let other = expected_report_data(judgement(&OTHER_SEAL_PK, &ADDRESS), &NONCE);
        assert_eq!(
            check_report_data(&other, judgement(&SEAL_PK, &ADDRESS), &NONCE),
            Err(ReportDataError::SignerUnexpected)
        );
    }

    #[test]
    fn trailing_bytes_and_short_buffers_are_refused() {
        let mut padded = expected_report_data(witness(&ADDRESS), &NONCE);
        padded[60] = 1;
        assert_eq!(
            check_report_data(&padded, witness(&ADDRESS), &NONCE),
            Err(ReportDataError::SignerUnexpected)
        );
        for layout in [witness(&ADDRESS), judgement(&SEAL_PK, &ADDRESS)] {
            let full = expected_report_data(layout, &NONCE);
            assert_eq!(
                check_report_data(&full[..32], layout, &NONCE),
                Err(ReportDataError::Replayed)
            );
            // Longer than 64 bytes is not this layout either.
            let mut long = full.to_vec();
            long.push(0);
            assert_eq!(
                check_report_data(&long, layout, &NONCE),
                Err(ReportDataError::SignerUnexpected)
            );
            assert_eq!(
                check_report_data(&[], layout, &NONCE),
                Err(ReportDataError::SignerUnexpected)
            );
        }
    }

    #[test]
    fn errors_render_labels_only() {
        for err in [ReportDataError::Replayed, ReportDataError::SignerUnexpected] {
            assert_eq!(format!("{err:?}"), format!("{err}"));
        }
    }
}
