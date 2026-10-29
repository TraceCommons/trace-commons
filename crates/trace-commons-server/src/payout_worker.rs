// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The payout rails a deployment runs, and one worker per enabled rail
//! (payout spec section 3 "Independence", section 4 "Modes and the kill
//! switch" and "Fail-closed rules", section 7 "Schedule").
//!
//! [`PayoutRails`] is built from configuration and can construct the
//! `near_ai` rail only: an unknown rail id, or a test rail such as the
//! keyed mock, is refused (`payout_rail_unknown`). Every rail's mode
//! defaults to `disabled`, which builds no client and spawns no worker.
//!
//! Each enabled rail gets its own task, its own [`tokio::sync::Notify`] and
//! its own pass. A settlement wakes every rail through [`PayoutNotifier`];
//! `notify_one` stores a single permit when the worker is mid-pass, so a
//! burst of settlements during a pass causes exactly one more pass, and no
//! separate run-again flag is kept. The retry tick ([`PAYOUT_RETRY_TICK`])
//! catches a notify lost to a crash, retries pending lines and finishes
//! closing links. One rail's slow or stopped pass never delays another's.
//!
//! Readiness reads each rail's cached status (mode, kill switch as last
//! read, rate epoch label, startup capacity probe, last pass outcome), so
//! the unauthenticated readiness probe never reaches the database or the
//! rail.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use tokio::sync::Notify;
use trace_commons_gate_api::{NEAR_AI_RAIL_ID, PayoutRail, RailId, TRACE_CREDIT_RAIL_ID};

use crate::db::{Database, PayoutStore};
use crate::near_ai_api::near_ai_staging;
use crate::near_ai_rail::NearAiRail;
use crate::near_ai_rail_config::{NearAiRailConfig, NearAiReferralConfig};
use crate::near_ai_transfer::{NearAiTransferClient, NearAiTransferCredentials};
use crate::payout_pass::{
    PAYOUT_LOCAL_SEND_LIMIT, PayoutPass, PayoutPassOutcome, PayoutPassReport, PayoutPassSettings,
    PayoutRecipientOpener, ReferralStreamSettings,
};
use crate::payout_rate::PayoutMode;
use crate::referral_bonus::ReferralBonusHook;

/// How often each enabled rail's worker runs with no settlement to wake it
/// (spec section 7, "Retry tick"; proposed, section 12 open item 1).
pub const PAYOUT_RETRY_TICK: Duration = Duration::from_secs(60);

/// Why the rails could not be built. One safe label each.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum PayoutRailsError {
    /// A rail id this build cannot construct.
    #[error("payout_rail_unknown")]
    Unknown,
    /// The rail's client could not be built from its settings.
    #[error("payout_rail_client_invalid")]
    ClientInvalid,
}

impl PayoutRailsError {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "payout_rail_unknown",
            Self::ClientInvalid => "payout_rail_client_invalid",
        }
    }
}

/// Builds the rail named `rail_id` from configuration. Only `near_ai` is
/// known; it is `None` while disabled. Anything else, a test rail included,
/// is refused: configuration can never put a mock in front of money.
pub fn build_rail(
    rail_id: &str,
    near_ai: &NearAiRailConfig,
) -> Result<Option<Arc<dyn PayoutRail>>, PayoutRailsError> {
    build_rail_at(rail_id, near_ai, None)
}

/// [`build_rail`], with the `near_ai` client at a plain-HTTP loopback base
/// when `loopback` is set ([`NearAiTransferClient::at_loopback`], which
/// refuses anything but a loopback IP literal), for tests against a local
/// mock. Still `near_ai` only.
fn build_rail_at(
    rail_id: &str,
    near_ai: &NearAiRailConfig,
    loopback: Option<&str>,
) -> Result<Option<Arc<dyn PayoutRail>>, PayoutRailsError> {
    if rail_id != NEAR_AI_RAIL_ID {
        return Err(PayoutRailsError::Unknown);
    }
    match near_ai {
        NearAiRailConfig::Disabled => Ok(None),
        NearAiRailConfig::Enabled(settings) => {
            near_ai_rail_from(&settings.credentials, loopback).map(Some)
        }
    }
}

