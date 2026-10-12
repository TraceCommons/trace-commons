// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust (Flow 3).
//!
//! See `docs/superpowers/specs/2026-09-26-earned-account-trust-design.md`.
//! This module runs the batch workers over the database seams: it records the
//! hash-only facts the growth rule reads, stores evaluations, and reproduces
//! them for the explain route and drill. The rule itself is the pure
//! `account_trust_rule`.
//!
//! A `shadow` evaluation never changes an allowance. An `applied` one is
//! written only when the admission policy itself names `tiered-v1`, bound to
//! the account's input generation in one snapshot
//! (`Database::evaluate_and_record_applied_account_trust`); admission reads
//! the latest fresh one through V114's read and falls to the base allowance
//! when it is missing, stale or invalidated.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::account_trust::TrustAccount;
use crate::account_trust_rule::{Evaluation, GrowthPolicy, evaluate};
use crate::db::Database;
use crate::error::DatabaseError;

/// Accounts fetched per enumeration page.
const ACCOUNT_PAGE: i64 = 200;
/// Upper bound on sources one recorder run visits, whatever the caller asks.
pub const RECORD_FACTS_MAX_LIMIT: i64 = 10_000;
/// Sources one recorder run visits when the caller names no limit.
pub const RECORD_FACTS_DEFAULT_LIMIT: i64 = 1_000;

/// Label-only counts from one recorder run. No identifier of any account,
/// submission or source appears here.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct RecordFactsSummary {
    pub dry_run: bool,
    pub accounts_scanned: u64,
    /// Sources found without a fact, up to the limit.
    pub candidates: u64,
    /// Facts recorded (or, on a dry run, that would be attempted), by outcome.
    pub recorded_by_outcome: BTreeMap<&'static str, u64>,
    /// The definer function declined the source: its server row no longer
    /// matches (for example, it changed status between listing and recording).
    pub declined: u64,
    /// Database errors on a single source; the run continues past them.
    pub failed: u64,
    /// The limit stopped the run before every account was visited.
    pub limit_reached: bool,
}

/// Walks every open account in an anchored tenant and records its missing
/// facts through `trace_record_account_trust_fact`. Idempotent: a re-run
/// finds no candidates, and a concurrent run records nothing twice (the
/// insert is `ON CONFLICT DO NOTHING` on the fact's primary key).
pub async fn record_account_trust_facts(
    db: &dyn Database,
    limit: Option<i64>,
    dry_run: bool,
) -> Result<RecordFactsSummary, DatabaseError> {
    let limit = limit
        .unwrap_or(RECORD_FACTS_DEFAULT_LIMIT)
        .clamp(0, RECORD_FACTS_MAX_LIMIT);
    let mut summary = RecordFactsSummary {
        dry_run,
        ..RecordFactsSummary::default()
    };
    let mut remaining = limit;
    let mut after: Option<TrustAccount> = None;
    'pages: loop {
        let page = db
            .list_account_trust_worker_accounts(after.as_ref(), ACCOUNT_PAGE)
            .await?;
        let Some(last) = page.last().cloned() else {
            break;
        };
        for account in &page {
            if remaining == 0 {
                summary.limit_reached = true;
                break 'pages;
            }
            summary.accounts_scanned += 1;
            let candidates = db
                .list_account_trust_fact_candidates(account, remaining)
                .await?;
            summary.candidates += candidates.len() as u64;
            remaining -= candidates.len() as i64;
            if dry_run {
                continue;
            }
            for source in candidates {
                match db.record_account_trust_fact(account, source).await {
                    Ok(Some(outcome)) => {
                        *summary
                            .recorded_by_outcome
                            .entry(outcome.as_str())
                            .or_default() += 1;
                    }
                    Ok(None) => summary.declined += 1,
                    Err(error) => {
                        summary.failed += 1;
                        tracing::warn!(
                            source_kind = source.kind(),
                            error_label = %database_error_label(&error),
                            "earned-trust fact recording skipped one source"
                        );
                    }
                }
            }
        }
        after = Some(last);
    }
    Ok(summary)
}

/// A fixed label per error class. The error's text can carry SQL detail and
/// is never logged.
pub fn database_error_label(error: &DatabaseError) -> &'static str {
    match error {
        DatabaseError::Pool(_) => "pool",
        DatabaseError::Postgres(_) => "postgres",
        _ => "other",
    }
}

// ---------------------------------------------------------------------------
// Shadow evaluation, explain, and the reproduction drill.
// ---------------------------------------------------------------------------

/// A candidate's evaluations. Admission never reads them.
pub const SHADOW_MODE: &str = "shadow";
/// The admission policy's evaluations: what admission reads (V86's CHECK,
/// V114's read).
pub const APPLIED_MODE: &str = "applied";

