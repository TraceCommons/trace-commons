// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR AI inference-credit rail: settles converted Trace Commons credit
//! as a raise of a NEAR AI organization's spend ceiling.
//!
//! NEAR AI offers only an absolute, per-credit-type ceiling write with no
//! compare-and-swap and no idempotency key, plus a history of every write
//! with its author and reason (spec section 1). This rail makes that safe
//! enough to use by refusing to build on anything it cannot account for:
//!
//! 1. scan history for this delivery's marker -- if present, the write
//!    already landed (a timeout after send) and is never repeated;
//! 2. require the active row of the credit type to be absent (first write)
//!    or exactly the row we last recorded -- anything else freezes;
//! 3. write `prior + delta` with the marker in `changeReason`;
//! 4. read back and confirm our row superseded the row we read.
//!
//! Every anomaly is a freeze for an operator, never a retry.

use crate::near_ai_admin::{
    AdminApiError, LimitsHistoryEntry, NearAiAdminApi, NearAiOrgId, SpendLimitRequest,
    UpdateLimitsRequest,
};
use crate::settlement_rail::{
    ConversionRate, RailDelivery, RailError, RailOutcome, RailReceipt, RailVerification,
    SettlementRail, sha256_ref,
};
use async_trait::async_trait;
use secrecy::SecretString;
use std::sync::Arc;
use std::time::Duration;

pub const RAIL_ID: &str = "near_ai_org_limits";
pub const INSTRUMENT_ID: &str = "near_ai_credit_usd";
/// NEAR AI amounts are nano-USD, scale 9 (`cloud-api:crates/api/src/models.rs:4623-4637`).
pub const INSTRUMENT_DECIMALS: u8 = 9;
/// NEAR AI refuses a ceiling above $1B (`cloud-api:crates/services/src/admin/mod.rs:1504`).
pub const NEAR_AI_MAX_SPEND_LIMIT_NANO_USD: u128 = 1_000_000_000_000_000_000;
/// Our `changedBy`, the author label the precondition recognises.
pub const CHANGED_BY: &str = "trace-commons-settlement";
pub const SOURCE: &str = "trace-commons";
pub const MARKER_PREFIX: &str = "trace-commons:settle:v1:";
const HISTORY_PAGE: i64 = 100;
const HISTORY_MAX_PAGES: i64 = 100;

pub const ENV_MODE: &str = "TRACE_COMMONS_NEAR_AI_CREDIT_RAIL";
pub const ENV_BASE_URL: &str = "TRACE_COMMONS_NEAR_AI_ADMIN_BASE_URL";
pub const ENV_KEY: &str = "TRACE_COMMONS_NEAR_AI_ADMIN_KEY";
pub const ENV_READ_KEY: &str = "TRACE_COMMONS_NEAR_AI_ADMIN_READ_KEY";
pub const ENV_USER_AGENT: &str = "TRACE_COMMONS_NEAR_AI_ADMIN_USER_AGENT";
pub const ENV_RATE: &str = "TRACE_COMMONS_NEAR_AI_CREDIT_RATE";
pub const ENV_MAX_DELIVERY: &str = "TRACE_COMMONS_NEAR_AI_CREDIT_MAX_DELIVERY_NANO_USD";
pub const ENV_CREDIT_TYPE: &str = "TRACE_COMMONS_NEAR_AI_CREDIT_TYPE";
pub const ENV_TIMEOUT_MS: &str = "TRACE_COMMONS_NEAR_AI_ADMIN_TIMEOUT_MS";

/// The marker written into `changeReason` for one delivery.
pub fn marker(delivery_hash: &str) -> String {
    format!("{MARKER_PREFIX}{delivery_hash}")
}

/// The hash recorded in place of a NEAR AI history row id.
pub fn history_row_ref(row_id: &str) -> String {
    sha256_ref("trace-commons:near-ai-limits-row:v1", &[row_id])
}

/// The operator kill switch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RailMode {
    Disabled,
    /// Everything except the PATCH.
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

