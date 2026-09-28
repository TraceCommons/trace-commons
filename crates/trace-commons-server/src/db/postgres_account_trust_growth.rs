// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Earned account trust: the batch seams the fact recorder and the shadow
//! evaluator run through. Every read and write here goes through a
//! `SECURITY DEFINER` function owned by a NOLOGIN, NOBYPASSRLS guard role;
//! the login needs only `trace_account_trust_worker` (V85), never a table
//! privilege on the facts. Nothing in this file is read by admission.

use uuid::Uuid;

use super::PgBackend;
use crate::account_trust::{TrustAccount, TrustFactSource};
use crate::error::DatabaseError;

impl PgBackend {
    /// One page of open accounts in anchored tenants, in `(tenant, account)`
    /// order after `after`. The only cross-tenant read the worker makes, and
    /// it returns nothing but the two keys.
    pub async fn list_account_trust_worker_accounts(
        &self,
        after: Option<&TrustAccount>,
        limit: i64,
    ) -> Result<Vec<TrustAccount>, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let after_tenant = after.map(|a| a.tenant_id().to_string());
        let after_account = after.map(TrustAccount::account_id);
        let rows = client
            .query(
                "SELECT tenant_id, account_id FROM trace_account_trust_worker_accounts($1,$2,$3)",
                &[&after_tenant, &after_account, &limit.clamp(0, 10_000)],
            )
            .await?;
        Ok(rows
            .into_iter()
            .map(|row| TrustAccount::from_worker_enumeration(row.get(0), row.get(1)))
            .collect())
    }

    /// The sources in this account's tenant that name this account and have
    /// no fact yet. Tenant-scoped under forced RLS.
    pub async fn list_account_trust_fact_candidates(
        &self,
        account: &TrustAccount,
        limit: i64,
    ) -> Result<Vec<TrustFactSource>, DatabaseError> {
        let tenant = account.tenant_id();
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT source_kind, source_id FROM trace_account_trust_fact_candidates($1,$2,$3)",
                &[&tenant, &account.account_id(), &limit.clamp(0, 10_000)],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .into_iter()
            .filter_map(|row| {
                let kind: String = row.get(0);
                let source: Uuid = row.get(1);
                TrustFactSource::from_storage(&kind, source)
            })
            .collect())
    }
}