/// One `near_ai` rail object over `credentials`: the rail's own, or the
/// referral stream's dedicated one.
fn near_ai_rail_from(
    credentials: &NearAiTransferCredentials,
    loopback: Option<&str>,
) -> Result<Arc<dyn PayoutRail>, PayoutRailsError> {
    let client = match loopback {
        None => NearAiTransferClient::new(credentials.clone()),
        Some(base) => NearAiTransferClient::at_loopback(
            base,
            credentials.clone(),
            std::time::Duration::from_secs(5),
        ),
    }
    .map_err(|_| PayoutRailsError::ClientInvalid)?;
    Ok(Arc::new(NearAiRail::new(client)))
}

/// Wakes every enabled rail's worker. No payload, no tenant. Cheap to
/// clone; a notifier with no rails does nothing.
#[derive(Clone, Default)]
pub struct PayoutNotifier {
    rails: Arc<Vec<Arc<Notify>>>,
}

impl std::fmt::Debug for PayoutNotifier {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutNotifier")
            .field("rails", &self.rails.len())
            .finish()
    }
}

impl PayoutNotifier {
    /// A notifier over these rails' wakers. [`PayoutRails::notifier`] is the
    /// one a deployment uses.
    pub fn new(rails: Vec<Arc<Notify>>) -> Self {
        Self {
            rails: Arc::new(rails),
        }
    }

    /// Called after a settlement batch commits.
    pub fn notify_all(&self) {
        for rail in self.rails.iter() {
            rail.notify_one();
        }
    }
}

/// A rail's cached state, for readiness.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PayoutRailStatus {
    /// The kill switch as last read; `None` before the first read.
    pub halted: Option<bool>,
    /// The version label of the rail's rate in force now, from its rate
    /// source at boot; never the rate, never a stored epoch alone.
    pub rate_epoch: Option<String>,
    /// The startup capacity probe answered with sending enabled.
    pub startup_probe_ok: bool,
    pub last_outcome: Option<&'static str>,
    /// The last pass left a closing link it could not close.
    pub close_blocked: bool,
}

/// A rail's referral stream (referral rewards spec section 5).
struct ReferralEntry {
    /// The rail over the referral credential; `None` in a `dry_run` with
    /// no credential configured.
    rail: Option<Arc<dyn PayoutRail>>,
    settings: ReferralStreamSettings,
}

/// One configured rail.
pub struct PayoutRailEntry {
    rail_id: RailId,
    mode: PayoutMode,
    rail: Option<Arc<dyn PayoutRail>>,
    settings: Option<PayoutPassSettings>,
    referral: Option<ReferralEntry>,
    notify: Arc<Notify>,
    status: Mutex<PayoutRailStatus>,
}

impl PayoutRailEntry {
    pub fn rail_id(&self) -> &RailId {
        &self.rail_id
    }

    pub fn mode(&self) -> PayoutMode {
        self.mode
    }

    /// The referral stream's mode as configured: `disabled` without one.
    /// The pass runs it at no more than the rail's own mode.
    pub fn referral_mode(&self) -> PayoutMode {
        self.referral
            .as_ref()
            .map_or(PayoutMode::Disabled, |referral| referral.settings.mode)
    }

