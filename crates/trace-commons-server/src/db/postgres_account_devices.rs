// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The account devices list: the devices a contributor linked to their
//! account through NEAR sign-in, which they can rename and revoke.
//!
//! A device belongs to an account through its `trace_near_provisioned_devices`
//! row. Every query here is tenant-scoped under forced RLS and account-scoped
//! by an explicit predicate, so a caller never reads or changes another
//! account's device; an unknown and a foreign device look the same.

use super::*;

impl PgBackend {
    pub(super) async fn account_devices(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        session_token_hash: Option<&str>,
    ) -> Result<Vec<crate::db::AccountDeviceSummary>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // `this_device` matches the presented hash against the current token
        // and the previous one, because a rotated session keeps answering on
        // its previous token during the grace window.
        let rows = tx
            .query(
                "SELECT d.device_key_id, d.public_key, d.label, d.onboarding_origin,
                        n.created_at,
                        (SELECT max(s.last_seen_at) FROM trace_sessions s
                          WHERE s.tenant_id = trace_current_tenant_id()
                            AND s.account_id = n.account_id
                            AND s.device_key_id = d.device_key_id) AS last_seen_at,
                        EXISTS (SELECT 1 FROM trace_sessions s
                          WHERE s.tenant_id = trace_current_tenant_id()
                            AND s.account_id = n.account_id
                            AND s.device_key_id = d.device_key_id
                            AND s.revoked_at IS NULL
                            AND $2::text IS NOT NULL
                            AND (s.token_hash = $2 OR s.prev_token_hash = $2)) AS this_device,
                        contributed.traces AS contributed_traces,
                        contributed.last_at AS last_contributed_at
                   FROM trace_near_provisioned_devices n
                   JOIN device_keys d
                     ON d.tenant_id = n.tenant_id AND d.device_key_id = n.device_key_id
                   -- What the device has contributed that still stands: its
                   -- submissions under its own principal or an upload
                   -- principal bound to it (V135), while that principal is
                   -- linked to this account. The statuses are History's
                   -- accepted, submitted (received, no verdict yet) and
                   -- quarantined; a status not named here is not counted.
                   CROSS JOIN LATERAL (
                       SELECT count(*) AS traces, max(s.received_at) AS last_at
                         FROM trace_submissions s
                        WHERE s.tenant_id = trace_current_tenant_id()
                          AND s.status IN ('received', 'accepted', 'quarantined')
                          AND s.withdrawn_at IS NULL
                          AND s.revoked_at IS NULL
                          AND s.purged_at IS NULL
                          AND EXISTS (SELECT 1 FROM trace_account_principals p
                                       WHERE p.tenant_id = n.tenant_id
                                         AND p.principal_ref = n.principal_ref
                                         AND p.account_id = n.account_id
                                         AND p.unlinked_at IS NULL)
                          AND (s.auth_principal_ref = n.principal_ref
                               OR s.auth_principal_ref IN (
                                   SELECT u.upload_principal_ref
                                     FROM trace_device_upload_principals u
                                    WHERE u.tenant_id = n.tenant_id
                                      AND u.device_key_id = d.device_key_id))
                   ) contributed
                  WHERE n.tenant_id = trace_current_tenant_id()
                    AND n.account_id = $1
                    AND d.revoked_at IS NULL
                  ORDER BY n.created_at, d.device_key_id",
                &[&account_id, &session_token_hash],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(rows
            .into_iter()
            .map(|row| crate::db::AccountDeviceSummary {
                device_key_id: row.get("device_key_id"),
                public_key: row.get("public_key"),
                label: row.get("label"),
                onboarding_origin: row.get("onboarding_origin"),
                created_at: row.get("created_at"),
                last_seen_at: row.get("last_seen_at"),
                this_device: row.get("this_device"),
                contributed_traces: row.get("contributed_traces"),
                last_contributed_at: row.get("last_contributed_at"),
            })
            .collect())
    }

