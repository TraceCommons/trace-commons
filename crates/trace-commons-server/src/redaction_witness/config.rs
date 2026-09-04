// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Operator configuration for the witness PII-backstop bypass.
//!
//! What a configured bypass buys is narrow and worth stating before the
//! constants: a verified certificate keeps a submission **out of the
//! `AwaitingPiiBackstop` hold**, and nothing else. It changes no risk tier,
//! lifts no quarantine, and never means the trace is clean. The trailing
//! deterministic sweep -- `rescrub_trace_envelope` and the residual scan --
//! has already run synchronously on the submitted bytes before the hold is
//! decided, so a bypass here skips the backstop's *classifier* stage only.
//! That ordering is the whole safety argument; the ingest binary pins it with
//! a test.
//!
//! # Off by default, and fail-closed when on
//!
//! [`witness_bypass_config_from_values`] follows
//! [`crate::near_attestation::measurements::expected_measurements_from_env`]
//! exactly: `Ok(None)` means the switch is off, which is **not an acceptance
//! of anything**. There is deliberately no `Ok(None)` meaning "enabled but
//! unpinned" -- an enabled bypass missing any of its three controls is a
//! refusal naming the control, which the binary turns into a boot refusal.
//!
//! Three separate control names rather than one, because they send an
//! operator to three different lines of their config. Conflating them is a
//! real failure mode and the tests below forbid it.
//!
//! # Why the policy allowlist is its own control
//!
//! The allowlist is its own hole, not a second spelling of the pin. A
//! `deterministic-only` witness never ran a prose classifier; admitting its
//! alias means **no classifier ever reads that trace's prose** -- neither the
//! witness's nor the server's. Nothing in code can tell that apart from a
//! deliberate operator choice, so the only refusal available here is on
//! *emptiness*, and the danger is documented for operators in
//! `docs/operator/pii-backstop.md`. Only `full-pipeline` aliases belong in it.
//!
//! # Logging
//!
//! Nothing here logs. [`WitnessBypassConfigError`] carries only `&'static str`
//! control names and [`WitnessPinError`], both of which are safe under either
//! formatter; no environment value reaches an error string.

use std::collections::BTreeSet;

use super::verification::{
    DEFAULT_CERTIFICATE_MAX_AGE_SECONDS, EXPECTED_MEASUREMENT_CONTROL, WitnessFreshness,
    WitnessFreshnessError, WitnessPin, WitnessPinError,
};

/// Environment variable holding the master switch. Absent or anything other
/// than an affirmative value means the bypass is off.
pub const BYPASS_ENABLED_ENV: &str = "TRACE_COMMONS_WITNESS_BYPASS_ENABLED";

/// Environment variable holding the pinned witness signing address.
pub const SIGNING_ADDRESS_ENV: &str = "TRACE_COMMONS_WITNESS_SIGNING_ADDRESS";

/// Environment variable holding the comma-separated pinned measurement set.
pub const EXPECTED_MEASUREMENTS_ENV: &str = "TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS";

/// Environment variable holding the comma-separated redaction-policy alias
/// allowlist.
pub const ALLOWED_POLICY_VERSIONS_ENV: &str = "TRACE_COMMONS_WITNESS_ALLOWED_POLICY_VERSIONS";

/// Environment variable holding how many seconds a certificate stays
/// acceptable.
///
/// Unset means [`DEFAULT_CERTIFICATE_MAX_AGE_SECONDS`], not "no window". This
/// is the one control here that defaults to a value rather than to a refusal,
/// and deliberately: every other control names something only the operator
/// can know, while a replay window has a defensible default and an operator
/// who sets nothing should get it rather than an unbounded one. A value that
/// is present and unparseable is still a refusal -- an operator who typed
/// something meant something, and silently falling back to the default would
/// hide a window they believe they narrowed.
pub const CERTIFICATE_MAX_AGE_ENV: &str = "TRACE_COMMONS_WITNESS_CERTIFICATE_MAX_AGE_SECONDS";

/// Missing-control name reported when the bypass is enabled with no pinned
/// signing address.
pub const SIGNING_ADDRESS_CONTROL: &str = "witness_signing_address";

/// Missing-control name reported when the bypass is enabled with no non-blank
/// entry in the policy allowlist.
pub const POLICY_ALLOWLIST_CONTROL: &str = "witness_allowed_policy_versions";

