// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! One-time codes that link a headless device to an account, and the
//! device-key session renewal only such a device may use (owner decisions
//! 2026-10-27).
//!
//! A signed-in device mints a code; a headless device redeems it and is
//! linked exactly as a NEAR AI sign-in links a device
//! ([`super::account_onboarding::attach_near_ai_login_device`]), under the
//! anchor of the device that minted the code. The redeemed device has no
//! NEAR AI login to renew its 12-hour session with, so it renews by signing
//! with its device key; a device that joined any other way cannot.

use super::account_onboarding::attach_near_ai_login_device;
use super::*;
use crate::account_onboarding::{
    DEVICE_KEY_REGISTERED_ELSEWHERE, DEVICE_KEY_REVOKED, ProvisionedNearAccount,
};
use crate::db::{
    AccountDeviceCodeStatus, DeviceSignInOutcome, MintAccountDeviceCodeOutcome,
    NewAccountDeviceCode, NewSession,
};
use crate::near_ai_signup_policy::NearAiSignupPolicyTemplate;

/// The refusals `attach_near_ai_login_device` names when the key cannot be
/// linked to this account: held by another tenant, its principal linked to
/// another account, revoked, or differently recorded in this tenant.
fn is_device_key_conflict(error: &DatabaseError) -> bool {
    matches!(
        error,
        DatabaseError::Pool(label)
            if label == DEVICE_KEY_REGISTERED_ELSEWHERE
                || label == "near_provisioning_device_bound_to_other_account"
                || label == "near_provisioning_refused"
                || label == DEVICE_KEY_REVOKED
    )
}

