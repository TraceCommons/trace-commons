// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR-AI-specific half of measurement pinning.
//!
//! The pinning itself -- [`MeasurementField`], [`ExpectedMeasurements`],
//! [`check_measurements`] and [`check_measurements_opt`] -- lives in the
//! permissive `trace-commons-attestation` crate, because a contributor must be
//! able to check a redaction witness's image before sending it raw bytes, and
//! that code cannot sit behind this crate's AGPL boundary. It is re-exported
//! here so `crate::near_attestation::measurements::*` keeps resolving.
//!
//! What stays here is what is about NEAR AI rather than about TDX: the names
//! of this deployment's control and environment variable, and the comparison
//! between a NEAR AI report's unsigned JSON self-description and what the
//! hardware actually signed. A witness will pin its own image under its own
//! control name and has no JSON envelope of this shape at all, so a shared
//! constant would be generic in spelling and wrong in every deployment.

pub use trace_commons_attestation::measurements::*;

use std::fmt;

use super::UnverifiedJsonMeasurements;
use super::quote::VerifiedQuote;

/// Environment variable holding the pinned measurement set.
pub const EXPECTED_MEASUREMENTS_ENV: &str = "TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS";

/// Missing-control name reported when nothing has been pinned.
pub const EXPECTED_MEASUREMENTS_CONTROL: &str = "near_ai_expected_measurements";

/// Load the pinned set from [`EXPECTED_MEASUREMENTS_ENV`].
///
/// `Ok(None)` means the variable is unset or empty, which is *not* an
/// acceptance -- see [`check_measurements_opt`], which must still be given
/// [`EXPECTED_MEASUREMENTS_CONTROL`] so the refusal names this deployment's
/// control.
pub fn expected_measurements_from_env()
-> Result<Option<ExpectedMeasurements>, ExpectedMeasurementsError> {
    ExpectedMeasurements::from_env_value(std::env::var(EXPECTED_MEASUREMENTS_ENV).ok().as_deref())
}

/// The value of a register as a NEAR AI report *claims* it in unsigned JSON.
fn read_claim(field: MeasurementField, claim: &UnverifiedJsonMeasurements) -> &str {
    match field {
        MeasurementField::Mrtd => &claim.mrtd,
        MeasurementField::Rtmr0 => &claim.rtmr0,
        MeasurementField::Rtmr1 => &claim.rtmr1,
        MeasurementField::Rtmr2 => &claim.rtmr2,
        MeasurementField::Rtmr3 => &claim.rtmr3,
    }
}

/// A register the report's unsigned JSON claims differently from the quote.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonClaimAnomaly {
    pub field: MeasurementField,
    pub claimed: String,
    pub verified: String,
}

impl fmt::Display for JsonClaimAnomaly {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: report claims {}, quote says {}",
            self.field, self.claimed, self.verified
        )
    }
}

