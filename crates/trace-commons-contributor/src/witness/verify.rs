//! [`VerifiedWitness`], and the checks that are the only way to obtain one.
//!
//! This module is the guard the whole witness path rests on. `VerifiedWitness`
//! has private fields and exactly one constructor, and that constructor
//! performs the verification. `witness/transport.rs` holds the function that
//! transmits raw bytes and takes a `&VerifiedWitness`; it cannot construct
//! one, because these fields are private to this module.
//!
//! `there_is_no_way_to_build_a_verified_witness_but_verification` asserts that
//! structurally, over this file's own source. That is not a stylistic
//! preference: a second constructor added here in a year would silently make
//! the ordering property a convention again, and no behavioural test can see
//! a constructor that nothing yet calls.

use trace_commons_attestation::address::decode_address;
use trace_commons_attestation::measurements::{MeasurementVerdict, check_measurements_opt};
use trace_commons_attestation::quote::{Collateral, VerifiedQuote, verify_quote};

use super::transport::{AttestationEvidence, WitnessNonce};
use super::{
    WITNESS_ADDRESS_AT, WITNESS_EXPECTED_MEASUREMENT_CONTROL, WITNESS_NONCE_AT, WITNESS_NONCE_LEN,
    WITNESS_QUOTE_DOMAIN, WITNESS_REPORT_DATA_LEN, WitnessTrust, WitnessTrustError,
};

/// A witness whose measurement this client has verified, for this exchange.
///
/// Bound to one nonce and therefore to one verification. It is deliberately
/// **not** `Clone`, `Copy`, `Serialize` or `Default`: every one of those would
/// let a verified witness outlive the exchange it was verified for, which is
/// the same as not having verified it. It carries no lifetime to a quote
/// either -- the quote is checked and dropped; what survives is the decision.
///
/// Fields are private. The only constructor is [`verify_witness`].
pub struct VerifiedWitness {
    /// The base URL raw bytes may be sent to. Read only by
    /// `transport::witness_contribution`.
    url: String,
    /// The address whose signature the returned certificate must recover to.
    /// Carried from the pin rather than from the quote, so a witness cannot
    /// widen what will be accepted by reporting a second address.
    signing_address: String,
}

impl VerifiedWitness {
    /// The witness base URL. `pub(super)`: only the transport needs it, and a
    /// public accessor would let any caller reach the URL while holding
    /// nothing but a reference, which is most of the way to sending to it.
    pub(super) fn url(&self) -> &str {
        &self.url
    }

    /// The pinned signing address the certificate must recover to.
    pub(super) fn signing_address(&self) -> &str {
        &self.signing_address
    }
}

impl std::fmt::Debug for VerifiedWitness {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The URL is withheld: the repo's diagnostics convention is
        // label-only, and which witness a contributor uses is theirs.
        formatter
            .debug_struct("VerifiedWitness")
            .field("url", &"<withheld>")
            .field("signing_address", &"<withheld>")
            .finish()
    }
}

/// Verify an attestation, and on success return the only value that permits
/// sending raw bytes.
///
/// `now_unix` is a parameter rather than a clock read here for the reason
/// `verify_quote` documents: collateral validity windows are evaluated
/// against it, and a caller that could not choose it could not test the
/// expiry path. **Production callers must pass real wall-clock time.**
pub fn verify_witness(
    url: &str,
    evidence: &AttestationEvidence,
    collateral: &Collateral,
    nonce: &WitnessNonce,
    now_unix: u64,
    pin: &WitnessTrust,
) -> Result<VerifiedWitness, WitnessTrustError> {
    let quote = hex::decode(evidence.quote_hex.trim())
        .map_err(|_| WitnessTrustError::WitnessQuoteUnverified)?;
    let verified = verify_witness_quote(&quote, collateral, now_unix)?;

    check_quote(&verified, nonce, pin)?;

    // The one construction site in this crate. See the module docs, and
    // `there_is_no_way_to_build_a_verified_witness_but_verification`.
    Ok(VerifiedWitness {
        url: url.to_string(),
        signing_address: pin.signing_address.clone(),
    })
}