/// Why an enabled bypass could not be configured.
///
/// Every variant is a refusal to construct, and the binary turns any of them
/// into a boot refusal. There is no partially configured bypass: a
/// `WitnessBypassConfig` that exists names a well-formed signing address, at
/// least one measurement, and at least one policy alias.
///
/// `Debug` delegates to `Display`, as everywhere else in this module, because
/// `tracing::error!(?err)` is how a boot refusal reaches a log here.
#[derive(Clone, PartialEq, Eq, thiserror::Error)]
pub enum WitnessBypassConfigError {
    /// The bypass is enabled and a required control is unset or blank.
    #[error("witness bypass refused: missing control {control}")]
    MissingControl { control: &'static str },
    /// A control is set but does not validate. Surfaces
    /// [`WitnessPin`]'s own variant rather than restating it: the pin is the
    /// authority on what a well-formed address and measurement set are, and a
    /// second opinion here could only disagree with it.
    #[error("witness bypass refused: {0}")]
    Pin(#[source] WitnessPinError),
    /// The certificate max age is set to something that is not a positive
    /// number of seconds.
    ///
    /// Carries no value. The operator's own configuration string is not
    /// contributor content, but it is not needed either: there is exactly one
    /// variable and one shape it has to take.
    #[error(
        "witness bypass refused: {CERTIFICATE_MAX_AGE_ENV} is not a positive number of seconds"
    )]
    CertificateMaxAgeMalformed,
    /// The certificate max age validates as a number but not as a window.
    #[error("witness bypass refused: {0}")]
    Freshness(#[source] WitnessFreshnessError),
}

impl std::fmt::Debug for WitnessBypassConfigError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Display::fmt(self, formatter)
    }
}

/// What the operator has decided to trust for the backstop bypass.
///
/// Holds a [`WitnessPin`] rather than re-validating its halves, and adds the
/// one thing the pin has no opinion about: which redaction-policy aliases a
/// certificate may carry. All three are here because the bypass is worth
/// nothing without any one of them.
///
/// Values are operator configuration. No contributor content reaches this
/// type, and `Debug` is derived for the same reason [`WitnessPin`]'s is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WitnessBypassConfig {
    pin: WitnessPin,
    allowed_policy_versions: BTreeSet<String>,
}

impl WitnessBypassConfig {
    /// The pin to hand [`verify_witness_certificate`](super::verification::verify_witness_certificate).
    pub fn pin(&self) -> &WitnessPin {
        &self.pin
    }

    /// Whether a certificate's redaction-policy alias is one the operator
    /// admits.
    ///
    /// Compared exactly, byte for byte, for the same reason measurements are:
    /// an alias is an opaque identifier a witness reports rather than a value
    /// with two circulating spellings, so a case-folding comparison could
    /// only conflate two distinct policies. A case difference against an
    /// honest witness fails closed, which is the safe direction.
    pub fn policy_version_allowed(&self, alias: &str) -> bool {
        self.allowed_policy_versions.contains(alias)
    }

    /// How many aliases this config admits. A caller reporting the strength
    /// of the check wants this; nothing else does.
    pub fn allowed_policy_version_count(&self) -> usize {
        self.allowed_policy_versions.len()
    }
}

/// Load the bypass configuration from the process environment.
///
/// A thin wrapper over [`witness_bypass_config_from_values`], which is where
/// the decisions live and where the tests exercise them: `std::env::set_var`
/// is process-wide and cannot be used under a parallel test runner. The same
/// split, for the same reason, is at
/// `near_attestation::measurements::expected_measurements_from_env`.
pub fn witness_bypass_config_from_env()
-> Result<Option<WitnessBypassConfig>, WitnessBypassConfigError> {
    witness_bypass_config_from_values(
        std::env::var(BYPASS_ENABLED_ENV).ok().as_deref(),
        std::env::var(SIGNING_ADDRESS_ENV).ok().as_deref(),
        std::env::var(EXPECTED_MEASUREMENTS_ENV).ok().as_deref(),
        std::env::var(ALLOWED_POLICY_VERSIONS_ENV).ok().as_deref(),
        std::env::var(CERTIFICATE_MAX_AGE_ENV).ok().as_deref(),
    )
}