    pub fn status(&self) -> PayoutRailStatus {
        self.status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn update(&self, change: impl FnOnce(&mut PayoutRailStatus)) {
        change(
            &mut self
                .status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        );
    }

    /// The rail, when it is enabled. The link routes probe a recipient
    /// through it.
    pub fn rail(&self) -> Option<&Arc<dyn PayoutRail>> {
        self.rail.as_ref()
    }

    /// Records the kill switch as an admin route just set it.
    pub fn record_halted(&self, halted: bool) {
        self.update(|status| status.halted = Some(halted));
    }

    /// The pass settings this rail runs with, `None` while disabled. The
    /// pass runs only while a rate is in force now
    /// ([`PayoutRails::set_rate_epoch`] with the current source's version).
    pub fn pass_settings(&self) -> Option<PayoutPassSettings> {
        let rate_in_force = self.status().rate_epoch.is_some();
        self.settings.map(|settings| PayoutPassSettings {
            rate_in_force,
            ..settings
        })
    }

    fn record_report(&self, report: &PayoutPassReport) {
        self.update(|status| {
            status.last_outcome = Some(report.outcome.label());
            match report.outcome {
                PayoutPassOutcome::Halted => status.halted = Some(true),
                PayoutPassOutcome::Completed | PayoutPassOutcome::Stopped { .. } => {
                    status.halted = Some(false);
                    status.close_blocked = report.close_blocked > 0;
                }
                _ => {}
            }
        });
    }

    /// Whether the rail may be relied on in production (spec section 4,
    /// "Fail-closed rules"): `live`, outside a staging build, an
    /// implementation that declares itself qualified, a rate epoch in force,
    /// a startup capacity probe answering sending enabled, and not halted.
    pub fn qualified(&self) -> bool {
        let status = self.status();
        self.mode == PayoutMode::Live
            && !near_ai_staging()
            && self
                .rail
                .as_ref()
                .is_some_and(|rail| rail.production_qualified())
            && status.rate_epoch.is_some()
            && status.startup_probe_ok
            && status.halted != Some(true)
    }
}

/// One rail's readiness entry. Labels only.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct PayoutRailReadiness {
    pub rail_id: String,
    /// `disabled`, `dry_run`, `live`, or `internal` for `trace_credit`.
    pub mode: &'static str,
    pub halted: bool,
    pub rate_epoch: Option<String>,
    pub qualified: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_pass: Option<&'static str>,
    /// `payout_link_close_blocked` while the last pass left a link closing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<&'static str>,
}

/// The rails this deployment runs.
pub struct PayoutRails {
    entries: Vec<Arc<PayoutRailEntry>>,
    /// The referral bonus work the `near_ai` pass runs after accrual, when
    /// referral mode is not `disabled`.
    referral_bonus: Option<Arc<ReferralBonusHook>>,
}

impl std::fmt::Debug for PayoutRails {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut list = formatter.debug_list();
        for entry in &self.entries {
            list.entry(&(entry.rail_id.as_str(), entry.mode.as_label()));
        }
        list.finish()
    }
}

impl Default for PayoutRails {
    fn default() -> Self {
        Self::disabled()
    }
}

impl PayoutRails {
    /// Every rail this build knows, each `disabled`.
    pub fn disabled() -> Self {
        Self::from_config(&NearAiRailConfig::Disabled).expect("a disabled rail builds nothing")
    }

    /// Builds each known rail from its configuration, with no referral
    /// stream.
    pub fn from_config(near_ai: &NearAiRailConfig) -> Result<Self, PayoutRailsError> {
        Self::from_configs(near_ai, &NearAiReferralConfig::Disabled)
    }

    /// Builds each known rail and the `near_ai` referral stream from their
    /// configurations. A referral stream on a disabled rail builds nothing
    /// (configuration already refuses one).
    pub fn from_configs(
        near_ai: &NearAiRailConfig,
        referral: &NearAiReferralConfig,
    ) -> Result<Self, PayoutRailsError> {
        Self::from_config_at(near_ai, referral, None)
    }

    /// [`Self::from_config`] with the `near_ai` client at a loopback mock
    /// (see `build_rail_at`). For tests; a deployment has no setting that
    /// reaches it, and the host it pays stays the compiled-in one.
    pub fn from_config_at_loopback(
        near_ai: &NearAiRailConfig,
        base: &str,
    ) -> Result<Self, PayoutRailsError> {
        Self::from_config_at(near_ai, &NearAiReferralConfig::Disabled, Some(base))
    }

