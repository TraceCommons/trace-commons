//! Pinning the TDX image measurements an enclave is allowed to run.
//!
//! Nonce binding proves a report is fresh. [`crate::quote::verify_quote`]
//! proves the quote is genuine. Neither says anything about *what software*
//! the enclave is running: a real, Intel-vouched TDX machine running an
//! entirely different image passes both. This module closes that gap by
//! comparing the measurement registers against a set the operator has pinned
//! in advance.
//!
//! Two properties are load-bearing.
//!
//! **The comparison is against the verified quote, never a report JSON.**
//! [`check_measurements`] takes a [`VerifiedQuote`], whose `mrtd` and `rtmr`
//! were read out of the signature-covered quote structure. The same registers
//! also appear in a NEAR AI report's unsigned `info.tcb_info`, and pinning
//! against *those* would amount to asking the server whether it is running
//! what it says it is running. That JSON copy has exactly one legitimate use
//! -- reporting that a server's claim about itself disagrees with what the
//! hardware signed -- and it lives with the type that describes that
//! envelope, not here.
//!
//! **Only `mrtd` and `rtmr0..3` are pinnable.** A NEAR AI report also carries
//! `compose_hash`, `os_image_hash` and `mr_aggregated`, but those exist only
//! in the unsigned JSON and are not recoverable from the quote without
//! reproducing dstack's RTMR extension derivation, which this crate does not
//! do. Naming one of them in the expected set is therefore a *config error*
//! ([`ExpectedMeasurementsError::NotVerifiableFromQuote`]) and not a quietly
//! skipped key -- a value labelled "pinned" that nothing verifies is worse
//! than no pinning at all, because it reads as a control that is present.
//!
//! Absence of an expected set fails closed: [`check_measurements_opt`] refuses
//! with a named missing control rather than passing. The control *name* is the
//! caller's, not this crate's: the hosted server pins a NEAR AI endpoint and a
//! redaction witness pins its own image, and a shared generic-looking constant
//! would name neither honestly.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::fmt;

use crate::quote::VerifiedQuote;

/// Hex length of a TDX measurement register (SHA-384, 48 bytes).
const MEASUREMENT_HEX_LEN: usize = 96;

/// A measurement register that can actually be checked against a verified
/// quote.
///
/// The ordering is the canonical one and is what makes verdicts deterministic
/// regardless of the order the operator wrote the pins in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MeasurementField {
    Mrtd,
    Rtmr0,
    Rtmr1,
    Rtmr2,
    Rtmr3,
}

impl MeasurementField {
    pub fn as_str(self) -> &'static str {
        match self {
            MeasurementField::Mrtd => "mrtd",
            MeasurementField::Rtmr0 => "rtmr0",
            MeasurementField::Rtmr1 => "rtmr1",
            MeasurementField::Rtmr2 => "rtmr2",
            MeasurementField::Rtmr3 => "rtmr3",
        }
    }

    fn parse(key: &str) -> Option<Self> {
        match key {
            "mrtd" => Some(MeasurementField::Mrtd),
            "rtmr0" => Some(MeasurementField::Rtmr0),
            "rtmr1" => Some(MeasurementField::Rtmr1),
            "rtmr2" => Some(MeasurementField::Rtmr2),
            "rtmr3" => Some(MeasurementField::Rtmr3),
            _ => None,
        }
    }

    /// The value of this register as read out of a verified quote.
    ///
    /// Public because a caller comparing some *other* copy of these registers
    /// against the signed ones -- an endpoint's unsigned self-description, say
    /// -- needs the trustworthy side of that comparison, and this is it.
    pub fn read(self, quote: &VerifiedQuote) -> &str {
        match self {
            MeasurementField::Mrtd => &quote.mrtd,
            MeasurementField::Rtmr0 => &quote.rtmr[0],
            MeasurementField::Rtmr1 => &quote.rtmr[1],
            MeasurementField::Rtmr2 => &quote.rtmr[2],
            MeasurementField::Rtmr3 => &quote.rtmr[3],
        }
    }

    /// Every field, in canonical order.
    pub const ALL: [MeasurementField; 5] = [
        MeasurementField::Mrtd,
        MeasurementField::Rtmr0,
        MeasurementField::Rtmr1,
        MeasurementField::Rtmr2,
        MeasurementField::Rtmr3,
    ];
}

