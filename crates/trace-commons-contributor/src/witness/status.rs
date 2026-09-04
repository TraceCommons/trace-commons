//! What a shell may show about the witness, and what happened last time.
//!
//! The witness is reachable today only by hand-editing a config file or
//! setting three environment variables. This module is the reading and
//! writing surface the three app shells drive through
//! `trace-commons-contributor-ffi`, and it exists mostly to stop one
//! particular bug.
//!
//! # The conflation this module exists to prevent
//!
//! There are two states a shell could easily render with the same words, and
//! they are opposites:
//!
//! - **No witness configured.** Local redaction runs, byte for byte as it
//!   always has. This is a legitimate, supported mode. Nothing is wrong.
//! - **A witness configured with nothing pinned.** Every submission is
//!   REFUSED, before any network call. Nothing gets uploaded at all.
//!
//! A boolean -- `witness_enabled`, `has_witness`, anything of that shape --
//! collapses those two into one bit and guarantees the bug, because the
//! second state answers "is a witness configured?" with yes and then behaves
//! like a total outage. So there is no boolean anywhere in this module.
//! [`WitnessTrustState`] is the only answer, it has one variant per
//! condition, and the refusing conditions share a `Refusing` prefix rather
//! than a shared "not working" catch-all.
//!
//! # Room for the attested-inference work
//!
//! A witness may come to refuse a submission because a trace's inferences
//! did not carry verified receipts. That is a different instruction to a
//! contributor than "pin a measurement", so it gets its own state
//! ([`WitnessTrustState::RefusingInferenceReceiptsMissing`]) and its own
//! refusal label, rather than being folded into the unpinned case. A
//! certificate may also come to carry a count of how many of a trace's
//! inferences carried a verified receipt; [`InferenceReceiptCount`] is where
//! that arrives, and it is read out of the certificate leniently so this
//! build tolerates a certificate that has it and a certificate that does
//! not.
//!
//! # What a count is, and what it is not
//!
//! `n_of_m` is a count of inferences that carried a verified receipt. It is
//! never the word "attested" on a surface, and it never says a trace is
//! clean: a certificate attests mechanics and a verdict, and nothing else.

use std::sync::Mutex;

use serde::Serialize;

use crate::config::ContributorConfig;
use crate::witness::WITNESS_EXPECTED_MEASUREMENT_CONTROL;

/// Whether the client will use a witness, and if not, why not.
///
/// One variant per condition, deliberately. See the module docs for why
/// there is no boolean here and must never be one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WitnessTrustState {
    /// No witness is configured. Local redaction runs, exactly as it does
    /// with this feature absent. **Not a degraded state and not an error.**
    Absent,
    /// A witness is configured and at least one measurement set is pinned.
    /// Submissions go through it.
    Pinned,
    /// A witness is configured and nothing is pinned. **Every submission is
    /// refused**, before any network call, because a client with no pin
    /// cannot judge any quote it receives. The fix is to pin the measurement
    /// the deployment reports.
    RefusingUnpinned,
    /// A witness is configured and its pinned measurements could not be
    /// parsed. Also a total refusal. A different mistake from
    /// [`Self::RefusingUnpinned`] -- a contributor who mistyped a
    /// measurement should not be told they pinned none.
    RefusingPinMalformed,
    /// A witness is configured and pinned, and refuses this client's
    /// submissions because their inferences did not carry verified receipts.
    ///
    /// **Reserved.** No code path produces this today; it is declared so the
    /// attested-inference work can start returning it without moving any
    /// other discriminant, and so a shell written now has a branch for it.
    RefusingInferenceReceiptsMissing,
    /// This device is not enrolled, so there is no configuration to hold a
    /// witness.
    ///
    /// Not produced by [`witness_status`], which is handed a config that
    /// already exists. It is here so that the shells have ONE state type and
    /// one sentence table covering every case a witness card can be in:
    /// splitting the two "there is no config" cases into a second enum would
    /// hand a shell two vocabularies and let it decide for itself that this
    /// one looks like [`Self::Absent`].
    NotEnrolled,
    /// The configuration could not be read, so what happens to a session is
    /// unknown. **A refusal**: nothing goes out from a client that cannot
    /// read its own settings.
    SettingsUnreadable,
}

