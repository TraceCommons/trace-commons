// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR AI inference-credit rail: settles converted Trace Commons credit
//! as a NEAR AI organization's cumulative `trace_commons` credit.
//!
//! The model is NEAR AI's own staking-farm sync
//! (`cloud-api:crates/services/src/staking_farm.rs:319-430`): a dedicated
//! credit type that no other writer touches, whose limit is set to a
//! cumulative total and only ever rises (`max(computed, last)`), written
//! under the organization accounting lock. Publishing our cumulative total is
//! therefore idempotent -- a retry, a duplicate or a late delivery of an
//! older total changes nothing -- and needs no compare-and-swap.
//!
//! Two delivery modes sit behind the same [`SettlementRail`] seam:
//!
//! - [`PushTraceCommonsCreditRail`]: we call the proposed admin route with
//!   our total. Needs a NEAR AI read-write admin key.
//! - [`PullTraceCommonsCreditRail`]: NEAR AI fetches a signed statement of
//!   our total on its own schedule (see `near_ai_credit_statement`). Needs no
//!   admin key; a read-only key, if configured, lets the reconciler observe.

use crate::near_ai_admin::{
    AdminApiError, NearAiAdminApi, NearAiOrgId, SetTraceCommonsCreditRequest,
};
use crate::near_ai_credit_statement::StatementSigner;
use crate::settlement_rail::{
    ConversionRate, CumulativeStatement, Published, RailError, RecipientRef, SettlementRail,
};
use async_trait::async_trait;
use secrecy::SecretString;
use std::sync::Arc;
use std::time::Duration;

pub const RAIL_ID: &str = "near_ai_trace_commons";
pub const INSTRUMENT_ID: &str = "near_ai_credit_usd";
/// NEAR AI amounts are nano-USD, scale 9 (`cloud-api:crates/api/src/models.rs:4623-4637`).
pub const INSTRUMENT_DECIMALS: u8 = 9;
/// NEAR AI refuses a limit above $1B (`cloud-api:crates/services/src/admin/mod.rs:1504`).
pub const NEAR_AI_MAX_NANO_USD: u128 = 1_000_000_000_000_000_000;

pub const ENV_MODE: &str = "TRACE_COMMONS_NEAR_AI_CREDIT_RAIL";
pub const ENV_DELIVERY: &str = "TRACE_COMMONS_NEAR_AI_CREDIT_DELIVERY";
pub const ENV_BASE_URL: &str = "TRACE_COMMONS_NEAR_AI_ADMIN_BASE_URL";
pub const ENV_KEY: &str = "TRACE_COMMONS_NEAR_AI_ADMIN_KEY";
pub const ENV_READ_KEY: &str = "TRACE_COMMONS_NEAR_AI_ADMIN_READ_KEY";
pub const ENV_USER_AGENT: &str = "TRACE_COMMONS_NEAR_AI_ADMIN_USER_AGENT";
pub const ENV_RATE: &str = "TRACE_COMMONS_NEAR_AI_CREDIT_RATE";
pub const ENV_MAX_CUMULATIVE: &str = "TRACE_COMMONS_NEAR_AI_CREDIT_MAX_CUMULATIVE_NANO_USD";
pub const ENV_STATEMENT_KEY: &str = "TRACE_COMMONS_NEAR_AI_CREDIT_STATEMENT_KEY";
pub const ENV_TIMEOUT_MS: &str = "TRACE_COMMONS_NEAR_AI_ADMIN_TIMEOUT_MS";

/// The operator kill switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailMode {
    Disabled,
    /// Everything except the external write.
    DryRun,
    Live,
}

impl RailMode {
    /// Unset or unrecognised is `Disabled`: a typo never turns the rail on.
    pub fn parse(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("live") => Self::Live,
            Some("dry_run") => Self::DryRun,
            _ => Self::Disabled,
        }
    }
}

/// How the total reaches NEAR AI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Delivery {
    Push,
    Pull,
}

