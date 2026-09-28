// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust (Flow 3), shadow only.
//!
//! See `docs/superpowers/specs/2026-09-26-earned-account-trust-design.md`.
//! This module records the hash-only facts the growth rule reads. Nothing
//! here changes an allowance: the production admission policy still refuses
//! any `growth_rule` other than `"none"` (`account_trust::parse_bounded_policy`).

use std::collections::BTreeMap;

use serde::Serialize;

use crate::account_trust::TrustAccount;
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