impl WitnessTrustState {
    /// True when this state means submissions do not go out at all.
    ///
    /// [`Self::Absent`] is deliberately **not** refusing: nothing is wrong
    /// with a client that has no witness.
    pub fn is_refusing(self) -> bool {
        match self {
            // Not enrolled is not a refusal: nothing about a witness is
            // being declined, the device simply has no account yet.
            Self::Absent | Self::Pinned | Self::NotEnrolled => false,
            Self::RefusingUnpinned
            | Self::RefusingPinMalformed
            | Self::RefusingInferenceReceiptsMissing
            | Self::SettingsUnreadable => true,
        }
    }

    /// The stable integer this state travels as across the C ABI.
    ///
    /// Values are frozen. A new state takes the next unused number; none of
    /// these ever move, because a shell compiled against an older header
    /// still holds the old numbers.
    pub fn abi_code(self) -> i32 {
        match self {
            Self::Absent => 0,
            Self::Pinned => 1,
            Self::RefusingUnpinned => 2,
            Self::RefusingPinMalformed => 3,
            Self::RefusingInferenceReceiptsMissing => 4,
            Self::NotEnrolled => -1,
            Self::SettingsUnreadable => -2,
        }
    }

    /// The state an ABI code names, or `None` for a code this build has
    /// never heard of.
    ///
    /// `None` rather than a default. A shell handing over a number from a
    /// newer library must be told this build cannot name it, not given the
    /// nearest state -- and the nearest state is always the one that claims
    /// too little, since every value added later is a condition this build
    /// has no sentence for.
    pub fn from_abi_code(code: i32) -> Option<Self> {
        Some(match code {
            0 => Self::Absent,
            1 => Self::Pinned,
            2 => Self::RefusingUnpinned,
            3 => Self::RefusingPinMalformed,
            4 => Self::RefusingInferenceReceiptsMissing,
            -1 => Self::NotEnrolled,
            -2 => Self::SettingsUnreadable,
            _ => return None,
        })
    }

    /// The refusal label a shell can show, or `None` when nothing is being
    /// refused.
    ///
    /// The unpinned label is the same constant the submission path reports,
    /// so a contributor greps one word rather than two spellings of the same
    /// condition.
    pub fn refusal_label(self) -> Option<&'static str> {
        match self {
            Self::Absent | Self::Pinned => None,
            Self::RefusingUnpinned => Some(WITNESS_EXPECTED_MEASUREMENT_CONTROL),
            Self::RefusingPinMalformed => Some("witness_expected_measurement_malformed"),
            Self::RefusingInferenceReceiptsMissing => Some("witness_inference_receipts_missing"),
            Self::NotEnrolled => None,
            Self::SettingsUnreadable => Some("witness_settings_unreadable"),
        }
    }
}