/// A configuration refusal: names the missing or invalid control and
/// nothing else.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("missing_control:{control}")]
pub struct ConfigRefused {
    pub control: &'static str,
}

const fn refuse(control: &'static str) -> ConfigRefused {
    ConfigRefused { control }
}

/// Resolved rail configuration. `Debug` omits every secret.
#[derive(Clone)]
pub struct NearAiCreditRailConfig {
    pub mode: RailMode,
    pub delivery: Delivery,
    pub base_url: Option<String>,
    pub write_key: Option<SecretString>,
    pub read_key: Option<SecretString>,
    pub user_agent: Option<String>,
    pub statement_signer: Option<StatementSigner>,
    pub rate: ConversionRate,
    pub max_cumulative_nano_usd: u128,
    pub timeout: Duration,
}

impl std::fmt::Debug for NearAiCreditRailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearAiCreditRailConfig")
            .field("mode", &self.mode)
            .field("delivery", &self.delivery)
            .field("write_key_present", &self.write_key.is_some())
            .field("read_key_present", &self.read_key.is_some())
            .field("statement_key_present", &self.statement_signer.is_some())
            .field("rate", &self.rate.canonical())
            .field("max_cumulative_nano_usd", &self.max_cumulative_nano_usd)
            .finish_non_exhaustive()
    }
}

fn is_loopback_http(url: &str) -> bool {
    url.starts_with("http://127.0.0.1:") || url.starts_with("http://[::1]:")
}

impl NearAiCreditRailConfig {
    /// Read the configuration through `lookup` (the process environment in
    /// production, a map in tests). `Ok(None)` means disabled. An enabled
    /// mode with a missing or invalid control refuses; there is no fallback
    /// to a less restrictive mode or another delivery.
    pub fn from_lookup(
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Option<Self>, ConfigRefused> {
        let get = |name: &str| {
            lookup(name)
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
        };
        let mode = RailMode::parse(get(ENV_MODE).as_deref());
        if mode == RailMode::Disabled {
            return Ok(None);
        }
        let delivery = match get(ENV_DELIVERY)
            .as_deref()
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            None | Some("push") => Delivery::Push,
            Some("pull") => Delivery::Pull,
            Some(_) => return Err(refuse("near_ai_credit_delivery")),
        };
        let key_of =
            |name: &str, control: &'static str| -> Result<Option<SecretString>, ConfigRefused> {
                match get(name) {
                    None => Ok(None),
                    Some(v) if v.starts_with("adm_") && !v.contains(char::is_whitespace) => {
                        Ok(Some(SecretString::from(v)))
                    }
                    Some(_) => Err(refuse(control)),
                }
            };
        let write_key = key_of(ENV_KEY, "near_ai_admin_key")?;
        let read_key = key_of(ENV_READ_KEY, "near_ai_admin_read_key")?;
        let base_url = get(ENV_BASE_URL);
        if let Some(url) = &base_url {
            if !(url.starts_with("https://") || is_loopback_http(url)) {
                return Err(refuse("near_ai_admin_base_url"));
            }
        }
        let user_agent = get(ENV_USER_AGENT);
        let statement_signer = match get(ENV_STATEMENT_KEY) {
            None => None,
            Some(v) => Some(
                StatementSigner::from_base64_pkcs8(&v)
                    .map_err(|_| refuse("near_ai_credit_statement_key"))?,
            ),
        };
        let needs_admin_api = match delivery {
            Delivery::Push => {
                match mode {
                    RailMode::Live if write_key.is_none() => {
                        return Err(refuse("near_ai_admin_key"));
                    }
                    RailMode::DryRun if write_key.is_none() && read_key.is_none() => {
                        return Err(refuse("near_ai_admin_read_key"));
                    }
                    _ => {}
                }
                true
            }
            Delivery::Pull => {
                if statement_signer.is_none() {
                    return Err(refuse("near_ai_credit_statement_key"));
                }
                read_key.is_some()
            }
        };
        if needs_admin_api {
            if base_url.is_none() {
                return Err(refuse("near_ai_admin_base_url"));
            }
            if user_agent.is_none() {
                return Err(refuse("near_ai_admin_user_agent"));
            }
        }
        let rate = get(ENV_RATE)
            .ok_or(refuse("near_ai_credit_rate"))
            .and_then(|v| ConversionRate::parse(&v).map_err(|_| refuse("near_ai_credit_rate")))?;
        let max_cumulative_nano_usd = get(ENV_MAX_CUMULATIVE)
            .ok_or(refuse("near_ai_credit_max_cumulative"))?
            .parse::<u128>()
            .ok()
            .filter(|v| *v > 0 && *v <= NEAR_AI_MAX_NANO_USD)
            .ok_or(refuse("near_ai_credit_max_cumulative"))?;
        let timeout = match get(ENV_TIMEOUT_MS) {
            None => Duration::from_secs(15),
            Some(v) => v
                .parse::<u64>()
                .ok()
                .filter(|ms| (100..=120_000).contains(ms))
                .map(Duration::from_millis)
                .ok_or(refuse("near_ai_admin_timeout_ms"))?,
        };
        Ok(Some(Self {
            mode,
            delivery,
            base_url,
            write_key,
            read_key,
            user_agent,
            statement_signer,
            rate,
            max_cumulative_nano_usd,
            timeout,
        }))
    }

    pub fn from_env() -> Result<Option<Self>, ConfigRefused> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }
}