    /// [`Self::from_configs`] at a loopback mock, for tests.
    pub fn from_configs_at_loopback(
        near_ai: &NearAiRailConfig,
        referral: &NearAiReferralConfig,
        base: &str,
    ) -> Result<Self, PayoutRailsError> {
        Self::from_config_at(near_ai, referral, Some(base))
    }

    fn from_config_at(
        near_ai: &NearAiRailConfig,
        referral: &NearAiReferralConfig,
        loopback: Option<&str>,
    ) -> Result<Self, PayoutRailsError> {
        let rail = build_rail_at(NEAR_AI_RAIL_ID, near_ai, loopback)?;
        let referral = match (near_ai, referral) {
            (NearAiRailConfig::Enabled(_), NearAiReferralConfig::Enabled(settings)) => {
                Some(ReferralEntry {
                    rail: settings
                        .credentials
                        .as_ref()
                        .map(|credentials| near_ai_rail_from(credentials, loopback))
                        .transpose()?,
                    settings: ReferralStreamSettings {
                        mode: settings.mode,
                        account_ceiling: settings.account_ceiling,
                    },
                })
            }
            _ => None,
        };
        let settings = match near_ai {
            NearAiRailConfig::Disabled => None,
            NearAiRailConfig::Enabled(settings) => Some(PayoutPassSettings {
                mode: settings.mode,
                account_ceiling: settings.account_ceiling,
                local_send_limit: PAYOUT_LOCAL_SEND_LIMIT,
                // Set from the boot's rate source by `set_rate_epoch`;
                // nothing runs on a rate until one is recorded.
                rate_in_force: false,
            }),
        };
        Ok(Self {
            referral_bonus: None,
            entries: vec![Arc::new(PayoutRailEntry {
                rail_id: RailId::near_ai(),
                mode: near_ai.mode(),
                rail,
                settings,
                referral,
                notify: Arc::new(Notify::new()),
                status: Mutex::new(PayoutRailStatus::default()),
            })],
        })
    }

    pub fn entries(&self) -> impl Iterator<Item = &Arc<PayoutRailEntry>> {
        self.entries.iter()
    }

    /// Attaches the referral bonus work to the `near_ai` pass. A hook in
    /// `disabled` mode is not attached.
    pub fn with_referral_bonus(mut self, hook: Arc<ReferralBonusHook>) -> Self {
        if hook.mode() != PayoutMode::Disabled {
            self.referral_bonus = Some(hook);
        }
        self
    }

    /// The referral bonus work, when attached. The referral admin routes
    /// reach its store through it.
    pub fn referral_bonus(&self) -> Option<&Arc<ReferralBonusHook>> {
        self.referral_bonus.as_ref()
    }

    /// The rail named `rail_id`; an unknown id is refused, never defaulted.
    pub fn get(&self, rail_id: &str) -> Result<&Arc<PayoutRailEntry>, PayoutRailsError> {
        self.entries
            .iter()
            .find(|entry| entry.rail_id.as_str() == rail_id)
            .ok_or(PayoutRailsError::Unknown)
    }

    /// Wakes every enabled rail.
    pub fn notifier(&self) -> PayoutNotifier {
        PayoutNotifier {
            rails: Arc::new(
                self.entries
                    .iter()
                    .filter(|entry| entry.rail.is_some())
                    .map(|entry| entry.notify.clone())
                    .collect(),
            ),
        }
    }

    /// Records the version of `rail_id`'s rate in force now, from its rate
    /// source at boot (never the latest stored epoch): readiness reports it,
    /// qualification needs it, and the rail's pass runs only with it.
    pub fn set_rate_epoch(&self, rail_id: &str, version: Option<String>) {
        if let Ok(entry) = self.get(rail_id) {
            entry.update(|status| status.rate_epoch = version);
        }
    }

