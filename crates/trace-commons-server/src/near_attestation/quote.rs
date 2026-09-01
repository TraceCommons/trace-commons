// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Verification of the Intel TDX quote carried inside a NEAR AI attestation
//! report.
//!
//! [`super::AttestationReport::quote_binds_nonce`] proves that a nonce we
//! chose is present inside the quote. On its own that is not worth much: a
//! quote is just bytes, and anyone able to serve us a report can serve us a
//! fabricated quote with our nonce written into it. This module is what makes
//! the nonce binding mean something -- it checks the quote against Intel's
//! DCAP collateral, so the report data we read out has been signed by a
//! Quoting Enclave whose attestation key chains to Intel's root, on a
//! platform whose TCB level Intel vouches for.
//!
//! The registers exposed on [`VerifiedQuote`] are read out of the *verified*
//! quote structure. They are deliberately not copied from the report's
//! `info.tcb_info` JSON, which is unsigned and is the server's own claim
//! about itself; pinning against that would verify nothing. Measurement
//! pinning (Task 3) must consume [`VerifiedQuote`], never
//! [`super::Measurements`].
//!
//! `compose_hash`, `os_image_hash` and `mr_aggregated` are deliberately
//! absent here. They exist only in the unsigned JSON and are not recoverable
//! from the quote without reproducing dstack's RTMR extension derivation,
//! which this module does not do.

use dcap_qvl::QuoteCollateralV3;
use sha2::{Digest, Sha256};

/// Intel DCAP collateral (TCB info, QE identity, CRLs, PCK chain) for the
/// platform a quote came from.
///
/// This is `dcap_qvl`'s own type, re-exported under a local name so callers
/// do not have to name the dependency. Obtain it with [`parse_collateral`].
pub type Collateral = QuoteCollateralV3;

/// Why a quote was refused.
///
/// Variants deliberately carry no text from the underlying library. `dcap_qvl`
/// error chains can quote collateral endpoint URLs, and this crate's logs and
/// audit rows are hash-only. `detail_hash` is a truncated SHA-256 of the
/// underlying message: two operators seeing the same hash are looking at the
/// same failure, without the message itself ever reaching a log line.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum QuoteVerifyError {
    /// The collateral JSON did not deserialize.
    #[error("attestation collateral did not parse (detail {detail_hash})")]
    CollateralMalformed { detail_hash: String },
    /// The quote did not parse, or failed signature/TCB verification against
    /// the supplied collateral at the supplied time.
    #[error("TDX quote failed verification against Intel collateral (detail {detail_hash})")]
    VerificationFailed { detail_hash: String },
    /// The quote verified, but carries an SGX enclave report rather than a
    /// TDX report, so it has no MRTD or RTMRs.
    #[error("verified quote is not a TDX report")]
    NotTdx,
}

/// A truncated digest of an error message, safe to log.
fn detail_hash(message: &str) -> String {
    let digest = Sha256::digest(message.as_bytes());
    hex::encode(&digest[..8])
}

/// A TDX quote that verified against Intel collateral, and the values read
/// out of it.
///
/// Every field here comes from the verified quote structure. Nothing on this
/// struct is copied from the attestation report's JSON.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedQuote {
    /// The 64-byte TDX report data. NEAR AI puts the signing address in the
    /// first 20 bytes and the requested nonce in bytes 32..64.
    pub report_data: Vec<u8>,
    /// MRTD -- the measurement of the TD's initial memory image, hex-encoded.
    pub mrtd: String,
    /// RTMR0..RTMR3, hex-encoded, in index order.
    pub rtmr: [String; 4],
    /// Intel's TCB verdict for the platform, e.g. `UpToDate`.
    pub tcb_status: String,
    /// Intel security advisory IDs attached to that verdict, if any.
    pub advisory_ids: Vec<String>,
}

/// Parse Intel DCAP collateral from its JSON serialization.
pub fn parse_collateral(json: &str) -> Result<Collateral, QuoteVerifyError> {
    serde_json::from_str(json).map_err(|e| QuoteVerifyError::CollateralMalformed {
        detail_hash: detail_hash(&e.to_string()),
    })
}

