// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The `near_ai` rail's settings, read from the environment only (payout
//! spec section 2, step 6, and section 4, "Fail-closed rules").
//!
//! The mode defaults to `disabled`, and a disabled rail needs nothing else.
//! In `dry_run` or `live`, every other setting must be present and well
//! formed, or the rail refuses to start with `near_ai_payout_config_missing`.
//! There is no base-URL setting: the host is the compiled-in
//! [`crate::near_ai_api::NEAR_AI_API_BASE_URL`].
//!
//! A refusal names the setting (names are not secret), never its value.

use std::fmt;

use secrecy::SecretString;
use trace_commons_gate_api::pipeline::AtomicUnits;
use uuid::Uuid;

use crate::near_ai_transfer::{
    NearAiCreditSource, NearAiTransferCredentials, is_near_ai_org_credential,
};
use crate::payout_rate::{NEAR_AI_PAYOUT_MODE_ENV, PayoutMode};

/// The payout credential: `oc_` plus 64 hex. Secret.
pub const NEAR_AI_PAYOUT_CREDENTIAL_ENV: &str = "TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL";
/// The credential's `id` (a UUID). Not secret; used only for its log hash.
pub const NEAR_AI_PAYOUT_CREDENTIAL_ID_ENV: &str = "TRACE_COMMONS_NEAR_AI_PAYOUT_CREDENTIAL_ID";
/// The commons' reseller org (a UUID).
pub const NEAR_AI_PAYOUT_SENDER_ORG_ENV: &str = "TRACE_COMMONS_NEAR_AI_PAYOUT_SENDER_ORG";
/// `payment`, `grant` or `staking_farm`: the credit type the sender org
/// sends from. Required outside `disabled`; there is no default, because
/// the right one depends on how the sender org was funded.
pub const NEAR_AI_PAYOUT_SOURCE_ENV: &str = "TRACE_COMMONS_NEAR_AI_PAYOUT_SOURCE";
/// The lifetime ceiling per account on this rail, in nano-USD, above zero.
pub const NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO_ENV: &str =
    "TRACE_COMMONS_NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO";

/// Every setting the rail reads. None names a host.
pub const NEAR_AI_PAYOUT_ENV_NAMES: [&str; 6] = [
    NEAR_AI_PAYOUT_MODE_ENV,
    NEAR_AI_PAYOUT_CREDENTIAL_ENV,
    NEAR_AI_PAYOUT_CREDENTIAL_ID_ENV,
    NEAR_AI_PAYOUT_SENDER_ORG_ENV,
    NEAR_AI_PAYOUT_SOURCE_ENV,
    NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO_ENV,
];

/// Why the rail's settings were refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NearAiRailConfigError {
    /// The mode is not `disabled`, `dry_run` or `live`.
    #[error("payout_mode_invalid")]
    ModeInvalid,
    /// A setting needed outside `disabled` is missing or malformed.
    #[error("near_ai_payout_config_missing")]
    Missing { setting: &'static str },
}

impl NearAiRailConfigError {
    pub fn label(self) -> &'static str {
        match self {
            Self::ModeInvalid => "payout_mode_invalid",
            Self::Missing { .. } => "near_ai_payout_config_missing",
        }
    }

    /// The setting's name, safe to log beside the label.
    pub fn setting(self) -> &'static str {
        match self {
            Self::ModeInvalid => NEAR_AI_PAYOUT_MODE_ENV,
            Self::Missing { setting } => setting,
        }
    }
}

/// An enabled rail's settings. `Debug` prints the mode and nothing else.
#[derive(Clone)]
pub struct NearAiRailSettings {
    pub mode: PayoutMode,
    pub credentials: NearAiTransferCredentials,
    pub account_ceiling: AtomicUnits,
}

impl fmt::Debug for NearAiRailSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NearAiRailSettings")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

/// The rail's configuration.
#[derive(Debug, Clone)]
pub enum NearAiRailConfig {
    Disabled,
    Enabled(NearAiRailSettings),
}

