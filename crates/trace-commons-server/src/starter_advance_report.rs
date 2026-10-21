// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The NEAR AI starter advance's dry-run report: what
//! `starter-plausibility-v1` would decide for each open account, as
//! label-only counts. Sends, grants and stores nothing. See
//! `docs/superpowers/specs/2026-10-21-nearai-starter-advance-design.md`,
//! section 5, "Dry run before live".

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::account_trust::TrustAccount;
use crate::account_trust_growth::database_error_label;
use crate::db::Database;
use crate::error::DatabaseError;
use crate::starter_plausibility::{
    StarterDryRunReport, StarterEnvelopeFeatures, StarterPolicy, evaluate,
};

/// Accounts fetched per enumeration page.
const ACCOUNT_PAGE: i64 = 200;
/// Accounts one report visits when the caller names no limit.
pub const REPORT_DEFAULT_LIMIT: i64 = 1_000;
/// Upper bound on accounts one report visits, whatever the caller asks.
pub const REPORT_MAX_LIMIT: i64 = 10_000;

/// The two reads the report needs. Every [`Database`] provides them: the
/// account enumeration is the earned-trust worker seam (open accounts in
/// anchored tenants, V85), the features are V131's.
#[async_trait]
pub trait StarterReportSource: Send + Sync {
    async fn accounts_after(
        &self,
        after: Option<&TrustAccount>,
        limit: i64,
    ) -> Result<Vec<TrustAccount>, DatabaseError>;

    async fn account_features(
        &self,
        account: &TrustAccount,
    ) -> Result<Vec<StarterEnvelopeFeatures>, DatabaseError>;
}

#[async_trait]
impl<D: Database + ?Sized> StarterReportSource for D {
    async fn accounts_after(
        &self,
        after: Option<&TrustAccount>,
        limit: i64,
    ) -> Result<Vec<TrustAccount>, DatabaseError> {
        self.list_account_trust_worker_accounts(after, limit).await
    }

    async fn account_features(
        &self,
        account: &TrustAccount,
    ) -> Result<Vec<StarterEnvelopeFeatures>, DatabaseError> {
        self.starter_plausibility_features_for_account(account)
            .await
    }
}