impl fmt::Display for MeasurementField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Keys a report carries but that cannot be checked against a quote.
///
/// Naming one of these is a config error rather than an unknown key, because
/// the operator's mistake is different and so is the fix: the value is real,
/// it is just not something the hardware signed.
const JSON_ONLY_KEYS: [&str; 3] = ["compose_hash", "os_image_hash", "mr_aggregated"];

/// Why an expected-measurement configuration was rejected.
///
/// Every variant is a refusal to load. There is deliberately no "ignored the
/// key and carried on" path: a silently dropped `mrtdd=...` would leave an
/// operator believing `mrtd` was pinned when nothing was.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ExpectedMeasurementsError {
    #[error("expected-measurement entry {index} is not in key=value form")]
    MalformedEntry { index: usize },
    #[error("expected-measurement entry {index} has an empty key")]
    EmptyKey { index: usize },
    #[error(
        "unknown expected-measurement key {key:?}; expected one of mrtd, rtmr0, rtmr1, rtmr2, rtmr3"
    )]
    UnknownField { key: String },
    #[error(
        "expected-measurement key {key:?} appears only in the report's unsigned JSON and cannot be \
         checked against the signed quote; pin mrtd and rtmr0..rtmr3 instead"
    )]
    NotVerifiableFromQuote { key: String },
    #[error("expected-measurement key {key} is set more than once")]
    DuplicateField { key: MeasurementField },
    #[error(
        "expected value for {key} must be {MEASUREMENT_HEX_LEN} hex characters, got {len} \
         characters"
    )]
    ValueWrongLength { key: MeasurementField, len: usize },
    #[error("expected value for {key} is not hex")]
    ValueNotHex { key: MeasurementField },
    #[error("expected-measurement configuration named no measurements")]
    NoPins,
}

/// A set of measurement registers an operator has pinned.
///
/// Only the registers named are checked. An empty set cannot be constructed:
/// [`ExpectedMeasurementsError::NoPins`] is raised instead, so "configured"
/// always means "checks at least one thing".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedMeasurements {
    pins: BTreeMap<MeasurementField, String>,
}

impl ExpectedMeasurements {
    /// Parse a comma-separated `key=value` list.
    ///
    /// `Ok(None)` means the configuration is absent or empty, which is *not*
    /// an acceptance -- see [`check_measurements_opt`].
    ///
    /// Whitespace around entries, keys and values is ignored, and keys are
    /// matched case-insensitively. Neither leniency can hide a pin: a
    /// misspelled key is still an error.
    pub fn from_env_value(value: Option<&str>) -> Result<Option<Self>, ExpectedMeasurementsError> {
        let Some(raw) = value else {
            return Ok(None);
        };
        if raw.trim().is_empty() {
            return Ok(None);
        }

        let mut pins: BTreeMap<MeasurementField, String> = BTreeMap::new();
        for (index, entry) in raw.split(',').enumerate() {
            let entry = entry.trim();
            // A stray separator is tolerated because it cannot hide a pin.
            if entry.is_empty() {
                continue;
            }
            let (key, value) = entry
                .split_once('=')
                .ok_or(ExpectedMeasurementsError::MalformedEntry { index })?;
            let key = key.trim();
            let value = value.trim();
            if key.is_empty() {
                return Err(ExpectedMeasurementsError::EmptyKey { index });
            }
            let lowered = key.to_ascii_lowercase();
            let field = match MeasurementField::parse(&lowered) {
                Some(field) => field,
                None if JSON_ONLY_KEYS.contains(&lowered.as_str()) => {
                    return Err(ExpectedMeasurementsError::NotVerifiableFromQuote { key: lowered });
                }
                None => return Err(ExpectedMeasurementsError::UnknownField { key: lowered }),
            };
            if value.len() != MEASUREMENT_HEX_LEN {
                return Err(ExpectedMeasurementsError::ValueWrongLength {
                    key: field,
                    len: value.len(),
                });
            }
            if !value.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(ExpectedMeasurementsError::ValueNotHex { key: field });
            }
            match pins.entry(field) {
                Entry::Occupied(_) => {
                    return Err(ExpectedMeasurementsError::DuplicateField { key: field });
                }
                Entry::Vacant(slot) => {
                    slot.insert(value.to_ascii_lowercase());
                }
            }
        }

