// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Writes to `trace_account_bindings` (V97; Z2 native passkey identity).
//!
//! S1 reads the table inside `validate_session`. S2 adds its one writer,
//! native passkey `create/finish`, which reaches it only through
//! [`PgBackend::insert_passkey_origin_account`] inside
//! `create_passkey_origin_account_in_tx`; V98 grants the runtime the INSERT.

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

    /// The V98 cross-tenant count of `unbound` passkey-origin accounts, on the
    /// runtime pool with no tenant context. The definer function is the only
    /// thing that can see across tenants; this caller cannot.
    pub(super) async fn unbound_passkey_account_count(&self) -> Result<i64, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let row = client
            .query_one("SELECT public.trace_unbound_passkey_account_count()", &[])
            .await
            .map_err(DatabaseError::Postgres)?;
        Ok(row.get(0))
    }

    /// Native passkey `create/finish`'s one transaction (Z2 S2).
    ///
    /// Order matters for two properties:
    ///
    /// - **The ceiling holds under concurrency.** Every creation takes the
    ///   same transaction-scoped advisory lock before it counts, so two
    ///   finishes that both passed the check at `start` serialize here and the
    ///   second counts the first's committed row.
    /// - **All or nothing.** The tenant, the account with its binding row (via
    ///   [`Self::insert_passkey_origin_account`], the only sanctioned writer of
    ///   a passkey-origin account), the credential, the session and the audit
    ///   row are one transaction. Any failure (a replayed credential id, a
    ///   session hash collision, a dropped connection) returns before commit,
    ///   and the dropped transaction rolls every row back, the tenant included.
    ///
    /// Writing the tenant here is not the Slice 1 bug class: the caller mints
    /// the tenant id from the OS RNG after the attestation verified; it never
    /// comes from a request.
    pub(super) async fn create_passkey_origin_account_in_tx(
        &self,
        new: crate::db::NewPasskeyOriginAccount<'_>,
    ) -> Result<crate::db::PasskeyOriginAccountOutcome, DatabaseError> {
        if new.session.client_kind != crate::account_native_auth::NATIVE_SESSION_CLIENT_KIND {
            return Err(DatabaseError::Serialization(
                "passkey_origin_session_must_be_native".to_string(),
            ));
        }
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, new.tenant_id).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended('trace_unbound_passkey_account_ceiling', 0))",
            &[],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        let unbound: i64 = tx
            .query_one("SELECT public.trace_unbound_passkey_account_count()", &[])
            .await
            .map_err(DatabaseError::Postgres)?
            .get(0);
        if unbound >= new.ceiling {
            // Dropping the transaction rolls it back; nothing was written.
            return Ok(crate::db::PasskeyOriginAccountOutcome::CeilingReached);
        }
        // A fresh random tenant must be new. No ON CONFLICT: a collision is an
        // error, never a silent adoption of an existing tenant.
        tx.execute(
            "INSERT INTO trace_tenants (tenant_id) VALUES (trace_current_tenant_id())",
            &[],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        Self::insert_passkey_origin_account(&tx, new.account_id).await?;
        tx.execute(
            "INSERT INTO trace_webauthn_credentials (
                tenant_id, credential_id, account_id, passkey, label, created_at
             ) VALUES (trace_current_tenant_id(), $1, $2, $3, $4, now())",
            &[
                &new.credential_id,
                &new.account_id,
                &new.passkey,
                &new.label,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.execute(
            "INSERT INTO trace_sessions (
                tenant_id, session_id, account_id, token_hash,
                client_kind, auth_credential_id, created_at, last_seen_at, expires_at
             ) VALUES (trace_current_tenant_id(), $1, $2, $3, $4, $5, now(), now(), $6)",
            &[
                &Uuid::new_v4(),
                &new.account_id,
                &new.session.token_hash,
                &new.session.client_kind,
                &new.credential_id,
                &new.session.expires_at,
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        let actor_ref = crate::account_session::account_actor_ref(
            &crate::account_session::AccountId::from_uuid(new.account_id),
        );
        tx.execute(
            "INSERT INTO trace_account_audit (
                tenant_id, action, actor_ref, outcome, safe_metadata
             ) VALUES (trace_current_tenant_id(), 'account_passkey_created', $1, 'success', $2)",
            &[
                &actor_ref,
                &serde_json::json!({ "binding": "unbound", "labeled": new.label.is_some() }),
            ],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(crate::db::PasskeyOriginAccountOutcome::Created)
    }

    /// One account's binding state under its tenant's RLS; no row is legacy.
    pub(super) async fn binding_state_for_account(
        &self,
        tenant_id: &str,
        account_id: Uuid,
    ) -> Result<crate::account_binding::AccountBindingState, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let state: Option<String> = tx
            .query_opt(
                "SELECT state FROM trace_account_bindings
                  WHERE tenant_id = trace_current_tenant_id() AND account_id = $1",
                &[&account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .map(|row| row.get(0));
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        crate::account_binding::AccountBindingState::from_stored(state.as_deref())
            .map_err(|error| DatabaseError::Serialization(error.to_string()))
    }
}
