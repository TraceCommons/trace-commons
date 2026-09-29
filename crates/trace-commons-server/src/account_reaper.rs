// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The unbound-account reaper (Z2, slice S5; V99).
//!
//! Cancel leaves an unbound passkey account inert; this is what reclaims it
//! after it has sat unbound and unused: 7 days for an account never used
//! again after creation, 30 days for one that signed in again. The delete is a
//! cross-tenant sweep, so it runs through `trace_reap_unbound_accounts`, a
//! `SECURITY DEFINER` function, on its own small pool whose login holds only
//! the `trace_unbound_account_reaper` role. It never touches the runtime pool.
//!
//! Everything the sweep reports is a count. No tenant, account, credential
//! or session identifier leaves the database.

use deadpool_postgres::{Manager, Pool};

use crate::error::DatabaseError;

/// The decided default for an account that signed in again after creation:
/// 30 days with no session activity.
pub const DEFAULT_TTL_DAYS: i64 = 30;
/// The decided default for an account never used again after creation (one
/// session row, last seen within an hour of creation; see V99): 7 days.
pub const DEFAULT_NEVER_USED_TTL_DAYS: i64 = 7;
/// The SQL function refuses anything under one day, whatever is configured.
pub const MIN_TTL_DAYS: i64 = 1;
pub const MAX_TTL_DAYS: i64 = 3650;
/// The most accounts one call may delete; the SQL function enforces it too.
pub const MAX_BATCH: i32 = 1000;
pub const DEFAULT_BATCH: i32 = 100;

/// Counts only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReapSummary {
    /// Accounts deleted, with their tenant, binding, credential and sessions.
    pub reaped: u64,
    /// Candidates left alone: bound or locked by a concurrent request, active
    /// again since the scan, or refused by a foreign key.
    pub skipped: u64,
}

/// A connection to the reaper's dedicated login.
#[derive(Clone)]
pub struct UnboundAccountReaper {
    pool: Pool,
}

impl UnboundAccountReaper {
    /// Build the pool from the reaper login's connection string. The login
    /// must inherit `trace_unbound_account_reaper` (V99) and nothing else;
    /// never pass the runtime URL.
    pub fn connect(url: &str) -> Result<Self, DatabaseError> {
        let config = url.parse::<tokio_postgres::Config>().map_err(|e| {
            DatabaseError::Pool(format!("invalid unbound-reaper PostgreSQL URL: {e}"))
        })?;
        let manager = Manager::new(config, tokio_postgres::NoTls);
        let pool = Pool::builder(manager).max_size(2).build()?;
        Ok(Self { pool })
    }

    /// Delete up to `limit` unbound accounts: those idle for `ttl_days`, and
    /// those never used again after creation for `never_used_ttl_days`, which
    /// may not exceed `ttl_days` (the SQL function refuses it). Idempotent: a
    /// second call finds nothing the first one left eligible.
    pub async fn reap(
        &self,
        ttl_days: i64,
        never_used_ttl_days: i64,
        limit: i32,
    ) -> Result<ReapSummary, DatabaseError> {
        let to_seconds = |days: i64| {
            days.checked_mul(86_400)
                .ok_or_else(|| DatabaseError::Pool("unbound_reaper_ttl_out_of_range".into()))
        };
        let ttl_seconds = to_seconds(ttl_days)?;
        let never_used_ttl_seconds = to_seconds(never_used_ttl_days)?;
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "SELECT reaped, skipped FROM trace_reap_unbound_accounts($1, $2, $3)",
                &[&ttl_seconds, &never_used_ttl_seconds, &limit],
            )
            .await?;
        let reaped: i64 = row.get(0);
        let skipped: i64 = row.get(1);
        Ok(ReapSummary {
            reaped: u64::try_from(reaped).unwrap_or(0),
            skipped: u64::try_from(skipped).unwrap_or(0),
        })
    }
}