/// The whole witness configuration a settings screen may show.
///
/// Deliberately has no `is_configured()`: see the module docs. Read
/// [`Self::state`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WitnessStatus {
    /// The only answer to "what is the witness doing".
    pub state: WitnessTrustState,
    /// The configured base URL, or `None` when nothing is configured.
    ///
    /// This is one of the few values this crate hands a shell verbatim. A
    /// settings screen that will not show what it is asking a contributor to
    /// trust with their raw session is not a settings screen. The
    /// contributor typed it; it is not a secret and it is not derived from
    /// session content.
    pub url: Option<String>,
    /// The address whose signature this client accepts on a certificate.
    pub signing_address: Option<String>,
    /// How many measurement sets are pinned. Zero with a `url` present is
    /// [`WitnessTrustState::RefusingUnpinned`], never a pass.
    ///
    /// Exactly `pinned_measurements.len()`, always. It exists as its own
    /// field because it shipped first and three shells read it; the two can
    /// never disagree, and `the_count_is_the_length_of_the_list` says so.
    pub pinned_measurement_count: usize,
    /// The pinned measurement sets themselves, **verbatim**, in the order
    /// they are stored.
    ///
    /// # Why the entries and not just the count
    ///
    /// Without them the editor on every shell is write-only: reconfiguring
    /// a pinned witness means retyping every pin from memory, and an empty
    /// box is indistinguishable from a deliberately cleared one -- so a
    /// shell either refuses a contributor who only wanted to change the URL,
    /// or grows a "keep what is there" mode that silently saves something
    /// nobody looked at. Read-back removes the need for both.
    ///
    /// # Verbatim, and why that matters
    ///
    /// These are the strings `WitnessSettings::expected_measurements` holds,
    /// unparsed and unreformatted, which is exactly what
    /// `measurements_json` takes. A shell pre-fills its editor from this
    /// list and hands it straight back; it never re-serialises an
    /// `ExpectedMeasurements`, because a shell that reformats a pin is a
    /// shell that can reformat it wrongly.
    ///
    /// # A malformed entry is returned as it is stored
    ///
    /// Not omitted, and not a refusal to read. The state is already
    /// [`WitnessTrustState::RefusingPinMalformed`], which says the pin
    /// cannot be parsed; the entry is returned so the contributor can SEE
    /// the typo and fix it. Omitting it would silently delete their work the
    /// next time they saved, and refusing the read would leave them with a
    /// witness that refuses every submission and no way to look at why.
    pub pinned_measurements: Vec<String>,
}

impl WitnessStatus {
    /// The sentence for how many measurements are pinned, or `None` when
    /// there is no witness to count for.
    ///
    /// Lives here rather than on a shell because two shells asked for it and
    /// both declined to write one, and a bare numeral on a privacy surface
    /// is a shell inventing wording by omission. `None` for
    /// [`WitnessTrustState::Absent`] and [`WitnessTrustState::NotEnrolled`]:
    /// a count of the pins on a witness that does not exist is not a shorter
    /// sentence, it is a wrong one. `None` too for
    /// [`WitnessTrustState::SettingsUnreadable`], where the count is not
    /// known at all.
    pub fn pinned_measurement_line(&self) -> Option<String> {
        match self.state {
            WitnessTrustState::Absent
            | WitnessTrustState::NotEnrolled
            | WitnessTrustState::SettingsUnreadable => None,
            WitnessTrustState::Pinned
            | WitnessTrustState::RefusingUnpinned
            | WitnessTrustState::RefusingPinMalformed
            | WitnessTrustState::RefusingInferenceReceiptsMissing => Some(
                crate::witness_copy::witness_pinned_count_line(self.pinned_measurement_count),
            ),
        }
    }
}

/// Read the witness configuration out of a contributor config.
///
/// The single place the config is turned into a state, so the FFI, the CLI
/// and any future surface cannot each decide for themselves what an
/// unparsable pin means.
pub fn witness_status(cfg: &ContributorConfig) -> WitnessStatus {
    let Some(settings) = cfg.witness.as_ref() else {
        return WitnessStatus {
            state: WitnessTrustState::Absent,
            url: None,
            signing_address: None,
            pinned_measurement_count: 0,
            pinned_measurements: Vec::new(),
        };
    };
    let state = match settings.trust() {
        Ok(trust) if trust.is_pinned() => WitnessTrustState::Pinned,
        Ok(_) => WitnessTrustState::RefusingUnpinned,
        Err(_) => WitnessTrustState::RefusingPinMalformed,
    };
    WitnessStatus {
        state,
        url: Some(settings.url.clone()),
        signing_address: Some(settings.signing_address.clone()),
        // The count of what is CONFIGURED, not of what parsed: a malformed
        // pin reports the entries the contributor wrote, so a settings
        // screen can say "three pins, one of them unreadable" rather than
        // "no pins", which would read as the unpinned refusal instead.
        pinned_measurement_count: settings.expected_measurements.len(),
        // Cloned verbatim. Not parsed and re-emitted: the value that goes
        // back through `measurements_json` must be the value that is stored,
        // or a round trip through a settings screen quietly rewrites a pin.
        pinned_measurements: settings.expected_measurements.clone(),
    }
}

