// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::account_onboarding::{
    NativeProvisioningPending, ProvisionedNearAccount, VerifiedNearProvisioning,
};
use crate::db::NewSession;
use crate::near_account_identity::{NearAccountIdentity, random_near_tenant_id};
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

    /// Map a blind index to the tenant that already holds it, with no tenant
    /// context.
    ///
    /// The lookup must happen before a tenant exists to scope it to, because
    /// under the salted scheme the tenant is no longer a function of the
    /// account: a returning contributor is recognised by their index and by
    /// nothing else. This runs on the narrow `trace_login_resolver` pool, the
    /// same role V30 introduced for the unauthenticated redeem path, and is
    /// safe without a tenant predicate for the same reason: `anchor_hash` is
    /// globally UNIQUE, so at most one row exists across all tenants. The
    /// caller re-enters an RLS-scoped transaction on the resolved tenant before
    /// any write. Do not widen this role's grant to a non-unique column.
    async fn near_anchor_tenant(&self, anchor_hash: &str) -> Result<Option<String>, DatabaseError> {
        let pool = self.login_resolver_pool.as_ref().ok_or_else(|| {
            DatabaseError::Pool("missing-control: login-resolver-pool-unconfigured".into())
        })?;
        let client = pool.get().await?;
        let row = client
            .query_opt(
                "SELECT tenant_id FROM trace_near_account_anchors WHERE anchor_hash = $1",
                &[&anchor_hash],
            )
            .await?;
        Ok(row.map(|r| r.get::<_, String>(0)))
    }

    pub(super) async fn near_provision(
        &self,
        proof: VerifiedNearProvisioning,
        session: NewSession<'_>,
        identity: &NearAccountIdentity,
    ) -> Result<ProvisionedNearAccount, DatabaseError> {
        if session.client_kind != crate::account_native_auth::NATIVE_SESSION_CLIENT_KIND {
            return Err(refused());
        }
        // The anchor is now HMAC(pepper, network || account_name) and the sealed
        // name is the only copy of that name we keep. Both come out of one call
        // so the seal is bound to the index it is stored beside; see
        // `NearAccountIdentity::context`.
        let (anchor_hash, sealed_account_name) = identity
            .seal(proof.network(), proof.account_id())
            .map_err(|_| refused())?;
        let sealed_json = serde_json::to_value(&sealed_account_name).map_err(|_| refused())?;
        let pepper_ref = identity.pepper_ref_hash().to_string();
        let key_ref = identity.key_ref_hash();
        // A returning contributor keeps their existing tenant; a new one gets a
        // tenant drawn from the OS RNG that is a function of no public input.
        //
        // Two concurrent first-logins for the same account both resolve to no
        // tenant and both mint one. The advisory lock inside the transaction
        // serializes them on the anchor, so the loser's `ON CONFLICT DO NOTHING`
        // claims nothing, rolls its whole transaction back -- minted tenant
        // included -- and reports the anchor as taken. One retry then resolves
        // the winner's tenant and both requests land on the same account, which
        // is the behaviour the unsalted derivation got for free by giving both
        // racers the same tenant id. Bounded at one retry: a second failure to
        // resolve means something other than a race.
        let mut tenant = match self.near_anchor_tenant(&anchor_hash).await? {
            Some(existing) => existing,
            None => random_near_tenant_id(),
        };
        for attempt in 0..2 {
            match self
                .near_provision_in_tenant(
                    &proof,
                    &session,
                    &tenant,
                    &anchor_hash,
                    &sealed_json,
                    &pepper_ref,
                    &key_ref,
                )
                .await?
            {
                Some(provisioned) => return Ok(provisioned),
                None if attempt == 0 => {
                    tenant = self
                        .near_anchor_tenant(&anchor_hash)
                        .await?
                        .ok_or_else(refused)?;
                }
                None => return Err(refused()),
            }
        }
        Err(refused())
    }

    /// One provisioning attempt against a decided tenant.
    ///
    /// `Ok(None)` means the anchor was claimed by a different tenant while this
    /// transaction was waiting on the advisory lock; everything this attempt
    /// wrote, including the tenant row it minted, is rolled back by dropping the
    /// transaction unread. Every other failure is an error, not a retry.
    #[allow(clippy::too_many_arguments)]
    async fn near_provision_in_tenant(
        &self,
        proof: &VerifiedNearProvisioning,
        session: &NewSession<'_>,
        tenant: &str,
        anchor_hash: &str,
        sealed_json: &serde_json::Value,
        pepper_ref: &str,
        key_ref: &str,
    ) -> Result<Option<ProvisionedNearAccount>, DatabaseError> {
        let tenant = tenant.to_string();
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
            // ON CONFLICT on the globally UNIQUE anchor rather than a bare
            // INSERT: a bare insert would abort the transaction with an error
            // indistinguishable from a real failure, and this case is a race
            // with a legitimate concurrent first-login, not a fault.
            let claimed = tx.execute("INSERT INTO trace_near_account_anchors(tenant_id,anchor_hash,account_id,sealed_account_name,index_pepper_ref,account_name_key_ref) VALUES($1,$2,$3,$4,$5,$6) ON CONFLICT (anchor_hash) DO NOTHING", &[&tenant,&anchor_hash,&id,&sealed_json,&pepper_ref,&key_ref]).await?;
            if claimed == 0 {
                return Ok(None);
            }
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
        Ok(Some(ProvisionedNearAccount {
            tenant_id: tenant,
            account_id: account,
            device_key_id: device,
            anchor_hash: anchor_hash.to_string(),
        }))
    }

    /// Provision a contributor from a verified NEAR AI login (#836).
    ///
    /// The sibling of [`Self::near_provision`], and deliberately a sibling
    /// rather than a shared function with branches: the wallet path is
    /// unchanged by this work, and threading five flags through it to serve
    /// two identity systems would put the change inside the path it was
    /// supposed to leave alone.
    ///
    /// **The duplication is real and is the cost of that choice.** The race
    /// handling below -- resolve the tenant, attempt, retry once on a lost
    /// anchor claim -- is the subtle part and now exists twice. If the two
    /// drift, the login path is the one that will drift silently, because the
    /// wallet path has the older test coverage. Extracting the wrapper is the
    /// obvious follow-up; it was not done here because it edits the wallet
    /// path.
    pub(super) async fn near_ai_login_provision(
        &self,
        login: &crate::near_ai_login::VerifiedNearAiLogin,
        device_public_key: &[u8; 32],
        session: NewSession<'_>,
        identity: &NearAccountIdentity,
    ) -> Result<ProvisionedNearAccount, DatabaseError> {
        if session.client_kind != crate::account_native_auth::NATIVE_SESSION_CLIENT_KIND {
            return Err(refused());
        }
        // The anchor is HMAC(pepper, subject_id) under the *login* domain, and
        // the sealed subject is the only copy of that id we keep -- the same
        // shape the wallet path uses for an account name, so rotation reads
        // both rows identically.
        let anchor_hash = identity.login_index_label(login.subject_id());
        let sealed_subject = identity
            .seal_login_subject(login.subject_id())
            .map_err(|_| refused())?;
        let sealed_json = serde_json::to_value(&sealed_subject).map_err(|_| refused())?;
        let pepper_ref = identity.pepper_ref_hash().to_string();
        let key_ref = identity.key_ref_hash();
        let mut tenant = match self.near_anchor_tenant(&anchor_hash).await? {
            Some(existing) => existing,
            None => crate::near_account_identity::random_near_ai_tenant_id(),
        };
        for attempt in 0..2 {
            match self
                .near_ai_login_provision_in_tenant(
                    login,
                    device_public_key,
                    &session,
                    &tenant,
                    &anchor_hash,
                    &sealed_json,
                    &pepper_ref,
                    &key_ref,
                )
                .await?
            {
                Some(provisioned) => return Ok(provisioned),
                None if attempt == 0 => {
                    tenant = self
                        .near_anchor_tenant(&anchor_hash)
                        .await?
                        .ok_or_else(refused)?;
                }
                None => return Err(refused()),
            }
        }
        Err(refused())
    }

    /// One login provisioning attempt against a decided tenant.
    ///
    /// `Ok(None)` means the anchor was claimed by another tenant while this
    /// transaction waited on the advisory lock; everything written here,
    /// including the minted tenant, is rolled back by dropping the transaction.
    #[allow(clippy::too_many_arguments)]
    async fn near_ai_login_provision_in_tenant(
        &self,
        login: &crate::near_ai_login::VerifiedNearAiLogin,
        device_public_key: &[u8; 32],
        session: &NewSession<'_>,
        tenant: &str,
        anchor_hash: &str,
        sealed_json: &serde_json::Value,
        pepper_ref: &str,
        key_ref: &str,
    ) -> Result<Option<ProvisionedNearAccount>, DatabaseError> {
        let tenant = tenant.to_string();
        let device = trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(
            device_public_key,
        );
        let principal = super::onboarding_device_principal_ref(&tenant, &device);
        let public_key = base64::engine::general_purpose::STANDARD.encode(device_public_key);
        let provider = login.auth_provider().to_string();
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &tenant).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
            &[&anchor_hash],
        )
        .await?;
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
            // `identity_source` and `auth_provider` are what make this row
            // distinguishable from a wallet anchor in the table that decides
            // admission. The database enforces their pairing.
            let claimed = tx.execute("INSERT INTO trace_near_account_anchors(tenant_id,anchor_hash,account_id,sealed_account_name,index_pepper_ref,account_name_key_ref,identity_source,auth_provider) VALUES($1,$2,$3,$4,$5,$6,'near_ai_login',$7) ON CONFLICT (anchor_hash) DO NOTHING", &[&tenant,&anchor_hash,&id,&sealed_json,&pepper_ref,&key_ref,&provider]).await?;
            if claimed == 0 {
                return Ok(None);
            }
            id
        };
        // No `trace_near_identities` row: that table binds a wallet public key
        // to a NEAR account name, and this path has neither. A login proves an
        // account, not a key.
        tx.execute("INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,$3,NULL,'near_ai') ON CONFLICT(device_key_id) DO NOTHING", &[&device,&tenant,&public_key]).await?;
        if tx.query_opt("SELECT 1 FROM device_keys WHERE tenant_id=$1 AND device_key_id=$2 AND public_key=$3 AND onboarding_origin='near_ai' AND revoked_at IS NULL", &[&tenant,&device,&public_key]).await?.is_none() { return Err(refused()); }
        tx.execute("INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3) ON CONFLICT(tenant_id,principal_ref) DO NOTHING", &[&tenant,&account,&principal]).await?;
        if tx.query_opt("SELECT 1 FROM trace_account_principals WHERE tenant_id=$1 AND account_id=$2 AND principal_ref=$3 AND unlinked_at IS NULL", &[&tenant,&account,&principal]).await?.is_none() { return Err(refused()); }
        tx.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5) ON CONFLICT(tenant_id,principal_ref) DO NOTHING", &[&tenant,&principal,&account,&device,&anchor_hash]).await?;
        tx.execute("INSERT INTO trace_sessions(tenant_id,session_id,account_id,token_hash,client_kind,expires_at) VALUES($1,$2,$3,$4,'native',$5)", &[&tenant,&Uuid::new_v4(),&account,&session.token_hash,&session.expires_at]).await?;
        // Hash-only, like its wallet sibling: the audit row names the identity
        // system and says admission was not granted here. No subject, no
        // provider label that could narrow who this is, no token.
        tx.execute("INSERT INTO trace_account_audit(tenant_id,action,actor_ref,outcome,safe_metadata) VALUES($1,'near_ai_login_provisioned',$2,'success',$3)", &[&tenant,&principal,&serde_json::json!({"identity":"near_ai_login","admission":"not_granted"})]).await?;
        tx.commit().await?;
        Ok(Some(ProvisionedNearAccount {
            tenant_id: tenant,
            account_id: account,
            device_key_id: device,
            anchor_hash: anchor_hash.to_string(),
        }))
    }

    /// The provisioned admission anchor for one authenticated principal.
    ///
    /// Accepts either provisioning origin (#836): `near` is a wallet, `near_ai`
    /// a NEAR AI login. The two are separate identity systems and their anchors
    /// live in disjoint keyspaces, but both grant admission the same way, so
    /// this reads either. The caller's tenant prefix already decided which
    /// namespace the request is on, and a row is only ever written under the
    /// matching one.
    pub(super) async fn near_anchor_for_principal(
        &self,
        tenant: &str,
        principal: &str,
    ) -> Result<Option<String>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx.query_opt("SELECT n.anchor_hash FROM trace_near_provisioned_devices n JOIN device_keys d ON d.tenant_id=n.tenant_id AND d.device_key_id=n.device_key_id JOIN trace_account_principals p ON p.tenant_id=n.tenant_id AND p.account_id=n.account_id AND p.principal_ref=n.principal_ref JOIN trace_accounts a ON a.tenant_id=n.tenant_id AND a.account_id=n.account_id WHERE n.tenant_id=$1 AND n.principal_ref=$2 AND d.revoked_at IS NULL AND d.onboarding_origin IN ('near','near_ai') AND p.unlinked_at IS NULL AND a.closed_at IS NULL", &[&tenant,&principal]).await?;
        tx.commit().await?;
        Ok(row.map(|r| r.get(0)))
    }
}
