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

/// The value of a register as a NEAR AI report *claims* it in unsigned JSON,
/// or `None` where the report makes no such claim.
///
/// `Option` rather than a placeholder: a NEAR AI report's `info.tcb_info`
/// carries mrtd and rtmr0..3 and nothing else, so MRCONFIGID has no claimed
/// counterpart to disagree with. Returning `""` for it would make every
/// honest endpoint report an anomaly on that register, which is exactly the
/// kind of noise that gets an anomaly list ignored.
fn read_claim(field: MeasurementField, claim: &UnverifiedJsonMeasurements) -> Option<&str> {
    match field {
        MeasurementField::Mrtd => Some(&claim.mrtd),
        MeasurementField::Rtmr0 => Some(&claim.rtmr0),
        MeasurementField::Rtmr1 => Some(&claim.rtmr1),
        MeasurementField::Rtmr2 => Some(&claim.rtmr2),
        MeasurementField::Rtmr3 => Some(&claim.rtmr3),
        // Deliberate, not an oversight: NEAR AI's `info.tcb_info` carries
        // mrtd and rtmr0..3, so there is no claimed mrconfigid to disagree
        // with. If a report ever starts claiming one, this arm must start
        // comparing it -- and because an uncompared field breaks no build,
        // unlike adding a `MeasurementField`, the assumption is pinned by
        // `tests::a_report_that_claims_mrconfigid_must_not_pass_unnoticed`.
        MeasurementField::MrConfigId => None,
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
/// Only `mrtd` and `rtmr0..3` are comparable. `compose_hash`, `os_image_hash`
/// and `mr_aggregated` have no verified counterpart, and `mrconfigid` is the
/// mirror case -- verified but not claimed -- so neither side is examined.
pub fn json_claim_anomalies(
    claim: &UnverifiedJsonMeasurements,
    verified: &VerifiedQuote,
) -> Vec<JsonClaimAnomaly> {
    MeasurementField::ALL
        .iter()
        .filter_map(|field| {
            let claimed = read_claim(*field, claim)?;
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
        // A second assertion used to stand here comparing
        // `expected_measurements_from_env()` against
        // `from_env_value(std::env::var(EXPECTED_MEASUREMENTS_ENV).ok())`,
        // claiming to prove the loader reads *this* name. It could not: with
        // the variable unset -- which is how it is in CI and in every local
        // run -- both sides are `Ok(None)` no matter which name the loader
        // consults, so a loader reading TRACE_COMMONS_WITNESS_... would have
        // passed it unchanged. It is deleted rather than repaired because
        // the only repair is to set the variable, and `set_var` is
        // process-wide and unsound under a parallel test runner that has
        // other tests reading the environment. The spelling assertion above
        // is what is actually falsifiable here; that the loader is
        // `from_env_value` of that constant is one line, visible at the call
        // site.
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
    fn mrconfigid_is_verified_here_but_never_claimed_so_it_is_not_an_anomaly() {
        // MRCONFIGID is pinnable against the quote but a NEAR AI report's
        // info.tcb_info does not carry it. `read_claim` therefore returns
        // None for it. An arm returning `""` instead would make every honest
        // endpoint report an mrconfigid anomaly forever -- so assert it is
        // absent even when every register the report *does* claim is a lie,
        // which is the case where a spurious extra entry would hide.
        assert!(
            MeasurementField::ALL.contains(&MeasurementField::MrConfigId),
            "the field must be in ALL, or this test proves nothing"
        );
        let v = verified();
        let mut claim = fixture_report().unverified_json_measurements();
        claim.mrtd = "1".repeat(96);
        claim.rtmr0 = "2".repeat(96);
        claim.rtmr1 = "3".repeat(96);
        claim.rtmr2 = "4".repeat(96);
        claim.rtmr3 = "5".repeat(96);

        let fields: Vec<MeasurementField> = json_claim_anomalies(&claim, &v)
            .iter()
            .map(|a| a.field)
            .collect();
        assert_eq!(
            fields,
            vec![
                MeasurementField::Mrtd,
                MeasurementField::Rtmr0,
                MeasurementField::Rtmr1,
                MeasurementField::Rtmr2,
                MeasurementField::Rtmr3,
            ],
            "every claimed register lies and must be reported; mrconfigid is \
             not claimed and must not be"
        );
    }

    /// Does this report's `info.tcb_info` name MRCONFIGID, under any of the
    /// spellings a producer might plausibly emit?
    ///
    /// Normalising away case and `_`/`-` is the point: matching the literal
    /// `mrconfigid` would miss `mr_config_id`, which is the spelling this
    /// workspace's own `VerifiedQuote` uses for the same register.
    fn tcb_info_names_mrconfigid(report_json: &str) -> bool {
        let parsed: serde_json::Value =
            serde_json::from_str(report_json).expect("report JSON parses");
        parsed["info"]["tcb_info"]
            .as_object()
            .expect("info.tcb_info is an object")
            .keys()
            .any(|k| k.to_ascii_lowercase().replace(['_', '-'], "") == "mrconfigid")
    }

    #[test]
    fn a_report_that_claims_mrconfigid_must_not_pass_unnoticed() {
        // `read_claim` returns None for MrConfigId, which is right only while
        // no report claims the register. Adding a `MeasurementField` fails to
        // compile until every arm is updated; a report that *starts* carrying
        // mrconfigid breaks nothing at all -- the arm just keeps not
        // comparing a field that now has both sides. Nothing in the type
        // system catches that, so this does.
        //
        // The checked-in fixture is TRIMMED (see its `_fixture_note`), so on
        // its own it could not show that NEAR AI's untrimmed response omits
        // mrconfigid. That was settled separately on 2026-09-02 with an
        // untrimmed live capture: `info.tcb_info` carries app_compose,
        // compose_hash, device_id, event_log, mrtd, os_image_hash and
        // rtmr0-3, and no key anywhere in the document names the register.
        // What this test catches is the case that matters for maintenance: a
        // re-captured fixture whose tcb_info carries mrconfigid. Then the fix
        // is to add the field to `UnverifiedJsonMeasurements` and make the arm
        // compare it, not to relax this assertion.
        assert!(
            !tcb_info_names_mrconfigid(FIXTURE),
            "the captured report's tcb_info now names mrconfigid; read_claim's \
             `MeasurementField::MrConfigId => None` arm no longer holds"
        );

        // The detector must be able to say yes, or the assertion above is
        // vacuous -- a matcher that can never match passes forever.
        let claiming = FIXTURE.replace(
            "\"tcb_info\": {",
            "\"tcb_info\": {\n      \"mr_config_id\": \"00\",",
        );
        assert_ne!(claiming, FIXTURE, "the fixture's tcb_info key moved");
        assert!(
            tcb_info_names_mrconfigid(&claiming),
            "the detector missed an mrconfigid claim it was handed"
        );

        // And the omission really is one-sided: the quote supplies the
        // register, so it is only the claimed half that is absent.
        let v = verified();
        assert_eq!(
            MeasurementField::MrConfigId.read(&v),
            v.mr_config_id,
            "mrconfigid is verified; only the claimed side is missing"
        );
        assert!(
            !v.mr_config_id.is_empty(),
            "an empty verified value would make the comparison above vacuous"
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