/// Resolved rail configuration. `Debug` omits the keys.
#[derive(Clone)]
pub struct NearAiCreditRailConfig {
    pub mode: RailMode,
    pub base_url: String,
    pub write_key: Option<SecretString>,
    pub read_key: Option<SecretString>,
    pub user_agent: String,
    pub rate: ConversionRate,
    pub max_delivery_nano_usd: u128,
    pub credit_type: String,
    pub timeout: Duration,
}

impl std::fmt::Debug for NearAiCreditRailConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NearAiCreditRailConfig")
            .field("mode", &self.mode)
            .field("write_key_present", &self.write_key.is_some())
            .field("read_key_present", &self.read_key.is_some())
            .field("rate", &self.rate.canonical())
            .field("max_delivery_nano_usd", &self.max_delivery_nano_usd)
            .field("credit_type", &self.credit_type)
            .finish_non_exhaustive()
    }
}

fn is_loopback_http(url: &str) -> bool {
    url.starts_with("http://127.0.0.1:") || url.starts_with("http://[::1]:")
}

impl NearAiCreditRailConfig {
    /// Read the configuration through `lookup` (the process environment in
    /// production, a map in tests).
    ///
    /// `Ok(None)` means the rail is disabled. Any enabled mode with a
    /// missing or invalid control refuses; there is no fallback to a less
    /// restrictive mode. `live` requires the read-write key. `dry_run`
    /// requires some key, because it reads history.
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
        let base_url = get(ENV_BASE_URL).ok_or(refuse("near_ai_admin_base_url"))?;
        if !(base_url.starts_with("https://") || is_loopback_http(&base_url)) {
            return Err(refuse("near_ai_admin_base_url"));
        }
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
        match mode {
            RailMode::Live if write_key.is_none() => return Err(refuse("near_ai_admin_key")),
            RailMode::DryRun if write_key.is_none() && read_key.is_none() => {
                return Err(refuse("near_ai_admin_read_key"));
            }
            _ => {}
        }
        let user_agent = get(ENV_USER_AGENT).ok_or(refuse("near_ai_admin_user_agent"))?;
        let rate = get(ENV_RATE)
            .ok_or(refuse("near_ai_credit_rate"))
            .and_then(|v| ConversionRate::parse(&v).map_err(|_| refuse("near_ai_credit_rate")))?;
        let max_delivery_nano_usd = get(ENV_MAX_DELIVERY)
            .ok_or(refuse("near_ai_credit_max_delivery"))?
            .parse::<u128>()
            .ok()
            .filter(|v| *v > 0 && *v <= NEAR_AI_MAX_SPEND_LIMIT_NANO_USD)
            .ok_or(refuse("near_ai_credit_max_delivery"))?;
        let credit_type = get(ENV_CREDIT_TYPE).unwrap_or_else(|| "grant".to_string());
        if !matches!(
            credit_type.as_str(),
            "grant" | "payment" | "staking_farm" | "postpay"
        ) {
            return Err(refuse("near_ai_credit_type"));
        }
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
            base_url,
            write_key,
            read_key,
            user_agent,
            rate,
            max_delivery_nano_usd,
            credit_type,
            timeout,
        }))
    }

    pub fn from_env() -> Result<Option<Self>, ConfigRefused> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }
}

/// The rail.
pub struct NearAiInferenceCreditRail {
    api: Arc<dyn NearAiAdminApi>,
    credit_type: String,
    max_delivery_nano_usd: u128,
    dry_run: bool,
}

impl NearAiInferenceCreditRail {
    pub fn new(
        api: Arc<dyn NearAiAdminApi>,
        credit_type: impl Into<String>,
        max_delivery_nano_usd: u128,
        dry_run: bool,
    ) -> Self {
        Self {
            api,
            credit_type: credit_type.into(),
            max_delivery_nano_usd,
            dry_run,
        }
    }

    /// Read every history row, newest first. 404 is an empty history.
    async fn full_history(&self, org: NearAiOrgId) -> Result<Vec<LimitsHistoryEntry>, RailError> {
        let mut rows = Vec::new();
        let mut offset = 0i64;
        for _ in 0..HISTORY_MAX_PAGES {
            let page = self
                .api
                .limits_history_page(org, HISTORY_PAGE, offset)
                .await
                .map_err(read_error)?;
            let Some(page) = page else {
                return Ok(rows);
            };
            let got = page.history.len() as i64;
            rows.extend(page.history);
            offset += got;
            if got == 0 || offset >= page.total {
                return Ok(rows);
            }
        }
        Err(RailError::frozen("near_ai_history_too_long"))
    }