    /// The startup capacity probe of every `live` rail. A failure marks the
    /// rail unqualified in readiness; it does not refuse boot.
    pub async fn startup_probe(&self) {
        for entry in &self.entries {
            let Some(rail) = entry.rail.as_ref() else {
                continue;
            };
            if entry.mode != PayoutMode::Live {
                continue;
            }
            let ok = match rail.preflight().await {
                Ok(capacity) => capacity.sending_enabled,
                Err(refusal) => {
                    tracing::warn!(
                        rail = %entry.rail_id,
                        label = refusal.safe_label(),
                        "payout rail startup capacity probe refused"
                    );
                    false
                }
            };
            entry.update(|status| status.startup_probe_ok = ok);
            // The referral credential's capacity, logged only: a refusal
            // stops the referral stream's sends pass by pass, never the
            // rail's, and readiness stays the rail's.
            if let Some(referral) = entry
                .referral
                .as_ref()
                .filter(|referral| referral.settings.mode == PayoutMode::Live)
                .and_then(|referral| referral.rail.as_ref())
            {
                match referral.preflight().await {
                    Ok(capacity) if capacity.sending_enabled => {}
                    Ok(_) => tracing::warn!(
                        rail = %entry.rail_id,
                        label = "referral_sending_disabled",
                        "payout referral stream startup capacity probe: sending off"
                    ),
                    Err(refusal) => tracing::warn!(
                        rail = %entry.rail_id,
                        label = refusal.safe_label(),
                        "payout referral stream startup capacity probe refused"
                    ),
                }
            }
        }
    }

    /// One readiness entry per rail, `trace_credit` included as `internal`:
    /// its settlement is the existing ledger write, which no pass drives.
    pub fn readiness(&self) -> Vec<PayoutRailReadiness> {
        let mut out = vec![PayoutRailReadiness {
            rail_id: TRACE_CREDIT_RAIL_ID.to_owned(),
            mode: "internal",
            halted: false,
            rate_epoch: None,
            qualified: true,
            last_pass: None,
            reason: None,
        }];
        for entry in &self.entries {
            let status = entry.status();
            let halted = status.halted == Some(true);
            out.push(PayoutRailReadiness {
                rail_id: entry.rail_id.as_str().to_owned(),
                mode: entry.mode.as_label(),
                halted,
                rate_epoch: status.rate_epoch.clone(),
                qualified: entry.qualified(),
                last_pass: status.last_outcome,
                reason: if halted {
                    Some("payout_halted")
                } else if entry.mode != PayoutMode::Disabled && status.rate_epoch.is_none() {
                    Some(crate::payout_pass::PAYOUT_RATE_UNAVAILABLE)
                } else if status.close_blocked {
                    Some("payout_link_close_blocked")
                } else {
                    None
                },
            });
        }
        // The referral stream, when attached: `referral_tripped` while it
        // switched itself to `dry_run` (spec section 7). It sends nothing in
        // this build, so it is never qualified.
        if let Some(referral) = &self.referral_bonus {
            let tripped = referral.tripped() == Some(true);
            out.push(PayoutRailReadiness {
                rail_id: crate::referral_bonus::NEAR_AI_REFERRAL_CONTROL_RAIL.to_owned(),
                mode: if tripped {
                    PayoutMode::DryRun.as_label()
                } else {
                    referral.mode().as_label()
                },
                halted: tripped,
                rate_epoch: None,
                qualified: false,
                last_pass: None,
                reason: tripped.then_some(crate::referral_abuse::REFERRAL_TRIPPED),
            });
        }
        out
    }

    /// Spawns one worker per enabled rail. A disabled rail spawns nothing.
    pub fn spawn_workers(
        &self,
        payouts: Arc<dyn PayoutStore>,
        corpus: Arc<dyn Database>,
        opener: Arc<dyn PayoutRecipientOpener>,
    ) -> Vec<tokio::task::JoinHandle<()>> {
        self.entries
            .iter()
            .filter_map(|entry| {
                let rail = entry.rail.clone()?;
                let settings = entry.pass_settings()?;
                let mut pass = PayoutPass::new(
                    rail,
                    payouts.clone(),
                    corpus.clone(),
                    opener.clone(),
                    settings,
                );
                if let Some(referral) = &self.referral_bonus
                    && referral.rail_id() == entry.rail_id()
                {
                    pass = pass.with_referral_bonus(referral.clone());
                }
                if let Some(referral) = entry.referral.as_ref() {
                    pass = pass.with_referral_stream(referral.rail.clone(), referral.settings);
                }
                let runner = Arc::new(EntryPass {
                    pass,
                    entry: entry.clone(),
                });
                Some(tokio::spawn(run_worker(
                    runner,
                    entry.notify.clone(),
                    PAYOUT_RETRY_TICK,
                )))
            })
            .collect()
    }
}