/// The checks, in order, each with its own error variant.
///
/// Separated from [`verify_witness`] so the checks are testable against a
/// `VerifiedQuote` constructed directly -- its fields are public -- without a
/// real quote, real collateral or a network. That is what lets the negative
/// cases below be exhaustive rather than aspirational.
///
/// **Order is load-bearing.** The signer is checked before the measurement:
/// a quote naming a machine that did not sign says nothing about the
/// measurement of the machine that did, so reporting a measurement mismatch
/// for it would send an operator to the wrong image.
pub fn check_quote(
    quote: &VerifiedQuote,
    nonce: &WitnessNonce,
    pin: &WitnessTrust,
) -> Result<(), WitnessTrustError> {
    // Reconstructed whole and compared in one shot, not field by field. A
    // field-by-field check has to decide what to do about the bytes it did
    // not name, and the honest answer -- they must be zero -- is one a
    // reconstruction gets for free and a sequence of slice comparisons gets
    // only if somebody remembers.
    let Some(address) = decode_address(&pin.signing_address) else {
        // A malformed pin is not a quote failure. It is reported as an
        // unexpected signer rather than silently admitting anything, because
        // the client cannot tell whose machine this is without a pin to
        // compare against.
        return Err(WitnessTrustError::WitnessSignerUnexpected);
    };

    let mut expected = [0u8; WITNESS_REPORT_DATA_LEN];
    expected[..WITNESS_QUOTE_DOMAIN.len()].copy_from_slice(WITNESS_QUOTE_DOMAIN);
    expected[WITNESS_ADDRESS_AT..WITNESS_ADDRESS_AT + address.len()].copy_from_slice(&address);
    expected[WITNESS_NONCE_AT..WITNESS_NONCE_AT + WITNESS_NONCE_LEN]
        .copy_from_slice(nonce.as_bytes());

    if quote.report_data != expected {
        // Which half is wrong decides the error, because the two mean
        // different things to a contributor: a wrong nonce is a replay, and a
        // wrong address is somebody else's machine. The nonce is read first
        // because a replayed quote for the *right* machine is still a replay.
        let carries_our_nonce = quote
            .report_data
            .get(WITNESS_NONCE_AT..WITNESS_NONCE_AT + WITNESS_NONCE_LEN)
            == Some(nonce.as_bytes());
        return Err(if carries_our_nonce {
            WitnessTrustError::WitnessSignerUnexpected
        } else {
            WitnessTrustError::WitnessQuoteReplayed
        });
    }

    // Any pinned set matching is a pass. Nothing configured is a refusal.
    let mut last_reported = None;
    for expected_set in &pin.measurements {
        match check_measurements_opt(
            Some(expected_set),
            quote,
            WITNESS_EXPECTED_MEASUREMENT_CONTROL,
        ) {
            MeasurementVerdict::Pinned { .. } => return Ok(()),
            verdict @ MeasurementVerdict::Mismatch { .. } => {
                last_reported = Some(verdict.to_string());
            }
            MeasurementVerdict::Refused { .. } => {}
        }
    }

    Err(WitnessTrustError::WitnessMeasurementUnpinned {
        control: WITNESS_EXPECTED_MEASUREMENT_CONTROL,
        // `None` when nothing was pinned, which is what distinguishes an
        // unconfigured client from one whose pin did not match.
        reported: last_reported,
    })
}

// Kept below check_quote so the construction-site guard covers both public guards.
#[cfg(not(test))]
fn verify_witness_quote(
    quote: &[u8],
    collateral: &Collateral,
    now: u64,
) -> Result<VerifiedQuote, WitnessTrustError> {
    verify_quote(quote, collateral, now).map_err(|_| WitnessTrustError::WitnessQuoteUnverified)
}

