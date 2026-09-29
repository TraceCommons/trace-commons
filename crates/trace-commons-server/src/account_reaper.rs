// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The passkey-account reaper (Z2, slice S5; V99).
//!
//! Cancel leaves an unbound passkey account inert, and S3's refuse branch
//! leaves a closed one. This is what reclaims both:
//!
//! - an `unbound` account 7 days after its binding was created, whether or not
//!   it signed in again in the meantime (decided 2026-09-29: reap on bound
//!   status, not on use);
//! - a `closed` account 30 days after `trace_accounts.closed_at`, which S3
//!   sets in the transaction that closes the binding.
//!
//! Either way the account is reaped only once it holds no live session. The
//! reap deletes the account and what cascades from it (binding, credentials,
//! sessions, login links); the tenant row is never deleted. An account-keyed
//! row that does not cascade refuses the candidate. The delete is a
//! cross-tenant sweep, so
//! it runs through `trace_reap_unbound_accounts`, a `SECURITY DEFINER`
//! function, on its own small pool whose login holds only the
//! `trace_unbound_account_reaper` role. It never touches the runtime pool.
//!
//! Everything the sweep reports is a count. No tenant, account, credential
//! or session identifier leaves the database.

use std::time::Duration;

use deadpool_postgres::{Manager, Pool};

use crate::error::DatabaseError;

/// The decided default for an `unbound` account: 7 days from its binding's
/// creation.
pub const DEFAULT_UNBOUND_TTL_DAYS: i64 = 7;
/// The decided default for a `closed` account: 30 days from `closed_at`.
pub const DEFAULT_CLOSED_TTL_DAYS: i64 = 30;
/// The SQL function refuses anything under one day, whatever is configured.
pub const MIN_TTL_DAYS: i64 = 1;
pub const MAX_TTL_DAYS: i64 = 3650;
/// The most accounts one call may delete; the SQL function enforces it too.
pub const MAX_BATCH: i32 = 1000;
pub const DEFAULT_BATCH: i32 = 100;
/// How long boot waits for the reaper login before failing.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

/// Counts only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ReapSummary {
    /// Unbound accounts deleted, with the rows that cascade from them
    /// (binding, credentials, sessions). Their tenant rows are kept.
    pub reaped_unbound: u64,
    /// Closed accounts deleted, likewise.
    pub reaped_closed: u64,
    /// Candidates left alone: locked by a concurrent request, holding a live
    /// session since the scan, refused by a non-cascading foreign key into the
    /// account, timed out on a lock, or chosen as a deadlock victim.
    pub skipped: u64,
}

impl ReapSummary {
    /// Every account deleted, of either kind.
    pub fn reaped(&self) -> u64 {
        self.reaped_unbound + self.reaped_closed
    }
}

/// A connection to the reaper's dedicated login.
#[derive(Clone)]
pub struct UnboundAccountReaper {
    pool: Pool,
}

impl UnboundAccountReaper {
    /// Build the pool from the reaper login's connection string. The login
    /// must inherit `trace_unbound_account_reaper` (V99) and nothing else;
    /// never pass the runtime URL. The pool connects lazily; call
    /// [`Self::verify_login`] at boot.
    pub fn connect(url: &str) -> Result<Self, DatabaseError> {
        let mut config = url.parse::<tokio_postgres::Config>().map_err(|e| {
            DatabaseError::Pool(format!("invalid unbound-reaper PostgreSQL URL: {e}"))
        })?;
        if config.get_connect_timeout().is_none() {
            config.connect_timeout(CONNECT_TIMEOUT);
        }
        let manager = Manager::new(config, tokio_postgres::NoTls);
        let pool = Pool::builder(manager).max_size(2).build()?;
        Ok(Self { pool })
    }

    /// Take one connection from the pool, so a login that cannot connect
    /// fails boot instead of the first tick.
    pub async fn verify_login(&self) -> Result<(), DatabaseError> {
        let _connection = self.pool.get().await?;
        Ok(())
    }

    /// Delete up to `limit` accounts: `unbound` ones whose binding is older
    /// than `unbound_ttl_days`, and `closed` ones closed longer ago than
    /// `closed_ttl_days`. Idempotent: a second call finds nothing the first
    /// one left eligible.
    pub async fn reap(
        &self,
        unbound_ttl_days: i64,
        closed_ttl_days: i64,
        limit: i32,
    ) -> Result<ReapSummary, DatabaseError> {
        let to_seconds = |days: i64| {
            days.checked_mul(86_400)
                .ok_or_else(|| DatabaseError::Pool("unbound_reaper_ttl_out_of_range".into()))
        };
        let unbound_ttl_seconds = to_seconds(unbound_ttl_days)?;
        let closed_ttl_seconds = to_seconds(closed_ttl_days)?;
        let client = self.pool.get().await?;
        let row = client
            .query_one(
                "SELECT reaped_unbound, reaped_closed, skipped
                   FROM trace_reap_unbound_accounts($1, $2, $3)",
                &[&unbound_ttl_seconds, &closed_ttl_seconds, &limit],
            )
            .await?;
        let count = |i: usize| u64::try_from(row.get::<_, i64>(i)).unwrap_or(0);
        Ok(ReapSummary {
            reaped_unbound: count(0),
            reaped_closed: count(1),
            skipped: count(2),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Nothing listens on port 1, so the connection is refused at once. The
    /// PostgreSQL suite covers a reachable server rejecting the login.
    #[tokio::test]
    async fn an_unreachable_reaper_login_fails_the_startup_check() {
        let reaper = UnboundAccountReaper::connect("postgres://nobody@127.0.0.1:1/nothing")
            .expect("the pool builds lazily");
        assert!(reaper.verify_login().await.is_err());
    }
}