    pub(super) async fn rename_device_of_account(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        device_key_id: &str,
        label: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let affected = tx
            .execute(
                "UPDATE device_keys d
                    SET label = $3
                   FROM trace_near_provisioned_devices n
                  WHERE d.tenant_id = trace_current_tenant_id()
                    AND n.tenant_id = d.tenant_id
                    AND n.device_key_id = d.device_key_id
                    AND n.account_id = $1
                    AND d.device_key_id = $2
                    AND d.revoked_at IS NULL",
                &[&account_id, &device_key_id, &label],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(affected > 0)
    }

    pub(super) async fn revoke_device_of_account(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        device_key_id: &str,
        actor_ref: &str,
        caller_session_token_hash: Option<&str>,
    ) -> Result<crate::db::RevokeAccountDeviceResult, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // Revoke the key first; its RETURNING row is the ownership check for
        // the writes after it, so a foreign or already-revoked device changes
        // nothing. The row lock it takes is what a concurrent sign-in's
        // `FOR SHARE` read of the key waits on (`claim_provisioned_device_key`),
        // so a sign-in either commits its session before this transaction
        // reads the sessions, or sees the key revoked and is refused.
        let revoked = tx
            .query_opt(
                "UPDATE device_keys d
                    SET revoked_at = now()
                   FROM trace_near_provisioned_devices n
                  WHERE d.tenant_id = trace_current_tenant_id()
                    AND n.tenant_id = d.tenant_id
                    AND n.device_key_id = d.device_key_id
                    AND n.account_id = $1
                    AND d.device_key_id = $2
                    AND d.revoked_at IS NULL
              RETURNING n.principal_ref",
                &[&account_id, &device_key_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let Some(row) = revoked else {
            tx.commit().await.map_err(DatabaseError::Postgres)?;
            return Ok(crate::db::RevokeAccountDeviceResult::default());
        };
        let principal_ref: String = row.get(0);
        let grants_revoked = tx
            .execute(
                "UPDATE trace_tenant_access_grants
                    SET status = 'revoked',
                        revoked_at = now(),
                        revoked_by_principal_ref = $2,
                        reason = 'account_device_revoked',
                        updated_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND principal_ref = $1
                    AND status = 'active'",
                &[&principal_ref, &actor_ref],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let sessions_revoked = tx
            .execute(
                "UPDATE trace_sessions
                    SET revoked_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND device_key_id = $2
                    AND revoked_at IS NULL",
                &[&account_id, &device_key_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        // Native sessions that name neither a device key nor a credential
        // (issued before V139, or by the login-link native sign-in) cannot be
        // told apart from the revoked device's own, so they end too: a lost
        // laptop that signed in before V139 is not left live. A NEAR AI
        // device among them renews on its own with a keyed session; any
        // other signs in again. A native session that names a credential
        // (`auth_credential_id`) was minted by a passkey sign-in, and V144
        // revoked every one of those; browser and NEAR sessions and the
        // caller's own are left alone. Only live rows: past the idle cap
        // `validate_session` already refuses them.
        let unattributed_sessions_revoked = tx
            .execute(
                "UPDATE trace_sessions
                    SET revoked_at = now()
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND device_key_id IS NULL
                    AND auth_credential_id IS NULL
                    AND client_kind = $2
                    AND revoked_at IS NULL
                    AND expires_at > now()
                    AND last_seen_at > now() - INTERVAL '3 days'
                    AND ($3::text IS NULL
                         OR (token_hash <> $3
                             AND prev_token_hash IS DISTINCT FROM $3))",
                &[
                    &account_id,
                    &crate::account_native_auth::NATIVE_SESSION_CLIENT_KIND,
                    &caller_session_token_hash,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(crate::db::RevokeAccountDeviceResult {
            removed: true,
            sessions_revoked,
            grants_revoked,
            unattributed_sessions_revoked,
        })
    }
}