impl NearAiRailConfig {
    pub fn mode(&self) -> PayoutMode {
        match self {
            Self::Disabled => PayoutMode::Disabled,
            Self::Enabled(settings) => settings.mode,
        }
    }

    /// Reads the settings through `get`, which answers `None` for an unset
    /// variable. Pure, so tests need not touch the process environment.
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, NearAiRailConfigError> {
        let mode = PayoutMode::parse(get(NEAR_AI_PAYOUT_MODE_ENV).as_deref())
            .map_err(|_| NearAiRailConfigError::ModeInvalid)?;
        if mode == PayoutMode::Disabled {
            return Ok(Self::Disabled);
        }
        let read = |setting: &'static str| {
            get(setting)
                .filter(|value| !value.is_empty() && value.trim() == value)
                .ok_or(NearAiRailConfigError::Missing { setting })
        };
        let malformed = |setting: &'static str| NearAiRailConfigError::Missing { setting };

        let token = read(NEAR_AI_PAYOUT_CREDENTIAL_ENV)?;
        if !is_near_ai_org_credential(&token) {
            return Err(malformed(NEAR_AI_PAYOUT_CREDENTIAL_ENV));
        }
        let credential_id = parse_uuid(&read(NEAR_AI_PAYOUT_CREDENTIAL_ID_ENV)?)
            .ok_or(malformed(NEAR_AI_PAYOUT_CREDENTIAL_ID_ENV))?;
        let sender_org = parse_uuid(&read(NEAR_AI_PAYOUT_SENDER_ORG_ENV)?)
            .ok_or(malformed(NEAR_AI_PAYOUT_SENDER_ORG_ENV))?;
        let source = NearAiCreditSource::parse(&read(NEAR_AI_PAYOUT_SOURCE_ENV)?)
            .ok_or(malformed(NEAR_AI_PAYOUT_SOURCE_ENV))?;
        let ceiling = read(NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO_ENV)?;
        let account_ceiling = ceiling
            .bytes()
            .all(|byte| byte.is_ascii_digit())
            .then(|| ceiling.parse::<u128>().ok())
            .flatten()
            .filter(|value| *value > 0)
            .map(AtomicUnits::from_raw)
            .ok_or(malformed(NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO_ENV))?;

        Ok(Self::Enabled(NearAiRailSettings {
            mode,
            credentials: NearAiTransferCredentials {
                token: SecretString::from(token),
                credential_id,
                sender_org,
                source,
            },
            account_ceiling,
        }))
    }

    /// [`NearAiRailConfig::from_vars`] over the process environment. A value
    /// that is not Unicode counts as unset, so it refuses outside
    /// `disabled` (and an unset mode is `disabled`).
    pub fn from_env() -> Result<Self, NearAiRailConfigError> {
        if matches!(
            std::env::var(NEAR_AI_PAYOUT_MODE_ENV),
            Err(std::env::VarError::NotUnicode(_))
        ) {
            return Err(NearAiRailConfigError::ModeInvalid);
        }
        Self::from_vars(|name| std::env::var(name).ok())
    }
}

// ---------------------------------------------------------------------------
// The referral stream (referral rewards spec section 5): a second program on
// each `near_ai` link, sent from a dedicated #1271 credential whose
// `max_total_amount` is the program's lifetime budget.
// ---------------------------------------------------------------------------