#[cfg(test)]
fn quote_fixtures() -> &'static std::sync::Mutex<std::collections::HashMap<Vec<u8>, VerifiedQuote>>
{
    static FIXTURES: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<Vec<u8>, VerifiedQuote>>,
    > = std::sync::OnceLock::new();
    FIXTURES.get_or_init(Default::default)
}

/// One-shot DCAP-only fixture. Random quote bytes isolate concurrent exchanges;
/// real nonce, address and measurement checks still run in verify_witness.
#[cfg(test)]
pub(crate) fn register_quote_fixture(quote: VerifiedQuote) -> QuoteFixture {
    let key = uuid::Uuid::new_v4().as_bytes().to_vec();
    assert!(
        quote_fixtures()
            .lock()
            .unwrap()
            .insert(key.clone(), quote)
            .is_none()
    );
    QuoteFixture(key)
}

#[cfg(test)]
pub(crate) struct QuoteFixture(pub(crate) Vec<u8>);

#[cfg(test)]
impl Drop for QuoteFixture {
    fn drop(&mut self) {
        quote_fixtures().lock().unwrap().remove(&self.0);
    }
}

#[cfg(test)]
fn verify_witness_quote(
    quote: &[u8],
    collateral: &Collateral,
    now: u64,
) -> Result<VerifiedQuote, WitnessTrustError> {
    if let Some(verified) = quote_fixtures().lock().unwrap().remove(quote) {
        return Ok(verified);
    }
    verify_quote(quote, collateral, now).map_err(|_| WitnessTrustError::WitnessQuoteUnverified)
}