fn org_of(recipient: &RecipientRef) -> Result<NearAiOrgId, RailError> {
    NearAiOrgId::parse(recipient.expose()).ok_or(RailError::frozen("near_ai_recipient_malformed"))
}

fn bounded(cumulative: u128, max: u128) -> Result<i64, RailError> {
    if cumulative > max {
        return Err(RailError::frozen("near_ai_cumulative_above_ceiling"));
    }
    i64::try_from(cumulative).map_err(|_| RailError::frozen("near_ai_cumulative_above_ceiling"))
}

fn units_of(nano_usd: i64) -> Result<u128, RailError> {
    u128::try_from(nano_usd).map_err(|_| RailError::frozen("near_ai_admin_response_malformed"))
}

fn read_error(e: AdminApiError) -> RailError {
    match e {
        AdminApiError::Forbidden => RailError::frozen("near_ai_admin_forbidden"),
        AdminApiError::NotFound | AdminApiError::Rejected => RailError::frozen(e.label()),
        other => RailError::retry(other.label()),
    }
}

fn write_error(e: AdminApiError) -> RailError {
    match e {
        AdminApiError::Forbidden => RailError::frozen("near_ai_admin_read_only_key"),
        AdminApiError::NotFound | AdminApiError::Rejected => RailError::frozen(e.label()),
        // Unreachable, ambiguous, unauthorized, unavailable, malformed: a
        // cumulative write is safe to repeat.
        other => RailError::retry(other.label()),
    }
}

async fn observe_with(
    api: &dyn NearAiAdminApi,
    recipient: &RecipientRef,
) -> Result<u128, RailError> {
    let org = org_of(recipient)?;
    let current = api
        .get_trace_commons_credit(org)
        .await
        .map_err(read_error)?;
    units_of(current.cumulative_nano_usd)
}

/// Push delivery: we state the total to NEAR AI's admin route.
pub struct PushTraceCommonsCreditRail {
    api: Arc<dyn NearAiAdminApi>,
    max_cumulative_nano_usd: u128,
    dry_run: bool,
}

impl PushTraceCommonsCreditRail {
    pub fn new(api: Arc<dyn NearAiAdminApi>, max_cumulative_nano_usd: u128, dry_run: bool) -> Self {
        Self {
            api,
            max_cumulative_nano_usd,
            dry_run,
        }
    }
}