/// `disabled` (default), `dry_run` or `live`.
pub const NEAR_AI_REFERRAL_MODE_ENV: &str = "TRACE_COMMONS_NEAR_AI_REFERRAL_MODE";
/// The referral credential: `oc_` plus 64 hex, never the payout credential.
/// Secret.
pub const NEAR_AI_REFERRAL_CREDENTIAL_ENV: &str = "TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL";
/// The referral credential's `id` (a UUID). Not secret; log hash only.
pub const NEAR_AI_REFERRAL_CREDENTIAL_ID_ENV: &str = "TRACE_COMMONS_NEAR_AI_REFERRAL_CREDENTIAL_ID";
/// The most referral credit one referrer is sent across every link, in
/// nano-USD, above zero. Default [`REFERRAL_ACCOUNT_CEILING_NANO_DEFAULT`].
pub const REFERRAL_ACCOUNT_CEILING_NANO_ENV: &str = "TRACE_COMMONS_REFERRAL_ACCOUNT_CEILING_NANO";
/// $250 in nano-USD (spec section 9, R4).
pub const REFERRAL_ACCOUNT_CEILING_NANO_DEFAULT: u128 = 250_000_000_000;

/// Every setting the referral stream reads. None names a host.
pub const NEAR_AI_REFERRAL_ENV_NAMES: [&str; 4] = [
    NEAR_AI_REFERRAL_MODE_ENV,
    NEAR_AI_REFERRAL_CREDENTIAL_ENV,
    NEAR_AI_REFERRAL_CREDENTIAL_ID_ENV,
    REFERRAL_ACCOUNT_CEILING_NANO_ENV,
];

/// Why the referral stream's settings were refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum NearAiReferralConfigError {
    /// The mode is not `disabled`, `dry_run` or `live`.
    #[error("near_ai_referral_mode_invalid")]
    ModeInvalid,
    /// A setting the mode needs is missing or malformed, or the rail's own
    /// mode does not allow the referral mode.
    #[error("near_ai_referral_config_missing")]
    Missing { setting: &'static str },
}

impl NearAiReferralConfigError {
    pub fn label(self) -> &'static str {
        match self {
            Self::ModeInvalid => "near_ai_referral_mode_invalid",
            Self::Missing { .. } => "near_ai_referral_config_missing",
        }
    }

    /// The setting's name, safe to log beside the label.
    pub fn setting(self) -> &'static str {
        match self {
            Self::ModeInvalid => NEAR_AI_REFERRAL_MODE_ENV,
            Self::Missing { setting } => setting,
        }
    }
}

/// An enabled referral stream. `credentials` is always set in `live`; in
/// `dry_run` it is set only when configured (a dry run makes no call with
/// it). The sender org and credit source are the rail's own. `Debug`
/// prints the mode and nothing else.
#[derive(Clone)]
pub struct NearAiReferralSettings {
    pub mode: PayoutMode,
    pub credentials: Option<NearAiTransferCredentials>,
    pub account_ceiling: AtomicUnits,
}

impl fmt::Debug for NearAiReferralSettings {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("NearAiReferralSettings")
            .field("mode", &self.mode)
            .finish_non_exhaustive()
    }
}

/// The referral stream's configuration.
#[derive(Debug, Clone, Default)]
pub enum NearAiReferralConfig {
    #[default]
    Disabled,
    Enabled(NearAiReferralSettings),
}

impl NearAiReferralConfig {
    pub fn mode(&self) -> PayoutMode {
        match self {
            Self::Disabled => PayoutMode::Disabled,
            Self::Enabled(settings) => settings.mode,
        }
    }