/// One pass, as a worker runs it.
#[async_trait]
pub trait PassRunner: Send + Sync {
    async fn run_pass(&self);
}

struct EntryPass {
    pass: PayoutPass,
    entry: Arc<PayoutRailEntry>,
}

#[async_trait]
impl PassRunner for EntryPass {
    async fn run_pass(&self) {
        let report = self.pass.run_once().await;
        self.entry.record_report(&report);
    }
}

/// A rail's worker: a pass, then wait for a settlement or the tick, for
/// ever. A notify that arrives during a pass leaves one permit, so the next
/// wait returns at once: exactly one more pass, however many arrived.
pub async fn run_worker(runner: Arc<dyn PassRunner>, notify: Arc<Notify>, tick: Duration) {
    loop {
        runner.run_pass().await;
        tokio::select! {
            _ = notify.notified() => {}
            _ = tokio::time::sleep(tick) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use secrecy::SecretString;
    use trace_commons_gate_api::pipeline::AtomicUnits;
    use uuid::Uuid;

    use super::*;
    use crate::near_ai_rail_config::NearAiRailSettings;
    use crate::near_ai_transfer::{NearAiCreditSource, NearAiTransferCredentials};

    fn enabled(mode: PayoutMode) -> NearAiRailConfig {
        NearAiRailConfig::Enabled(NearAiRailSettings {
            mode,
            credentials: NearAiTransferCredentials {
                token: SecretString::from(format!("oc_{}", "ab".repeat(32))),
                credential_id: Uuid::from_u128(2),
                sender_org: Uuid::from_u128(1),
                source: NearAiCreditSource::Payment,
            },
            account_ceiling: AtomicUnits::from_raw(1_000),
        })
    }

    #[test]
    fn the_registry_cannot_build_a_mock_or_unknown_rail_from_config() {
        for refused in [
            "test_keyed",
            "near",
            "trace_credit",
            "",
            "NEAR_AI",
            "near_ai ",
        ] {
            for config in [NearAiRailConfig::Disabled, enabled(PayoutMode::Live)] {
                assert_eq!(
                    build_rail(refused, &config).err(),
                    Some(PayoutRailsError::Unknown),
                    "{refused:?}"
                );
            }
        }
        assert!(
            build_rail("near_ai", &NearAiRailConfig::Disabled)
                .unwrap()
                .is_none()
        );
        let rail = build_rail("near_ai", &enabled(PayoutMode::Live))
            .unwrap()
            .expect("an enabled near_ai rail");
        assert_eq!(rail.descriptor().rail_id.as_str(), "near_ai");
        assert_eq!(
            rail.adapter_identity(),
            crate::near_ai_rail::NEAR_AI_ADAPTER_IDENTITY
        );

        let rails = PayoutRails::from_config(&enabled(PayoutMode::Live)).unwrap();
        let ids: Vec<&str> = rails
            .entries()
            .map(|entry| entry.rail_id().as_str())
            .collect();
        assert_eq!(ids, vec!["near_ai"]);
        assert_eq!(
            rails.get("test_keyed").err(),
            Some(PayoutRailsError::Unknown)
        );
    }

    #[test]
    fn every_mode_defaults_to_disabled() {
        assert_eq!(PayoutMode::default(), PayoutMode::Disabled);
        assert_eq!(PayoutMode::parse(None), Ok(PayoutMode::Disabled));
        let unset = NearAiRailConfig::from_vars(|_| None).unwrap();
        assert_eq!(unset.mode(), PayoutMode::Disabled);
        for rails in [
            PayoutRails::default(),
            PayoutRails::disabled(),
            PayoutRails::from_config(&unset).unwrap(),
        ] {
            for entry in rails.entries() {
                assert_eq!(entry.mode(), PayoutMode::Disabled);
                assert!(entry.rail.is_none(), "a disabled rail builds no client");
                assert!(!entry.qualified());
            }
            // Nothing to wake, nothing spawned.
            assert!(rails.notifier().rails.is_empty());
        }
    }

    #[test]
    fn readiness_lists_trace_credit_and_each_rail() {
        let rails = PayoutRails::from_config(&enabled(PayoutMode::DryRun)).unwrap();
        rails.set_rate_epoch("near_ai", Some("table-v1".into()));
        let readiness = rails.readiness();
        assert_eq!(readiness.len(), 2);
        assert_eq!(readiness[0].rail_id, "trace_credit");
        assert_eq!(readiness[0].mode, "internal");
        assert_eq!(readiness[1].rail_id, "near_ai");
        assert_eq!(readiness[1].mode, "dry_run");
        assert_eq!(readiness[1].rate_epoch.as_deref(), Some("table-v1"));
        assert!(!readiness[1].qualified, "dry_run is never qualified");

        let entry = rails.get("near_ai").unwrap();
        entry.record_halted(true);
        let halted = &rails.readiness()[1];
        assert!(halted.halted);
        assert_eq!(halted.reason, Some("payout_halted"));
    }

    #[test]
    fn a_live_rail_is_qualified_only_with_every_condition() {
        let rails = PayoutRails::from_config(&enabled(PayoutMode::Live)).unwrap();
        let entry = rails.get("near_ai").unwrap();
        assert!(!entry.qualified(), "no epoch, no probe");
        rails.set_rate_epoch("near_ai", Some("table-v1".into()));
        assert!(!entry.qualified(), "no startup probe");
        entry.update(|status| status.startup_probe_ok = true);
        assert_eq!(entry.qualified(), !near_ai_staging());
        entry.record_halted(true);
        assert!(!entry.qualified());
    }

    #[test]
    fn an_enabled_rail_without_a_current_rate_is_unqualified_and_runs_no_rate() {
        for mode in [PayoutMode::DryRun, PayoutMode::Live] {
            let rails = PayoutRails::from_config(&enabled(mode)).unwrap();
            let entry = rails.get("near_ai").unwrap();
            entry.update(|status| status.startup_probe_ok = true);
            rails.set_rate_epoch("near_ai", None);
            assert!(!entry.qualified());
            assert!(!entry.pass_settings().unwrap().rate_in_force);
            let readiness = &rails.readiness()[1];
            assert_eq!(readiness.rate_epoch, None);
            assert_eq!(readiness.reason, Some("payout_rate_unavailable"));

            rails.set_rate_epoch("near_ai", Some("table-v1".into()));
            assert!(entry.pass_settings().unwrap().rate_in_force);
            assert_eq!(rails.readiness()[1].reason, None);
        }
        // A disabled rail has no pass and no rate reason.
        let rails = PayoutRails::disabled();
        assert!(rails.get("near_ai").unwrap().pass_settings().is_none());
        assert_eq!(rails.readiness()[1].reason, None);
    }

    struct CountingRunner {
        runs: AtomicUsize,
        first_started: Notify,
        release_first: Notify,
        ran: tokio::sync::mpsc::UnboundedSender<usize>,
    }

    #[async_trait]
    impl PassRunner for CountingRunner {
        async fn run_pass(&self) {
            let n = self.runs.fetch_add(1, Ordering::SeqCst) + 1;
            if n == 1 {
                self.first_started.notify_one();
                self.release_first.notified().await;
            }
            self.ran.send(n).unwrap();
        }
    }

    #[tokio::test]
    async fn a_notify_during_a_pass_causes_exactly_one_more() {
        let (ran, mut done) = tokio::sync::mpsc::unbounded_channel();
        let runner = Arc::new(CountingRunner {
            runs: AtomicUsize::new(0),
            first_started: Notify::new(),
            release_first: Notify::new(),
            ran,
        });
        let notify = Arc::new(Notify::new());
        let notifier = PayoutNotifier {
            rails: Arc::new(vec![notify.clone()]),
        };
        let worker = tokio::spawn(run_worker(
            runner.clone(),
            notify,
            Duration::from_secs(3_600),
        ));
        runner.first_started.notified().await;
        // A burst of settlements while the first pass runs.
        for _ in 0..5 {
            notifier.notify_all();
        }
        runner.release_first.notify_one();
        assert_eq!(done.recv().await, Some(1));
        assert_eq!(done.recv().await, Some(2));
        // No third pass: the burst left one permit, not five.
        tokio::time::sleep(Duration::from_millis(200)).await;
        assert_eq!(runner.runs.load(Ordering::SeqCst), 2);
        // A later settlement wakes it once more.
        notifier.notify_all();
        assert_eq!(done.recv().await, Some(3));
        worker.abort();
    }

    fn referral(mode: PayoutMode, with_credential: bool) -> NearAiReferralConfig {
        NearAiReferralConfig::Enabled(crate::near_ai_rail_config::NearAiReferralSettings {
            mode,
            credentials: with_credential.then(|| NearAiTransferCredentials {
                token: SecretString::from(format!("oc_{}", "cd".repeat(32))),
                credential_id: Uuid::from_u128(3),
                sender_org: Uuid::from_u128(1),
                source: NearAiCreditSource::Payment,
            }),
            account_ceiling: AtomicUnits::from_raw(500),
        })
    }

    #[test]
    fn the_referral_stream_rides_on_its_rail_and_defaults_to_disabled() {
        // No referral configuration: disabled.
        let rails = PayoutRails::from_config(&enabled(PayoutMode::Live)).unwrap();
        assert_eq!(
            rails.get("near_ai").unwrap().referral_mode(),
            PayoutMode::Disabled
        );
        // A referral stream on a disabled rail builds nothing.
        let rails = PayoutRails::from_configs(
            &NearAiRailConfig::Disabled,
            &referral(PayoutMode::Live, true),
        )
        .unwrap();
        assert_eq!(
            rails.get("near_ai").unwrap().referral_mode(),
            PayoutMode::Disabled
        );
        for (mode, with_credential) in [
            (PayoutMode::DryRun, false),
            (PayoutMode::DryRun, true),
            (PayoutMode::Live, true),
        ] {
            let rails = PayoutRails::from_configs(
                &enabled(PayoutMode::Live),
                &referral(mode, with_credential),
            )
            .unwrap();
            let entry = rails.get("near_ai").unwrap();
            assert_eq!(entry.referral_mode(), mode);
            let referral = entry.referral.as_ref().unwrap();
            assert_eq!(referral.rail.is_some(), with_credential);
            assert_eq!(
                referral.settings.account_ceiling,
                AtomicUnits::from_raw(500)
            );
            if let Some(rail) = referral.rail.as_ref() {
                assert_eq!(rail.descriptor().rail_id.as_str(), "near_ai");
                // Its own rail object, not the rail's.
                assert!(!Arc::ptr_eq(rail, entry.rail().unwrap()));
            }
            // The referral credential never shows in Debug.
            let rendered = format!("{rails:?}");
            assert!(!rendered.contains("cdcd"), "{rendered}");
        }
    }

    #[test]
    fn debug_prints_rail_ids_and_modes_only() {
        let rails = PayoutRails::from_config(&enabled(PayoutMode::Live)).unwrap();
        let rendered = format!("{rails:?} {:?}", rails.notifier());
        assert!(rendered.contains("near_ai") && rendered.contains("live"));
        assert!(!rendered.contains("abab"), "{rendered}");
        assert!(!rendered.contains(&Uuid::from_u128(1).to_string()));
    }
}