/// Decide the bypass configuration from four raw values.
///
/// `Ok(None)` means the switch is off. It is not an acceptance: with it off
/// every content-bearing trace holds exactly as it does today and an arriving
/// certificate is ignored entirely.
///
/// With the switch on, each of the three controls is required, and a missing
/// one is `Err`, never a quieter config. Checked in the order an operator
/// fills them in -- address, measurements, allowlist -- so the first thing
/// they are told about is the first thing they have to fix.
pub fn witness_bypass_config_from_values(
    enabled: Option<&str>,
    signing_address: Option<&str>,
    measurements: Option<&str>,
    allowed_policy_versions: Option<&str>,
    certificate_max_age: Option<&str>,
) -> Result<Option<WitnessBypassConfig>, WitnessBypassConfigError> {
    if !affirmative(enabled) {
        return Ok(None);
    }

    let signing_address =
        non_blank(signing_address).ok_or(WitnessBypassConfigError::MissingControl {
            control: SIGNING_ADDRESS_CONTROL,
        })?;

    // Blank-or-absent is one state here, not two. An operator who set the
    // variable to whitespace has pinned nothing, and telling them the
    // variable is "present" would send them looking at the wrong line.
    let measurements = comma_separated(measurements);
    if measurements.is_empty() {
        return Err(WitnessBypassConfigError::MissingControl {
            control: EXPECTED_MEASUREMENT_CONTROL,
        });
    }

    let allowed_policy_versions = comma_separated(allowed_policy_versions);
    if allowed_policy_versions.is_empty() {
        return Err(WitnessBypassConfigError::MissingControl {
            control: POLICY_ALLOWLIST_CONTROL,
        });
    }

    // The pin validates the address and the measurement set. This module
    // composes rather than re-checking: a second opinion on what a well-formed
    // address is could only disagree with the one verification actually uses.
    // Absent or blank keeps the default window; present-and-unparseable is a
    // refusal. See `CERTIFICATE_MAX_AGE_ENV` for why this one control has a
    // default at all.
    let max_age_seconds = match non_blank(certificate_max_age) {
        None => DEFAULT_CERTIFICATE_MAX_AGE_SECONDS,
        Some(raw) => raw
            .parse::<i64>()
            .map_err(|_| WitnessBypassConfigError::CertificateMaxAgeMalformed)?,
    };
    let freshness =
        WitnessFreshness::new(max_age_seconds).map_err(WitnessBypassConfigError::Freshness)?;

    let pin = WitnessPin::new(signing_address, measurements.iter().cloned())
        .map_err(WitnessBypassConfigError::Pin)?
        .with_freshness(freshness);

    Ok(Some(WitnessBypassConfig {
        pin,
        allowed_policy_versions: allowed_policy_versions.into_iter().collect(),
    }))
}

/// Whether a switch value reads as on. Anything else -- including absent,
/// blank, and any value the operator invented -- is off, because the default
/// for this feature has to be the safe one.
fn affirmative(value: Option<&str>) -> bool {
    // Matches the spelling set `near_legion_claim` and `celestine_sloth_claim`
    // already accept, so an operator does not have to remember which of this
    // deployment's switches take "yes".
    value
        .map(|raw| {
            let raw = raw.trim().to_ascii_lowercase();
            raw == "1" || raw == "true" || raw == "yes"
        })
        .unwrap_or(false)
}

fn non_blank(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|raw| !raw.is_empty())
}