/// Which evaluations a run writes or an explain reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EvaluationMode {
    /// Under the shadow candidate, at an `as_of` the caller may choose.
    Shadow,
    /// Under the admission policy's own `tiered-v1` rule, at the database's
    /// transaction time, bound to the inputs it read.
    Applied,
}

impl EvaluationMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Shadow => SHADOW_MODE,
            Self::Applied => APPLIED_MODE,
        }
    }
}

/// A stable, hash-only name for an account, for operator surfaces:
/// `sha256("trace-account-trust-ref.v1\n" || tenant_id || "\n" || account_id)`.
/// The explain route takes this, never a tenant or account ID.
pub fn account_trust_ref(account: &TrustAccount) -> String {
    let mut hash = Sha256::new();
    hash.update(b"trace-account-trust-ref.v1\n");
    hash.update(account.tenant_id().as_bytes());
    hash.update(b"\n");
    hash.update(account.account_id().to_string().as_bytes());
    format!("sha256:{}", hex::encode(hash.finalize()))
}

/// Label-only counts from one evaluation run.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct EvaluateSummary {
    pub mode: &'static str,
    pub growth_policy_version: String,
    pub accounts_scanned: u64,
    /// Accounts with at least one fact, each of which got an evaluation row.
    pub accounts_evaluated: u64,
    /// Accounts evaluated, by tier.
    pub tier_distribution: BTreeMap<u32, u64>,
    pub tier_changed: u64,
    pub penalty_active: u64,
    /// Qualified contributions the weekly cap discarded, summed.
    pub units_capped: u64,
    pub failed: u64,
    pub limit_reached: bool,
}

/// Evaluates every open account in an anchored tenant that has facts and
/// stores an evaluation for each. Accounts with no facts get no row.
///
/// `Shadow` evaluates at `as_of`, and nothing reads the rows to set an
/// allowance. `Applied` ignores `as_of`: each account is evaluated at the
/// database's transaction time in the snapshot it read its inputs from, and
/// admission reads the row while it stays fresh and its inputs unchanged.
pub async fn evaluate_account_trust(
    db: &dyn Database,
    policy: &GrowthPolicy,
    mode: EvaluationMode,
    as_of: DateTime<Utc>,
    limit: Option<i64>,
) -> Result<EvaluateSummary, DatabaseError> {
    let limit = limit
        .unwrap_or(RECORD_FACTS_DEFAULT_LIMIT)
        .clamp(0, RECORD_FACTS_MAX_LIMIT);
    let mut summary = EvaluateSummary {
        mode: mode.as_str(),
        growth_policy_version: policy.version().to_string(),
        ..EvaluateSummary::default()
    };
    let mut after: Option<TrustAccount> = None;
    'pages: loop {
        let page = db
            .list_account_trust_worker_accounts(after.as_ref(), ACCOUNT_PAGE)
            .await?;
        let Some(last) = page.last().cloned() else {
            break;
        };
        for account in &page {
            if summary.accounts_scanned as i64 >= limit {
                summary.limit_reached = true;
                break 'pages;
            }
            summary.accounts_scanned += 1;
            let outcome = match mode {
                EvaluationMode::Applied => {
                    db.evaluate_and_record_applied_account_trust(account, policy)
                        .await
                }
                EvaluationMode::Shadow => {
                    async {
                        let facts = db.account_trust_evaluation_inputs(account).await?;
                        if facts.is_empty() {
                            return Ok(None);
                        }
                        let evaluation = evaluate(policy, &facts, as_of);
                        let changed = db
                            .record_account_trust_evaluation(account, SHADOW_MODE, &evaluation)
                            .await?;
                        Ok::<_, DatabaseError>(Some((evaluation, changed)))
                    }
                    .await
                }
            };
            match outcome {
                Ok(None) => {}
                Ok(Some((evaluation, changed))) => {
                    summary.accounts_evaluated += 1;
                    *summary
                        .tier_distribution
                        .entry(evaluation.tier)
                        .or_default() += 1;
                    summary.tier_changed += u64::from(changed);
                    summary.penalty_active += u64::from(evaluation.penalty_active);
                    summary.units_capped += u64::from(evaluation.units_capped);
                }
                Err(error) => {
                    summary.failed += 1;
                    tracing::warn!(
                        error_label = database_error_label(&error),
                        "earned-trust evaluation skipped one account"
                    );
                }
            }
        }
        after = Some(last);
    }
    Ok(summary)
}

/// The stored and recomputed evaluation of one account, side by side.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExplainReport {
    pub account_ref: String,
    pub growth_policy_version: String,
    pub mode: &'static str,
    pub stored: Option<Evaluation>,
    /// The rule re-run at the stored `as_of` over the account's current facts
    /// and gate fields.
    pub recomputed: Option<Evaluation>,
    /// `stored == recomputed`. False when nothing is stored.
    pub reproduced: bool,
}