/// Verify a raw TDX quote against Intel collateral, as of `now_unix`.
///
/// `now_unix` is the only clock this verification consults -- `dcap_qvl` makes
/// no wall-clock call of its own. Collateral issue dates and `nextUpdate`
/// deadlines, and certificate and CRL validity windows, are all evaluated
/// against it. **Production callers must pass real wall-clock time and supply
/// freshly fetched collateral.** Passing a pinned time is correct only in
/// tests, where it is what keeps a checked-in collateral fixture from turning
/// into a test that fails on a date rather than on a code change.
pub fn verify_quote(
    quote: &[u8],
    collateral: &Collateral,
    now_unix: u64,
) -> Result<VerifiedQuote, QuoteVerifyError> {
    let verified = dcap_qvl::verify::verify(quote, collateral, now_unix).map_err(|e| {
        QuoteVerifyError::VerificationFailed {
            detail_hash: detail_hash(&format!("{e:#}")),
        }
    })?;

    let td = verified.report.as_td10().ok_or(QuoteVerifyError::NotTdx)?;

    Ok(VerifiedQuote {
        report_data: td.report_data.to_vec(),
        mrtd: hex::encode(td.mr_td),
        rtmr: [
            hex::encode(td.rt_mr0),
            hex::encode(td.rt_mr1),
            hex::encode(td.rt_mr2),
            hex::encode(td.rt_mr3),
        ],
        tcb_status: verified.status,
        advisory_ids: verified.advisory_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::near_attestation::AttestationReport;

    const FIXTURE: &str = include_str!("../../tests/fixtures/near_ai_attestation_report.json");
    const COLLATERAL: &str =
        include_str!("../../tests/fixtures/near_ai_attestation_collateral.json");

    /// 2026-09-01T12:00:00Z. The fixture report and its collateral were both
    /// captured on 2026-09-01; the collateral's `nextUpdate` is
    /// 2026-09-30T23:45:01Z. `verify_quote` consults no clock but this one,
    /// so pinning it here means these tests fail on a code change and never
    /// on a calendar date. `collateral_expiry_is_measured_against_the_passed
    /// _clock` below is what keeps that claim honest.
    const FIXTURE_CAPTURED_AT: u64 = 1_788_264_000;

    /// Byte offset of `report_data` inside a v4 TDX quote: 48-byte quote
    /// header, then TDReport10's 520 bytes of SVNs and measurements.
    const REPORT_DATA_OFFSET: usize = 568;

    fn fixture_collateral() -> Collateral {
        parse_collateral(COLLATERAL).expect("collateral fixture parses")
    }

    fn fixture_nonce() -> String {
        let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        v["_fixture_nonce"].as_str().unwrap().to_string()
    }

    fn fixture_quote() -> Vec<u8> {
        AttestationReport::from_json(FIXTURE)
            .unwrap()
            .quote_bytes()
            .unwrap()
    }

    #[test]
    fn a_real_quote_verifies_and_exposes_its_report_data() {
        let v = verify_quote(&fixture_quote(), &fixture_collateral(), FIXTURE_CAPTURED_AT)
            .expect("real quote verifies");
        // report_data[32..64] is the nonce, per NEAR AI's verifier README.
        assert_eq!(hex::encode(&v.report_data[32..64]), fixture_nonce());
        assert_eq!(v.tcb_status, "UpToDate");
        assert!(v.advisory_ids.is_empty());
    }

    #[test]
    fn a_tampered_quote_does_not_verify() {
        // The single most important test here. If a mutated quote still
        // verifies, this module is decoration.
        //
        // The mutated byte is inside report_data -- the field this whole
        // module exists to be able to trust -- and not, as the task brief
        // originally proposed, the quote's last byte. See
        // trailing_padding_is_not_covered_by_the_signature for why that
        // distinction is load-bearing.
        let mut q = fixture_quote();
        q[REPORT_DATA_OFFSET + 40] ^= 0xff;
        let err = verify_quote(&q, &fixture_collateral(), FIXTURE_CAPTURED_AT)
            .expect_err("a quote with a flipped report_data bit must not verify");
        assert!(matches!(err, QuoteVerifyError::VerificationFailed { .. }));
    }

    #[test]
    fn tampering_with_a_measurement_register_does_not_verify() {
        // Task 3 pins MRTD and the RTMRs. Pinning is only worth doing if the
        // signature actually covers them, so prove it does.
        let mut q = fixture_quote();
        q[184] ^= 0x01; // first byte of MRTD
        assert!(verify_quote(&q, &fixture_collateral(), FIXTURE_CAPTURED_AT).is_err());
    }

    #[test]
    fn trailing_padding_is_not_covered_by_the_signature() {
        // Documents a real and slightly alarming property, so that nobody
        // later "simplifies" the tamper test above into flipping the last
        // byte and gets a green suite that proves nothing. A v4 quote ends
        // in zero padding that no signature covers: flipping it changes
        // nothing the verifier examines, and the quote still verifies.
        let mut q = fixture_quote();
        let last = q.len() - 1;
        assert_eq!(q[last], 0, "fixture quote ends in padding");
        q[last] ^= 0xff;
        let v = verify_quote(&q, &fixture_collateral(), FIXTURE_CAPTURED_AT)
            .expect("a flipped trailing padding byte is not a detectable tamper");
        // The values that matter are unchanged, which is why this is
        // tolerable rather than a finding.
        assert_eq!(hex::encode(&v.report_data[32..64]), fixture_nonce());
    }

    #[test]
    fn a_truncated_quote_does_not_verify() {
        let q = fixture_quote();
        assert!(
            verify_quote(
                &q[..q.len() / 2],
                &fixture_collateral(),
                FIXTURE_CAPTURED_AT
            )
            .is_err()
        );
    }

    #[test]
    fn the_measurements_come_out_of_the_verified_quote() {
        // These are the exact values Task 3 will pin. They are asserted here
        // as literals so that a change in how they are read -- for instance a
        // well-meaning refactor that starts sourcing them from the report's
        // unsigned info.tcb_info JSON -- shows up as a test failure rather
        // than as silently unverified pinning.
        let v = verify_quote(&fixture_quote(), &fixture_collateral(), FIXTURE_CAPTURED_AT).unwrap();
        assert_eq!(
            v.mrtd,
            "b24d3b24e9e3c16012376b52362ca09856c4adecb709d5fac33addf1c47e193da075b125b6c364115771390a5461e217"
        );
        assert_eq!(
            v.rtmr[0],
            "bc122d143ab768565ba5c3774ff5f03a63c89a4df7c1f5ea38d3bd173409d14f8cbdcc36d40e703cccb996a9d9687590"
        );
        assert_eq!(
            v.rtmr[3],
            "8f993f8b7a99d5e4ea49a3413a0d6311efa6a61be3ec6cae1d13b353dd1835544084cba4b4e767c17f5c513da1857de8"
        );
    }

    #[test]
    fn collateral_expiry_is_measured_against_the_passed_clock() {
        // The reason a checked-in collateral fixture is not a time bomb. The
        // fixture's tcbInfo covers 2026-08-31T23:45:01Z .. 2026-09-30T23:45:01Z
        // and dcap_qvl consults no clock but the one we pass, so both edges
        // are reachable from a test on any future date.
        let q = fixture_quote();
        let c = fixture_collateral();
        // A day before the collateral was issued.
        assert!(verify_quote(&q, &c, 1_785_542_400).is_err());
        // A day after it expired.
        assert!(verify_quote(&q, &c, 1_790_899_200).is_err());
        // And the pinned time in between still works, so the two failures
        // above are about the clock and not about the fixture being broken.
        assert!(verify_quote(&q, &c, FIXTURE_CAPTURED_AT).is_ok());
    }

    #[test]
    fn malformed_collateral_is_a_named_error() {
        let err = parse_collateral("{").expect_err("truncated JSON must not parse");
        assert!(matches!(err, QuoteVerifyError::CollateralMalformed { .. }));
        // The error's rendering must not carry the library's own message.
        assert!(!err.to_string().contains("EOF"));
    }
}