        if pins.is_empty() {
            return Err(ExpectedMeasurementsError::NoPins);
        }
        Ok(Some(ExpectedMeasurements { pins }))
    }

    /// The registers this set pins, in canonical order.
    pub fn pinned_fields(&self) -> Vec<MeasurementField> {
        self.pins.keys().copied().collect()
    }
}

/// One register whose verified value is not the pinned one.
///
/// Measurement values are public image identifiers, not secrets, so carrying
/// them here and into log lines is deliberate: an operator holding both halves
/// can go straight to the image, which is the whole point of naming the field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeasurementMismatch {
    pub field: MeasurementField,
    pub expected: String,
    pub actual: String,
}

impl fmt::Display for MeasurementMismatch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} differs (expected {}, got {})",
            self.field, self.expected, self.actual
        )
    }
}

/// The outcome of checking a verified quote against a pinned set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeasurementVerdict {
    /// Every pinned register matched. `fields` is what was actually checked,
    /// so a caller can report the strength of the check and not merely that it
    /// passed.
    Pinned { fields: Vec<MeasurementField> },
    /// At least one pinned register differs. Every differing register is
    /// listed: "attestation failed" sends an operator to the wrong place,
    /// "rtmr2 differs" sends them to the image.
    ///
    /// `fields` is the whole pinned set, exactly as on
    /// [`MeasurementVerdict::Pinned`], and it is here so that the answer to
    /// "how strong was this check" does not change meaning between the two
    /// verdicts. Derive it from `mismatches` and a run that pinned all five
    /// registers and saw `rtmr2` drift becomes indistinguishable from one
    /// that only ever pinned `rtmr2` -- reported at exactly the moment an
    /// operator is judging how much the check was worth.
    Mismatch {
        fields: Vec<MeasurementField>,
        mismatches: Vec<MeasurementMismatch>,
    },
    /// Nothing was pinned, so nothing was checked. This is a refusal, not a
    /// pass.
    Refused { control: &'static str },
}

impl MeasurementVerdict {
    /// True only for [`MeasurementVerdict::Pinned`]. A refusal is not a pass.
    pub fn is_pinned(&self) -> bool {
        matches!(self, MeasurementVerdict::Pinned { .. })
    }

    /// The registers that differ, in canonical order. Empty unless the verdict
    /// is a mismatch.
    pub fn mismatched_fields(&self) -> Vec<MeasurementField> {
        match self {
            MeasurementVerdict::Mismatch { mismatches, .. } => {
                mismatches.iter().map(|m| m.field).collect()
            }
            _ => Vec::new(),
        }
    }
}

impl fmt::Display for MeasurementVerdict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MeasurementVerdict::Pinned { fields } => {
                let names: Vec<&str> = fields.iter().map(|x| x.as_str()).collect();
                write!(f, "measurements match pinned set ({})", names.join(", "))
            }
            MeasurementVerdict::Mismatch { mismatches, .. } => {
                let rendered: Vec<String> = mismatches.iter().map(|m| m.to_string()).collect();
                write!(f, "measurements do not match: {}", rendered.join("; "))
            }
            MeasurementVerdict::Refused { control } => {
                write!(f, "measurement pinning refused: missing control {control}")
            }
        }
    }
}

