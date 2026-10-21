//! The report-data layout every Trace Commons enclave binds into its TDX
//! quote, and the client-side rebuild that checks it.
//!
//! ```text
//! | domain tag (8) | signing address (20) | client nonce (32) | zeros (4) |
//! ```
//!
//! The domain tag is domain separation between services that share this
//! layout: a redaction-witness quote (`tcwitns1`) must never read as an
//! Insights judgement-service quote (`tcjudge1`), or the reverse. The tag is
//! therefore a parameter of every check here, never a default.
//!
//! This lives in the permissive attestation crate so that the contributor
//! client and the hosted ingest drill can both verify a service's quote
//! without either depending on the other. The AGPL services that *produce*
//! these bytes keep their own copy of the layout (`witness_service::enclave`
//! in `trace-commons-server`) and pin it to the same literals in their own
//! tests; a permissive crate cannot depend on them, and they gain nothing
//! from a shared constant that a literal test does not already hold.

/// TDX report data is 64 bytes.
pub const REPORT_DATA_LEN: usize = 64;
/// Length of the domain tag at the front of the report data.
pub const DOMAIN_LEN: usize = 8;
/// Offset of the 20-byte signing address.
pub const ADDRESS_AT: usize = 8;
/// Offset of the client nonce.
pub const NONCE_AT: usize = 28;
/// The client nonce length. 32 bytes, fresh per verification.
pub const NONCE_LEN: usize = 32;

/// The redaction witness's domain tag.
pub const WITNESS_QUOTE_DOMAIN: &[u8; DOMAIN_LEN] = b"tcwitns1";
/// The Insights judgement service's domain tag.
pub const JUDGEMENT_QUOTE_DOMAIN: &[u8; DOMAIN_LEN] = b"tcjudge1";

/// Why a quote's report data is not the one this exchange expects.
///
/// Two variants, because they mean different things to a contributor: a
/// quote that does not carry this exchange's nonce is a replay, and one that
/// does but names another address (or another domain, or stray bytes) is
/// somebody else's machine. `Debug` delegates to `Display`, so neither form
/// renders any bytes of the report data.
#[derive(Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReportDataError {
    /// The report data does not carry this exchange's nonce.
    #[error("the quote is not bound to this exchange's nonce")]
    Replayed,
    /// The nonce matches, but the domain, address or padding does not.
    #[error("the quote does not name the expected signer under this domain")]
    SignerUnexpected,
}

impl std::fmt::Debug for ReportDataError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// Lay a domain tag, signing address and nonce into the 64 bytes TDX signs.
/// The trailing four bytes are zero.
pub fn expected_report_data(
    domain: &[u8; DOMAIN_LEN],
    address: &[u8; 20],
    nonce: &[u8; NONCE_LEN],
) -> [u8; REPORT_DATA_LEN] {
    let mut expected = [0u8; REPORT_DATA_LEN];
    expected[..DOMAIN_LEN].copy_from_slice(domain);
    expected[ADDRESS_AT..ADDRESS_AT + address.len()].copy_from_slice(address);
    expected[NONCE_AT..NONCE_AT + NONCE_LEN].copy_from_slice(nonce);
    expected
}

/// Check a quote's report data against the expected domain, address and
/// nonce.
///
/// Reconstructed whole and compared in one shot, not field by field: a
/// field-by-field check has to decide what to do about the bytes it did not
/// name, and the honest answer -- they must be zero -- is one a
/// reconstruction gets for free. A buffer of any length other than 64 fails
/// the comparison.
///
/// On a mismatch the nonce decides the error, and is read first because a
/// replayed quote for the *right* machine is still a replay.
pub fn check_report_data(
    report_data: &[u8],
    domain: &[u8; DOMAIN_LEN],
    address: &[u8; 20],
    nonce: &[u8; NONCE_LEN],
) -> Result<(), ReportDataError> {
    let expected = expected_report_data(domain, address, nonce);
    if report_data == expected.as_slice() {
        return Ok(());
    }
    let carries_our_nonce =
        report_data.get(NONCE_AT..NONCE_AT + NONCE_LEN) == Some(nonce.as_slice());
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
    }

    #[test]
    fn the_rebuild_lays_out_domain_address_nonce_and_zeros() {
        let built = expected_report_data(JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE);
        assert_eq!(&built[..8], b"tcjudge1");
        assert_eq!(&built[8..28], &ADDRESS);
        assert_eq!(&built[28..60], &NONCE);
        assert_eq!(&built[60..], &[0u8; 4]);
    }

    #[test]
    fn a_quote_bound_under_each_domain_passes_under_that_domain() {
        for domain in [WITNESS_QUOTE_DOMAIN, JUDGEMENT_QUOTE_DOMAIN] {
            let report = expected_report_data(domain, &ADDRESS, &NONCE);
            check_report_data(&report, domain, &ADDRESS, &NONCE)
                .expect("a quote bound under this domain is accepted under it");
        }
    }

    #[test]
    fn a_witness_quote_is_refused_as_a_judgement_quote_and_the_reverse() {
        let witness = expected_report_data(WITNESS_QUOTE_DOMAIN, &ADDRESS, &NONCE);
        assert_eq!(
            check_report_data(&witness, JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE),
            Err(ReportDataError::SignerUnexpected)
        );
        let judgement = expected_report_data(JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE);
        assert_eq!(
            check_report_data(&judgement, WITNESS_QUOTE_DOMAIN, &ADDRESS, &NONCE),
            Err(ReportDataError::SignerUnexpected)
        );
    }

    #[test]
    fn another_nonce_is_a_replay_and_another_address_is_another_signer() {
        let replayed = expected_report_data(JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &OTHER_NONCE);
        assert_eq!(
            check_report_data(&replayed, JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE),
            Err(ReportDataError::Replayed)
        );
        // A replayed quote for another machine is still a replay: the nonce
        // is read first.
        let both = expected_report_data(JUDGEMENT_QUOTE_DOMAIN, &OTHER_ADDRESS, &OTHER_NONCE);
        assert_eq!(
            check_report_data(&both, JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE),
            Err(ReportDataError::Replayed)
        );
        let stranger = expected_report_data(JUDGEMENT_QUOTE_DOMAIN, &OTHER_ADDRESS, &NONCE);
        assert_eq!(
            check_report_data(&stranger, JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE),
            Err(ReportDataError::SignerUnexpected)
        );
    }

    #[test]
    fn trailing_bytes_and_short_buffers_are_refused() {
        let mut padded = expected_report_data(JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE);
        padded[60] = 1;
        assert_eq!(
            check_report_data(&padded, JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE),
            Err(ReportDataError::SignerUnexpected)
        );
        let full = expected_report_data(JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE);
        assert_eq!(
            check_report_data(&full[..32], JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE),
            Err(ReportDataError::Replayed)
        );
        // Longer than 64 bytes is not this layout either.
        let mut long = full.to_vec();
        long.push(0);
        assert_eq!(
            check_report_data(&long, JUDGEMENT_QUOTE_DOMAIN, &ADDRESS, &NONCE),
            Err(ReportDataError::SignerUnexpected)
        );
    }

    #[test]
    fn errors_render_labels_only() {
        for err in [ReportDataError::Replayed, ReportDataError::SignerUnexpected] {
            assert_eq!(format!("{err:?}"), format!("{err}"));
        }
    }
}