    /// Reads the referral settings through `get`, against the rail's own
    /// configuration. Fail-closed:
    ///
    /// - unset or `disabled` needs nothing;
    /// - `dry_run` or `live` needs the rail itself enabled;
    /// - `live` needs the rail `live` and the referral credential and its
    ///   id, and the credential must not be the payout credential (its
    ///   `max_total_amount` is the referral budget, so sharing one would
    ///   spend the earned budget on referrals);
    /// - a configured credential or ceiling must be well formed in any mode.
    ///
    /// Every refusal is `near_ai_referral_config_missing` naming the setting.
    pub fn from_vars(
        get: impl Fn(&str) -> Option<String>,
        rail: &NearAiRailConfig,
    ) -> Result<Self, NearAiReferralConfigError> {
        let mode = PayoutMode::parse(get(NEAR_AI_REFERRAL_MODE_ENV).as_deref())
            .map_err(|_| NearAiReferralConfigError::ModeInvalid)?;
        if mode == PayoutMode::Disabled {
            return Ok(Self::Disabled);
        }
        let missing = |setting: &'static str| NearAiReferralConfigError::Missing { setting };
        let rail_settings = match rail {
            NearAiRailConfig::Enabled(settings) => settings,
            NearAiRailConfig::Disabled => return Err(missing(NEAR_AI_PAYOUT_MODE_ENV)),
        };
        if mode == PayoutMode::Live && rail_settings.mode != PayoutMode::Live {
            return Err(missing(NEAR_AI_PAYOUT_MODE_ENV));
        }
        // Set means set: an empty or padded value is malformed, not unset.
        let read = |setting: &'static str| -> Result<Option<String>, NearAiReferralConfigError> {
            match get(setting) {
                None => Ok(None),
                Some(value) if !value.is_empty() && value.trim() == value => Ok(Some(value)),
                Some(_) => Err(missing(setting)),
            }
        };

        let token = read(NEAR_AI_REFERRAL_CREDENTIAL_ENV)?;
        let credential_id = read(NEAR_AI_REFERRAL_CREDENTIAL_ID_ENV)?;
        let credentials = match (token, credential_id) {
            (None, None) if mode == PayoutMode::DryRun => None,
            (None, _) => return Err(missing(NEAR_AI_REFERRAL_CREDENTIAL_ENV)),
            (Some(_), None) => return Err(missing(NEAR_AI_REFERRAL_CREDENTIAL_ID_ENV)),
            (Some(token), Some(credential_id)) => {
                use secrecy::ExposeSecret;
                if !is_near_ai_org_credential(&token)
                    || token == rail_settings.credentials.token.expose_secret()
                {
                    return Err(missing(NEAR_AI_REFERRAL_CREDENTIAL_ENV));
                }
                let credential_id = parse_uuid(&credential_id)
                    .filter(|id| *id != rail_settings.credentials.credential_id)
                    .ok_or(missing(NEAR_AI_REFERRAL_CREDENTIAL_ID_ENV))?;
                Some(NearAiTransferCredentials {
                    token: SecretString::from(token),
                    credential_id,
                    sender_org: rail_settings.credentials.sender_org,
                    source: rail_settings.credentials.source,
                })
            }
        };
        let account_ceiling = match read(REFERRAL_ACCOUNT_CEILING_NANO_ENV)? {
            None => AtomicUnits::from_raw(REFERRAL_ACCOUNT_CEILING_NANO_DEFAULT),
            Some(ceiling) => {
                parse_positive_units(&ceiling).ok_or(missing(REFERRAL_ACCOUNT_CEILING_NANO_ENV))?
            }
        };
        Ok(Self::Enabled(NearAiReferralSettings {
            mode,
            credentials,
            account_ceiling,
        }))
    }

    /// [`NearAiReferralConfig::from_vars`] over the process environment. A
    /// value that is not Unicode counts as unset, except the mode, which
    /// refuses.
    pub fn from_env(rail: &NearAiRailConfig) -> Result<Self, NearAiReferralConfigError> {
        if matches!(
            std::env::var(NEAR_AI_REFERRAL_MODE_ENV),
            Err(std::env::VarError::NotUnicode(_))
        ) {
            return Err(NearAiReferralConfigError::ModeInvalid);
        }
        Self::from_vars(|name| std::env::var(name).ok(), rail)
    }
}

/// Decimal digits only, above zero, within `u128`.
fn parse_positive_units(value: &str) -> Option<AtomicUnits> {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| value.parse::<u128>().ok())
        .flatten()
        .filter(|value| *value > 0)
        .map(AtomicUnits::from_raw)
}

