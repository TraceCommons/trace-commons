// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writes to `trace_account_bindings` (V97; Z2 native passkey identity).
//!
//! S1 only reads the table, inside `validate_session`. The one writer it adds
//! is the helper below, which S2's passkey `create/finish` is meant to call;
//! nothing calls it yet, and the runtime role has no INSERT on the table until
//! the migration that ships S2 grants it.

use super::*;

impl PgBackend {
    /// Insert a passkey-origin account AND its `unbound` binding row, in the
    /// caller's transaction.
    ///
    /// This is the only sanctioned way to create a passkey-origin account.
    /// A binding row's absence means "legacy, never gated", so a passkey
    /// account written without one would silently skip the unbound gate. Doing
    /// both inserts here, on a transaction the caller must already hold, means
    /// there is no call that writes one row without the other, and a rollback
    /// or a crash before commit leaves neither.
    ///
    /// Both rows take their tenant from `trace_current_tenant_id()`, never from
    /// an argument, so they cannot land in different tenants, and a
    /// transaction with no tenant context fails on the NOT NULL tenant column
    /// instead of writing anything. The caller creates the tenant row first
    /// (S2 mints it in the same transaction).
    pub async fn insert_passkey_origin_account(
        tx: &tokio_postgres::Transaction<'_>,
        account_id: Uuid,
    ) -> Result<(), DatabaseError> {
        tx.execute(
            "INSERT INTO trace_accounts (tenant_id, account_id)
             VALUES (trace_current_tenant_id(), $1)",
            &[&account_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.execute(
            "INSERT INTO trace_account_bindings (tenant_id, account_id, origin, state)
             VALUES (trace_current_tenant_id(), $1, 'passkey', 'unbound')",
            &[&account_id],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        Ok(())
    }
}