/// Compare a verified quote's registers against a pinned set.
///
/// `actual` is a [`VerifiedQuote`] and nothing else. A NEAR AI report's
/// `info.tcb_info` carries the same register names, and comparing against
/// those would verify the server's own claim about itself. There is
/// deliberately no overload taking an unverified measurement set: the type is
/// the guard, and a doc comment is not.
///
/// Hex comparison is ASCII-case-insensitive and otherwise exact; a shorter
/// expected value never matches by prefix.
pub fn check_measurements(
    expected: &ExpectedMeasurements,
    actual: &VerifiedQuote,
) -> MeasurementVerdict {
    let mut mismatches = Vec::new();
    for (field, want) in &expected.pins {
        let got = field.read(actual);
        if !want.eq_ignore_ascii_case(got) {
            mismatches.push(MeasurementMismatch {
                field: *field,
                expected: want.clone(),
                actual: got.to_string(),
            });
        }
    }
    if mismatches.is_empty() {
        MeasurementVerdict::Pinned {
            fields: expected.pinned_fields(),
        }
    } else {
        MeasurementVerdict::Mismatch {
            fields: expected.pinned_fields(),
            mismatches,
        }
    }
}

/// As [`check_measurements`], but fails closed when nothing is pinned.
///
/// An operator who has configured no expected set gets a refusal naming the
/// missing control, never a green tick that means nothing. `control` is that
/// name, and it is the caller's because the thing being pinned is: the hosted
/// server reports `near_ai_expected_measurements`, and a witness reports its
/// own. A constant here would be generic in shape and wrong in every specific
/// deployment.
pub fn check_measurements_opt(
    expected: Option<&ExpectedMeasurements>,
    actual: &VerifiedQuote,
    control: &'static str,
) -> MeasurementVerdict {
    match expected {
        Some(expected) => check_measurements(expected, actual),
        None => MeasurementVerdict::Refused { control },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quote::{parse_collateral, verify_quote};

    const FIXTURE: &str = include_str!("../tests/fixtures/near_ai_attestation_report.json");
    const COLLATERAL: &str = include_str!("../tests/fixtures/near_ai_attestation_collateral.json");
    /// See `quote::tests::FIXTURE_CAPTURED_AT`: `verify_quote` consults no
    /// clock but this one, so these tests fail on a code change, never on a
    /// calendar date.
    const FIXTURE_CAPTURED_AT: u64 = 1_788_264_000;

    /// A control name standing in for whatever the caller pins. Deliberately
    /// not the hosted server's `near_ai_expected_measurements`: this crate
    /// does not know that name, and a test asserting it here would be
    /// asserting something this module cannot get wrong.
    const TEST_CONTROL: &str = "test_expected_measurements";

    /// The raw quote bytes out of the fixture report.
    ///
    /// The hosted server reaches these through
    /// `AttestationReport::quote_bytes`, which is exactly a hex decode of
    /// `intel_quote`. That type is AGPL and stays behind the boundary, so
    /// this reads the same field out of the same fixture directly.
    fn fixture_quote() -> Vec<u8> {
        let v: serde_json::Value = serde_json::from_str(FIXTURE).unwrap();
        hex::decode(v["intel_quote"].as_str().unwrap()).unwrap()
    }

    /// The verified quote, produced through the real verification path from
    /// checked-in fixtures. No network.
    fn verified() -> VerifiedQuote {
        let collateral = parse_collateral(COLLATERAL).expect("collateral fixture parses");
        verify_quote(&fixture_quote(), &collateral, FIXTURE_CAPTURED_AT)
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
    fn matching_measurements_pass_and_report_what_was_checked() {
        let v = verified();
        let verdict = check_measurements(&expected_matching(&v), &v);
        assert_eq!(
            verdict,
            MeasurementVerdict::Pinned {
                fields: MeasurementField::ALL.to_vec()
            },
            "{verdict}"
        );
        assert!(verdict.is_pinned());
    }

    #[test]
    fn one_changed_measurement_fails_and_names_which() {
        // "Attestation failed" sends an operator to the wrong place; "rtmr2
        // differs" sends them to the image.
        let v = verified();
        let mut tampered = v.rtmr[2].clone();
        // Flip the leading nibble to something it is not, so the mutation is
        // guaranteed to be a real change and not a no-op rewrite.
        let head = if tampered.starts_with('0') { '1' } else { '0' };
        tampered.replace_range(0..1, &head.to_string());
        assert_ne!(tampered, v.rtmr[2], "the mutation must actually change it");
        assert_eq!(
            tampered.len(),
            v.rtmr[2].len(),
            "and must not change the length, or this would fail for that reason"
        );

        let raw = format!("mrtd={},rtmr2={}", v.mrtd, tampered);
        let expected = ExpectedMeasurements::from_env_value(Some(&raw))
            .unwrap()
            .unwrap();
        let verdict = check_measurements(&expected, &v);

        assert_eq!(verdict.mismatched_fields(), vec![MeasurementField::Rtmr2]);
        let MeasurementVerdict::Mismatch { fields, mismatches } = &verdict else {
            panic!("expected a mismatch, got {verdict}");
        };
        // The whole pinned set, not just the register that drifted. A
        // mismatch that reported only `rtmr2` here would be
        // indistinguishable from a deployment that only ever pinned `rtmr2`.
        assert_eq!(fields, &expected.pinned_fields());
        assert!(fields.len() > 1, "this fixture pins more than one register");
        assert_eq!(mismatches.len(), 1);
        assert_eq!(mismatches[0].field.as_str(), "rtmr2");
        assert_eq!(mismatches[0].expected, tampered);
        assert_eq!(mismatches[0].actual, v.rtmr[2]);
        // The rendering an operator sees must name the register.
        assert!(verdict.to_string().contains("rtmr2"), "{verdict}");
        assert!(!verdict.is_pinned());
    }

    #[test]
    fn every_differing_register_is_named_not_just_the_first() {
        // A verdict that stops at the first difference understates the blast
        // radius: an operator told only "mrtd differs" may conclude one
        // register drifted when the whole image is different.
        let v = verified();
        let raw = format!(
            "mrtd={},rtmr0={},rtmr3={}",
            "a".repeat(96),
            v.rtmr[0],
            "b".repeat(96)
        );
        let expected = ExpectedMeasurements::from_env_value(Some(&raw))
            .unwrap()
            .unwrap();
        assert_eq!(
            check_measurements(&expected, &v).mismatched_fields(),
            vec![MeasurementField::Mrtd, MeasurementField::Rtmr3],
            "rtmr0 was pinned correctly and must not be reported"
        );
    }

    #[test]
    fn an_absent_expected_set_refuses_rather_than_passing() {
        let v = verified();
        assert_eq!(ExpectedMeasurements::from_env_value(None), Ok(None));
        // An explicitly empty value is the same case, not a config error: it
        // still pins nothing, and the refusal below is what matters.
        assert_eq!(ExpectedMeasurements::from_env_value(Some("   ")), Ok(None));

        let verdict = check_measurements_opt(None, &v, TEST_CONTROL);
        assert_eq!(
            verdict,
            MeasurementVerdict::Refused {
                control: "test_expected_measurements"
            },
            "{verdict}"
        );
        assert!(!verdict.is_pinned(), "a refusal is not a pass");
        // The refusal an operator reads must name the control, or it sends
        // them looking for a failure instead of a missing setting.
        assert!(
            verdict.to_string().contains("test_expected_measurements"),
            "{verdict}"
        );
    }

    #[test]
    fn a_present_expected_set_is_checked_rather_than_refused() {
        // The control arm of the test above. Without it,
        // `check_measurements_opt` could refuse unconditionally and still
        // pass everything asserted there.
        let v = verified();
        let expected = expected_matching(&v);
        assert!(
            check_measurements_opt(Some(&expected), &v, TEST_CONTROL).is_pinned(),
            "a pinned set must be compared, not refused"
        );
    }

    #[test]
    fn a_set_that_pins_nothing_cannot_be_constructed() {
        // Guards the gap between "configured" and "checks something". A value
        // of "," parses to zero pins; if that yielded a set, check_measurements
        // would return Pinned { fields: [] } -- a green tick over an empty
        // check, which is exactly the false-confidence failure this task
        // exists to prevent.
        assert_eq!(
            ExpectedMeasurements::from_env_value(Some(",,")),
            Err(ExpectedMeasurementsError::NoPins)
        );
    }

    #[test]
    fn an_expected_set_naming_an_unknown_field_is_a_config_error() {
        // The test that matters most. A silently ignored `mrtdd=...` would let
        // an operator believe mrtd was pinned when nothing was. Asserting the
        // specific variant and key, rather than is_err(), is what confirms the
        // rejection is *because of the typo* and not because the parser
        // tripped over something else in the string.
        let good = "a".repeat(96);
        let err = ExpectedMeasurements::from_env_value(Some(&format!("mrtdd={good}")))
            .expect_err("a misspelled key must be rejected");
        assert_eq!(
            err,
            ExpectedMeasurementsError::UnknownField {
                key: "mrtdd".to_string()
            }
        );

        // And the control: the same string with the key spelled correctly is
        // accepted, so the rejection above is about the key and nothing else.
        let ok = ExpectedMeasurements::from_env_value(Some(&format!("mrtd={good}")))
            .expect("the correctly spelled key is accepted")
            .expect("and yields a set");
        assert_eq!(ok.pinned_fields(), vec![MeasurementField::Mrtd]);

        // A typo must never be absorbed by a well-spelled neighbour either.
        let err = ExpectedMeasurements::from_env_value(Some(&format!("mrtd={good},rtmr9={good}")))
            .expect_err("a typo alongside a valid pin is still a config error");
        assert_eq!(
            err,
            ExpectedMeasurementsError::UnknownField {
                key: "rtmr9".to_string()
            }
        );
    }

    #[test]
    fn json_only_keys_are_a_config_error_with_their_own_reason() {
        // compose_hash, os_image_hash and mr_aggregated exist only in the
        // report's unsigned JSON. Accepting them would produce a pin that
        // nothing verifies; skipping them silently would be worse. Both are
        // refused, with a message that tells the operator why rather than
        // calling a real field a typo.
        let good = "a".repeat(96);
        for key in ["compose_hash", "os_image_hash", "mr_aggregated"] {
            let err =
                ExpectedMeasurements::from_env_value(Some(&format!("{key}={good}"))).unwrap_err();
            assert_eq!(
                err,
                ExpectedMeasurementsError::NotVerifiableFromQuote {
                    key: key.to_string()
                },
                "{key}"
            );
        }
    }

    #[test]
    fn comparison_is_case_insensitive_on_hex() {
        // Two layers, asserted separately, because the first alone makes the
        // second unfalsifiable: parsing lowercases the value, so a
        // case-*sensitive* comparator would still pass the parsed-path test.
        let v = verified();
        let upper = v.mrtd.to_ascii_uppercase();
        assert_ne!(
            upper, v.mrtd,
            "the fixture's mrtd must actually contain letters, or this test proves nothing"
        );

        // 1. Through the config path: an uppercase pin, and an uppercase key.
        let expected = ExpectedMeasurements::from_env_value(Some(&format!("MRTD={upper}")))
            .unwrap()
            .unwrap();
        let verdict = check_measurements(&expected, &v);
        assert!(verdict.is_pinned(), "{verdict}");

        // 2. In the comparator itself, bypassing that normalization. If the
        //    parse-time lowercasing is ever dropped, this is what still holds.
        let smuggled = ExpectedMeasurements {
            pins: BTreeMap::from([(MeasurementField::Mrtd, upper)]),
        };
        assert!(check_measurements(&smuggled, &v).is_pinned());
    }

    #[test]
    fn a_short_expected_value_never_matches_by_prefix() {
        // Two independent guards, because either alone could rot.
        let v = verified();
        let truncated = &v.mrtd[..40];
        assert!(
            v.mrtd.starts_with(truncated),
            "the truncated value must genuinely be a prefix, or a prefix-matching \
             implementation would pass this test for the wrong reason"
        );

        // 1. Parsing refuses a wrong-length value outright.
        assert_eq!(
            ExpectedMeasurements::from_env_value(Some(&format!("mrtd={truncated}"))),
            Err(ExpectedMeasurementsError::ValueWrongLength {
                key: MeasurementField::Mrtd,
                len: 40
            })
        );

        // 2. And the comparator itself does not prefix-match, proven by
        //    building the set directly and bypassing that validation. If the
        //    length check above is ever relaxed, this is what still holds.
        let smuggled = ExpectedMeasurements {
            pins: BTreeMap::from([(MeasurementField::Mrtd, truncated.to_string())]),
        };
        assert_eq!(
            check_measurements(&smuggled, &v).mismatched_fields(),
            vec![MeasurementField::Mrtd]
        );
    }

    #[test]
    fn a_non_hex_expected_value_is_a_config_error() {
        let mut value = "a".repeat(95);
        value.push('z');
        assert_eq!(
            ExpectedMeasurements::from_env_value(Some(&format!("rtmr1={value}"))),
            Err(ExpectedMeasurementsError::ValueNotHex {
                key: MeasurementField::Rtmr1
            })
        );
    }

    #[test]
    fn malformed_entries_are_config_errors() {
        let good = "a".repeat(96);
        assert_eq!(
            ExpectedMeasurements::from_env_value(Some("mrtd")),
            Err(ExpectedMeasurementsError::MalformedEntry { index: 0 })
        );
        assert_eq!(
            ExpectedMeasurements::from_env_value(Some(&format!("={good}"))),
            Err(ExpectedMeasurementsError::EmptyKey { index: 0 })
        );
        // A register pinned twice is ambiguous: silently keeping one of the
        // two values would pin something the operator did not choose.
        assert_eq!(
            ExpectedMeasurements::from_env_value(Some(&format!("rtmr0={good},rtmr0={good}"))),
            Err(ExpectedMeasurementsError::DuplicateField {
                key: MeasurementField::Rtmr0
            })
        );
    }

    #[test]
    fn whitespace_around_entries_is_tolerated() {
        let v = verified();
        let raw = format!("  mrtd = {} , rtmr0 = {} , ", v.mrtd, v.rtmr[0]);
        let expected = ExpectedMeasurements::from_env_value(Some(&raw))
            .unwrap()
            .unwrap();
        assert_eq!(
            expected.pinned_fields(),
            vec![MeasurementField::Mrtd, MeasurementField::Rtmr0]
        );
        assert!(check_measurements(&expected, &v).is_pinned());
    }

    #[test]
    fn a_partial_pin_checks_only_what_it_names() {
        let v = verified();
        let expected = ExpectedMeasurements::from_env_value(Some(&format!("rtmr3={}", v.rtmr[3])))
            .unwrap()
            .unwrap();
        assert_eq!(
            check_measurements(&expected, &v),
            MeasurementVerdict::Pinned {
                fields: vec![MeasurementField::Rtmr3]
            },
            "an operator must be able to see that only rtmr3 was checked"
        );
    }

    #[test]
    fn field_read_returns_the_quotes_own_registers() {
        // `read` is public so that a caller comparing an endpoint's unsigned
        // self-description against the signed values has the trustworthy
        // side. If it ever returned the wrong register, every such comparison
        // would silently compare the wrong pair.
        let v = verified();
        assert_eq!(MeasurementField::Mrtd.read(&v), v.mrtd);
        for (i, field) in [
            MeasurementField::Rtmr0,
            MeasurementField::Rtmr1,
            MeasurementField::Rtmr2,
            MeasurementField::Rtmr3,
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(field.read(&v), v.rtmr[i], "{field}");
        }
        // And the registers are not all the same value, or the loop above
        // would pass under any permutation of them.
        assert_ne!(v.rtmr[0], v.rtmr[1]);
        assert_ne!(v.rtmr[1], v.rtmr[2]);
        assert_ne!(v.rtmr[2], v.rtmr[3]);
        assert_ne!(v.mrtd, v.rtmr[0]);
    }
}
