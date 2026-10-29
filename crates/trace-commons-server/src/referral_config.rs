// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The referral program's settings, read from the environment only
//! (docs/superpowers/specs/2026-10-29-referral-rewards-design.md, sections
//! 3 and 4).
//!
//! - `TRACE_COMMONS_REFERRAL_PROGRAM_START`, RFC 3339. Unset means the
//!   program has not started: nothing can be applied and no code made.
//! - `TRACE_COMMONS_REFERRAL_PROGRAM_END`, RFC 3339, optional, after the
//!   start.
//! - `TRACE_COMMONS_REFERRAL_LATE_DAYS`, how long after its creation an
//!   account may still apply a code: default 30, 1 to 365.
//! - `TRACE_COMMONS_REFERRAL_EARN_DAYS`, how long after a referee's account
//!   creation the referrer earns from it: default 92. Boot refuses a value
//!   below 92 (`referral_earn_days_below_promise`, owner decision R1: "3
//!   months promised but it can be extended"), and above 3650.
//! - `TRACE_COMMONS_REFERRAL_BONUS_BPS`, the bonus in basis points of each
//!   referee accrual: default 1000 (10%), 1 to 10000.
//!
//! A malformed value refuses boot with `referral_config_invalid` and the
//! setting's name (names are not secret), never the value. There is no
//! fallback to a default for a value that is present but wrong.

use chrono::{DateTime, Duration, Utc};

pub const REFERRAL_PROGRAM_START_ENV: &str = "TRACE_COMMONS_REFERRAL_PROGRAM_START";
pub const REFERRAL_PROGRAM_END_ENV: &str = "TRACE_COMMONS_REFERRAL_PROGRAM_END";
pub const REFERRAL_LATE_DAYS_ENV: &str = "TRACE_COMMONS_REFERRAL_LATE_DAYS";
pub const REFERRAL_EARN_DAYS_ENV: &str = "TRACE_COMMONS_REFERRAL_EARN_DAYS";
pub const REFERRAL_BONUS_BPS_ENV: &str = "TRACE_COMMONS_REFERRAL_BONUS_BPS";

/// Every setting this module reads.
pub const REFERRAL_ENV_NAMES: [&str; 5] = [
    REFERRAL_PROGRAM_START_ENV,
    REFERRAL_PROGRAM_END_ENV,
    REFERRAL_LATE_DAYS_ENV,
    REFERRAL_EARN_DAYS_ENV,
    REFERRAL_BONUS_BPS_ENV,
];

/// The promised earning period, in days: three months, rounded up.
pub const REFERRAL_EARN_DAYS_FLOOR: u32 = 92;
/// The longest earning period a setting may name. Ten years: far past any
/// extension the owner has discussed, and short enough that the arithmetic
/// on it can never overflow.
pub const REFERRAL_EARN_DAYS_MAX: u32 = 3650;
pub const REFERRAL_LATE_DAYS_DEFAULT: u32 = 30;
pub const REFERRAL_LATE_DAYS_MAX: u32 = 365;
pub const REFERRAL_BONUS_BPS_DEFAULT: u32 = 1000;
pub const REFERRAL_BONUS_BPS_MAX: u32 = 10_000;

/// Why the settings were refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ReferralConfigError {
    /// A setting is present but malformed or out of range, or the end is
    /// not after the start.
    #[error("referral_config_invalid")]
    Invalid { setting: &'static str },
    /// The earning period is shorter than the promised three months.
    #[error("referral_earn_days_below_promise")]
    EarnDaysBelowPromise,
}

impl ReferralConfigError {
    pub fn label(self) -> &'static str {
        match self {
            Self::Invalid { .. } => "referral_config_invalid",
            Self::EarnDaysBelowPromise => "referral_earn_days_below_promise",
        }
    }

    /// The setting's name, safe to log beside the label.
    pub fn setting(self) -> &'static str {
        match self {
            Self::Invalid { setting } => setting,
            Self::EarnDaysBelowPromise => REFERRAL_EARN_DAYS_ENV,
        }
    }
}