impl PgBackend {
    pub(super) async fn mint_device_code(
        &self,
        code: NewAccountDeviceCode<'_>,
    ) -> Result<MintAccountDeviceCodeOutcome, DatabaseError> {
        self.ensure_trace_tenant(code.tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, code.tenant_id).await?;
        let minter = tx
            .query_opt(
                "SELECT 1 FROM trace_near_provisioned_devices n
                   JOIN device_keys d
                     ON d.tenant_id = n.tenant_id AND d.device_key_id = n.device_key_id
                  WHERE n.tenant_id = trace_current_tenant_id()
                    AND n.account_id = $1
                    AND d.device_key_id = $2
                    AND d.revoked_at IS NULL
                    -- A device that joined by code never mints one: a code
                    -- links a key that renews by itself, so a server that
                    -- could mint could hand that on past its own revoke.
                    AND NOT EXISTS (SELECT 1 FROM trace_account_device_codes c
                                     WHERE c.tenant_id = n.tenant_id
                                       AND c.redeemed_device_key_id = d.device_key_id)",
                &[&code.account_id, &code.minter_device_key_id],
            )
            .await?;
        if minter.is_none() {
            return Ok(MintAccountDeviceCodeOutcome::MinterNotADevice);
        }
        // Serialize mints per account so the outstanding cap holds.
        let lock_key = format!("device-codes:{}:{}", code.tenant_id, code.account_id);
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&lock_key],
        )
        .await?;
        let outstanding: i64 = tx
            .query_one(
                "SELECT count(*) FROM trace_account_device_codes
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND redeemed_at IS NULL
                    AND expires_at > now()",
                &[&code.account_id],
            )
            .await?
            .get(0);
        if outstanding >= code.outstanding_cap {
            return Ok(MintAccountDeviceCodeOutcome::TooManyOutstanding);
        }
        let code_id = Uuid::new_v4();
        let inserted = tx
            .execute(
                "INSERT INTO trace_account_device_codes
                    (tenant_id, code_id, account_id, code_hash, minted_by_device_key_id,
                     mint_nonce_hash, label, expires_at)
                 VALUES (trace_current_tenant_id(), $1, $2, $3, $4, $5, $6, $7)
                 ON CONFLICT DO NOTHING",
                &[
                    &code_id,
                    &code.account_id,
                    &code.code_hash,
                    &code.minter_device_key_id,
                    &code.mint_nonce_hash,
                    &code.label,
                    &code.expires_at,
                ],
            )
            .await?;
        if inserted == 0 {
            return Ok(MintAccountDeviceCodeOutcome::NonceReused);
        }
        tx.commit().await?;
        Ok(MintAccountDeviceCodeOutcome::Minted { code_id })
    }

    pub(super) async fn device_code_status(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        code_id: Uuid,
    ) -> Result<Option<AccountDeviceCodeStatus>, DatabaseError> {
        self.ensure_trace_tenant(tenant_id).await?;
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let row = tx
            .query_opt(
                "SELECT expires_at <= now(), redeemed_device_key_id
                   FROM trace_account_device_codes
                  WHERE tenant_id = trace_current_tenant_id()
                    AND account_id = $1
                    AND code_id = $2",
                &[&account_id, &code_id],
            )
            .await?;
        tx.commit().await?;
        Ok(row.map(|row| AccountDeviceCodeStatus {
            expired: row.get(0),
            redeemed_device_key_id: row.get(1),
        }))
    }

    pub(super) async fn redeem_device_code(
        &self,
        tenant_id: &str,
        code_hash: &str,
        device_public_key: &[u8; 32],
        session: NewSession<'_>,
        grant_template: Option<&NearAiSignupPolicyTemplate>,
    ) -> Result<DeviceSignInOutcome, DatabaseError> {
        // No `ensure_trace_tenant`: this path is unauthenticated and the
        // tenant is the caller's own text, so creating it here would let
        // anyone commit tenant rows. A tenant with no code row finds nothing.
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // The row lock makes a second redeem of the same code wait, then find
        // it spent.
        let Some(code) = tx
            .query_opt(
                "SELECT code_id, account_id, minted_by_device_key_id, label
                   FROM trace_account_device_codes
                  WHERE tenant_id = trace_current_tenant_id()
                    AND code_hash = $1
                    AND redeemed_at IS NULL
                    AND expires_at > now()
                    FOR UPDATE",
                &[&code_hash],
            )
            .await?
        else {
            return Ok(DeviceSignInOutcome::NotFound);
        };
        let code_id: Uuid = code.get(0);
        let account: Uuid = code.get(1);
        let minter: String = code.get(2);
        let label: Option<String> = code.get(3);
        // The new device joins under the minting device's anchor, and only
        // while that device is still active on an open account: revoking the
        // laptop voids the codes it minted. FOR SHARE: a revoke of the
        // minter holds its key row until it commits, so this read waits and
        // then sees it revoked, rather than linking a device under a laptop
        // revoked mid-redeem.
        let Some(anchor) = tx
            .query_opt(
                "SELECT n.anchor_hash FROM trace_near_provisioned_devices n
                   JOIN device_keys d
                     ON d.tenant_id = n.tenant_id AND d.device_key_id = n.device_key_id
                   JOIN trace_accounts a
                     ON a.tenant_id = n.tenant_id AND a.account_id = n.account_id
                  WHERE n.tenant_id = trace_current_tenant_id()
                    AND n.account_id = $1
                    AND n.device_key_id = $2
                    AND d.revoked_at IS NULL
                    AND a.closed_at IS NULL
                    FOR SHARE OF d",
                &[&account, &minter],
            )
            .await?
        else {
            return Ok(DeviceSignInOutcome::NotFound);
        };
        let anchor_hash: String = anchor.get(0);
        let (device, principal) = match attach_near_ai_login_device(
            &tx,
            tenant_id,
            &account,
            device_public_key,
            &anchor_hash,
            &session,
            grant_template,
        )
        .await
        {
            Ok(linked) => linked,
            Err(error) if is_device_key_conflict(&error) => {
                return Ok(DeviceSignInOutcome::DeviceKeyElsewhere);
            }
            Err(error) => return Err(error),
        };
        tx.execute(
            "UPDATE trace_account_device_codes
                SET redeemed_at = now(), redeemed_device_key_id = $2
              WHERE tenant_id = trace_current_tenant_id() AND code_id = $1",
            &[&code_id, &device],
        )
        .await?;
        if let Some(label) = label.as_deref() {
            tx.execute(
                "UPDATE device_keys SET label = $2
                  WHERE tenant_id = trace_current_tenant_id()
                    AND device_key_id = $1
                    AND label IS NULL",
                &[&device, &label],
            )
            .await?;
        }
        tx.execute(
            "INSERT INTO trace_account_audit(tenant_id,action,actor_ref,outcome,safe_metadata)
             VALUES($1,'account_device_code_redeemed',$2,'success','{}'::jsonb)",
            &[&tenant_id, &principal],
        )
        .await?;
        tx.commit().await?;
        Ok(DeviceSignInOutcome::SignedIn(ProvisionedNearAccount {
            tenant_id: tenant_id.to_string(),
            account_id: account,
            device_key_id: device,
            anchor_hash,
        }))
    }

    pub(super) async fn renew_session_by_device_key(
        &self,
        tenant_id: &str,
        device_public_key: &[u8; 32],
        nonce_hash: &str,
        session: NewSession<'_>,
        min_interval: chrono::Duration,
    ) -> Result<DeviceSignInOutcome, DatabaseError> {
        use base64::Engine;
        let device = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
            device_public_key,
        );
        let public_key = base64::engine::general_purpose::STANDARD.encode(device_public_key);
        // No `ensure_trace_tenant`, as in redeem: the tenant is the caller's
        // own text, and a tenant with no linked device finds nothing.
        let mut client = self.trace_pool().get().await.map_err(DatabaseError::from)?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // Eligible: an active key, recorded with this public key, provisioned
        // to an open account through a code that account minted, its
        // principal still linked.
        let Some(row) = tx
            .query_opt(
                "SELECT n.account_id, n.anchor_hash
                   FROM trace_near_provisioned_devices n
                   JOIN device_keys d
                     ON d.tenant_id = n.tenant_id AND d.device_key_id = n.device_key_id
                   JOIN trace_accounts a
                     ON a.tenant_id = n.tenant_id AND a.account_id = n.account_id
                   JOIN trace_account_principals p
                     ON p.tenant_id = n.tenant_id AND p.principal_ref = n.principal_ref
                    AND p.account_id = n.account_id
                  WHERE n.tenant_id = trace_current_tenant_id()
                    AND d.device_key_id = $1
                    AND d.public_key = $2
                    AND d.revoked_at IS NULL
                    AND a.closed_at IS NULL
                    AND p.unlinked_at IS NULL
                    AND EXISTS (SELECT 1 FROM trace_account_device_codes c
                                 WHERE c.tenant_id = n.tenant_id
                                   AND c.account_id = n.account_id
                                   AND c.redeemed_device_key_id = d.device_key_id)
                    FOR SHARE OF d",
                &[&device, &public_key],
            )
            .await?
        else {
            return Ok(DeviceSignInOutcome::NotFound);
        };
        let account: Uuid = row.get(0);
        let anchor_hash: String = row.get(1);
        let lock_key = format!("device-session-renew:{tenant_id}:{device}");
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1, 0))",
            &[&lock_key],
        )
        .await?;
        let recent = tx
            .query_opt(
                "SELECT 1 FROM trace_device_session_renewals
                  WHERE tenant_id = trace_current_tenant_id()
                    AND device_key_id = $1
                    AND created_at > now() - make_interval(secs => $2)",
                &[&device, &(min_interval.num_seconds() as f64)],
            )
            .await?;
        if recent.is_some() {
            return Ok(DeviceSignInOutcome::TooSoon);
        }
        let inserted = tx
            .execute(
                "INSERT INTO trace_device_session_renewals(tenant_id, nonce_hash, device_key_id)
                 VALUES (trace_current_tenant_id(), $1, $2)
                 ON CONFLICT DO NOTHING",
                &[&nonce_hash, &device],
            )
            .await?;
        if inserted == 0 {
            return Ok(DeviceSignInOutcome::NonceReused);
        }
        tx.execute(
            "INSERT INTO trace_sessions(tenant_id,session_id,account_id,token_hash,client_kind,expires_at,device_key_id)
             VALUES($1,$2,$3,$4,'native',$5,$6)",
            &[
                &tenant_id,
                &Uuid::new_v4(),
                &account,
                &session.token_hash,
                &session.expires_at,
                &device,
            ],
        )
        .await?;
        let principal = super::onboarding_device_principal_ref(tenant_id, &device);
        tx.execute(
            "INSERT INTO trace_account_audit(tenant_id,action,actor_ref,outcome,safe_metadata)
             VALUES($1,'account_device_session_renewed',$2,'success','{}'::jsonb)",
            &[&tenant_id, &principal],
        )
        .await?;
        tx.commit().await?;
        Ok(DeviceSignInOutcome::SignedIn(ProvisionedNearAccount {
            tenant_id: tenant_id.to_string(),
            account_id: account,
            device_key_id: device,
            anchor_hash,
        }))
    }
}
