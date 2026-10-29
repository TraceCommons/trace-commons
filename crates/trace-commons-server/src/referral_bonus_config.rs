// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The referral bonus pass's settings, read from the environment only
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, sections
//! 4, 5 and 7).
//!
//! Minimal on purpose: the codes-and-apply slice owns the program settings
//! (start, end, late days) and the sends slice owns the referral credential.
//! `TRACE_COMMONS_REFERRAL_EARN_DAYS` and `TRACE_COMMONS_REFERRAL_BONUS_BPS`
//! may also be parsed there; the integrator keeps one parser.
//!
//! - `TRACE_COMMONS_NEAR_AI_REFERRAL_MODE`: `disabled` (default) or
//!   `dry_run`. `live` needs the referral credential, which this build does
//!   not read, so it refuses the start (`near_ai_referral_config_missing`).
//! - `TRACE_COMMONS_REFERRAL_BONUS_BPS`: 1 to 10000, default 1000.
//! - `TRACE_COMMONS_REFERRAL_EARN_DAYS`: default and minimum 92; a smaller
//!   value refuses the start (`referral_earn_days_below_promise`).
//! - `TRACE_COMMONS_REFERRAL_ACCOUNT_CEILING_NANO`: above zero, default
//!   250 000 000 000 ($250).
//! - The abuse thresholds of [`ReferralAbuseSettings`].
//!
//! Every setting is checked whatever the mode, so a typo is found before the
//! stream is switched on. A refusal names the setting, never its value.

use crate::payout_rate::PayoutMode;
use crate::referral_abuse::ReferralAbuseSettings;

pub const NEAR_AI_REFERRAL_MODE_ENV: &str = "TRACE_COMMONS_NEAR_AI_REFERRAL_MODE";
pub const REFERRAL_BONUS_BPS_ENV: &str = "TRACE_COMMONS_REFERRAL_BONUS_BPS";
pub const REFERRAL_EARN_DAYS_ENV: &str = "TRACE_COMMONS_REFERRAL_EARN_DAYS";
pub const REFERRAL_ACCOUNT_CEILING_NANO_ENV: &str = "TRACE_COMMONS_REFERRAL_ACCOUNT_CEILING_NANO";

/// R1: "3 months promised but it can be extended."
pub const REFERRAL_EARN_DAYS_PROMISED: u32 = 92;
pub const REFERRAL_BONUS_BPS_DEFAULT: u32 = 1_000;
/// R4: $250 per referrer, in nano-USD.
pub const REFERRAL_ACCOUNT_CEILING_NANO_DEFAULT: u128 = 250_000_000_000;

/// Why the settings were refused. One safe label each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReferralBonusConfigError {
    #[error("near_ai_referral_mode_invalid")]
    ModeInvalid,
    /// `live` without the referral credential (not read by this build).
    #[error("near_ai_referral_config_missing")]
    LiveUnconfigured,
    #[error("referral_bonus_bps_invalid")]
    BpsInvalid,
    #[error("referral_earn_days_below_promise")]
    EarnDaysBelowPromise,
    #[error("referral_earn_days_invalid")]
    EarnDaysInvalid,
    #[error("referral_account_ceiling_invalid")]
    CeilingInvalid,
    #[error("referral_abuse_setting_invalid")]
    Abuse { setting: &'static str },
}

impl ReferralBonusConfigError {
    pub fn label(self) -> &'static str {
        match self {
            Self::ModeInvalid => "near_ai_referral_mode_invalid",
            Self::LiveUnconfigured => "near_ai_referral_config_missing",
            Self::BpsInvalid => "referral_bonus_bps_invalid",
            Self::EarnDaysBelowPromise => "referral_earn_days_below_promise",
            Self::EarnDaysInvalid => "referral_earn_days_invalid",
            Self::CeilingInvalid => "referral_account_ceiling_invalid",
            Self::Abuse { .. } => "referral_abuse_setting_invalid",
        }
    }

    /// The setting's name, safe to log beside the label.
    pub fn setting(self) -> &'static str {
        match self {
            Self::ModeInvalid | Self::LiveUnconfigured => NEAR_AI_REFERRAL_MODE_ENV,
            Self::BpsInvalid => REFERRAL_BONUS_BPS_ENV,
            Self::EarnDaysBelowPromise | Self::EarnDaysInvalid => REFERRAL_EARN_DAYS_ENV,
            Self::CeilingInvalid => REFERRAL_ACCOUNT_CEILING_NANO_ENV,
            Self::Abuse { setting } => setting,
        }
    }
}