/// How many of a trace's inferences carried a verified receipt, out of how
/// many inferences it had.
///
/// Reported as a pair, always. There is no boolean form of this and there
/// must not be one: "attested" as a flag would be false on nearly every real
/// session while reading as an accusation, and true on a session with one
/// inference in it while reading as a guarantee.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InferenceReceiptCount {
    /// Inferences that carried a verified receipt.
    pub n: u32,
    /// Inferences in the trace.
    pub m: u32,
}

/// Read `n_of_m` out of a witness certificate, if it carries one.
///
/// Lenient by design: this build must accept a certificate from a witness
/// that emits the field and one from a witness that does not, and the field
/// is being added by other work in parallel. Both `{"n":3,"m":7}` and
/// `[3,7]` are accepted.
///
/// `n > m` is read as no count rather than as a count. A certificate
/// claiming more receipts than inferences is not a small error to render
/// carefully -- it is a certificate saying something impossible, and showing
/// it would put a nonsense claim on a contributor's screen.
pub fn n_of_m_from_certificate(certificate_json: &str) -> Option<InferenceReceiptCount> {
    let value: serde_json::Value = serde_json::from_str(certificate_json).ok()?;
    let field = value.get("n_of_m")?;
    let (n, m) = if let Some(array) = field.as_array() {
        if array.len() != 2 {
            return None;
        }
        (array[0].as_u64()?, array[1].as_u64()?)
    } else {
        (field.get("n")?.as_u64()?, field.get("m")?.as_u64()?)
    };
    if n > m || n > u64::from(u32::MAX) || m > u64::from(u32::MAX) {
        return None;
    }
    Some(InferenceReceiptCount {
        n: n as u32,
        m: m as u32,
    })
}

/// What the last submission this process made did about a witness.
///
/// Four outcomes, not a pair of booleans, for the same reason
/// [`WitnessTrustState`] is not a boolean: "no certificate" is the correct
/// and expected outcome of a client with no witness, and the alarming
/// outcome of a client with one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WitnessLastResult {
    /// This process has not observed a submission yet. Not "nothing
    /// happened" -- a shell that has just started has no business claiming
    /// anything about the last submission.
    NotObserved,
    /// The last submission was built by local redaction, because no witness
    /// was configured. Expected, not a downgrade.
    LocalRedaction,
    /// A certificate was obtained and verified against the bytes the witness
    /// returned.
    ///
    /// The certificate covers redaction mechanics and a residual-risk
    /// verdict. It does not say the trace is clean, and no surface built on
    /// this variant may say that it does.
    Certified {
        /// Present only when the certificate carried the field.
        n_of_m: Option<InferenceReceiptCount>,
    },
    /// The last submission was refused. `label` is the fixed refusal label
    /// the submission path reported.
    Refused {
        label: String,
        /// Whether a certificate was in hand when the refusal happened. A
        /// witness that answered with a certificate that does not cover its
        /// own artifact is a different problem from a witness that never
        /// answered, and only this field tells them apart.
        certificate_obtained: bool,
    },
}

impl WitnessLastResult {
    /// The JSON a shell reads. Built here rather than derived, so the object
    /// is flat and total: every key is present in every outcome, and a shell
    /// never has to decide what a missing key meant.
    pub fn to_json(&self) -> serde_json::Value {
        let (outcome, obtained, verified, refusal, n_of_m) = match self {
            Self::NotObserved => ("not_observed", false, false, None, None),
            Self::LocalRedaction => ("local_redaction", false, false, None, None),
            Self::Certified { n_of_m } => ("certified", true, true, None, *n_of_m),
            Self::Refused {
                label,
                certificate_obtained,
            } => ("refused", *certificate_obtained, false, Some(label), None),
        };
        serde_json::json!({
            "outcome": outcome,
            "certificate_obtained": obtained,
            "certificate_verified": verified,
            "refusal": refusal,
            "n_of_m": n_of_m,
        })
    }
}