/// Recomputes the latest stored evaluation of `account` in `mode` under
/// `policy` at its stored `as_of`. Counts, tier, digest and policy version
/// only: never a submission ID or trace content.
pub async fn explain_account_trust(
    db: &dyn Database,
    policy: &GrowthPolicy,
    mode: EvaluationMode,
    account: &TrustAccount,
) -> Result<ExplainReport, DatabaseError> {
    let stored = db
        .latest_account_trust_evaluation(account, policy.version(), mode.as_str())
        .await?;
    let recomputed = match &stored {
        Some(stored) => {
            let facts = db.account_trust_evaluation_inputs(account).await?;
            Some(evaluate(policy, &facts, stored.as_of))
        }
        None => None,
    };
    Ok(ExplainReport {
        account_ref: account_trust_ref(account),
        growth_policy_version: policy.version().to_string(),
        mode: mode.as_str(),
        reproduced: stored.is_some() && stored == recomputed,
        stored,
        recomputed,
    })
}

/// Finds the account an `account_ref` names by walking the worker
/// enumeration. Linear in the number of accounts: an operator route, not a
/// hot path.
pub async fn find_account_by_ref(
    db: &dyn Database,
    account_ref: &str,
) -> Result<Option<TrustAccount>, DatabaseError> {
    let mut after: Option<TrustAccount> = None;
    loop {
        let page = db
            .list_account_trust_worker_accounts(after.as_ref(), ACCOUNT_PAGE)
            .await?;
        let Some(last) = page.last().cloned() else {
            return Ok(None);
        };
        if let Some(found) = page
            .into_iter()
            .find(|a| account_trust_ref(a) == account_ref)
        {
            return Ok(Some(found));
        }
        after = Some(last);
    }
}

/// Hash-only evidence from one reproduction drill.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct ReproductionDrillSummary {
    pub mode: &'static str,
    pub growth_policy_version: String,
    pub accounts_scanned: u64,
    /// Accounts with a stored evaluation in this mode and policy version.
    pub evaluations_checked: u64,
    pub reproduced: u64,
    pub not_reproduced: u64,
    pub failed: u64,
    pub limit_reached: bool,
    /// `sha256` over the sorted `(account_ref, reproduced, stored digest)`
    /// lines: evidence that names what was checked without naming an account.
    pub evidence_hash: String,
}

impl ReproductionDrillSummary {
    /// Switch-on condition 4 of the spec asks for 100% over the whole
    /// population; a drill that checked nothing proves nothing.
    pub fn passed(&self) -> bool {
        self.evaluations_checked > 0
            && self.not_reproduced == 0
            && self.failed == 0
            && !self.limit_reached
    }
}

/// Recomputes the latest stored evaluation in `mode` of up to `limit`
/// accounts and reports how many reproduce exactly.
pub async fn account_trust_reproduction_drill(
    db: &dyn Database,
    policy: &GrowthPolicy,
    mode: EvaluationMode,
    limit: Option<i64>,
) -> Result<ReproductionDrillSummary, DatabaseError> {
    let limit = limit
        .unwrap_or(RECORD_FACTS_MAX_LIMIT)
        .clamp(0, RECORD_FACTS_MAX_LIMIT);
    let mut summary = ReproductionDrillSummary {
        mode: mode.as_str(),
        growth_policy_version: policy.version().to_string(),
        ..ReproductionDrillSummary::default()
    };
    let mut lines: Vec<String> = Vec::new();
    let mut after: Option<TrustAccount> = None;
    'pages: loop {
        let page = db
            .list_account_trust_worker_accounts(after.as_ref(), ACCOUNT_PAGE)
            .await?;
        let Some(last) = page.last().cloned() else {
            break;
        };
        for account in &page {
            if summary.accounts_scanned as i64 >= limit {
                summary.limit_reached = true;
                break 'pages;
            }
            summary.accounts_scanned += 1;
            match explain_account_trust(db, policy, mode, account).await {
                Ok(report) => {
                    let Some(stored) = report.stored.as_ref() else {
                        continue;
                    };
                    summary.evaluations_checked += 1;
                    if report.reproduced {
                        summary.reproduced += 1;
                    } else {
                        summary.not_reproduced += 1;
                    }
                    lines.push(format!(
                        "{} {} {}",
                        report.account_ref, report.reproduced, stored.facts_digest
                    ));
                }
                Err(error) => {
                    summary.failed += 1;
                    tracing::warn!(
                        error_label = database_error_label(&error),
                        "earned-trust reproduction drill skipped one account"
                    );
                }
            }
        }
        after = Some(last);
    }
    lines.sort();
    let mut hash = Sha256::new();
    hash.update(b"trace-account-trust-drill.v1\n");
    hash.update(summary.growth_policy_version.as_bytes());
    hash.update(b"\n");
    for line in lines {
        hash.update(line.as_bytes());
        hash.update(b"\n");
    }
    summary.evidence_hash = format!("sha256:{}", hex::encode(hash.finalize()));
    Ok(summary)
}