    fn of_type<'a>(&self, rows: &'a [LimitsHistoryEntry]) -> Vec<&'a LimitsHistoryEntry> {
        rows.iter()
            .filter(|r| r.credit_type == self.credit_type)
            .collect()
    }
}

fn read_error(e: AdminApiError) -> RailError {
    match e {
        AdminApiError::Forbidden => RailError::frozen("near_ai_admin_forbidden"),
        AdminApiError::Malformed => RailError::frozen("near_ai_admin_response_malformed"),
        other => RailError::retry(other.label()),
    }
}

fn write_error(e: AdminApiError) -> RailError {
    match e {
        AdminApiError::Unreachable
        | AdminApiError::Ambiguous
        | AdminApiError::Unauthorized
        | AdminApiError::Unavailable
        | AdminApiError::Malformed => RailError::retry(e.label()),
        AdminApiError::Forbidden => RailError::frozen("near_ai_admin_read_only_key"),
        AdminApiError::NotFound | AdminApiError::Rejected => RailError::frozen(e.label()),
    }
}

fn amount_of(row: &LimitsHistoryEntry) -> Result<u128, RailError> {
    if row.spend_limit.scale != 9 || !row.spend_limit.currency.eq_ignore_ascii_case("USD") {
        return Err(RailError::frozen("near_ai_limit_unit_unexpected"));
    }
    u128::try_from(row.spend_limit.amount).map_err(|_| RailError::frozen("near_ai_limit_negative"))
}

fn is_ours(row: &LimitsHistoryEntry) -> bool {
    row.changed_by.as_deref() == Some(CHANGED_BY)
        && row
            .change_reason
            .as_deref()
            .is_some_and(|r| r.starts_with(MARKER_PREFIX))
}

impl NearAiInferenceCreditRail {
    /// Given history (newest first) containing our marker, build the receipt
    /// and check the row our write superseded is the one we expected.
    fn landed(
        &self,
        typed: &[&LimitsHistoryEntry],
        marker: &str,
        delivery: &RailDelivery,
        replayed: bool,
    ) -> Result<RailOutcome, RailError> {
        let ours: Vec<usize> = typed
            .iter()
            .enumerate()
            .filter(|(_, r)| r.change_reason.as_deref() == Some(marker))
            .map(|(i, _)| i)
            .collect();
        let (Some(&newest), Some(&oldest)) = (ours.first(), ours.last()) else {
            return Err(RailError::frozen("near_ai_write_not_found"));
        };
        let written = amount_of(typed[newest])?;
        // NEAR AI retries a failed transaction internally; a duplicate of our
        // row is tolerable only if every copy carries the same amount.
        for &i in &ours {
            if amount_of(typed[i])? != written {
                return Err(RailError::frozen("near_ai_write_duplicated_divergent"));
            }
        }
        let superseded = typed.get(oldest + 1).copied();
        let prior_units = match superseded {
            Some(row) => amount_of(row)?,
            None => 0,
        };
        let receipt = RailReceipt {
            prior_units,
            written_units: written,
            external_receipt_hash: history_row_ref(&typed[newest].id),
            replayed,
        };
        let expected_base = match (&delivery.expected_prior, superseded) {
            (None, None) => true,
            (Some(p), Some(row)) => {
                history_row_ref(&row.id) == p.external_receipt_hash
                    && prior_units == p.written_units
            }
            _ => false,
        };
        let anomaly = if !expected_base {
            Some("near_ai_grant_interleaved_write")
        } else if written != prior_units.saturating_add(delivery.units) {
            Some("near_ai_write_amount_mismatch")
        } else if newest != 0 && !self.all_ours(&typed[..newest]) {
            Some("near_ai_grant_bucket_foreign_write")
        } else {
            None
        };
        Ok(RailOutcome { receipt, anomaly })
    }