/// A UUID in its hyphenated form only.
fn parse_uuid(value: &str) -> Option<Uuid> {
    (value.len() == 36)
        .then(|| Uuid::parse_str(value).ok())
        .flatten()
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use secrecy::ExposeSecret;

    use super::*;

    const TOKEN: &str = "oc_00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";

    fn complete(mode: &str) -> HashMap<&'static str, String> {
        HashMap::from([
            (NEAR_AI_PAYOUT_MODE_ENV, mode.to_string()),
            (NEAR_AI_PAYOUT_CREDENTIAL_ENV, TOKEN.to_string()),
            (
                NEAR_AI_PAYOUT_CREDENTIAL_ID_ENV,
                Uuid::from_u128(2).to_string(),
            ),
            (
                NEAR_AI_PAYOUT_SENDER_ORG_ENV,
                Uuid::from_u128(1).to_string(),
            ),
            (NEAR_AI_PAYOUT_SOURCE_ENV, "grant".to_string()),
            (NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO_ENV, "1000".to_string()),
        ])
    }

    fn parse(
        vars: &HashMap<&'static str, String>,
    ) -> Result<NearAiRailConfig, NearAiRailConfigError> {
        NearAiRailConfig::from_vars(|name| vars.get(name).cloned())
    }

    #[test]
    fn config_refuses_missing_or_malformed_values_outside_disabled() {
        // Unset and `disabled` need nothing else.
        assert!(matches!(
            NearAiRailConfig::from_vars(|_| None),
            Ok(NearAiRailConfig::Disabled)
        ));
        let only_mode = HashMap::from([(NEAR_AI_PAYOUT_MODE_ENV, "disabled".to_string())]);
        assert!(matches!(parse(&only_mode), Ok(NearAiRailConfig::Disabled)));

        // An unknown mode is refused, never read as disabled.
        for mode in ["", "Live", "on", "dry-run"] {
            let mut vars = complete("live");
            vars.insert(NEAR_AI_PAYOUT_MODE_ENV, mode.to_string());
            assert_eq!(
                parse(&vars).unwrap_err(),
                NearAiRailConfigError::ModeInvalid,
                "{mode:?}"
            );
        }

        for mode in ["dry_run", "live"] {
            let settings = match parse(&complete(mode)).unwrap() {
                NearAiRailConfig::Enabled(settings) => settings,
                NearAiRailConfig::Disabled => panic!("{mode} is enabled"),
            };
            assert_eq!(settings.mode.as_label(), mode);
            assert_eq!(settings.credentials.token.expose_secret(), TOKEN);
            assert_eq!(settings.credentials.sender_org, Uuid::from_u128(1));
            assert_eq!(settings.credentials.source, NearAiCreditSource::Grant);
            assert_eq!(settings.account_ceiling, AtomicUnits::from_raw(1000));

            // Each setting, missing, refuses with its own name.
            for setting in &NEAR_AI_PAYOUT_ENV_NAMES[1..] {
                let mut vars = complete(mode);
                vars.remove(setting);
                let error = parse(&vars).unwrap_err();
                assert_eq!(error.label(), "near_ai_payout_config_missing");
                assert_eq!(error.setting(), *setting);
            }

            let malformed: [(&str, &[&str]); 5] = [
                (
                    NEAR_AI_PAYOUT_CREDENTIAL_ENV,
                    &["", "oc_", "oc_abc", &TOKEN[3..], " oc_00", "bearer"],
                ),
                (
                    NEAR_AI_PAYOUT_CREDENTIAL_ID_ENV,
                    &["", "2", "00000000000000000000000000000002", "not-a-uuid"],
                ),
                (
                    NEAR_AI_PAYOUT_SENDER_ORG_ENV,
                    &["", "org", " 00000000-0000-0000-0000-000000000001"],
                ),
                (
                    NEAR_AI_PAYOUT_SOURCE_ENV,
                    &["", "Grant", "PAYMENT", "staking-farm", "transfer"],
                ),
                (
                    NEAR_AI_PAYOUT_ACCOUNT_CEILING_NANO_ENV,
                    &[
                        "",
                        "0",
                        "-1",
                        "1.5",
                        "+5",
                        "1e9",
                        "340282366920938463463374607431768211456",
                    ],
                ),
            ];
            for (setting, values) in malformed {
                for value in values {
                    let mut vars = complete(mode);
                    vars.insert(setting, value.to_string());
                    let error = parse(&vars).unwrap_err();
                    assert_eq!(
                        error,
                        NearAiRailConfigError::Missing { setting },
                        "{setting}={value:?}"
                    );
                }
            }
        }
    }

    /// The source is the operator's choice among the three #1271 names; the
    /// staging sender is funded with paid credit, so `payment` must work,
    /// and nothing picks one when it is unset.
    #[test]
    fn the_source_is_configured_with_no_default() {
        for (value, source) in [
            ("payment", NearAiCreditSource::Payment),
            ("grant", NearAiCreditSource::Grant),
            ("staking_farm", NearAiCreditSource::StakingFarm),
        ] {
            let mut vars = complete("live");
            vars.insert(NEAR_AI_PAYOUT_SOURCE_ENV, value.to_string());
            let NearAiRailConfig::Enabled(settings) = parse(&vars).unwrap() else {
                panic!("live is enabled");
            };
            assert_eq!(settings.credentials.source, source, "{value}");
        }
        for mode in ["dry_run", "live"] {
            let mut vars = complete(mode);
            vars.remove(NEAR_AI_PAYOUT_SOURCE_ENV);
            assert_eq!(
                parse(&vars).unwrap_err(),
                NearAiRailConfigError::Missing {
                    setting: NEAR_AI_PAYOUT_SOURCE_ENV
                }
            );
        }
    }

    #[test]
    fn config_has_no_base_url_setting() {
        for name in NEAR_AI_PAYOUT_ENV_NAMES {
            for forbidden in ["URL", "HOST", "BASE", "ENDPOINT", "ORIGIN"] {
                assert!(!name.contains(forbidden), "{name} names a host");
            }
        }
        // A host setting someone might add is not read: the parse does not
        // ask for it.
        let asked = std::cell::RefCell::new(Vec::new());
        let vars = complete("live");
        NearAiRailConfig::from_vars(|name| {
            asked.borrow_mut().push(name.to_string());
            vars.get(name).cloned()
        })
        .unwrap();
        let asked = asked.into_inner();
        assert!(
            asked
                .iter()
                .all(|name| NEAR_AI_PAYOUT_ENV_NAMES.contains(&name.as_str()))
        );
        assert_eq!(asked.len(), NEAR_AI_PAYOUT_ENV_NAMES.len());
    }

    #[test]
    fn debug_and_errors_never_print_a_value() {
        let settings = parse(&complete("live")).unwrap();
        let rendered = format!("{settings:?}");
        assert!(!rendered.contains(&TOKEN[3..]), "{rendered}");
        assert!(
            !rendered.contains(&Uuid::from_u128(1).to_string()),
            "{rendered}"
        );
        assert!(!rendered.contains("1000"), "{rendered}");

        let mut vars = complete("live");
        vars.insert(NEAR_AI_PAYOUT_CREDENTIAL_ENV, "oc_leaked".to_string());
        let error = parse(&vars).unwrap_err();
        for rendered in [format!("{error}"), format!("{error:?}")] {
            assert!(!rendered.contains("leaked"), "{rendered}");
        }
    }

    const REFERRAL_TOKEN: &str =
        "oc_ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100";

    fn rail(mode: &str) -> NearAiRailConfig {
        parse(&complete(mode)).unwrap()
    }

    fn referral(mode: &str) -> HashMap<&'static str, String> {
        HashMap::from([
            (NEAR_AI_REFERRAL_MODE_ENV, mode.to_string()),
            (NEAR_AI_REFERRAL_CREDENTIAL_ENV, REFERRAL_TOKEN.to_string()),
            (
                NEAR_AI_REFERRAL_CREDENTIAL_ID_ENV,
                Uuid::from_u128(3).to_string(),
            ),
        ])
    }

    fn parse_referral(
        vars: &HashMap<&'static str, String>,
        rail: &NearAiRailConfig,
    ) -> Result<NearAiReferralConfig, NearAiReferralConfigError> {
        NearAiReferralConfig::from_vars(|name| vars.get(name).cloned(), rail)
    }

    #[test]
    fn referral_mode_defaults_to_disabled_and_needs_nothing() {
        for rail in [NearAiRailConfig::Disabled, rail("dry_run"), rail("live")] {
            assert!(matches!(
                NearAiReferralConfig::from_vars(|_| None, &rail),
                Ok(NearAiReferralConfig::Disabled)
            ));
            let only = HashMap::from([(NEAR_AI_REFERRAL_MODE_ENV, "disabled".to_string())]);
            assert!(matches!(
                parse_referral(&only, &rail),
                Ok(NearAiReferralConfig::Disabled)
            ));
        }
        for mode in ["", "Live", "on", "dry-run"] {
            let mut vars = referral("live");
            vars.insert(NEAR_AI_REFERRAL_MODE_ENV, mode.to_string());
            assert_eq!(
                parse_referral(&vars, &rail("live")).unwrap_err(),
                NearAiReferralConfigError::ModeInvalid,
                "{mode:?}"
            );
        }
    }

    #[test]
    fn referral_live_needs_the_rail_live_and_its_own_credential() {
        let NearAiReferralConfig::Enabled(settings) =
            parse_referral(&referral("live"), &rail("live")).unwrap()
        else {
            panic!("live is enabled");
        };
        assert_eq!(settings.mode, PayoutMode::Live);
        let credentials = settings.credentials.expect("live has a credential");
        assert_eq!(credentials.token.expose_secret(), REFERRAL_TOKEN);
        assert_eq!(credentials.credential_id, Uuid::from_u128(3));
        // The sender org and source are the rail's.
        assert_eq!(credentials.sender_org, Uuid::from_u128(1));
        assert_eq!(credentials.source, NearAiCreditSource::Grant);
        assert_eq!(
            settings.account_ceiling,
            AtomicUnits::from_raw(REFERRAL_ACCOUNT_CEILING_NANO_DEFAULT)
        );
        assert_eq!(REFERRAL_ACCOUNT_CEILING_NANO_DEFAULT, 250 * 1_000_000_000);

        // The rail itself must be live.
        for rail in [NearAiRailConfig::Disabled, rail("dry_run")] {
            assert_eq!(
                parse_referral(&referral("live"), &rail).unwrap_err(),
                NearAiReferralConfigError::Missing {
                    setting: NEAR_AI_PAYOUT_MODE_ENV
                }
            );
        }
        // A dry run needs the rail enabled.
        assert_eq!(
            parse_referral(&referral("dry_run"), &NearAiRailConfig::Disabled).unwrap_err(),
            NearAiReferralConfigError::Missing {
                setting: NEAR_AI_PAYOUT_MODE_ENV
            }
        );

        // Live without the credential, or its id, refuses with its name.
        for setting in [
            NEAR_AI_REFERRAL_CREDENTIAL_ENV,
            NEAR_AI_REFERRAL_CREDENTIAL_ID_ENV,
        ] {
            let mut vars = referral("live");
            vars.remove(setting);
            let error = parse_referral(&vars, &rail("live")).unwrap_err();
            assert_eq!(error.label(), "near_ai_referral_config_missing");
            assert_eq!(error.setting(), setting);
        }

        // Never the payout credential, or its id: the referral budget is a
        // credential of its own.
        let mut shared = referral("live");
        shared.insert(NEAR_AI_REFERRAL_CREDENTIAL_ENV, TOKEN.to_string());
        assert_eq!(
            parse_referral(&shared, &rail("live")).unwrap_err(),
            NearAiReferralConfigError::Missing {
                setting: NEAR_AI_REFERRAL_CREDENTIAL_ENV
            }
        );
        let mut shared_id = referral("live");
        shared_id.insert(
            NEAR_AI_REFERRAL_CREDENTIAL_ID_ENV,
            Uuid::from_u128(2).to_string(),
        );
        assert_eq!(
            parse_referral(&shared_id, &rail("live")).unwrap_err(),
            NearAiReferralConfigError::Missing {
                setting: NEAR_AI_REFERRAL_CREDENTIAL_ID_ENV
            }
        );
    }

    #[test]
    fn referral_dry_run_runs_without_a_credential_but_refuses_a_malformed_one() {
        let vars = HashMap::from([(NEAR_AI_REFERRAL_MODE_ENV, "dry_run".to_string())]);
        for rail in [rail("dry_run"), rail("live")] {
            let NearAiReferralConfig::Enabled(settings) = parse_referral(&vars, &rail).unwrap()
            else {
                panic!("dry_run is enabled");
            };
            assert_eq!(settings.mode, PayoutMode::DryRun);
            assert!(settings.credentials.is_none());
        }
        let malformed: [(&str, &[&str]); 3] = [
            (
                NEAR_AI_REFERRAL_CREDENTIAL_ENV,
                &["", "oc_", "oc_abc", " oc_00", "bearer"],
            ),
            (
                NEAR_AI_REFERRAL_CREDENTIAL_ID_ENV,
                &["", "3", "00000000000000000000000000000003"],
            ),
            (
                REFERRAL_ACCOUNT_CEILING_NANO_ENV,
                &[
                    "",
                    "0",
                    "-1",
                    "1.5",
                    "+5",
                    " 5",
                    "340282366920938463463374607431768211456",
                ],
            ),
        ];
        for mode in ["dry_run", "live"] {
            for (setting, values) in malformed {
                for value in values {
                    let mut vars = referral(mode);
                    vars.insert(setting, value.to_string());
                    assert_eq!(
                        parse_referral(&vars, &rail("live")).unwrap_err(),
                        NearAiReferralConfigError::Missing { setting },
                        "{mode} {setting}={value:?}"
                    );
                }
            }
        }
        let mut raised = referral("live");
        raised.insert(REFERRAL_ACCOUNT_CEILING_NANO_ENV, "500".to_string());
        let NearAiReferralConfig::Enabled(settings) =
            parse_referral(&raised, &rail("live")).unwrap()
        else {
            panic!("live is enabled");
        };
        assert_eq!(settings.account_ceiling, AtomicUnits::from_raw(500));
    }

    #[test]
    fn referral_config_reads_only_its_own_settings_and_prints_no_value() {
        for name in NEAR_AI_REFERRAL_ENV_NAMES {
            for forbidden in ["URL", "HOST", "BASE", "ENDPOINT", "ORIGIN"] {
                assert!(!name.contains(forbidden), "{name} names a host");
            }
            assert!(!NEAR_AI_PAYOUT_ENV_NAMES.contains(&name), "{name}");
        }
        let asked = std::cell::RefCell::new(Vec::new());
        let vars = referral("live");
        let parsed = NearAiReferralConfig::from_vars(
            |name| {
                asked.borrow_mut().push(name.to_string());
                vars.get(name).cloned()
            },
            &rail("live"),
        )
        .unwrap();
        assert!(
            asked
                .into_inner()
                .iter()
                .all(|name| NEAR_AI_REFERRAL_ENV_NAMES.contains(&name.as_str()))
        );
        let rendered = format!("{parsed:?}");
        assert!(!rendered.contains(&REFERRAL_TOKEN[3..]), "{rendered}");
        assert!(
            !rendered.contains(&Uuid::from_u128(3).to_string()),
            "{rendered}"
        );
        let mut leaked = referral("live");
        leaked.insert(NEAR_AI_REFERRAL_CREDENTIAL_ENV, "oc_leaked".to_string());
        let error = parse_referral(&leaked, &rail("live")).unwrap_err();
        for rendered in [format!("{error}"), format!("{error:?}")] {
            assert!(!rendered.contains("leaked"), "{rendered}");
        }
    }
}