#[async_trait]
impl SettlementRail for PushTraceCommonsCreditRail {
    fn rail_id(&self) -> &'static str {
        RAIL_ID
    }
    fn instrument_id(&self) -> &'static str {
        INSTRUMENT_ID
    }
    fn instrument_decimals(&self) -> u8 {
        INSTRUMENT_DECIMALS
    }

    async fn publish(&self, statement: &CumulativeStatement) -> Result<Published, RailError> {
        let amount = bounded(statement.cumulative_units, self.max_cumulative_nano_usd)?;
        let org = org_of(&statement.recipient)?;
        if self.dry_run {
            // Prove the read path and the recipient, then stop.
            self.api
                .get_trace_commons_credit(org)
                .await
                .map_err(read_error)?;
            return Err(RailError::dry_run("near_ai_rail_dry_run"));
        }
        let response = self
            .api
            .set_trace_commons_credit(
                org,
                &SetTraceCommonsCreditRequest {
                    cumulative_nano_usd: amount,
                    statement_ref: statement.statement_ref.clone(),
                },
            )
            .await
            .map_err(write_error)?;
        Ok(Published::Recorded {
            effective_units: units_of(response.cumulative_nano_usd)?,
            applied: response.applied,
        })
    }

    async fn observe(&self, recipient: &RecipientRef) -> Result<Option<u128>, RailError> {
        observe_with(self.api.as_ref(), recipient).await.map(Some)
    }
}

/// Pull delivery: NEAR AI fetches our signed statement. Publishing only
/// checks the total is servable; the statement is signed when served.
pub struct PullTraceCommonsCreditRail {
    observer: Option<Arc<dyn NearAiAdminApi>>,
    max_cumulative_nano_usd: u128,
}

impl PullTraceCommonsCreditRail {
    pub fn new(observer: Option<Arc<dyn NearAiAdminApi>>, max_cumulative_nano_usd: u128) -> Self {
        Self {
            observer,
            max_cumulative_nano_usd,
        }
    }
}