    fn all_ours(&self, rows: &[&LimitsHistoryEntry]) -> bool {
        rows.iter().all(|r| is_ours(r))
    }
}

#[async_trait]
impl SettlementRail for NearAiInferenceCreditRail {
    fn rail_id(&self) -> &'static str {
        RAIL_ID
    }

    fn instrument_id(&self) -> &'static str {
        INSTRUMENT_ID
    }

    fn instrument_decimals(&self) -> u8 {
        INSTRUMENT_DECIMALS
    }

    async fn deliver(&self, delivery: &RailDelivery) -> Result<RailOutcome, RailError> {
        if delivery.units == 0 {
            return Err(RailError::frozen("near_ai_delivery_zero"));
        }
        if delivery.units > self.max_delivery_nano_usd {
            return Err(RailError::frozen("near_ai_delivery_above_ceiling"));
        }
        let org = NearAiOrgId::parse(delivery.recipient.expose())
            .ok_or(RailError::frozen("near_ai_recipient_malformed"))?;
        let marker = marker(&delivery.delivery_hash);

        // 1. Did this delivery already land?
        let history = self.full_history(org).await?;
        let typed = self.of_type(&history);
        if typed
            .iter()
            .any(|r| r.change_reason.as_deref() == Some(marker.as_str()))
        {
            return self.landed(&typed, &marker, delivery, true);
        }

        // 2. Precondition: the active row is absent, or the one we recorded.
        let active = typed.iter().find(|r| r.effective_until.is_none()).copied();
        let prior_units = match (active, &delivery.expected_prior) {
            (None, None) => 0,
            (Some(row), Some(p))
                if is_ours(row)
                    && history_row_ref(&row.id) == p.external_receipt_hash
                    && amount_of(row)? == p.written_units =>
            {
                p.written_units
            }
            (Some(_), None) => {
                return Err(RailError::frozen("near_ai_grant_bucket_foreign_baseline"));
            }
            _ => return Err(RailError::frozen("near_ai_grant_bucket_foreign_write")),
        };

        // 3. Bound the new absolute value.
        let new_units = prior_units
            .checked_add(delivery.units)
            .filter(|v| *v <= NEAR_AI_MAX_SPEND_LIMIT_NANO_USD)
            .ok_or(RailError::frozen("near_ai_limit_would_exceed_maximum"))?;
        let amount = i64::try_from(new_units)
            .map_err(|_| RailError::frozen("near_ai_limit_would_exceed_maximum"))?;
        if self.dry_run {
            return Err(RailError::dry_run("near_ai_rail_dry_run"));
        }

        // 4. Write.
        let request = UpdateLimitsRequest {
            credit_type: self.credit_type.clone(),
            source: Some(SOURCE.to_string()),
            spend_limit: SpendLimitRequest {
                amount,
                currency: "USD".to_string(),
            },
            changed_by: Some(CHANGED_BY.to_string()),
            change_reason: Some(marker.clone()),
        };
        match self.api.update_limits(org, &request).await {
            Ok(_) => {}
            // The body is unreadable but the write committed; read back.
            Err(AdminApiError::Malformed) => {}
            Err(e) => return Err(write_error(e)),
        }

        // 5. Read back.
        let history = self.full_history(org).await?;
        let typed = self.of_type(&history);
        self.landed(&typed, &marker, delivery, false)
    }

    async fn verify(
        &self,
        delivery: &RailDelivery,
        receipt: &RailReceipt,
    ) -> Result<RailVerification, RailError> {
        let org = NearAiOrgId::parse(delivery.recipient.expose())
            .ok_or(RailError::frozen("near_ai_recipient_malformed"))?;
        let marker = marker(&delivery.delivery_hash);
        let history = self.full_history(org).await?;
        let typed = self.of_type(&history);
        let Some(pos) = typed
            .iter()
            .position(|r| history_row_ref(&r.id) == receipt.external_receipt_hash)
        else {
            return Ok(RailVerification::Missing);
        };
        let row = typed[pos];
        if row.change_reason.as_deref() != Some(marker.as_str())
            || amount_of(row)? != receipt.written_units
        {
            return Ok(RailVerification::Missing);
        }
        if pos == 0 {
            return Ok(RailVerification::Current);
        }
        if self.all_ours(&typed[..pos]) {
            Ok(RailVerification::Superseded)
        } else {
            Ok(RailVerification::Overwritten)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k| map.get(k).cloned()
    }

    fn live() -> Vec<(&'static str, &'static str)> {
        vec![
            (ENV_MODE, "live"),
            (ENV_BASE_URL, "https://cloud-api.near.ai/v1"),
            (ENV_KEY, "adm_test"),
            (ENV_USER_AGENT, "trace-commons-settlement/1"),
            (ENV_RATE, "1000/1"),
            (ENV_MAX_DELIVERY, "100000000000"),
        ]
    }

    #[test]
    fn unset_or_unknown_mode_is_disabled() {
        assert!(
            NearAiCreditRailConfig::from_lookup(env(&[]))
                .unwrap()
                .is_none()
        );
        assert!(
            NearAiCreditRailConfig::from_lookup(env(&[(ENV_MODE, "on")]))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn live_without_the_admin_key_refuses_with_a_safe_label() {
        let pairs: Vec<_> = live().into_iter().filter(|(k, _)| *k != ENV_KEY).collect();
        let err = NearAiCreditRailConfig::from_lookup(env(&pairs)).unwrap_err();
        assert_eq!(err.to_string(), "missing_control:near_ai_admin_key");
        // A read-only key does not satisfy live, and does not downgrade it.
        let mut with_read = pairs.clone();
        with_read.push((ENV_READ_KEY, "adm_read"));
        assert_eq!(
            NearAiCreditRailConfig::from_lookup(env(&with_read))
                .unwrap_err()
                .control,
            "near_ai_admin_key"
        );
    }

    #[test]
    fn each_required_control_refuses_by_name() {
        for (drop, control) in [
            (ENV_BASE_URL, "near_ai_admin_base_url"),
            (ENV_USER_AGENT, "near_ai_admin_user_agent"),
            (ENV_RATE, "near_ai_credit_rate"),
            (ENV_MAX_DELIVERY, "near_ai_credit_max_delivery"),
        ] {
            let pairs: Vec<_> = live().into_iter().filter(|(k, _)| *k != drop).collect();
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
            (ENV_MAX_DELIVERY, "0", "near_ai_credit_max_delivery"),
            (
                ENV_MAX_DELIVERY,
                "1000000000000000001",
                "near_ai_credit_max_delivery",
            ),
            (ENV_CREDIT_TYPE, "bonus", "near_ai_credit_type"),
        ] {
            let mut pairs = live();
            pairs.retain(|(k, _)| *k != name);
            pairs.push((name, value));
            let err = NearAiCreditRailConfig::from_lookup(env(&pairs)).unwrap_err();
            assert_eq!(err.control, control, "{name}");
            assert!(!err.to_string().contains(value));
        }
    }

    #[test]
    fn dry_run_needs_a_key_to_read_with_and_accepts_the_read_only_one() {
        let mut pairs: Vec<_> = live().into_iter().filter(|(k, _)| *k != ENV_KEY).collect();
        pairs.retain(|(k, _)| *k != ENV_MODE);
        pairs.push((ENV_MODE, "dry_run"));
        assert_eq!(
            NearAiCreditRailConfig::from_lookup(env(&pairs))
                .unwrap_err()
                .control,
            "near_ai_admin_read_key"
        );
        pairs.push((ENV_READ_KEY, "adm_read"));
        let cfg = NearAiCreditRailConfig::from_lookup(env(&pairs))
            .unwrap()
            .unwrap();
        assert_eq!(cfg.mode, RailMode::DryRun);
        assert!(!format!("{cfg:?}").contains("adm_read"));
    }

    #[test]
    fn a_complete_live_configuration_resolves() {
        let cfg = NearAiCreditRailConfig::from_lookup(env(&live()))
            .unwrap()
            .unwrap();
        assert_eq!(cfg.mode, RailMode::Live);
        assert_eq!(cfg.credit_type, "grant");
        assert_eq!(cfg.rate.canonical(), "1000/1");
        assert!(!format!("{cfg:?}").contains("adm_test"));
    }
}