/// The bonus pass's settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReferralBonusSettings {
    /// `Disabled` or `DryRun`: the pass writes bonus rows and holds in
    /// `DryRun` and sends nothing in either.
    pub mode: PayoutMode,
    pub bps: u32,
    pub earn_days: u32,
    /// Rail atomic units (nano-USD on `near_ai`).
    pub account_ceiling: u128,
    pub abuse: ReferralAbuseSettings,
}

impl Default for ReferralBonusSettings {
    fn default() -> Self {
        Self {
            mode: PayoutMode::Disabled,
            bps: REFERRAL_BONUS_BPS_DEFAULT,
            earn_days: REFERRAL_EARN_DAYS_PROMISED,
            account_ceiling: REFERRAL_ACCOUNT_CEILING_NANO_DEFAULT,
            abuse: ReferralAbuseSettings::default(),
        }
    }
}

fn plain_decimal(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

impl ReferralBonusSettings {
    /// Reads the settings through `get` (`None` for unset). Pure.
    pub fn from_vars(
        get: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, ReferralBonusConfigError> {
        let mode = PayoutMode::parse(get(NEAR_AI_REFERRAL_MODE_ENV).as_deref())
            .map_err(|_| ReferralBonusConfigError::ModeInvalid)?;
        if mode == PayoutMode::Live {
            return Err(ReferralBonusConfigError::LiveUnconfigured);
        }
        let bps = match get(REFERRAL_BONUS_BPS_ENV) {
            None => REFERRAL_BONUS_BPS_DEFAULT,
            Some(value) => plain_decimal(&value)
                .then(|| value.parse::<u32>().ok())
                .flatten()
                .filter(|bps| (1..=10_000).contains(bps))
                .ok_or(ReferralBonusConfigError::BpsInvalid)?,
        };
        let earn_days = match get(REFERRAL_EARN_DAYS_ENV) {
            None => REFERRAL_EARN_DAYS_PROMISED,
            Some(value) => {
                let days = plain_decimal(&value)
                    .then(|| value.parse::<u32>().ok())
                    .flatten()
                    // Ten years: far beyond any extension, and inside what
                    // a timestamp can add.
                    .filter(|days| *days <= 3_650)
                    .ok_or(ReferralBonusConfigError::EarnDaysInvalid)?;
                if days < REFERRAL_EARN_DAYS_PROMISED {
                    return Err(ReferralBonusConfigError::EarnDaysBelowPromise);
                }
                days
            }
        };
        let account_ceiling = match get(REFERRAL_ACCOUNT_CEILING_NANO_ENV) {
            None => REFERRAL_ACCOUNT_CEILING_NANO_DEFAULT,
            Some(value) => plain_decimal(&value)
                .then(|| value.parse::<u128>().ok())
                .flatten()
                .filter(|ceiling| *ceiling > 0)
                .ok_or(ReferralBonusConfigError::CeilingInvalid)?,
        };
        let abuse = ReferralAbuseSettings::from_vars(&get).map_err(|error| {
            ReferralBonusConfigError::Abuse {
                setting: error.setting,
            }
        })?;
        Ok(Self {
            mode,
            bps,
            earn_days,
            account_ceiling,
            abuse,
        })
    }

    /// [`Self::from_vars`] over the process environment. A value that is not
    /// Unicode is refused as malformed, never read as unset.
    pub fn from_env() -> Result<Self, ReferralBonusConfigError> {
        if matches!(
            std::env::var(NEAR_AI_REFERRAL_MODE_ENV),
            Err(std::env::VarError::NotUnicode(_))
        ) {
            return Err(ReferralBonusConfigError::ModeInvalid);
        }
        Self::from_vars(|name| std::env::var(name).ok())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn with(
        vars: &[(&'static str, &str)],
    ) -> Result<ReferralBonusSettings, ReferralBonusConfigError> {
        let vars: HashMap<&str, String> = vars
            .iter()
            .map(|(name, value)| (*name, (*value).to_owned()))
            .collect();
        ReferralBonusSettings::from_vars(|name| vars.get(name).cloned())
    }

    #[test]
    fn unset_is_disabled_with_the_spec_defaults() {
        let settings = with(&[]).unwrap();
        assert_eq!(settings, ReferralBonusSettings::default());
        assert_eq!(settings.mode, PayoutMode::Disabled);
        assert_eq!(settings.bps, 1_000);
        assert_eq!(settings.earn_days, 92);
        assert_eq!(settings.account_ceiling, 250_000_000_000);
    }

    #[test]
    fn dry_run_is_accepted_and_live_refuses_without_the_credential() {
        assert_eq!(
            with(&[(NEAR_AI_REFERRAL_MODE_ENV, "dry_run")])
                .unwrap()
                .mode,
            PayoutMode::DryRun
        );
        let live = with(&[(NEAR_AI_REFERRAL_MODE_ENV, "live")]).unwrap_err();
        assert_eq!(live.label(), "near_ai_referral_config_missing");
        assert_eq!(live.setting(), NEAR_AI_REFERRAL_MODE_ENV);
        let typo = with(&[(NEAR_AI_REFERRAL_MODE_ENV, "dryrun")]).unwrap_err();
        assert_eq!(typo, ReferralBonusConfigError::ModeInvalid);
    }

    #[test]
    fn the_earning_period_can_be_raised_and_never_lowered() {
        assert_eq!(
            with(&[(REFERRAL_EARN_DAYS_ENV, "180")]).unwrap().earn_days,
            180
        );
        assert_eq!(
            with(&[(REFERRAL_EARN_DAYS_ENV, "92")]).unwrap().earn_days,
            92
        );
        assert_eq!(
            with(&[(REFERRAL_EARN_DAYS_ENV, "91")]).unwrap_err().label(),
            "referral_earn_days_below_promise"
        );
        assert_eq!(
            with(&[(REFERRAL_EARN_DAYS_ENV, "0")]).unwrap_err().label(),
            "referral_earn_days_below_promise"
        );
        for bad in ["", "-100", "ninety", "100.5", "3651"] {
            assert_eq!(
                with(&[(REFERRAL_EARN_DAYS_ENV, bad)]).unwrap_err(),
                ReferralBonusConfigError::EarnDaysInvalid,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn bps_is_one_to_ten_thousand() {
        assert_eq!(with(&[(REFERRAL_BONUS_BPS_ENV, "1")]).unwrap().bps, 1);
        assert_eq!(
            with(&[(REFERRAL_BONUS_BPS_ENV, "10000")]).unwrap().bps,
            10_000
        );
        for bad in ["0", "10001", "", "10%", " 1000"] {
            assert_eq!(
                with(&[(REFERRAL_BONUS_BPS_ENV, bad)]).unwrap_err(),
                ReferralBonusConfigError::BpsInvalid,
                "{bad:?}"
            );
        }
    }

    #[test]
    fn the_ceiling_is_above_zero_and_the_abuse_settings_are_checked() {
        assert_eq!(
            with(&[(REFERRAL_ACCOUNT_CEILING_NANO_ENV, "5")])
                .unwrap()
                .account_ceiling,
            5
        );
        assert_eq!(
            with(&[(REFERRAL_ACCOUNT_CEILING_NANO_ENV, "0")]).unwrap_err(),
            ReferralBonusConfigError::CeilingInvalid
        );
        let abuse =
            with(&[(crate::referral_abuse::REFERRAL_ABUSE_TRIP_CODES_ENV, "many")]).unwrap_err();
        assert_eq!(abuse.label(), "referral_abuse_setting_invalid");
        assert_eq!(
            abuse.setting(),
            crate::referral_abuse::REFERRAL_ABUSE_TRIP_CODES_ENV
        );
    }
}
