//! Verification of the Intel TDX quote carried inside a NEAR AI attestation
//! report.
//!
//! `AttestationReport::quote_binds_nonce`, in the hosted server, proves that
//! a nonce we chose is present inside the quote. On its own that is not worth
//! much: a quote is just bytes, and anyone able to serve us a report can
//! serve us a fabricated quote with our nonce written into it. This module is
//! what makes the nonce binding mean something -- it checks the quote against
//! Intel's DCAP collateral, so the report data we read out has been signed by
//! a Quoting Enclave whose attestation key chains to Intel's root, on a
//! platform whose TCB level Intel vouches for.
//!
//! The registers exposed on [`VerifiedQuote`] are read out of the *verified*
//! quote structure. They are deliberately not copied from the report's
//! `info.tcb_info` JSON, which is unsigned and is the server's own claim
//! about itself; pinning against that would verify nothing. Measurement
//! pinning (Task 3) must consume [`VerifiedQuote`], never the server's
//! `UnverifiedJsonMeasurements`.
//!
//! `MRCONFIGID` is exposed alongside MRTD and the RTMRs because it, not
//! they, is the stable identity of the software a dstack CVM was asked to
//! run -- see [`crate::measurements::MeasurementField::MrConfigId`].
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
/// error chains can quote collateral endpoint URLs, and the hosted server's
/// logs and audit rows are hash-only. `detail_hash` is a truncated SHA-256 of
/// the underlying message: two operators seeing the same hash are looking at the
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
    /// MRCONFIGID -- the TD owner's configuration commitment, hex-encoded.
    ///
    /// dstack writes a version tag followed by the compose hash here, so this
    /// is the register that identifies *what code the CVM was asked to run*
    /// and is stable across instances and VM resizes. See
    /// [`crate::measurements::MeasurementField::MrConfigId`] for why that
    /// makes it the register a deployment pins.
    pub mr_config_id: String,
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
        mr_config_id: hex::encode(td.mr_config_id),
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

    // The fixtures live in this crate, so a packaged copy of it alone can
    // run its own tests. The server crate's five call sites read the same
    // two files from here rather than holding a second copy that would
    // drift.
    const FIXTURE: &str = include_str!("../tests/fixtures/near_ai_attestation_report.json");
    const COLLATERAL: &str = include_str!("../tests/fixtures/near_ai_attestation_collateral.json");

    /// 2026-09-01T12:00:00Z. The fixture report and its collateral were both
    /// captured on 2026-09-01; the collateral's `nextUpdate` is
    /// 2026-09-30T23:45:01Z. `verify_quote` consults no clock but this one,
    /// so pinning it here means these tests fail on a code change and never
    /// on a calendar date. `collateral_expiry_is_measured_against_the_passed
    /// _clock` below is what keeps that claim honest.
    const FIXTURE_CAPTURED_AT: u64 = 1_788_264_000;

    /// MRTD as read out of the verified fixture quote.
    const VERIFIED_MRTD: &str = "b24d3b24e9e3c16012376b52362ca09856c4adecb709d5fac33addf1c47e193da075b125b6c364115771390a5461e217";

    /// MRCONFIGID as read out of the verified fixture quote.
    ///
    /// Its structure is not opaque: byte 0 is dstack's config-id version tag
    /// (`01`), bytes 1..33 are the compose hash, and the remaining 15 bytes
    /// are zero padding. `mr_config_id_commits_to_the_reports_compose_hash`
    /// below is what keeps that reading honest.
    const VERIFIED_MR_CONFIG_ID: &str = "019385918de0a73b861ae833d99fb5be6f7e1c8a50487a835df4f277497c206825000000000000000000000000000000";

    /// Byte offset of `report_data` inside a v4 TDX quote: 48-byte quote
    /// header, then TDReport10's 520 bytes of SVNs and measurements.
    const REPORT_DATA_OFFSET: usize = 568;

    /// Byte offset of MRCONFIGID inside a v4 TDX quote: the 48-byte header,
    /// then TDReport10 up to and including MRTD (which starts at 184).
    const MR_CONFIG_ID_OFFSET: usize = 232;

    fn fixture_collateral() -> Collateral {
        parse_collateral(COLLATERAL).expect("collateral fixture parses")
    }

    fn fixture_nonce() -> String {
        let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        v["_fixture_nonce"].as_str().unwrap().to_string()
    }

    /// The raw quote bytes out of the fixture report.
    ///
    /// The hosted server reaches these through
    /// `AttestationReport::quote_bytes`, which is exactly a hex decode of
    /// `intel_quote`. That type is AGPL and stays behind the boundary, so
    /// this reads the same field out of the same fixture directly rather
    /// than dragging the server crate in as a dev-dependency.
    fn fixture_quote() -> Vec<u8> {
        let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        hex::decode(v["intel_quote"].as_str().unwrap()).unwrap()
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
        // most_of_the_quote_is_not_covered_by_the_signature for why that
        // distinction is load-bearing.
        let mut q = fixture_quote();
        q[REPORT_DATA_OFFSET + 40] ^= 0xff;
        let err = verify_quote(&q, &fixture_collateral(), FIXTURE_CAPTURED_AT)
            .expect_err("a quote with a flipped report_data bit must not verify");
        assert!(matches!(err, QuoteVerifyError::VerificationFailed { .. }));
    }

    #[test]
    fn tampering_with_a_measurement_register_does_not_verify() {
        // Task 3 pins MRTD and the RTMRs and Task 4 adds MRCONFIGID. Pinning
        // is only worth doing if the signature actually covers what is
        // pinned, so prove it does -- and name the variant, because a
        // mutation that made the quote fail to *parse* would satisfy a bare
        // is_err() while proving nothing about the signature.
        for (label, offset) in [("MRTD", 184usize), ("MRCONFIGID", MR_CONFIG_ID_OFFSET)] {
            let mut q = fixture_quote();
            q[offset] ^= 0x01;
            let err = verify_quote(&q, &fixture_collateral(), FIXTURE_CAPTURED_AT)
                .expect_err("a flipped measurement bit must not verify");
            assert!(
                matches!(err, QuoteVerifyError::VerificationFailed { .. }),
                "{label}: expected a signature failure, got {err:?}"
            );
        }
    }

    #[test]
    fn most_of_the_quote_is_not_covered_by_the_signature() {
        // Locks a surprising and much larger property than "there is some
        // padding at the end". An exhaustive per-byte scan of this 5006-byte
        // fixture -- flip one bit, re-verify, record -- finds that 3753 of
        // those bytes (75%) can be mutated and the quote still verifies with
        // identical mrtd, rtmr and report_data:
        //
        //   byte  632          low byte of the signature-data length
        //   bytes 764..=765    qe_cert_data_type
        //   bytes 1252..=1253  low bytes of the certification-data length
        //   bytes 1258..=5005  the embedded PCK certificate chain (PEM)
        //
        // and arbitrary trailing bytes can be appended without bound.
        //
        // The cause is by design rather than a defect: dcap_qvl verifies the
        // PCK chain from the *collateral* we supply and ignores the copy the
        // quote carries, so that copy is decoration. The consequence is
        // sharp, and is why this test is written down instead of being left
        // as folklore: "these quote bytes are unmodified" is very nearly a
        // meaningless statement. Nothing downstream may hash raw quote bytes
        // as an identity, dedup or evidence key -- such a hash is trivially
        // malleable. Hash the VerifiedQuote fields instead.
        let base = fixture_quote();
        let collateral = fixture_collateral();
        let expect_nonce = fixture_nonce();

        let mut mutations: Vec<(&str, Vec<u8>)> = Vec::new();
        for (label, offset) in [
            ("signature-data length", 632usize),
            ("qe_cert_data_type", 764),
            ("certification-data length", 1252),
            ("PCK chain, first PEM byte", 1258),
            ("PCK chain, inside the Intel root CA", 4900),
        ] {
            let mut q = base.clone();
            q[offset] ^= 0x01;
            mutations.push((label, q));
        }
        let mut appended = base.clone();
        appended.extend(std::iter::repeat_n(0xab, 4096));
        mutations.push(("4096 appended bytes", appended));

        for (label, q) in mutations {
            assert_ne!(
                q, base,
                "{label}: the mutation must actually change the bytes"
            );
            let v = verify_quote(&q, &collateral, FIXTURE_CAPTURED_AT)
                .unwrap_or_else(|e| panic!("{label}: expected this NOT to be detectable, got {e}"));
            // The values that matter are untouched, which is what makes this
            // tolerable rather than a finding.
            assert_eq!(hex::encode(&v.report_data[32..64]), expect_nonce, "{label}");
            assert_eq!(v.mrtd, VERIFIED_MRTD, "{label}");
        }
    }

    #[test]
    fn a_truncated_quote_does_not_verify() {
        // Named, not is_err(): a half-quote that fails to *parse* and one
        // whose signature does not check out are the same bare assertion and
        // very different facts. `dcap_qvl` reports both through its own error
        // type, which this crate collapses into `VerificationFailed` -- so
        // that is what must be seen here, and never `NotTdx`, which would
        // mean the truncated bytes had verified as some other report shape.
        let q = fixture_quote();
        let err = verify_quote(
            &q[..q.len() / 2],
            &fixture_collateral(),
            FIXTURE_CAPTURED_AT,
        )
        .expect_err("half a quote must not verify");
        assert!(
            matches!(err, QuoteVerifyError::VerificationFailed { .. }),
            "{err:?}"
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
        assert_eq!(v.mrtd, VERIFIED_MRTD);
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
    fn a_verified_quote_carries_mr_config_id() {
        // The register the witness deployment pins. It is on the parsed TD10
        // report and was simply never copied out, so this asserts the literal
        // rather than a length: a length assertion would be satisfied by any
        // other 48-byte register on the same struct.
        let v = verify_quote(&fixture_quote(), &fixture_collateral(), FIXTURE_CAPTURED_AT).unwrap();
        assert_eq!(v.mr_config_id.len(), 96, "48 bytes, hex-encoded");
        assert_eq!(v.mr_config_id, VERIFIED_MR_CONFIG_ID);
        // And it is a register of its own, not a second copy of one already
        // exposed. Without this, `mr_config_id: hex::encode(td.mr_td)` would
        // satisfy everything above.
        assert_ne!(v.mr_config_id, v.mrtd);
        for (i, r) in v.rtmr.iter().enumerate() {
            assert_ne!(&v.mr_config_id, r, "mr_config_id must not be rtmr{i}");
        }
    }

    #[test]
    fn mr_config_id_commits_to_the_reports_compose_hash() {
        // Why MRCONFIGID is the stable identity of *what code runs*: dstack
        // builds it as a version tag followed by the compose hash, so two
        // instances of the same compose file share it while RTMR3 (per-boot
        // instance id) and RTMR0 (VM sizing) do not.
        //
        // The compose hash read here comes from the report's *unsigned* JSON
        // and is untrusted; the point of the assertion is precisely that the
        // signed register commits to it, which is what lets an operator go
        // from a compose file to a pin.
        let v = verify_quote(&fixture_quote(), &fixture_collateral(), FIXTURE_CAPTURED_AT).unwrap();
        let report: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        let compose_hash = report["info"]["compose_hash"].as_str().unwrap();
        assert_eq!(compose_hash.len(), 64, "a 32-byte hash, hex-encoded");
        assert_eq!(
            &v.mr_config_id[..2],
            "01",
            "dstack config-id version tag, {}",
            v.mr_config_id
        );
        assert_eq!(&v.mr_config_id[2..66], compose_hash);
        assert_eq!(
            &v.mr_config_id[66..],
            "0".repeat(30),
            "the remaining 15 bytes are zero padding in this (v1) layout"
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
        for (label, clock) in [
            // 2026-08-01, a month before the collateral was issued.
            ("before issue", 1_785_542_400u64),
            // 2026-10-02, a day after it expired.
            ("after expiry", 1_790_899_200),
        ] {
            let err = verify_quote(&q, &c, clock)
                .expect_err("collateral outside its validity window must not verify");
            // Named rather than bare, so a fixture that stopped parsing
            // altogether could not masquerade as a clock refusal.
            assert!(
                matches!(err, QuoteVerifyError::VerificationFailed { .. }),
                "{label}: {err:?}"
            );
        }
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
