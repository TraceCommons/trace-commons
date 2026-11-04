// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The first run's estimate requests (V153): one row per
//! `POST /v1/public/first-run-copy`, and the operator's statistics over them.
//!
//! A write takes `trace_first_run_stats_writer` and a read takes
//! `trace_first_run_stats_reader`, each with `SET LOCAL ROLE`, which ends
//! with the transaction on every path. The table has no tenant, so no tenant
//! context is opened. See `migrations/V153__first_run_estimate_requests.sql`
//! for the stance.

use async_trait::async_trait;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Serialize;

use super::postgres::PgBackend;
use crate::error::DatabaseError;

/// One request, as stored. Figures are the coarsened ones the request
/// carried; a refusal stores none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FirstRunEstimateRecord {
    pub outcome: FirstRunEstimateOutcome,
    pub app_version: Option<String>,
    pub platform: Option<String>,
}

/// What the route answered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FirstRunEstimateOutcome {
    /// Lines were served: `default` or `usd`, with the lines' key and the
    /// figures received.
    Served {
        variant: &'static str,
        lines_key: String,
        trace_count: u64,
        total_bytes: u64,
    },
    /// The request was refused with this label.
    Refused { label: &'static str },
}

/// Requests for one day and outcome.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirstRunStatsDay {
    pub day: NaiveDate,
    pub outcome: String,
    pub requests: i64,
}

/// Requests for one value of a figure.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirstRunStatsValue {
    pub value: i64,
    pub requests: i64,
}

/// Requests per answer served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirstRunStatsOutcome {
    pub outcome: String,
    pub refusal_label: Option<String>,
    pub lines_key: Option<String>,
    pub requests: i64,
}

/// Requests per platform and app version.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirstRunStatsClient {
    pub platform: Option<String>,
    pub app_version: Option<String>,
    pub requests: i64,
}

/// The operator's statistics since an hour.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FirstRunEstimateStats {
    pub since: DateTime<Utc>,
    pub requests: i64,
    pub per_day: Vec<FirstRunStatsDay>,
    pub trace_counts: Vec<FirstRunStatsValue>,
    /// The size in whole MiB.
    pub total_mib: Vec<FirstRunStatsValue>,
    pub outcomes: Vec<FirstRunStatsOutcome>,
    pub clients: Vec<FirstRunStatsClient>,
}

/// The store. Implemented for PostgreSQL below.
#[async_trait]
pub trait FirstRunStatsStore: Send + Sync {
    /// Append one request. The database dates it to the hour.
    async fn record_first_run_estimate(
        &self,
        record: &FirstRunEstimateRecord,
    ) -> Result<(), DatabaseError>;

    /// Aggregates over the requests received at or after `since`.
    async fn first_run_estimate_stats(
        &self,
        since: DateTime<Utc>,
    ) -> Result<FirstRunEstimateStats, DatabaseError>;
}

fn as_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

#[async_trait]
impl FirstRunStatsStore for PgBackend {
    async fn record_first_run_estimate(
        &self,
        record: &FirstRunEstimateRecord,
    ) -> Result<(), DatabaseError> {
        let (outcome, refusal_label, lines_key, trace_count, total_bytes) = match &record.outcome {
            FirstRunEstimateOutcome::Served {
                variant,
                lines_key,
                trace_count,
                total_bytes,
            } => (
                *variant,
                None,
                Some(lines_key.as_str()),
                Some(as_i64(*trace_count)),
                Some(as_i64(*total_bytes)),
            ),
            FirstRunEstimateOutcome::Refused { label } => {
                ("refused", Some(*label), None, None, None)
            }
        };
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL ROLE trace_first_run_stats_writer")
            .await?;
        tx.execute(
            "INSERT INTO trace_first_run_estimate_requests
                 (outcome, refusal_label, lines_key, trace_count, total_bytes, app_version, platform)
             VALUES ($1, $2, $3, $4, $5, $6, $7)",
            &[
                &outcome,
                &refusal_label,
                &lines_key,
                &trace_count,
                &total_bytes,
                &record.app_version,
                &record.platform,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn first_run_estimate_stats(
        &self,
        since: DateTime<Utc>,
    ) -> Result<FirstRunEstimateStats, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL ROLE trace_first_run_stats_reader")
            .await?;
        let requests: i64 = tx
            .query_one(
                "SELECT count(*) FROM trace_first_run_estimate_requests WHERE received_hour >= $1",
                &[&since],
            )
            .await?
            .get(0);
        let per_day = tx
            .query(
                "SELECT (received_hour AT TIME ZONE 'UTC')::date AS day, outcome, count(*)
                   FROM trace_first_run_estimate_requests
                  WHERE received_hour >= $1
                  GROUP BY 1, 2 ORDER BY 1, 2",
                &[&since],
            )
            .await?
            .into_iter()
            .map(|row| FirstRunStatsDay {
                day: row.get(0),
                outcome: row.get(1),
                requests: row.get(2),
            })
            .collect();
        let values = |rows: Vec<tokio_postgres::Row>| -> Vec<FirstRunStatsValue> {
            rows.into_iter()
                .map(|row| FirstRunStatsValue {
                    value: row.get(0),
                    requests: row.get(1),
                })
                .collect()
        };
        let trace_counts = values(
            tx.query(
                "SELECT trace_count, count(*) FROM trace_first_run_estimate_requests
                  WHERE received_hour >= $1 AND trace_count IS NOT NULL
                  GROUP BY 1 ORDER BY 1",
                &[&since],
            )
            .await?,
        );
        let total_mib = values(
            tx.query(
                "SELECT total_bytes / 1048576, count(*) FROM trace_first_run_estimate_requests
                  WHERE received_hour >= $1 AND total_bytes IS NOT NULL
                  GROUP BY 1 ORDER BY 1",
                &[&since],
            )
            .await?,
        );
        let outcomes = tx
            .query(
                "SELECT outcome, refusal_label, lines_key, count(*)
                   FROM trace_first_run_estimate_requests
                  WHERE received_hour >= $1
                  GROUP BY 1, 2, 3 ORDER BY 4 DESC, 1, 2, 3",
                &[&since],
            )
            .await?
            .into_iter()
            .map(|row| FirstRunStatsOutcome {
                outcome: row.get(0),
                refusal_label: row.get(1),
                lines_key: row.get(2),
                requests: row.get(3),
            })
            .collect();
        let clients = tx
            .query(
                "SELECT platform, app_version, count(*)
                   FROM trace_first_run_estimate_requests
                  WHERE received_hour >= $1
                  GROUP BY 1, 2 ORDER BY 3 DESC, 1, 2",
                &[&since],
            )
            .await?
            .into_iter()
            .map(|row| FirstRunStatsClient {
                platform: row.get(0),
                app_version: row.get(1),
                requests: row.get(2),
            })
            .collect();
        tx.commit().await?;
        Ok(FirstRunEstimateStats {
            since,
            requests,
            per_day,
            trace_counts,
            total_mib,
            outcomes,
            clients,
        })
    }
}