#[async_trait]
impl SettlementRail for PullTraceCommonsCreditRail {
    fn rail_id(&self) -> &'static str {
        RAIL_ID
    }
    fn instrument_id(&self) -> &'static str {
        INSTRUMENT_ID
    }
    fn instrument_decimals(&self) -> u8 {
        INSTRUMENT_DECIMALS
    }

    async fn publish(&self, statement: &CumulativeStatement) -> Result<Published, RailError> {
        bounded(statement.cumulative_units, self.max_cumulative_nano_usd)?;
        org_of(&statement.recipient)?;
        Ok(Published::Deferred)
    }

    async fn observe(&self, recipient: &RecipientRef) -> Result<Option<u128>, RailError> {
        match &self.observer {
            None => Ok(None),
            Some(api) => observe_with(api.as_ref(), recipient).await.map(Some),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, String)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.clone()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn push_live() -> Vec<(&'static str, String)> {
        vec![
            (ENV_MODE, "live".into()),
            (ENV_BASE_URL, "https://cloud-api.near.ai/v1".into()),
            (ENV_KEY, "adm_test".into()),
            (ENV_USER_AGENT, "trace-commons-settlement/1".into()),
            (ENV_RATE, "1000/1".into()),
            (ENV_MAX_CUMULATIVE, "100000000000".into()),
        ]
    }

    fn with(
        mut pairs: Vec<(&'static str, String)>,
        name: &'static str,
        value: Option<&str>,
    ) -> Vec<(&'static str, String)> {
        pairs.retain(|(k, _)| *k != name);
        if let Some(v) = value {
            pairs.push((name, v.to_string()));
        }
        pairs
    }

    #[test]
    fn unset_or_unknown_mode_is_disabled() {
        assert!(
            NearAiCreditRailConfig::from_lookup(env(&[]))
                .unwrap()
                .is_none()
        );
        let on = vec![(ENV_MODE, "on".to_string())];
        assert!(
            NearAiCreditRailConfig::from_lookup(env(&on))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn push_live_without_the_admin_key_refuses_and_a_read_key_does_not_satisfy_it() {
        let pairs = with(push_live(), ENV_KEY, None);
        let err = NearAiCreditRailConfig::from_lookup(env(&pairs)).unwrap_err();
        assert_eq!(err.to_string(), "missing_control:near_ai_admin_key");
        let pairs = with(pairs, ENV_READ_KEY, Some("adm_read"));
        assert_eq!(
            NearAiCreditRailConfig::from_lookup(env(&pairs))
                .unwrap_err()
                .control,
            "near_ai_admin_key"
        );
    }

    #[test]
    fn each_required_push_control_refuses_by_name() {
        for (drop, control) in [
            (ENV_BASE_URL, "near_ai_admin_base_url"),
            (ENV_USER_AGENT, "near_ai_admin_user_agent"),
            (ENV_RATE, "near_ai_credit_rate"),
            (ENV_MAX_CUMULATIVE, "near_ai_credit_max_cumulative"),
        ] {
            let pairs = with(push_live(), drop, None);
            assert_eq!(
                NearAiCreditRailConfig::from_lookup(env(&pairs))
                    .unwrap_err()
                    .control,
                control
            );
        }
    }

    #[test]
    fn invalid_values_refuse_and_never_echo_the_value() {
        for (name, value, control) in [
            (ENV_KEY, "sk-not-admin", "near_ai_admin_key"),
            (
                ENV_BASE_URL,
                "http://cloud-api.near.ai/v1",
                "near_ai_admin_base_url",
            ),
            (ENV_RATE, "0/1", "near_ai_credit_rate"),
            (ENV_MAX_CUMULATIVE, "0", "near_ai_credit_max_cumulative"),
            (
                ENV_MAX_CUMULATIVE,
                "1000000000000000001",
                "near_ai_credit_max_cumulative",
            ),
            (ENV_DELIVERY, "carrier-pigeon", "near_ai_credit_delivery"),
            (
                ENV_STATEMENT_KEY,
                "not-base64-pkcs8",
                "near_ai_credit_statement_key",
            ),
        ] {
            let pairs = with(push_live(), name, Some(value));
            let err = NearAiCreditRailConfig::from_lookup(env(&pairs)).unwrap_err();
            assert_eq!(err.control, control, "{name}");
            assert!(!err.to_string().contains(value));
        }
    }

    #[test]
    fn pull_needs_the_statement_key_and_no_admin_key() {
        let pull = vec![
            (ENV_MODE, "live".to_string()),
            (ENV_DELIVERY, "pull".to_string()),
            (ENV_RATE, "1/1".to_string()),
            (ENV_MAX_CUMULATIVE, "1000".to_string()),
        ];
        assert_eq!(
            NearAiCreditRailConfig::from_lookup(env(&pull))
                .unwrap_err()
                .control,
            "near_ai_credit_statement_key"
        );
        let key = crate::near_ai_credit_statement::tests_support::fresh_pkcs8_base64();
        let pull = with(pull, ENV_STATEMENT_KEY, Some(&key));
        let cfg = NearAiCreditRailConfig::from_lookup(env(&pull))
            .unwrap()
            .unwrap();
        assert_eq!(cfg.delivery, Delivery::Pull);
        assert!(cfg.write_key.is_none());
        assert!(!format!("{cfg:?}").contains(&key));
        // A read key for observation brings the admin-API controls with it.
        let observed = with(pull, ENV_READ_KEY, Some("adm_read"));
        assert_eq!(
            NearAiCreditRailConfig::from_lookup(env(&observed))
                .unwrap_err()
                .control,
            "near_ai_admin_base_url"
        );
    }

    #[test]
    fn a_complete_push_configuration_resolves_without_printing_keys() {
        let cfg = NearAiCreditRailConfig::from_lookup(env(&push_live()))
            .unwrap()
            .unwrap();
        assert_eq!((cfg.mode, cfg.delivery), (RailMode::Live, Delivery::Push));
        assert_eq!(cfg.rate.canonical(), "1000/1");
        assert!(!format!("{cfg:?}").contains("adm_test"));
    }
}