/// Whether a refusal label means a certificate was in hand when the refusal
/// happened.
///
/// Two labels, both produced by checks that run only on a certificate this
/// client already holds. Everything else refused before one existed, and the
/// default is therefore `false` -- claiming a certificate was obtained when
/// none was is the direction that misleads.
pub fn certificate_obtained_for(label: &str) -> bool {
    matches!(
        label,
        "witness_certificate_mismatched" | "witness_certificate_unverified"
    )
}

/// The last observed result, for this process only.
///
/// Deliberately **not** persisted. A file would outlive a logout, and
/// `ConfigStore::wipe` would have to learn about it or the next contributor
/// to enroll on this machine would be shown the previous one's submission
/// outcome. The cost is that a freshly started shell reports
/// [`WitnessLastResult::NotObserved`] until it makes a submission, which is
/// the honest answer for a shell that has not seen one.
static LAST_RESULT: Mutex<WitnessLastResult> = Mutex::new(WitnessLastResult::NotObserved);

/// Record what a submission did about the witness.
pub fn record_last_result(result: WitnessLastResult) {
    let mut slot = LAST_RESULT.lock().unwrap_or_else(|p| p.into_inner());
    *slot = result;
}

/// Read what the last submission this process made did about the witness.
pub fn last_result() -> WitnessLastResult {
    LAST_RESULT
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::WitnessSettings;
    use crate::witness::{WITNESS_EXPECTED_MEASUREMENT_CONTROL, WitnessTrustError};

    fn cfg_with(witness: Option<WitnessSettings>) -> ContributorConfig {
        ContributorConfig {
            inference_receipt_endpoint: None,
            schema_version: crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION.to_string(),
            issuer_url: "https://issuer.example".into(),
            ingest_url: "https://ingest.example".into(),
            audience: "trace-commons-ingest".into(),
            tenant_id: "tenant".into(),
            instance_id: "instance".into(),
            user_subject: "subject".into(),
            device_key_id: "device".into(),
            consent_scopes: vec![],
            pii_filter: None,
            allowed_hosts: None,
            display_handle: None,
            public_bio: None,
            public_since: None,
            witness,
        }
    }

    const EVERY_STATE: [WitnessTrustState; 7] = [
        WitnessTrustState::Absent,
        WitnessTrustState::Pinned,
        WitnessTrustState::RefusingUnpinned,
        WitnessTrustState::RefusingPinMalformed,
        WitnessTrustState::RefusingInferenceReceiptsMissing,
        WitnessTrustState::NotEnrolled,
        WitnessTrustState::SettingsUnreadable,
    ];

    /// A pinned measurement set in `ExpectedMeasurements`' own spelling.
    fn a_pin() -> String {
        format!("mrtd={}", "ab".repeat(48))
    }

    #[test]
    fn no_witness_is_absent_and_is_not_refusing() {
        let status = witness_status(&cfg_with(None));
        assert_eq!(status.state, WitnessTrustState::Absent);
        assert!(
            !status.state.is_refusing(),
            "a client with no witness redacts locally; nothing is being refused"
        );
        assert_eq!(status.url, None);
        assert_eq!(status.pinned_measurement_count, 0);
    }

    #[test]
    fn a_configured_witness_with_no_pin_is_a_refusal_not_an_absence() {
        let status = witness_status(&cfg_with(Some(WitnessSettings {
            admission_evidence: false,
            url: "https://witness.example".into(),
            signing_address: "0xabc".into(),
            expected_measurements: vec![],
        })));
        assert_eq!(status.state, WitnessTrustState::RefusingUnpinned);
        assert_ne!(
            status.state,
            WitnessTrustState::Absent,
            "an unpinned witness refuses every submission; rendering it as \
             'no witness' tells a contributor nothing is wrong during a total outage"
        );
        assert!(status.state.is_refusing());
        assert_eq!(
            status.state.refusal_label(),
            Some(WITNESS_EXPECTED_MEASUREMENT_CONTROL)
        );
    }

    #[test]
    fn a_malformed_pin_is_its_own_state() {
        let status = witness_status(&cfg_with(Some(WitnessSettings {
            admission_evidence: false,
            url: "https://witness.example".into(),
            signing_address: "0xabc".into(),
            expected_measurements: vec!["mrtd=not-hex".into()],
        })));
        assert_eq!(status.state, WitnessTrustState::RefusingPinMalformed);
        assert_ne!(status.state, WitnessTrustState::RefusingUnpinned);
        assert_eq!(
            status.pinned_measurement_count, 1,
            "the contributor wrote one pin; reporting zero would read as the unpinned refusal"
        );
    }

    #[test]
    fn the_count_is_the_length_of_the_list_in_every_state() {
        for measurements in [
            vec![],
            vec![a_pin()],
            vec![a_pin(), a_pin(), a_pin()],
            vec!["mrtd=not-hex".to_string()],
            vec![a_pin(), "mrtd=not-hex".to_string()],
        ] {
            let status = witness_status(&cfg_with(Some(WitnessSettings {
                admission_evidence: false,
                url: "https://witness.example".into(),
                signing_address: "0xabc".into(),
                expected_measurements: measurements.clone(),
            })));
            assert_eq!(
                status.pinned_measurement_count,
                status.pinned_measurements.len(),
                "the count and the list disagree for {measurements:?}, so a shell shown \
                 both is shown two different answers"
            );
        }
    }

    #[test]
    fn the_pinned_entries_come_back_exactly_as_stored() {
        // Verbatim is the whole contract: these strings go straight back
        // into `measurements_json`, so anything this function normalises is
        // something a settings screen would silently rewrite.
        let stored = vec![
            format!("{},mrconfigid={}", a_pin(), "cd".repeat(48)),
            a_pin(),
        ];
        let status = witness_status(&cfg_with(Some(WitnessSettings {
            admission_evidence: false,
            url: "https://witness.example".into(),
            signing_address: "0xabc".into(),
            expected_measurements: stored.clone(),
        })));
        assert_eq!(status.pinned_measurements, stored);
    }

    #[test]
    fn a_malformed_entry_is_returned_rather_than_hidden() {
        // The state already says the pin cannot be parsed. Returning the
        // entry is what lets a contributor SEE the typo; omitting it would
        // delete their work on the next save, and refusing the read would
        // leave them refusing every submission with nothing to look at.
        let status = witness_status(&cfg_with(Some(WitnessSettings {
            admission_evidence: false,
            url: "https://witness.example".into(),
            signing_address: "0xabc".into(),
            expected_measurements: vec![a_pin(), "mrtd=not-hex".into()],
        })));
        assert_eq!(status.state, WitnessTrustState::RefusingPinMalformed);
        assert_eq!(
            status.pinned_measurements,
            vec![a_pin(), "mrtd=not-hex".to_string()],
            "the unreadable entry is the one the contributor most needs to see"
        );
    }

    #[test]
    fn a_count_sentence_exists_only_where_there_is_something_to_count() {
        let absent = witness_status(&cfg_with(None));
        assert_eq!(
            absent.pinned_measurement_line(),
            None,
            "a count of the pins on a witness that does not exist is a wrong sentence, \
             not a short one"
        );

        let pinned = witness_status(&cfg_with(Some(WitnessSettings {
            admission_evidence: false,
            url: "https://witness.example".into(),
            signing_address: "0xabc".into(),
            expected_measurements: vec![a_pin(), a_pin()],
        })));
        assert_eq!(
            pinned.pinned_measurement_line().as_deref(),
            Some("2 measurements are pinned.")
        );

        let unpinned = witness_status(&cfg_with(Some(WitnessSettings {
            admission_evidence: false,
            url: "https://witness.example".into(),
            signing_address: "0xabc".into(),
            expected_measurements: vec![],
        })));
        assert_eq!(
            unpinned.pinned_measurement_line().as_deref(),
            Some("No measurement is pinned.")
        );

        for state in [
            WitnessTrustState::NotEnrolled,
            WitnessTrustState::SettingsUnreadable,
        ] {
            let status = WitnessStatus {
                state,
                url: None,
                signing_address: None,
                pinned_measurement_count: 0,
                pinned_measurements: Vec::new(),
            };
            assert_eq!(status.pinned_measurement_line(), None, "{state:?}");
        }
    }

    #[test]
    fn a_pinned_witness_reports_its_url_and_count() {
        let status = witness_status(&cfg_with(Some(WitnessSettings {
            admission_evidence: false,
            url: "https://witness.example".into(),
            signing_address: "0xabc".into(),
            expected_measurements: vec![a_pin(), a_pin()],
        })));
        assert_eq!(status.state, WitnessTrustState::Pinned);
        assert!(!status.state.is_refusing());
        assert_eq!(status.url.as_deref(), Some("https://witness.example"));
        assert_eq!(status.signing_address.as_deref(), Some("0xabc"));
        assert_eq!(status.pinned_measurement_count, 2);
    }

    #[test]
    fn the_abi_codes_are_frozen_and_distinct() {
        // These numbers are the ABI. A shell compiled against an older
        // header still holds them, so moving one silently changes what a
        // shipped app renders.
        assert_eq!(WitnessTrustState::Absent.abi_code(), 0);
        assert_eq!(WitnessTrustState::Pinned.abi_code(), 1);
        assert_eq!(WitnessTrustState::RefusingUnpinned.abi_code(), 2);
        assert_eq!(WitnessTrustState::RefusingPinMalformed.abi_code(), 3);
        assert_eq!(
            WitnessTrustState::RefusingInferenceReceiptsMissing.abi_code(),
            4
        );
        assert_eq!(WitnessTrustState::NotEnrolled.abi_code(), -1);
        assert_eq!(WitnessTrustState::SettingsUnreadable.abi_code(), -2);
    }

    #[test]
    fn every_code_round_trips_and_an_unknown_one_is_named_as_unknown() {
        for state in EVERY_STATE {
            assert_eq!(
                WitnessTrustState::from_abi_code(state.abi_code()),
                Some(state),
                "{state:?} does not survive the ABI round trip"
            );
        }
        // Not a default. A number this build cannot name must be reported as
        // unnameable, because every value added later is a condition with no
        // sentence here.
        assert_eq!(WitnessTrustState::from_abi_code(5), None);
        assert_eq!(WitnessTrustState::from_abi_code(-3), None);
        assert_eq!(WitnessTrustState::from_abi_code(i32::MAX), None);
    }

    #[test]
    fn an_unreadable_config_refuses_and_an_unenrolled_device_does_not() {
        assert!(
            WitnessTrustState::SettingsUnreadable.is_refusing(),
            "a client that cannot read its own settings sends nothing"
        );
        assert!(
            !WitnessTrustState::NotEnrolled.is_refusing(),
            "an unenrolled device is not declining anything about a witness"
        );
        assert_ne!(
            WitnessTrustState::SettingsUnreadable,
            WitnessTrustState::Absent
        );
        assert_ne!(WitnessTrustState::NotEnrolled, WitnessTrustState::Absent);
    }

    #[test]
    fn refusing_for_receipts_is_distinct_from_refusing_for_a_pin() {
        // The two refusals are different instructions to a contributor: pin
        // a measurement, versus run inference somewhere that issues
        // receipts. A single "the witness refused" state would tell them
        // neither.
        let receipts = WitnessTrustState::RefusingInferenceReceiptsMissing;
        let unpinned = WitnessTrustState::RefusingUnpinned;
        assert_ne!(receipts, unpinned);
        assert_ne!(receipts.abi_code(), unpinned.abi_code());
        assert_ne!(receipts.refusal_label(), unpinned.refusal_label());
        assert!(receipts.is_refusing() && unpinned.is_refusing());
    }

    #[test]
    fn the_certificate_refusal_labels_are_the_ones_the_error_type_reports() {
        // `certificate_obtained_for` matches on labels. If a variant is
        // renamed, this fails rather than silently reporting "no
        // certificate" for a certificate that was in hand.
        assert!(certificate_obtained_for(
            WitnessTrustError::WitnessCertificateMismatched.refusal_label()
        ));
        assert!(certificate_obtained_for(
            WitnessTrustError::WitnessCertificateUnverified.refusal_label()
        ));
        assert!(!certificate_obtained_for(
            WitnessTrustError::WitnessQuoteUnverified.refusal_label()
        ));
        assert!(!certificate_obtained_for(
            WitnessTrustError::WitnessAttestationUnavailable.refusal_label()
        ));
        assert!(!certificate_obtained_for("something-invented"));
    }

    #[test]
    fn a_certificate_without_the_field_reports_no_count() {
        assert_eq!(n_of_m_from_certificate(r#"{"redacted_sha256":"x"}"#), None);
        assert_eq!(n_of_m_from_certificate("not json at all"), None);
    }

    #[test]
    fn both_spellings_of_n_of_m_are_read() {
        assert_eq!(
            n_of_m_from_certificate(r#"{"n_of_m":{"n":3,"m":7}}"#),
            Some(InferenceReceiptCount { n: 3, m: 7 })
        );
        assert_eq!(
            n_of_m_from_certificate(r#"{"n_of_m":[3,7]}"#),
            Some(InferenceReceiptCount { n: 3, m: 7 })
        );
        assert_eq!(
            n_of_m_from_certificate(r#"{"n_of_m":{"n":0,"m":0}}"#),
            Some(InferenceReceiptCount { n: 0, m: 0 })
        );
    }

    #[test]
    fn a_count_claiming_more_receipts_than_inferences_is_not_a_count() {
        assert_eq!(n_of_m_from_certificate(r#"{"n_of_m":{"n":9,"m":2}}"#), None);
        assert_eq!(n_of_m_from_certificate(r#"{"n_of_m":[9,2]}"#), None);
        assert_eq!(n_of_m_from_certificate(r#"{"n_of_m":[1,2,3]}"#), None);
    }

    #[test]
    fn the_result_json_is_flat_and_total() {
        for result in [
            WitnessLastResult::NotObserved,
            WitnessLastResult::LocalRedaction,
            WitnessLastResult::Certified { n_of_m: None },
            WitnessLastResult::Refused {
                label: "witness_quote_unverified".into(),
                certificate_obtained: false,
            },
        ] {
            let json = result.to_json();
            for key in [
                "outcome",
                "certificate_obtained",
                "certificate_verified",
                "refusal",
                "n_of_m",
            ] {
                assert!(
                    json.get(key).is_some(),
                    "{key} missing from {json} -- a shell must never have to guess \
                     what an absent key meant"
                );
            }
        }
    }

    #[test]
    fn a_local_redaction_result_is_not_a_missing_certificate() {
        let local = WitnessLastResult::LocalRedaction.to_json();
        let never = WitnessLastResult::NotObserved.to_json();
        assert_eq!(local["certificate_obtained"], serde_json::json!(false));
        assert_ne!(
            local["outcome"], never["outcome"],
            "a submission that ran locally and a shell that has seen no submission \
             are different facts"
        );
        assert_eq!(local["outcome"], serde_json::json!("local_redaction"));
    }

    #[test]
    fn a_refusal_holding_a_certificate_says_so() {
        let json = WitnessLastResult::Refused {
            label: "witness_certificate_mismatched".into(),
            certificate_obtained: true,
        }
        .to_json();
        assert_eq!(json["certificate_obtained"], serde_json::json!(true));
        assert_eq!(json["certificate_verified"], serde_json::json!(false));
        assert_eq!(
            json["refusal"],
            serde_json::json!("witness_certificate_mismatched")
        );
    }

    #[test]
    fn a_certified_result_carries_the_count_through() {
        let json = WitnessLastResult::Certified {
            n_of_m: Some(InferenceReceiptCount { n: 3, m: 7 }),
        }
        .to_json();
        assert_eq!(json["n_of_m"], serde_json::json!({"n": 3, "m": 7}));
        assert_eq!(json["certificate_verified"], serde_json::json!(true));
    }
}