/// Report where the endpoint's unsigned self-description disagrees with what
/// the hardware signed.
///
/// This is **reporting, never gating**. Nothing here may be used to admit a
/// quote: the trustworthy value is always the one from [`VerifiedQuote`], and
/// [`check_measurements`] already compares against it. What a non-empty result
/// means is that the endpoint is describing itself inaccurately in a way the
/// quote exposes -- worth surfacing to an operator on its own terms.
///
/// Only `mrtd` and `rtmr0..3` are comparable; `compose_hash`, `os_image_hash`
/// and `mr_aggregated` have no verified counterpart and are not examined.
pub fn json_claim_anomalies(
    claim: &UnverifiedJsonMeasurements,
    verified: &VerifiedQuote,
) -> Vec<JsonClaimAnomaly> {
    MeasurementField::ALL
        .iter()
        .filter_map(|field| {
            let claimed = read_claim(*field, claim);
            let actual = field.read(verified);
            (!claimed.eq_ignore_ascii_case(actual)).then(|| JsonClaimAnomaly {
                field: *field,
                claimed: claimed.to_string(),
                verified: actual.to_string(),
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::near_attestation::AttestationReport;
    use crate::near_attestation::quote::{parse_collateral, verify_quote};

    const FIXTURE: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_report.json"
    );
    const COLLATERAL: &str = include_str!(
        "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_collateral.json"
    );
    /// See `quote::tests::FIXTURE_CAPTURED_AT`: `verify_quote` consults no
    /// clock but this one, so these tests fail on a code change, never on a
    /// calendar date.
    const FIXTURE_CAPTURED_AT: u64 = 1_788_264_000;

    fn fixture_report() -> AttestationReport {
        AttestationReport::from_json(FIXTURE).expect("fixture parses")
    }

    /// The verified quote, produced through the real verification path from
    /// checked-in fixtures. No network.
    fn verified() -> VerifiedQuote {
        let collateral = parse_collateral(COLLATERAL).expect("collateral fixture parses");
        verify_quote(
            &fixture_report().quote_bytes().unwrap(),
            &collateral,
            FIXTURE_CAPTURED_AT,
        )
        .expect("fixture quote verifies")
    }

    /// An expected set built from the quote's own verified values.
    fn expected_matching(v: &VerifiedQuote) -> ExpectedMeasurements {
        let raw = format!(
            "mrtd={},rtmr0={},rtmr1={},rtmr2={},rtmr3={}",
            v.mrtd, v.rtmr[0], v.rtmr[1], v.rtmr[2], v.rtmr[3]
        );
        ExpectedMeasurements::from_env_value(Some(&raw))
            .expect("the quote's own values are a valid pin set")
            .expect("a non-empty value yields a set")
    }

    #[test]
    fn an_absent_expected_set_refuses_under_this_deployments_control_name() {
        // The generic refusal is tested in the permissive crate. What is
        // specific here, and what an operator actually reads in the drill
        // evidence, is that the name reported is *this* deployment's.
        let v = verified();
        let verdict = check_measurements_opt(None, &v, EXPECTED_MEASUREMENTS_CONTROL);
        assert_eq!(
            verdict,
            MeasurementVerdict::Refused {
                control: "near_ai_expected_measurements"
            },
            "{verdict}"
        );
        assert!(!verdict.is_pinned(), "a refusal is not a pass");
    }

    #[test]
    fn the_env_loader_reads_this_deployments_variable() {
        // `from_env_value` is covered in the permissive crate; what is
        // untested there is that the server reads
        // TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS and not some other
        // name. Asserting the constant's spelling is the falsifiable half --
        // the loader itself cannot be exercised here without mutating
        // process-wide environment under a parallel test runner.
        assert_eq!(
            EXPECTED_MEASUREMENTS_ENV,
            "TRACE_COMMONS_NEAR_AI_EXPECTED_MEASUREMENTS"
        );
        // And the loader is exactly `from_env_value` applied to that
        // variable's current value, whatever it happens to be. Unconditional
        // on purpose: a `if var.is_err()` guard would make this assertion
        // skippable by the ambient environment, which is the same
        // never-runs defect the pinning code exists to avoid.
        let current = std::env::var(EXPECTED_MEASUREMENTS_ENV).ok();
        assert_eq!(
            expected_measurements_from_env(),
            ExpectedMeasurements::from_env_value(current.as_deref())
        );
    }

    #[test]
    fn the_fixture_endpoints_json_claim_agrees_with_its_quote() {
        // Not a gate -- reporting only. On an honest endpoint there is nothing
        // to report, which is what makes a non-empty result meaningful.
        let v = verified();
        let anomalies = json_claim_anomalies(&fixture_report().unverified_json_measurements(), &v);
        assert!(anomalies.is_empty(), "{anomalies:?}");
    }

    #[test]
    fn a_lying_json_claim_is_reported_but_does_not_change_the_verdict() {
        // A server that misdescribes itself in unsigned JSON is worth
        // surfacing. It must not be able to affect whether the quote is
        // accepted, which is the whole reason pinning consumes VerifiedQuote.
        let v = verified();
        let mut claim = fixture_report().unverified_json_measurements();
        claim.rtmr1 = "c".repeat(96);
        assert_ne!(claim.rtmr1, v.rtmr[1], "the lie must actually differ");

        let anomalies = json_claim_anomalies(&claim, &v);
        assert_eq!(anomalies.len(), 1);
        assert_eq!(anomalies[0].field, MeasurementField::Rtmr1);
        assert_eq!(anomalies[0].verified, v.rtmr[1]);
        assert_eq!(anomalies[0].claimed, claim.rtmr1);

        // The pin, which reads the quote, is unmoved by the lie in either
        // direction: it still passes against the real values...
        assert!(check_measurements(&expected_matching(&v), &v).is_pinned());
        // ...and pinning the *claimed* value fails, because the quote is what
        // is compared.
        let expected =
            ExpectedMeasurements::from_env_value(Some(&format!("rtmr1={}", claim.rtmr1)))
                .unwrap()
                .unwrap();
        assert_eq!(
            check_measurements(&expected, &v).mismatched_fields(),
            vec![MeasurementField::Rtmr1]
        );
    }

    #[test]
    fn every_lying_register_is_reported_not_just_the_first() {
        // json_claim_anomalies iterates MeasurementField::ALL; a version that
        // stopped at the first disagreement, or that read the wrong register
        // from the claim, would still pass the single-lie test above.
        let v = verified();
        let mut claim = fixture_report().unverified_json_measurements();
        claim.mrtd = "d".repeat(96);
        claim.rtmr3 = "e".repeat(96);

        let anomalies = json_claim_anomalies(&claim, &v);
        assert_eq!(
            anomalies.iter().map(|a| a.field).collect::<Vec<_>>(),
            vec![MeasurementField::Mrtd, MeasurementField::Rtmr3]
        );
        assert_eq!(anomalies[0].claimed, claim.mrtd);
        assert_eq!(anomalies[0].verified, v.mrtd);
        assert_eq!(anomalies[1].claimed, claim.rtmr3);
        assert_eq!(anomalies[1].verified, v.rtmr[3]);
        // The rendering an operator sees must name the register and both
        // sides, or the report is unactionable.
        let rendered = anomalies[1].to_string();
        assert!(rendered.contains("rtmr3"), "{rendered}");
        assert!(rendered.contains(&v.rtmr[3]), "{rendered}");
    }
}
