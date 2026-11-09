// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Append-only record of Terms of Service acceptances per account and org
//! (V158).

use chrono::{DateTime, Utc};
use uuid::Uuid;

use super::postgres::PgBackend;
use crate::error::DatabaseError;

impl PgBackend {
    /// Insert the acceptance, or find the original row when the unique key
    /// (tenant, account, version, hash) was already recorded. Returns the
    /// server's `recorded_at` for that row.
    #[allow(clippy::too_many_arguments)]
    pub async fn record_terms_acceptance(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        auth_method: &str,
        client_kind: &str,
        terms_version: &str,
        content_sha256: &str,
        accepted_at: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        let recorded_at = record_terms_acceptance_in_tx(
            &tx,
            tenant_id,
            account_id,
            auth_method,
            client_kind,
            terms_version,
            content_sha256,
            accepted_at,
        )
        .await?;
        tx.commit().await?;
        Ok(recorded_at)
    }
}

#[allow(clippy::too_many_arguments)]
async fn record_terms_acceptance_in_tx(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant_id: &str,
    account_id: Uuid,
    auth_method: &str,
    client_kind: &str,
    terms_version: &str,
    content_sha256: &str,
    accepted_at: DateTime<Utc>,
) -> Result<DateTime<Utc>, DatabaseError> {
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_id],
    )
    .await?;
    let inserted = tx
        .query_opt(
            "INSERT INTO account_terms_acceptances
                 (tenant_id, account_id, auth_method, client_kind,
                  terms_version, content_sha256, accepted_at)
             VALUES ($1, $2, $3, $4, $5, $6, $7)
             ON CONFLICT (tenant_id, account_id, terms_version, content_sha256)
             DO NOTHING
             RETURNING recorded_at",
            &[
                &tenant_id,
                &account_id,
                &auth_method,
                &client_kind,
                &terms_version,
                &content_sha256,
                &accepted_at,
            ],
        )
        .await?;
    if let Some(row) = inserted {
        return Ok(row.get(0));
    }
    let row = tx
        .query_one(
            "SELECT recorded_at FROM account_terms_acceptances
             WHERE tenant_id = $1 AND account_id = $2
               AND terms_version = $3 AND content_sha256 = $4",
            &[&tenant_id, &account_id, &terms_version, &content_sha256],
        )
        .await?;
    Ok(row.get(0))
}