/// Split a comma-separated list, trimming each entry and dropping blanks.
///
/// A stray separator is tolerated because it cannot hide an entry -- the same
/// reading `ExpectedMeasurements::from_env_value` takes. Order is not
/// preserved and duplicates collapse: both lists are membership tests.
fn comma_separated(value: Option<&str>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(|raw| raw.split(','))
        .map(str::trim)
        .filter(|entry| !entry.is_empty())
        .map(str::to_string)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The four controls these tests are about, with the freshness window
    /// left unset.
    ///
    /// Unset is the interesting default here: it is the one control that
    /// falls back to a value rather than to a refusal, so every test that
    /// does not name it is also asserting that the fallback keeps working.
    /// The tests that ARE about the window call
    /// `witness_bypass_config_from_values` directly.
    fn from_values(
        enabled: Option<&str>,
        signing_address: Option<&str>,
        measurements: Option<&str>,
        allowed_policy_versions: Option<&str>,
    ) -> Result<Option<WitnessBypassConfig>, WitnessBypassConfigError> {
        witness_bypass_config_from_values(
            enabled,
            signing_address,
            measurements,
            allowed_policy_versions,
            None,
        )
    }

    const ADDRESS: &str = "0x00112233445566778899aabbccddeeff00112233";
    const MEASUREMENT: &str = "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";
    const ALIAS: &str = "ironclaw-deterministic-secret-path-v3+privacy-filter-self-hosted-v1";

    #[test]
    fn the_switch_off_yields_no_config_and_no_error() {
        let config = from_values(None, Some(ADDRESS), Some(MEASUREMENT), Some(ALIAS))
            .expect("an absent switch is not an error");
        assert!(config.is_none(), "the bypass must be off by default");
    }

    #[test]
    fn a_switch_set_to_false_is_still_off() {
        // "off by default" has to survive an operator writing the default down
        // explicitly, which is what the env template invites them to do.
        for spelling in ["false", "0", "no", "", "  ", "maybe"] {
            let config = from_values(
                Some(spelling),
                Some(ADDRESS),
                Some(MEASUREMENT),
                Some(ALIAS),
            )
            .expect("a negative switch is not an error");
            assert!(config.is_none(), "{spelling:?} must not enable the bypass");
        }
    }

    #[test]
    fn every_affirmative_spelling_enables_the_bypass() {
        for spelling in ["1", "true", "TRUE", " yes "] {
            let config = from_values(
                Some(spelling),
                Some(ADDRESS),
                Some(MEASUREMENT),
                Some(ALIAS),
            )
            .expect("a fully pinned bypass configures");
            assert!(config.is_some(), "{spelling:?} must enable the bypass");
        }
    }

    #[test]
    fn enabled_without_a_signing_address_refuses_by_control_name() {
        let err = from_values(Some("true"), None, Some(MEASUREMENT), Some(ALIAS))
            .expect_err("an enabled bypass with no address must refuse");
        assert_eq!(
            err,
            WitnessBypassConfigError::MissingControl {
                control: SIGNING_ADDRESS_CONTROL
            },
            "{err}",
        );
    }

    #[test]
    fn enabled_without_measurements_refuses_under_the_existing_control_name() {
        let err = from_values(Some("true"), Some(ADDRESS), None, Some(ALIAS))
            .expect_err("an enabled bypass with no measurements must refuse");
        assert_eq!(
            err,
            WitnessBypassConfigError::MissingControl {
                control: EXPECTED_MEASUREMENT_CONTROL
            },
            "{err}",
        );
        // The witness half already owns a control name for this. A second
        // spelling would send an operator hunting for a control that is not
        // the one the verifier reports.
        assert_eq!(EXPECTED_MEASUREMENT_CONTROL, "witness_expected_measurement");
    }

    #[test]
    fn enabled_with_an_empty_policy_allowlist_refuses_by_control_name() {
        let err = from_values(
            Some("true"),
            Some(ADDRESS),
            Some(MEASUREMENT),
            Some("  ,  "),
        )
        .expect_err("a blank allowlist is not an allowlist");
        assert_eq!(
            err,
            WitnessBypassConfigError::MissingControl {
                control: POLICY_ALLOWLIST_CONTROL
            },
            "{err}",
        );
    }

    #[test]
    fn the_three_control_names_are_distinct() {
        // They send an operator to three different lines of their config, so
        // two of them being equal is a defect no individual refusal test
        // would catch.
        let names = [
            SIGNING_ADDRESS_CONTROL,
            EXPECTED_MEASUREMENT_CONTROL,
            POLICY_ALLOWLIST_CONTROL,
        ];
        let distinct: BTreeSet<&str> = names.iter().copied().collect();
        assert_eq!(distinct.len(), names.len(), "{names:?}");
    }

    #[test]
    fn a_malformed_signing_address_surfaces_the_pins_own_variant() {
        let err = from_values(
            Some("true"),
            Some("0xnothex"),
            Some(MEASUREMENT),
            Some(ALIAS),
        )
        .expect_err("a malformed address must refuse");
        assert_eq!(
            err,
            WitnessBypassConfigError::Pin(WitnessPinError::SigningAddressMalformed),
            "{err}",
        );
    }

    #[test]
    fn a_configured_bypass_admits_exactly_the_aliases_it_was_given() {
        let config = from_values(
            Some("true"),
            Some(ADDRESS),
            Some(MEASUREMENT),
            Some(" alpha , beta ,, "),
        )
        .expect("configures")
        .expect("the switch is on");
        assert!(config.policy_version_allowed("alpha"));
        assert!(config.policy_version_allowed("beta"));
        assert_eq!(config.allowed_policy_version_count(), 2);
        // Not admitted: a policy nobody listed, and a case variant. Exact
        // comparison fails closed, which is the safe direction.
        assert!(!config.policy_version_allowed("gamma"));
        assert!(!config.policy_version_allowed("ALPHA"));
        // A superstring must not match by prefix or containment.
        assert!(!config.policy_version_allowed("alpha+classifier"));
    }

    #[test]
    fn the_pin_carries_every_measurement_that_was_configured() {
        let config = from_values(
            Some("true"),
            Some(ADDRESS),
            Some(" aaaa , bbbb "),
            Some(ALIAS),
        )
        .expect("configures")
        .expect("the switch is on");
        assert_eq!(config.pin().pinned_measurement_count(), 2);
    }

    #[test]
    fn no_configuration_value_reaches_an_error_string() {
        // Hash-only discipline. An operator's config is not contributor
        // content, but a signing address in a log line is exactly what this
        // repo's rule forbids, and the refusal path is the one that logs.
        let err = from_values(
            Some("true"),
            Some("0xSECRETMARKER"),
            Some("MEASUREMENTMARKER"),
            Some(ALIAS),
        )
        .expect_err("a malformed address must refuse");
        assert!(!format!("{err}").contains("SECRETMARKER"), "{err}");
        assert!(!format!("{err:?}").contains("SECRETMARKER"), "{err:?}");
        assert!(!format!("{err}").contains("MEASUREMENTMARKER"), "{err}");
    }

    #[test]
    fn the_env_names_are_the_ones_the_operator_doc_states() {
        // The loader cannot be exercised against the real environment under a
        // parallel test runner -- `set_var` is process-wide. Asserting the
        // spelling is the falsifiable half; see the same note at
        // near_attestation/measurements.rs.
        assert_eq!(BYPASS_ENABLED_ENV, "TRACE_COMMONS_WITNESS_BYPASS_ENABLED");
        assert_eq!(SIGNING_ADDRESS_ENV, "TRACE_COMMONS_WITNESS_SIGNING_ADDRESS");
        assert_eq!(
            EXPECTED_MEASUREMENTS_ENV,
            "TRACE_COMMONS_WITNESS_EXPECTED_MEASUREMENTS"
        );
        assert_eq!(
            ALLOWED_POLICY_VERSIONS_ENV,
            "TRACE_COMMONS_WITNESS_ALLOWED_POLICY_VERSIONS"
        );
        assert_eq!(
            CERTIFICATE_MAX_AGE_ENV,
            "TRACE_COMMONS_WITNESS_CERTIFICATE_MAX_AGE_SECONDS"
        );
    }

    /// An operator who sets nothing gets the default window, not no window.
    #[test]
    fn an_unset_max_age_leaves_the_default_window() {
        for raw in [None, Some(""), Some("   ")] {
            let config = witness_bypass_config_from_values(
                Some("true"),
                Some(ADDRESS),
                Some(MEASUREMENT),
                Some(ALIAS),
                raw,
            )
            .expect("the bypass configures")
            .expect("the switch is on");
            assert_eq!(
                config.pin().freshness().max_age_seconds(),
                DEFAULT_CERTIFICATE_MAX_AGE_SECONDS,
                "{raw:?} did not leave the default window"
            );
        }
    }

    /// And an operator who sets one gets theirs.
    #[test]
    fn a_configured_max_age_reaches_the_pin() {
        let config = witness_bypass_config_from_values(
            Some("true"),
            Some(ADDRESS),
            Some(MEASUREMENT),
            Some(ALIAS),
            Some(" 900 "),
        )
        .expect("the bypass configures")
        .expect("the switch is on");
        assert_eq!(config.pin().freshness().max_age_seconds(), 900);
    }

    /// A value that is present and unusable is a refusal, never a silent
    /// fallback to the default. An operator who typed something meant
    /// something, and widening their window back to 24h without saying so is
    /// the failure this whole module is shaped against.
    #[test]
    fn an_unusable_max_age_refuses_rather_than_defaulting() {
        for raw in ["not-a-number", "12.5", "1e3", "9999999999999999999999"] {
            let err = witness_bypass_config_from_values(
                Some("true"),
                Some(ADDRESS),
                Some(MEASUREMENT),
                Some(ALIAS),
                Some(raw),
            )
            .expect_err("an unusable window must refuse");
            assert_eq!(
                err,
                WitnessBypassConfigError::CertificateMaxAgeMalformed,
                "{raw} was accepted: {err}"
            );
        }

        for raw in ["0", "-1"] {
            let err = witness_bypass_config_from_values(
                Some("true"),
                Some(ADDRESS),
                Some(MEASUREMENT),
                Some(ALIAS),
                Some(raw),
            )
            .expect_err("a non-positive window must refuse");
            assert_eq!(
                err,
                WitnessBypassConfigError::Freshness(WitnessFreshnessError::MaxAgeNotPositive),
                "{raw} was accepted: {err}"
            );
        }
    }

    /// The refusals name the variable and carry nothing the operator typed.
    #[test]
    fn the_max_age_refusals_render_safely() {
        for err in [
            WitnessBypassConfigError::CertificateMaxAgeMalformed,
            WitnessBypassConfigError::Freshness(WitnessFreshnessError::MaxAgeNotPositive),
        ] {
            let rendered = format!("{err} {err:?}");
            assert!(rendered.starts_with("witness bypass refused"), "{rendered}");
        }
    }
}