/// Evaluates up to `limit` accounts (default [`REPORT_DEFAULT_LIMIT`], at
/// most [`REPORT_MAX_LIMIT`]) at `now`. An account whose features cannot be
/// read counts as `failed` and the run continues; an enumeration failure
/// ends the run with the error. Accounts with no features are scanned but
/// not evaluated.
pub async fn starter_dry_run_report(
    source: &(impl StarterReportSource + ?Sized),
    limit: Option<i64>,
    now: DateTime<Utc>,
) -> Result<StarterDryRunReport, DatabaseError> {
    let policy = StarterPolicy::v1();
    let mut remaining = limit
        .unwrap_or(REPORT_DEFAULT_LIMIT)
        .clamp(0, REPORT_MAX_LIMIT);
    let mut report = StarterDryRunReport::new();
    let mut after: Option<TrustAccount> = None;
    'pages: loop {
        let page = source.accounts_after(after.as_ref(), ACCOUNT_PAGE).await?;
        let Some(last) = page.last().cloned() else {
            break;
        };
        for account in &page {
            if remaining == 0 {
                report.limit_reached = true;
                break 'pages;
            }
            remaining -= 1;
            report.accounts_scanned += 1;
            match source.account_features(account).await {
                Ok(features) if features.is_empty() => {}
                Ok(features) => report.record(&evaluate(&policy, &features, now)),
                Err(error) => {
                    report.failed += 1;
                    tracing::warn!(
                        error_label = database_error_label(&error),
                        "starter features unreadable for one account"
                    );
                }
            }
        }
        after = Some(last);
    }
    Ok(report)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use uuid::Uuid;

    use super::*;

    /// Accounts in enumeration order, each with its features or a read error.
    struct Fixture {
        accounts: Vec<(TrustAccount, Option<Vec<StarterEnvelopeFeatures>>)>,
    }

    impl Fixture {
        fn new(
            count: usize,
            features: impl Fn(usize) -> Option<Vec<StarterEnvelopeFeatures>>,
        ) -> Self {
            Self {
                accounts: (0..count)
                    .map(|i| {
                        (
                            TrustAccount::from_worker_enumeration(
                                "tenant-a".to_string(),
                                Uuid::from_u128(i as u128 + 1),
                            ),
                            features(i),
                        )
                    })
                    .collect(),
            }
        }
    }

    #[async_trait]
    impl StarterReportSource for Fixture {
        async fn accounts_after(
            &self,
            after: Option<&TrustAccount>,
            limit: i64,
        ) -> Result<Vec<TrustAccount>, DatabaseError> {
            let start = match after {
                None => 0,
                Some(after) => {
                    self.accounts
                        .iter()
                        .position(|(account, _)| account == after)
                        .expect("cursor is a listed account")
                        + 1
                }
            };
            Ok(self
                .accounts
                .iter()
                .skip(start)
                .take(limit as usize)
                .map(|(account, _)| account.clone())
                .collect())
        }

        async fn account_features(
            &self,
            account: &TrustAccount,
        ) -> Result<Vec<StarterEnvelopeFeatures>, DatabaseError> {
            self.accounts
                .iter()
                .find(|(listed, _)| listed == account)
                .and_then(|(_, features)| features.clone())
                .ok_or_else(|| DatabaseError::Pool("fixture_read_failed".into()))
        }
    }

    fn one_envelope() -> Vec<StarterEnvelopeFeatures> {
        vec![StarterEnvelopeFeatures {
            version: crate::starter_plausibility::STARTER_FEATURES_VERSION.to_string(),
            agent: crate::starter_plausibility::StarterAgent::ClaudeCode,
            session_hash: Some("sha256:00".to_string()),
            content_hash: "sha256:01".to_string(),
            first_event_hour: Some(Utc::now()),
            last_event_hour: Some(Utc::now()),
            future_dated: false,
            model_family: crate::starter_plausibility::ModelFamily::Anthropic,
            tool_calls: 3,
            tool_results: 3,
            paired_tool_calls: 3,
            failed_tool_results: 0,
            known_tool_calls: 3,
            turn_gaps: 2,
            think_time_gaps: 1,
            gap_bucket_mask: 0b110,
            latency_bucket_mask: 0,
            assistant_events: 2,
            token_events: 2,
            input_token_steps: 1,
            input_token_rises: 1,
            cost_events: 1,
        }]
    }

    #[tokio::test]
    async fn report_counts_scanned_evaluated_and_failed_accounts() {
        // Accounts 0..450: every third has features, every tenth read fails.
        let fixture = Fixture::new(450, |i| {
            if i % 10 == 9 {
                None
            } else if i % 3 == 0 {
                Some(one_envelope())
            } else {
                Some(Vec::new())
            }
        });
        let report = starter_dry_run_report(&fixture, None, Utc::now())
            .await
            .unwrap();
        let failed = (0..450).filter(|i| i % 10 == 9).count() as u64;
        let with_features = (0..450).filter(|i| i % 10 != 9 && i % 3 == 0).count() as u64;
        assert_eq!(report.accounts_scanned, 450);
        assert_eq!(report.failed, failed);
        assert_eq!(report.accounts_with_features, with_features);
        assert_eq!(report.would_grant, 0);
        assert_eq!(
            report.not_yet_by_rule,
            BTreeMap::from([("volume", with_features)])
        );
        assert!(!report.limit_reached);
    }

    #[tokio::test]
    async fn report_stops_at_the_limit_across_pages() {
        let fixture = Fixture::new(450, |_| Some(Vec::new()));
        let report = starter_dry_run_report(&fixture, Some(250), Utc::now())
            .await
            .unwrap();
        assert_eq!(report.accounts_scanned, 250);
        assert!(report.limit_reached);

        let capped = starter_dry_run_report(&fixture, Some(REPORT_MAX_LIMIT + 5), Utc::now())
            .await
            .unwrap();
        assert_eq!(capped.accounts_scanned, 450);
        assert!(!capped.limit_reached);

        let none = starter_dry_run_report(&fixture, Some(-3), Utc::now())
            .await
            .unwrap();
        assert_eq!(none.accounts_scanned, 0);
    }

    #[tokio::test]
    async fn an_empty_enumeration_is_an_empty_report() {
        let report = starter_dry_run_report(&Fixture::new(0, |_| None), None, Utc::now())
            .await
            .unwrap();
        assert_eq!(report.accounts_scanned, 0);
        assert_eq!(report.rule_version, "starter-plausibility-v1");
    }
}