/// A `VerifiedWitness` for tests in sibling modules.
///
/// `#[cfg(test)]`, so it exists in no shipped artifact. It is here rather than
/// absent because the alternative is worse: `witness_contribution` cannot be
/// tested at all without one, DCAP verification cannot be satisfied without a
/// real Intel-signed quote, and an untested send path is not a safer trade
/// than a test-only constructor.
///
/// `there_is_no_way_to_build_a_verified_witness_but_verification` counts
/// constructors in the production half of this file only, and asserts that the
/// split kept both public functions -- so this cannot grow into a production
/// back door without that test failing.
#[cfg(test)]
pub(crate) fn verified_witness_for_test(url: &str, signing_address: &str) -> VerifiedWitness {
    VerifiedWitness {
        url: url.to_string(),
        signing_address: signing_address.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_attestation::measurements::ExpectedMeasurements;

    const ADDRESS: &str = "0x1111111111111111111111111111111111111111";
    const OTHER_ADDRESS: &str = "0x2222222222222222222222222222222222222222";
    const MRTD: &str = "aa";
    const MRCONFIGID: &str = "bb";
    const MRCONFIGID_NEXT: &str = "cc";
    const MRCONFIGID_STRANGER: &str = "dd";

    fn measurement(byte: &str) -> String {
        byte.repeat(48)
    }

    fn our_nonce() -> WitnessNonce {
        WitnessNonce::from_bytes([0x5au8; WITNESS_NONCE_LEN])
    }

    fn other_nonce() -> WitnessNonce {
        WitnessNonce::from_bytes([0xa5u8; WITNESS_NONCE_LEN])
    }

    /// A quote built directly. `VerifiedQuote`'s fields are public, so none of
    /// these tests needs a real quote, real collateral, or a network.
    fn quote_for(
        address: &str,
        nonce: &WitnessNonce,
        mrtd: &str,
        mrconfigid: &str,
    ) -> VerifiedQuote {
        let mut report_data = [0u8; WITNESS_REPORT_DATA_LEN];
        report_data[..8].copy_from_slice(WITNESS_QUOTE_DOMAIN);
        report_data[WITNESS_ADDRESS_AT..WITNESS_ADDRESS_AT + 20]
            .copy_from_slice(&decode_address(address).expect("a test address"));
        report_data[WITNESS_NONCE_AT..WITNESS_NONCE_AT + WITNESS_NONCE_LEN]
            .copy_from_slice(nonce.as_bytes());
        VerifiedQuote {
            report_data: report_data.to_vec(),
            mrtd: measurement(mrtd),
            mr_config_id: measurement(mrconfigid),
            rtmr: [
                measurement("00"),
                measurement("00"),
                measurement("00"),
                measurement("ff"),
            ],
            tcb_status: "UpToDate".into(),
            advisory_ids: Vec::new(),
        }
    }

    fn set(mrtd: &str, mrconfigid: &str) -> ExpectedMeasurements {
        ExpectedMeasurements::from_env_value(Some(&format!(
            "mrtd={},mrconfigid={}",
            measurement(mrtd),
            measurement(mrconfigid)
        )))
        .expect("the fixture parses")
        .expect("the fixture pins something")
    }

    fn trust() -> WitnessTrust {
        trust_with(&[(MRTD, MRCONFIGID)])
    }

    fn trust_with(sets: &[(&str, &str)]) -> WitnessTrust {
        WitnessTrust {
            signing_address: ADDRESS.to_string(),
            measurements: sets.iter().map(|(a, b)| set(a, b)).collect(),
        }
    }

    fn no_pins() -> WitnessTrust {
        WitnessTrust {
            signing_address: ADDRESS.to_string(),
            measurements: Vec::new(),
        }
    }

    #[test]
    fn dcap_fixture_is_one_shot_and_still_checks_the_exchange_nonce() {
        let collateral = trace_commons_attestation::quote::parse_collateral(include_str!(
            "../../../trace-commons-attestation/tests/fixtures/near_ai_attestation_collateral.json"
        ))
        .unwrap();
        let fixture = register_quote_fixture(quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID));
        let evidence = AttestationEvidence {
            quote_hex: hex::encode(&fixture.0),
            signing_address: ADDRESS.into(),
        };
        assert!(
            verify_witness(
                "https://fixture.invalid",
                &evidence,
                &collateral,
                &other_nonce(),
                1,
                &trust()
            )
            .is_err()
        );
        assert!(!quote_fixtures().lock().unwrap().contains_key(&fixture.0));
        assert!(
            verify_witness(
                "https://fixture.invalid",
                &evidence,
                &collateral,
                &our_nonce(),
                1,
                &trust()
            )
            .is_err()
        );
        let abandoned = register_quote_fixture(quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID));
        let key = abandoned.0.clone();
        drop(abandoned);
        assert!(!quote_fixtures().lock().unwrap().contains_key(&key));
    }

    #[test]
    fn a_correctly_bound_quote_from_a_pinned_image_passes() {
        // The positive control. Without it every refusal test below would
        // pass on a `check_quote` that refused unconditionally.
        check_quote(
            &quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID),
            &our_nonce(),
            &trust(),
        )
        .expect("a quote bound to our nonce from a pinned image is accepted");
    }

    #[test]
    fn a_quote_bound_to_someone_elses_nonce_is_a_replay() {
        let err = check_quote(
            &quote_for(ADDRESS, &other_nonce(), MRTD, MRCONFIGID),
            &our_nonce(),
            &trust(),
        )
        .expect_err("a quote that does not carry our nonce proves nothing about now");
        assert_eq!(err, WitnessTrustError::WitnessQuoteReplayed);
    }

    #[test]
    fn an_unpinned_client_refuses_and_names_the_missing_control() {
        let err = check_quote(
            &quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID),
            &our_nonce(),
            &no_pins(),
        )
        .expect_err("no pin configured is a refusal, not a pass");
        assert_eq!(
            err,
            WitnessTrustError::WitnessMeasurementUnpinned {
                control: WITNESS_EXPECTED_MEASUREMENT_CONTROL,
                reported: None,
            }
        );
        assert_eq!(err.refusal_label(), "witness_expected_measurement");
    }

    #[test]
    fn a_second_pinned_set_admits_an_upgrade_without_admitting_a_stranger() {
        let trust = trust_with(&[(MRTD, MRCONFIGID), (MRTD, MRCONFIGID_NEXT)]);
        check_quote(
            &quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID_NEXT),
            &our_nonce(),
            &trust,
        )
        .expect("the new measurement was allowlisted before the rollout");
        // And the old one still passes, which is what makes the upgrade
        // window survivable rather than a cutover.
        check_quote(
            &quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID),
            &our_nonce(),
            &trust,
        )
        .expect("the outgoing measurement is still admitted during the roll");

        let err = check_quote(
            &quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID_STRANGER),
            &our_nonce(),
            &trust,
        )
        .expect_err("a set that only ever grows stops being a pin");
        match err {
            WitnessTrustError::WitnessMeasurementUnpinned { control, reported } => {
                assert_eq!(control, WITNESS_EXPECTED_MEASUREMENT_CONTROL);
                let reported = reported.expect("a mismatch reports what it saw");
                assert!(
                    reported.contains(&measurement(MRCONFIGID_STRANGER)),
                    "the refusal must name the measurement an operator would allowlist: {reported}"
                );
            }
            other => panic!("expected an unpinned measurement, got {other:?}"),
        }
    }

    #[test]
    fn rtmr3_drift_does_not_fail_a_pin_on_mrtd_and_mrconfigid() {
        // Two instances of byte-identical code differ in RTMR3, which carries
        // a per-deployment random instance-id. Pinning it would fail closed on
        // the second replica; this test is what says we do not.
        let mut second = quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID);
        second.rtmr[3] = measurement("ab");
        check_quote(&second, &our_nonce(), &trust()).expect("rtmr3 is not pinned");

        // Same for rtmr0, which hashes SMBIOS tables that move on a VM
        // resize.
        let mut resized = quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID);
        resized.rtmr[0] = measurement("cd");
        check_quote(&resized, &our_nonce(), &trust()).expect("rtmr0 is not pinned");
    }

    #[test]
    fn a_quote_naming_another_signer_is_refused_as_an_unexpected_signer() {
        let err = check_quote(
            &quote_for(OTHER_ADDRESS, &our_nonce(), MRTD, MRCONFIGID),
            &our_nonce(),
            &trust(),
        )
        .expect_err("a quote for a machine that did not sign proves nothing");
        assert_eq!(err, WitnessTrustError::WitnessSignerUnexpected);
    }

    #[test]
    fn a_wrong_domain_tag_is_refused_rather_than_ignored() {
        // The bytes before the address are compared too. A check that only
        // looked at the address and the nonce would accept a quote bound
        // under some other protocol's domain, which is exactly what the tag
        // exists to prevent.
        let mut foreign = quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID);
        foreign.report_data[..8].copy_from_slice(b"tcwitns2");
        let err = check_quote(&foreign, &our_nonce(), &trust())
            .expect_err("a quote under another domain tag is not ours");
        assert_eq!(err, WitnessTrustError::WitnessSignerUnexpected);
    }

    #[test]
    fn trailing_bytes_after_the_nonce_must_be_zero() {
        // The four bytes the layout leaves zero. A field-by-field check would
        // not look at them; the whole-buffer comparison does, and this is what
        // says so.
        let mut padded = quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID);
        padded.report_data[60] = 0x01;
        let err = check_quote(&padded, &our_nonce(), &trust())
            .expect_err("bytes outside the layout are not ignored");
        assert_eq!(err, WitnessTrustError::WitnessSignerUnexpected);
    }

    #[test]
    fn a_short_report_data_is_refused() {
        let mut truncated = quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID);
        truncated.report_data.truncate(32);
        let err = check_quote(&truncated, &our_nonce(), &trust())
            .expect_err("a report body that is not 64 bytes is not this layout");
        assert_eq!(err, WitnessTrustError::WitnessQuoteReplayed);
    }

    #[test]
    fn a_malformed_pin_admits_nothing() {
        let trust = WitnessTrust {
            signing_address: "not-an-address".to_string(),
            measurements: vec![set(MRTD, MRCONFIGID)],
        };
        let err = check_quote(
            &quote_for(ADDRESS, &our_nonce(), MRTD, MRCONFIGID),
            &our_nonce(),
            &trust,
        )
        .expect_err("a client that cannot read its own pin must not accept anything");
        assert_eq!(err, WitnessTrustError::WitnessSignerUnexpected);
    }

    #[test]
    fn the_report_data_layout_matches_the_witness_service() {
        // These constants are duplicated across the AGPL/permissive boundary,
        // so they are pinned to literals here rather than to each other. If
        // `witness_service::enclave` moves one, its own tests fail there and
        // this one fails here, which is the most a split-licensed tree can do.
        assert_eq!(WITNESS_QUOTE_DOMAIN, b"tcwitns1");
        assert_eq!(WITNESS_ADDRESS_AT, 8);
        assert_eq!(WITNESS_NONCE_AT, 28);
        assert_eq!(WITNESS_NONCE_LEN, 32);
        assert_eq!(WITNESS_REPORT_DATA_LEN, 64);
        // The address occupies 8..28 and the nonce 28..60, so they abut
        // exactly and nothing is unaccounted for before byte 60.
        assert_eq!(WITNESS_ADDRESS_AT + 20, WITNESS_NONCE_AT);
    }

    #[test]
    fn there_is_no_way_to_build_a_verified_witness_but_verification() {
        // Structural, not behavioural: this is the guard the whole design
        // rests on, and no behavioural test can see a constructor that
        // nothing yet calls. Asserted by reading this module's own source,
        // which is the only thing that can.
        let source = include_str!("verify.rs");
        // Only the production half. This module's own tests construct the
        // type directly -- they have to, to assert what `Debug` withholds --
        // and counting those would make the assertion track the test suite
        // rather than the code it guards.
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(before, _)| before)
            .expect("this file has a test module");
        // Both public functions must be on the production side, or the split
        // cut early and the count below would be vacuous. `check_quote` is
        // declared after `verify_witness` and before the `#[cfg(test)]`
        // constructor, so requiring both pins the split point.
        assert!(
            production.contains("pub fn verify_witness"),
            "the split found no production code, so the count below is vacuous"
        );
        assert!(
            production.contains("pub fn check_quote"),
            "the split cut before check_quote, so the count below is vacuous"
        );
        // And the test-only constructor must really be test-only.
        assert!(
            !production.contains("verified_witness_for_test"),
            "the test-only constructor escaped into production code"
        );
        // `struct VerifiedWitness {` is the definition and `impl ...
        // VerifiedWitness {` opens a block; neither builds a value. Anything
        // else opening a `VerifiedWitness { ... }` literal does.
        let constructors = production
            .lines()
            .filter(|line| line.contains("VerifiedWitness {"))
            .filter(|line| {
                let trimmed = line.trim_start();
                !trimmed.starts_with("impl ")
                    && !trimmed.starts_with("struct ")
                    && !trimmed.starts_with("pub struct ")
                    && !trimmed.starts_with("//")
            })
            .count();
        assert_eq!(
            constructors, 1,
            "a second constructor bypasses the verification"
        );
    }

    #[test]
    fn a_verified_witness_debug_withholds_the_url() {
        let witness = VerifiedWitness {
            url: "https://witness.example".to_string(),
            signing_address: ADDRESS.to_string(),
        };
        let rendered = format!("{witness:?}");
        assert!(!rendered.contains("witness.example"));
        assert!(!rendered.contains(ADDRESS));
    }
}
