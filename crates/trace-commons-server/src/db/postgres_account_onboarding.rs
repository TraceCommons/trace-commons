// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::account_onboarding::{
    NativeProvisioningPending, ProvisionedNearAccount, VerifiedNearProvisioning,
};
use crate::db::NewSession;
use base64::Engine;

fn refused() -> DatabaseError {
    DatabaseError::Pool("near_provisioning_refused".into())
}

impl PgBackend {
    pub(super) async fn near_store_ceremony(
        &self,
        hash: &str,
        pending: NativeProvisioningPending,
        expires_at: i64,
    ) -> Result<(), DatabaseError> {
        let payload = serde_json::to_value(pending).map_err(|_| refused())?;
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.near_ceremony_hash',$1,true)",
            &[&hash],
        )
        .await?;
        tx.execute("INSERT INTO trace_near_provisioning_ceremonies(ceremony_hash,payload,expires_at) VALUES($1,$2,to_timestamp($3::double precision))", &[&hash,&payload,&(expires_at as f64)]).await?;
        tx.commit().await?;
        Ok(())
    }

    pub(super) async fn near_take_ceremony(
        &self,
        hash: &str,
    ) -> Result<Option<NativeProvisioningPending>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.near_ceremony_hash',$1,true)",
            &[&hash],
        )
        .await?;
        let row = tx.query_opt("DELETE FROM trace_near_provisioning_ceremonies WHERE ceremony_hash=$1 RETURNING payload, expires_at > clock_timestamp() AS live", &[&hash]).await?;
        tx.commit().await?;
        match row {
            Some(row) if row.get::<_, bool>("live") => Ok(Some(
                serde_json::from_value(row.get("payload")).map_err(|_| refused())?,
            )),
            _ => Ok(None),
        }
    }

    pub(super) async fn near_provision(
        &self,
        proof: VerifiedNearProvisioning,
        session: NewSession<'_>,
    ) -> Result<ProvisionedNearAccount, DatabaseError> {
        if session.client_kind != crate::account_native_auth::NATIVE_SESSION_CLIENT_KIND {
            return Err(refused());
        }
        let anchor_hash = format!("sha256:{}", hex::encode(proof.anchor_hash()));
        let tenant = format!("near-{}", hex::encode(proof.anchor_hash()));
        let device = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
            proof.device_public_key(),
        );
        let principal = super::onboarding_device_principal_ref(&tenant, &device);
        let public_key =
            base64::engine::general_purpose::STANDARD.encode(proof.device_public_key());
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &tenant).await?;
        // Serialize all key/device additions for one stable anchor. Hash collisions
        // merely serialize unrelated accounts; UNIQUE constraints remain decisive.
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
            &[&anchor_hash],
        )
        .await?;
        let live: bool = tx
            .query_one(
                "SELECT clock_timestamp() < to_timestamp($1::double precision)",
                &[&(proof.expires_at() as f64)],
            )
            .await?
            .get(0);
        if !live {
            return Err(refused());
        }
        tx.execute(
            "INSERT INTO trace_tenants(tenant_id) VALUES($1) ON CONFLICT DO NOTHING",
            &[&tenant],
        )
        .await?;
        let existing = tx.query_opt("SELECT a.account_id FROM trace_near_account_anchors n JOIN trace_accounts a USING(tenant_id,account_id) WHERE n.anchor_hash=$1 AND a.closed_at IS NULL", &[&anchor_hash]).await?;
        let account = if let Some(row) = existing {
            row.get::<_, Uuid>(0)
        } else {
            let id = Uuid::new_v4();
            tx.execute(
                "INSERT INTO trace_accounts(tenant_id,account_id) VALUES($1,$2)",
                &[&tenant, &id],
            )
            .await?;
            tx.execute("INSERT INTO trace_near_account_anchors(tenant_id,anchor_hash,account_id) VALUES($1,$2,$3)", &[&tenant,&anchor_hash,&id]).await?;
            id
        };
        // Never move an existing key from another account, revive revocations,
        // or create an invite/grant as a side effect of identity provisioning.
        tx.execute("INSERT INTO trace_near_identities(tenant_id,public_key,near_account_id,account_id) VALUES($1,$2,$3,$4) ON CONFLICT(public_key) DO NOTHING", &[&tenant,&proof.wallet_public_key(),&proof.account_id(),&account]).await?;
        if tx.query_opt("SELECT 1 FROM trace_near_identities WHERE tenant_id=$1 AND public_key=$2 AND account_id=$3 AND revoked_at IS NULL AND near_account_id=$4", &[&tenant,&proof.wallet_public_key(),&account,&proof.account_id()]).await?.is_none() { return Err(refused()); }
        tx.execute("INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,$3,NULL,'near') ON CONFLICT(device_key_id) DO NOTHING", &[&device,&tenant,&public_key]).await?;
        if tx.query_opt("SELECT 1 FROM device_keys WHERE tenant_id=$1 AND device_key_id=$2 AND public_key=$3 AND onboarding_origin='near' AND revoked_at IS NULL", &[&tenant,&device,&public_key]).await?.is_none() { return Err(refused()); }
        tx.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3) ON CONFLICT(tenant_id,principal_ref) DO NOTHING", &[&tenant,&account,&principal]).await?;
        if tx.query_opt("SELECT 1 FROM trace_account_principals WHERE tenant_id=$1 AND account_id=$2 AND principal_ref=$3 AND unlinked_at IS NULL", &[&tenant,&account,&principal]).await?.is_none() { return Err(refused()); }
        tx.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5) ON CONFLICT(tenant_id,principal_ref) DO NOTHING", &[&tenant,&principal,&account,&device,&anchor_hash]).await?;
        tx.execute("INSERT INTO trace_sessions(tenant_id,session_id,account_id,token_hash,client_kind,expires_at) VALUES($1,$2,$3,$4,'native',$5)", &[&tenant,&Uuid::new_v4(),&account,&session.token_hash,&session.expires_at]).await?;
        tx.execute("INSERT INTO trace_account_audit(tenant_id,action,actor_ref,outcome,safe_metadata) VALUES($1,'near_account_provisioned',$2,'success',$3)", &[&tenant,&principal,&serde_json::json!({"identity":"near","admission":"not_granted"})]).await?;
        tx.commit().await?;
        Ok(ProvisionedNearAccount {
            tenant_id: tenant,
            account_id: account,
            device_key_id: device,
            anchor_hash,
        })
    }

    pub(super) async fn near_anchor_for_principal(
        &self,
        tenant: &str,
        principal: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx.query_opt("SELECT n.anchor_hash FROM trace_near_provisioned_devices n JOIN device_keys d ON d.tenant_id=n.tenant_id AND d.device_key_id=n.device_key_id JOIN trace_account_principals p ON p.tenant_id=n.tenant_id AND p.account_id=n.account_id AND p.principal_ref=n.principal_ref JOIN trace_accounts a ON a.tenant_id=n.tenant_id AND a.account_id=n.account_id WHERE n.tenant_id=$1 AND n.principal_ref=$2 AND d.revoked_at IS NULL AND d.onboarding_origin='near' AND p.unlinked_at IS NULL AND a.closed_at IS NULL", &[&tenant,&principal]).await?;
        tx.commit().await?;
        Ok(row.map(|r| r.get(0)))
    }
}
