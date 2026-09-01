// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Orchestration for the `trace_register_stats` singleton row.
//!
//! Thin: both functions here just call the [`crate::db::Database`] methods
//! that hold the actual SQL. This module exists so the worker-route handler
//! and the RLS tests share one code path rather than each re-deriving how a
//! refresh is computed and written.

use crate::db::Database;
pub use crate::db::RegisterStatsRow;
use crate::error::DatabaseError;

/// Read the `trace_register_stats` singleton row as it stands.
pub async fn fetch_register_stats_row(
    db: &dyn Database,
) -> Result<RegisterStatsRow, DatabaseError> {
    db.fetch_register_stats_row().await
}

/// Recompute the public aggregate and write it. Batch-only and idempotent:
/// callers may run this as often as they like, and each run replaces the
/// row's figures wholesale rather than accumulating onto them.
///
/// `configured_tenant_ids` is passed straight through to
/// [`Database::compute_register_stats_totals`]: empty means "enumerate every
/// tenant" (via that method's own fallback query), non-empty scopes the
/// refresh to exactly those tenants. An empty *resolved* enumeration -- the
/// override was empty and the fallback query found nothing, which is what
/// happens when RLS silently eats an unset-tenant-context query -- makes
/// that method refuse with an `Err` rather than hand back a zero. This
/// function never writes when that happens: it always writes the totals
/// `compute_register_stats_totals` actually returned, never a substitute.
pub async fn run_register_stats_refresh(
    db: &dyn Database,
    configured_tenant_ids: &[String],
) -> Result<RegisterStatsRow, DatabaseError> {
    let totals = db
        .compute_register_stats_totals(configured_tenant_ids)
        .await?;
    db.write_register_stats_row(totals).await
}