/// The referral program's settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReferralConfig {
    pub program_start: Option<DateTime<Utc>>,
    pub program_end: Option<DateTime<Utc>>,
    pub late_days: u32,
    pub earn_days: u32,
    pub bonus_bps: u32,
}

impl Default for ReferralConfig {
    /// Not started, with every other setting at its default.
    fn default() -> Self {
        Self {
            program_start: None,
            program_end: None,
            late_days: REFERRAL_LATE_DAYS_DEFAULT,
            earn_days: REFERRAL_EARN_DAYS_FLOOR,
            bonus_bps: REFERRAL_BONUS_BPS_DEFAULT,
        }
    }
}

impl ReferralConfig {
    /// Reads the settings through `get`, which answers `None` for an unset
    /// variable. Pure, so tests need not touch the process environment.
    pub fn from_vars(get: impl Fn(&str) -> Option<String>) -> Result<Self, ReferralConfigError> {
        let read = |setting: &'static str| -> Result<Option<String>, ReferralConfigError> {
            match get(setting) {
                None => Ok(None),
                Some(value) if value.is_empty() => Ok(None),
                Some(value) if value.trim() != value => {
                    Err(ReferralConfigError::Invalid { setting })
                }
                Some(value) => Ok(Some(value)),
            }
        };
        let instant =
            |setting: &'static str| -> Result<Option<DateTime<Utc>>, ReferralConfigError> {
                read(setting)?
                    .map(|value| {
                        DateTime::parse_from_rfc3339(&value)
                            .map(|parsed| parsed.with_timezone(&Utc))
                            .map_err(|_| ReferralConfigError::Invalid { setting })
                    })
                    .transpose()
            };
        let number = |setting: &'static str| -> Result<Option<u32>, ReferralConfigError> {
            read(setting)?
                .map(|value| {
                    if !value.bytes().all(|byte| byte.is_ascii_digit()) {
                        return Err(ReferralConfigError::Invalid { setting });
                    }
                    value
                        .parse::<u32>()
                        .map_err(|_| ReferralConfigError::Invalid { setting })
                })
                .transpose()
        };

        let program_start = instant(REFERRAL_PROGRAM_START_ENV)?;
        let program_end = instant(REFERRAL_PROGRAM_END_ENV)?;
        if let Some(end) = program_end {
            // An end without a start names a program that never opens; an
            // end at or before the start, one that is never open.
            if program_start.is_none_or(|start| end <= start) {
                return Err(ReferralConfigError::Invalid {
                    setting: REFERRAL_PROGRAM_END_ENV,
                });
            }
        }
        let late_days = number(REFERRAL_LATE_DAYS_ENV)?.unwrap_or(REFERRAL_LATE_DAYS_DEFAULT);
        if !(1..=REFERRAL_LATE_DAYS_MAX).contains(&late_days) {
            return Err(ReferralConfigError::Invalid {
                setting: REFERRAL_LATE_DAYS_ENV,
            });
        }
        let earn_days = number(REFERRAL_EARN_DAYS_ENV)?.unwrap_or(REFERRAL_EARN_DAYS_FLOOR);
        if earn_days < REFERRAL_EARN_DAYS_FLOOR {
            return Err(ReferralConfigError::EarnDaysBelowPromise);
        }
        if earn_days > REFERRAL_EARN_DAYS_MAX {
            return Err(ReferralConfigError::Invalid {
                setting: REFERRAL_EARN_DAYS_ENV,
            });
        }
        let bonus_bps = number(REFERRAL_BONUS_BPS_ENV)?.unwrap_or(REFERRAL_BONUS_BPS_DEFAULT);
        if !(1..=REFERRAL_BONUS_BPS_MAX).contains(&bonus_bps) {
            return Err(ReferralConfigError::Invalid {
                setting: REFERRAL_BONUS_BPS_ENV,
            });
        }
        Ok(Self {
            program_start,
            program_end,
            late_days,
            earn_days,
            bonus_bps,
        })
    }

    /// Reads the process environment.
    pub fn from_env() -> Result<Self, ReferralConfigError> {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// Whether the program is open at `now`: started, and not ended.
    pub fn is_open_at(&self, now: DateTime<Utc>) -> bool {
        self.program_start.is_some_and(|start| now >= start)
            && self.program_end.is_none_or(|end| now < end)
    }

    /// The end of a referee's earning period. Computed when read, so raising
    /// the setting extends every live referral without a data change.
    pub fn earn_until(&self, referee_account_created_at: DateTime<Utc>) -> DateTime<Utc> {
        referee_account_created_at + Duration::days(i64::from(self.earn_days))
    }

    /// The last moment an account created at `account_created_at` may apply
    /// a code, or `None` when it never may (created before the program
    /// started, or no program).
    pub fn apply_until(&self, account_created_at: DateTime<Utc>) -> Option<DateTime<Utc>> {
        let start = self.program_start?;
        if account_created_at < start {
            return None;
        }
        let late = account_created_at + Duration::days(i64::from(self.late_days));
        Some(match self.program_end {
            Some(end) if end < late => end,
            _ => late,
        })
    }

    /// What the store checks an apply against.
    pub fn apply_policy(&self) -> crate::db::ReferralApplyPolicy {
        crate::db::ReferralApplyPolicy {
            program_start: self.program_start,
            program_end: self.program_end,
            late_days: self.late_days,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn config(vars: &[(&str, &str)]) -> Result<ReferralConfig, ReferralConfigError> {
        let vars: HashMap<String, String> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        ReferralConfig::from_vars(|name| vars.get(name).cloned())
    }

    fn at(value: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(value)
            .unwrap()
            .with_timezone(&Utc)
    }

    #[test]
    fn nothing_set_is_a_program_that_has_not_started() {
        let config = config(&[]).unwrap();
        assert_eq!(config, ReferralConfig::default());
        assert_eq!(config.late_days, 30);
        assert_eq!(config.earn_days, 92);
        assert_eq!(config.bonus_bps, 1000);
        assert!(!config.is_open_at(Utc::now()));
        assert_eq!(config.apply_until(Utc::now()), None);
    }

    #[test]
    fn the_program_is_open_from_its_start_until_its_end() {
        let config = config(&[
            (REFERRAL_PROGRAM_START_ENV, "2026-11-01T00:00:00Z"),
            (REFERRAL_PROGRAM_END_ENV, "2027-11-01T00:00:00Z"),
        ])
        .unwrap();
        assert!(!config.is_open_at(at("2026-10-31T23:59:59Z")));
        assert!(config.is_open_at(at("2026-11-01T00:00:00Z")));
        assert!(config.is_open_at(at("2027-10-31T23:59:59Z")));
        assert!(!config.is_open_at(at("2027-11-01T00:00:00Z")));
        let open_ended =
            super::tests::config(&[(REFERRAL_PROGRAM_START_ENV, "2026-11-01T00:00:00+01:00")])
                .unwrap();
        assert_eq!(open_ended.program_start, Some(at("2026-10-31T23:00:00Z")));
        assert!(open_ended.is_open_at(at("2036-01-01T00:00:00Z")));
    }

    #[test]
    fn the_end_must_follow_a_start() {
        for vars in [
            vec![(REFERRAL_PROGRAM_END_ENV, "2027-11-01T00:00:00Z")],
            vec![
                (REFERRAL_PROGRAM_START_ENV, "2026-11-01T00:00:00Z"),
                (REFERRAL_PROGRAM_END_ENV, "2026-11-01T00:00:00Z"),
            ],
            vec![
                (REFERRAL_PROGRAM_START_ENV, "2026-11-01T00:00:00Z"),
                (REFERRAL_PROGRAM_END_ENV, "2026-10-01T00:00:00Z"),
            ],
        ] {
            assert_eq!(
                config(&vars),
                Err(ReferralConfigError::Invalid {
                    setting: REFERRAL_PROGRAM_END_ENV
                }),
                "{vars:?}"
            );
        }
    }

    #[test]
    fn the_earning_period_never_drops_below_the_promise() {
        assert_eq!(
            config(&[(REFERRAL_EARN_DAYS_ENV, "91")]),
            Err(ReferralConfigError::EarnDaysBelowPromise)
        );
        assert_eq!(
            ReferralConfigError::EarnDaysBelowPromise.label(),
            "referral_earn_days_below_promise"
        );
        assert_eq!(
            config(&[(REFERRAL_EARN_DAYS_ENV, "0")]),
            Err(ReferralConfigError::EarnDaysBelowPromise)
        );
        assert_eq!(
            config(&[(REFERRAL_EARN_DAYS_ENV, "92")]).unwrap().earn_days,
            92
        );
        assert_eq!(
            config(&[(REFERRAL_EARN_DAYS_ENV, "183")])
                .unwrap()
                .earn_days,
            183,
            "an operator may extend it"
        );
        assert_eq!(
            config(&[(REFERRAL_EARN_DAYS_ENV, "3651")]),
            Err(ReferralConfigError::Invalid {
                setting: REFERRAL_EARN_DAYS_ENV
            })
        );
        let config = config(&[(REFERRAL_EARN_DAYS_ENV, "100")]).unwrap();
        assert_eq!(
            config.earn_until(at("2026-11-01T00:00:00Z")),
            at("2027-02-09T00:00:00Z")
        );
    }

    #[test]
    fn malformed_values_refuse_and_name_only_the_setting() {
        for (setting, value) in [
            (REFERRAL_PROGRAM_START_ENV, "2026-11-01"),
            (REFERRAL_PROGRAM_START_ENV, "tomorrow"),
            (REFERRAL_PROGRAM_START_ENV, " 2026-11-01T00:00:00Z"),
            (REFERRAL_LATE_DAYS_ENV, "0"),
            (REFERRAL_LATE_DAYS_ENV, "366"),
            (REFERRAL_LATE_DAYS_ENV, "-1"),
            (REFERRAL_LATE_DAYS_ENV, "+30"),
            (REFERRAL_LATE_DAYS_ENV, "30.5"),
            (REFERRAL_EARN_DAYS_ENV, "ninety-two"),
            (REFERRAL_EARN_DAYS_ENV, "99999999999"),
            (REFERRAL_BONUS_BPS_ENV, "0"),
            (REFERRAL_BONUS_BPS_ENV, "10001"),
            (REFERRAL_BONUS_BPS_ENV, "10%"),
        ] {
            let error = config(&[(setting, value)]).expect_err(value);
            assert_eq!(error.setting(), setting, "{setting}={value}");
            assert!(!error.to_string().contains(value), "{error}");
        }
        assert_eq!(
            config(&[(REFERRAL_BONUS_BPS_ENV, "10000")])
                .unwrap()
                .bonus_bps,
            10_000
        );
        assert_eq!(
            config(&[(REFERRAL_BONUS_BPS_ENV, "1")]).unwrap().bonus_bps,
            1
        );
        assert_eq!(
            config(&[(REFERRAL_LATE_DAYS_ENV, "")]).unwrap().late_days,
            30,
            "an empty value is unset"
        );
    }

    #[test]
    fn a_code_may_be_applied_for_late_days_after_creation_within_the_program() {
        let config = config(&[
            (REFERRAL_PROGRAM_START_ENV, "2026-11-01T00:00:00Z"),
            (REFERRAL_PROGRAM_END_ENV, "2026-12-15T00:00:00Z"),
        ])
        .unwrap();
        assert_eq!(config.apply_until(at("2026-10-31T00:00:00Z")), None);
        assert_eq!(
            config.apply_until(at("2026-11-02T00:00:00Z")),
            Some(at("2026-12-02T00:00:00Z"))
        );
        assert_eq!(
            config.apply_until(at("2026-12-01T00:00:00Z")),
            Some(at("2026-12-15T00:00:00Z")),
            "the program's end comes first"
        );
        let policy = config.apply_policy();
        assert_eq!(policy.program_start, config.program_start);
        assert_eq!(policy.program_end, config.program_end);
        assert_eq!(policy.late_days, 30);
    }
}
