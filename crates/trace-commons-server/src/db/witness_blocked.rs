// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Daily per-tenant totals of submissions a contributor's daemon could not
//! send because the witness was busy, timed out or unreachable (V154,
//! `trace_commons_protocol::witness_blocked`).
//!
//! A report is added inside the authenticated tenant's context, so the
//! central tenant predicate decides which rows it can touch. The operator's
//! totals cross tenants and come from `trace_witness_blocked_totals()`, which
//! returns sums and distinct-tenant counts, never a tenant id. See
//! `migrations/V154__witness_blocked_days.sql` for the stance.

use async_trait::async_trait;
use chrono::NaiveDate;
use serde::Serialize;
use trace_commons_protocol::witness_blocked::{WITNESS_BLOCKED_REPORT_MAX, WitnessBlockedCounts};

use super::postgres::PgBackend;
use crate::error::DatabaseError;

/// The most one tenant's total for one label and day may reach. Matches the
/// table's CHECK; an addition past it holds the total here.
pub const WITNESS_BLOCKED_DAY_CAP: i64 = 100_000;

/// One group of the operator's totals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WitnessBlockedTotal {
    pub blocked: i64,
    /// Distinct tenants with a non-zero total in this group.
    pub tenants: i64,
}

/// The totals for one day and label.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WitnessBlockedDay {
    pub day: NaiveDate,
    pub label: String,
    #[serde(flatten)]
    pub total: WitnessBlockedTotal,
}

/// The totals for one label over the window.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WitnessBlockedLabelTotal {
    pub label: String,
    #[serde(flatten)]
    pub total: WitnessBlockedTotal,
}

/// The operator's totals since a day (UTC, inclusive).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct WitnessBlockedStats {
    pub since: NaiveDate,
    pub per_day: Vec<WitnessBlockedDay>,
    pub per_label: Vec<WitnessBlockedLabelTotal>,
    pub all: WitnessBlockedTotal,
}

/// The store. Implemented for PostgreSQL below.
#[async_trait]
pub trait WitnessBlockedStore: Send + Sync {
    /// Add one report to today's totals for `tenant_id`. Labels with a zero
    /// count write nothing. A count above
    /// [`WITNESS_BLOCKED_REPORT_MAX`] is refused without writing anything.
    async fn record_witness_blocked(
        &self,
        tenant_id: &str,
        counts: &WitnessBlockedCounts,
    ) -> Result<(), DatabaseError>;

    /// The totals across every tenant since `since`.
    async fn witness_blocked_stats(
        &self,
        since: NaiveDate,
    ) -> Result<WitnessBlockedStats, DatabaseError>;
}

#[async_trait]
impl WitnessBlockedStore for PgBackend {
    async fn record_witness_blocked(
        &self,
        tenant_id: &str,
        counts: &WitnessBlockedCounts,
    ) -> Result<(), DatabaseError> {
        if counts
            .entries()
            .any(|(_, n)| n > WITNESS_BLOCKED_REPORT_MAX)
        {
            return Err(DatabaseError::Constraint(
                "witness_blocked_report_over_cap".to_string(),
            ));
        }
        let (labels, values): (Vec<&str>, Vec<i64>) = counts
            .entries()
            .map(|(label, n)| (label.as_str(), i64::from(n)))
            .unzip();
        if labels.is_empty() {
            return Ok(());
        }
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        tx.execute(
            "INSERT INTO trace_witness_blocked_days (tenant_id, label, blocked_count)
             SELECT trace_current_tenant_id(), r.label, r.n
               FROM unnest($1::text[], $2::bigint[]) AS r(label, n)
             ON CONFLICT (tenant_id, day, label) DO UPDATE
                SET blocked_count = LEAST(
                    $3::bigint,
                    trace_witness_blocked_days.blocked_count + EXCLUDED.blocked_count)",
            &[&labels, &values, &WITNESS_BLOCKED_DAY_CAP],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn witness_blocked_stats(
        &self,
        since: NaiveDate,
    ) -> Result<WitnessBlockedStats, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let rows = client
            .query(
                "SELECT day, label, blocked, tenants FROM trace_witness_blocked_totals($1)",
                &[&since],
            )
            .await?;
        let mut per_day = Vec::new();
        let mut per_label = Vec::new();
        let mut all = WitnessBlockedTotal {
            blocked: 0,
            tenants: 0,
        };
        for row in rows {
            let day: Option<NaiveDate> = row.get(0);
            let label: Option<String> = row.get(1);
            let total = WitnessBlockedTotal {
                blocked: row.get(2),
                tenants: row.get(3),
            };
            match (day, label) {
                (Some(day), Some(label)) => per_day.push(WitnessBlockedDay { day, label, total }),
                (None, Some(label)) => per_label.push(WitnessBlockedLabelTotal { label, total }),
                (None, None) => all = total,
                // Not a grouping the function returns.
                (Some(_), None) => {}
            }
        }
        Ok(WitnessBlockedStats {
            since,
            per_day,
            per_label,
            all,
        })
    }
}
