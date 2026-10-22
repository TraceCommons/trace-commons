// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;
use crate::account_onboarding::{
    NativeProvisioningPending, ProvisionedNearAccount, VerifiedNearProvisioning,
};
// The device key is already registered under a different tenant. Public, in
// `account_onboarding`, because bind shows this refusal to its caller.
use crate::account_onboarding::DEVICE_KEY_REGISTERED_ELSEWHERE;
use crate::db::NewSession;
use crate::near_account_identity::{NearAccountIdentity, random_near_tenant_id};
use base64::Engine;

fn refused() -> DatabaseError {
    DatabaseError::Pool("near_provisioning_refused".into())
}

/// The wallet public key is already bound to a different tenant or account.
const WALLET_KEY_REGISTERED_ELSEWHERE: &str = "near_provisioning_wallet_key_registered_elsewhere";

/// The device's principal is already linked to a different account in this
/// tenant.
const DEVICE_PRINCIPAL_BOUND_TO_OTHER_ACCOUNT: &str =
    "near_provisioning_device_bound_to_other_account";

/// Bind (Z2 S3) reached an account that is not an open, `unbound`
/// passkey-origin account: legacy, already bound, or closed.
const BIND_ACCOUNT_NOT_UNBOUND: &str = "near_ai_bind_account_not_unbound";

fn named_refusal(label: &str) -> DatabaseError {
    DatabaseError::Pool(label.into())
}

/// Register a provisioned device key under `tenant`, or accept the identical
/// live row a previous attempt left there.
///
/// The insert is `ON CONFLICT DO NOTHING` so a retry is not an error, which
/// means a no-op is ambiguous on its own: it is either our own earlier row or a
/// row under another tenant, which forced RLS hides from this transaction. The
/// re-read is scoped to `tenant` explicitly, so a key it cannot find here is
/// held elsewhere whatever role the pool connects as.
async fn claim_provisioned_device_key(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    device: &str,
    public_key: &str,
    origin: &str,
) -> Result<(), DatabaseError> {
    let inserted = tx
        .execute(
            "INSERT INTO device_keys(device_key_id,tenant_id,public_key,invite_subject_hash,onboarding_origin) VALUES($1,$2,$3,NULL,$4) ON CONFLICT(device_key_id) DO NOTHING",
            &[&device, &tenant, &public_key, &origin],
        )
        .await?;
    if inserted == 1 {
        return Ok(());
    }
    let Some(row) = tx
        .query_opt(
            "SELECT public_key, onboarding_origin, revoked_at IS NULL FROM device_keys WHERE tenant_id=$1 AND device_key_id=$2",
            &[&tenant, &device],
        )
        .await?
    else {
        return Err(named_refusal(DEVICE_KEY_REGISTERED_ELSEWHERE));
    };
    // Same tenant, but revoked or recorded differently: never revive or
    // rewrite it from here.
    let same = row.get::<_, String>(0) == public_key
        && row.get::<_, String>(1) == origin
        && row.get::<_, bool>(2);
    if same { Ok(()) } else { Err(refused()) }
}

/// Bind the proved wallet key to `account`, or accept the identical live
/// binding a previous attempt left. `public_key` is globally unique, so a key
/// this tenant cannot see, or one bound to another account here, is refused by
/// name and never moved.
async fn claim_wallet_identity(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: &Uuid,
    wallet_public_key: &str,
    near_account_id: &str,
) -> Result<(), DatabaseError> {
    let inserted = tx
        .execute(
            "INSERT INTO trace_near_identities(tenant_id,public_key,near_account_id,account_id) VALUES($1,$2,$3,$4) ON CONFLICT(public_key) DO NOTHING",
            &[&tenant, &wallet_public_key, &near_account_id, account],
        )
        .await?;
    if inserted == 1 {
        return Ok(());
    }
    let Some(row) = tx
        .query_opt(
            "SELECT account_id, near_account_id, revoked_at IS NULL FROM trace_near_identities WHERE tenant_id=$1 AND public_key=$2",
            &[&tenant, &wallet_public_key],
        )
        .await?
    else {
        return Err(named_refusal(WALLET_KEY_REGISTERED_ELSEWHERE));
    };
    if row.get::<_, Uuid>(0) != *account {
        return Err(named_refusal(WALLET_KEY_REGISTERED_ELSEWHERE));
    }
    if row.get::<_, String>(1) == near_account_id && row.get::<_, bool>(2) {
        Ok(())
    } else {
        Err(refused())
    }
}

/// Link the device principal to `account`, or accept the identical live link
/// a previous attempt left. A link to a different account is refused by name
/// rather than kept silently.
async fn link_provisioned_principal(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: &Uuid,
    principal: &str,
) -> Result<(), DatabaseError> {
    let inserted = tx
        .execute(
            "INSERT INTO trace_account_principals(tenant_id,account_id,principal_ref) VALUES($1,$2,$3) ON CONFLICT(tenant_id,principal_ref) DO NOTHING",
            &[&tenant, account, &principal],
        )
        .await?;
    if inserted == 1 {
        return Ok(());
    }
    let row = tx
        .query_opt(
            "SELECT account_id, unlinked_at IS NULL FROM trace_account_principals WHERE tenant_id=$1 AND principal_ref=$2",
            &[&tenant, &principal],
        )
        .await?
        .ok_or_else(refused)?;
    if row.get::<_, Uuid>(0) != *account {
        return Err(named_refusal(DEVICE_PRINCIPAL_BOUND_TO_OTHER_ACCOUNT));
    }
    if row.get::<_, bool>(1) {
        Ok(())
    } else {
        Err(refused())
    }
}

/// The anchor inputs a verified NEAR AI login produces, computed once for
/// provisioning and bind alike.
///
/// The anchor is HMAC(pepper, subject_id) under the *login* domain, and the
/// sealed subject is the only copy of that id we keep -- the same shape the
/// wallet path uses for an account name, so rotation reads both rows
/// identically.
struct NearAiAnchorMaterial {
    anchor_hash: String,
    sealed_json: serde_json::Value,
    pepper_ref: String,
    key_ref: String,
}

impl NearAiAnchorMaterial {
    fn for_login(
        login: &crate::near_ai_login::VerifiedNearAiLogin,
        identity: &NearAccountIdentity,
    ) -> Result<Self, DatabaseError> {
        let anchor_hash = identity.login_index_label(login.subject_id());
        let sealed_subject = identity
            .seal_login_subject(login.subject_id())
            .map_err(|_| refused())?;
        let sealed_json = serde_json::to_value(&sealed_subject).map_err(|_| refused())?;
        Ok(Self {
            anchor_hash,
            sealed_json,
            pepper_ref: identity.pepper_ref_hash().to_string(),
            key_ref: identity.key_ref_hash(),
        })
    }
}

/// Everything a NEAR AI login writes once its account is decided: the device
/// key (`near_ai` origin), the device principal linked to `account`, the
/// provisioned-device row naming `anchor_hash`, and a fresh native session.
///
/// Shared by provisioning (#836), where the account was just found or minted,
/// and by bind (Z2 S3), where it is the passkey account that started the
/// ceremony. One copy, so the two cannot drift in what a linked device is.
/// No `trace_near_identities` row: that table binds a wallet public key to a
/// NEAR account name, and this path has neither. A login proves an account,
/// not a key. Returns the device key id and the principal.
async fn attach_near_ai_login_device(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: &Uuid,
    device_public_key: &[u8; 32],
    anchor_hash: &str,
    session: &NewSession<'_>,
) -> Result<(String, String), DatabaseError> {
    let device =
        trace_commons_protocol::onboarding::device_key_id_from_public_key_bytes(device_public_key);
    let principal = super::onboarding_device_principal_ref(tenant, &device);
    let public_key = base64::engine::general_purpose::STANDARD.encode(device_public_key);
    claim_provisioned_device_key(tx, tenant, &device, &public_key, "near_ai").await?;
    link_provisioned_principal(tx, tenant, account, &principal).await?;
    tx.execute("INSERT INTO trace_near_provisioned_devices(tenant_id,principal_ref,account_id,device_key_id,anchor_hash) VALUES($1,$2,$3,$4,$5) ON CONFLICT(tenant_id,principal_ref) DO NOTHING", &[&tenant,&principal,account,&device,&anchor_hash]).await?;
    tx.execute("INSERT INTO trace_sessions(tenant_id,session_id,account_id,token_hash,client_kind,expires_at) VALUES($1,$2,$3,$4,'native',$5)", &[&tenant,&Uuid::new_v4(),account,&session.token_hash,&session.expires_at]).await?;
    Ok((device, principal))
}

impl PgBackend {
    pub(super) async fn near_store_ceremony(
        &self,
        hash: &str,
        pending: NativeProvisioningPending,
        expires_at: i64,
    ) -> Result<(), DatabaseError> {
        self.store_ceremony_payload(hash, &pending, expires_at)
            .await
    }

    /// Store one ceremony, whatever its shape.
    ///
    /// `trace_near_provisioning_ceremonies` is a ceremony handle and an opaque
    /// `payload`, so the row mechanics -- the GUC that the RLS policy reads,
    /// the expiry, the single-use delete on take -- are the same for every
    /// ceremony and are written once here. Two enrollment ceremonies with two
    /// copies of the RLS handshake is a rule that eventually diverges, and the
    /// half that diverges silently is whichever has the thinner tests.
    async fn store_ceremony_payload<T: serde::Serialize>(
        &self,
        hash: &str,
        pending: &T,
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
        self.take_ceremony_payload(hash).await
    }

    /// Store a NEAR AI login ceremony (#836) in the same table.
    pub(super) async fn near_ai_login_store_ceremony(
        &self,
        hash: &str,
        pending: &crate::account_onboarding::NearAiLoginPending,
        expires_at: i64,
    ) -> Result<(), DatabaseError> {
        self.store_ceremony_payload(hash, pending, expires_at).await
    }

    /// Consume a NEAR AI login ceremony. Single use and expiry-checked, by the
    /// same delete-and-return the wallet ceremony uses.
    pub(super) async fn near_ai_login_take_ceremony(
        &self,
        hash: &str,
    ) -> Result<Option<crate::account_onboarding::NearAiLoginPending>, DatabaseError> {
        self.take_ceremony_payload(hash).await
    }

    /// Consume one ceremony, whatever its shape. See
    /// [`Self::store_ceremony_payload`].
    ///
    /// The delete and the liveness test are one statement on purpose: a
    /// select-then-delete would let two racing finishes both read a live
    /// ceremony, and a ceremony is single use.
    async fn take_ceremony_payload<T: serde::de::DeserializeOwned>(
        &self,
        hash: &str,
    ) -> Result<Option<T>, DatabaseError> {
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
        self.provision_against_anchor(&anchor_hash, random_near_tenant_id, |tenant| {
            self.near_provision_in_tenant(
                &proof,
                &session,
                tenant,
                &anchor_hash,
                &sealed_json,
                &pepper_ref,
                &key_ref,
            )
        })
        .await
    }

    /// Resolve the tenant for an anchor, attempt a provisioning write, and
    /// retry once if the anchor was claimed underneath.
    ///
    /// **The race this handles.** Two concurrent first-logins for the same
    /// account both resolve to no tenant and both mint one. The advisory lock
    /// inside the write serializes them on the anchor, so the loser's
    /// `ON CONFLICT DO NOTHING` claims nothing, rolls its whole transaction
    /// back -- minted tenant included -- and reports the anchor as taken. One
    /// retry then resolves the winner's tenant and both requests land on the
    /// same account, which is the behaviour the pre-V61 derivation got for free
    /// by giving both racers the same tenant id.
    ///
    /// Bounded at one retry: a second failure to resolve means something other
    /// than a race.
    ///
    /// Shared by both enrollment ceremonies (#836). The wallet and the login
    /// write different rows, but they race identically, and this is the subtle
    /// half -- a second copy would be the one to drift, and it would drift
    /// silently because a race is not what a test reaches for first.
    ///
    /// Bind (Z2 S3) goes through it too, with "mint" meaning "the passkey
    /// account's own tenant": the race is the same race.
    async fn provision_against_anchor<T, M, F, Fut>(
        &self,
        anchor_hash: &str,
        mint_tenant: M,
        attempt: F,
    ) -> Result<T, DatabaseError>
    where
        M: FnOnce() -> String,
        F: Fn(String) -> Fut,
        Fut: std::future::Future<Output = Result<Option<T>, DatabaseError>>,
    {
        // A returning contributor keeps their existing tenant; a new one gets a
        // tenant drawn from the OS RNG that is a function of no public input.
        let mut tenant = match self.near_anchor_tenant(anchor_hash).await? {
            Some(existing) => existing,
            None => mint_tenant(),
        };
        for round in 0..2 {
            match attempt(tenant.clone()).await? {
                Some(provisioned) => return Ok(provisioned),
                None if round == 0 => {
                    tenant = self
                        .near_anchor_tenant(anchor_hash)
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
        tenant: String,
        anchor_hash: &str,
        sealed_json: &serde_json::Value,
        pepper_ref: &str,
        key_ref: &str,
    ) -> Result<Option<ProvisionedNearAccount>, DatabaseError> {
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
        claim_wallet_identity(
            &tx,
            &tenant,
            &account,
            proof.wallet_public_key(),
            proof.account_id(),
        )
        .await?;
        claim_provisioned_device_key(&tx, &tenant, &device, &public_key, "near").await?;
        link_provisioned_principal(&tx, &tenant, &account, &principal).await?;
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
    /// The race handling is **not** duplicated: both ceremonies go through
    /// [`Self::provision_against_anchor`]. It was duplicated in the PR that
    /// introduced this path and extracted here, in the PR that first makes the
    /// path reachable -- extracting it while the login path was still inert
    /// would have edited a live, deployed path for the benefit of code that
    /// did not run.
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
        let NearAiAnchorMaterial {
            anchor_hash,
            sealed_json,
            pepper_ref,
            key_ref,
        } = NearAiAnchorMaterial::for_login(login, identity)?;
        self.provision_against_anchor(
            &anchor_hash,
            crate::near_account_identity::random_near_ai_tenant_id,
            |tenant| {
                self.near_ai_login_provision_in_tenant(
                    login,
                    device_public_key,
                    &session,
                    tenant,
                    &anchor_hash,
                    &sealed_json,
                    &pepper_ref,
                    &key_ref,
                )
            },
        )
        .await
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
        tenant: String,
        anchor_hash: &str,
        sealed_json: &serde_json::Value,
        pepper_ref: &str,
        key_ref: &str,
    ) -> Result<Option<ProvisionedNearAccount>, DatabaseError> {
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
        let (device, principal) = attach_near_ai_login_device(
            &tx,
            &tenant,
            &account,
            device_public_key,
            anchor_hash,
            session,
        )
        .await?;
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

    /// Store a NEAR AI bind ceremony (Z2 S3) in the same ceremony table.
    pub(super) async fn near_ai_bind_store_ceremony(
        &self,
        hash: &str,
        pending: &crate::account_onboarding::NearAiBindPending,
        expires_at: i64,
    ) -> Result<(), DatabaseError> {
        self.store_ceremony_payload(hash, pending, expires_at).await
    }

    /// Consume a NEAR AI bind ceremony. The row is deleted before it is
    /// parsed, so a provisioning or wallet row presented here is consumed and
    /// refused (`NearAiBindPending` is `deny_unknown_fields` and needs a
    /// `purpose`), never read as a bind.
    pub(super) async fn near_ai_bind_take_ceremony(
        &self,
        hash: &str,
    ) -> Result<Option<crate::account_onboarding::NearAiBindPending>, DatabaseError> {
        self.take_ceremony_payload(hash).await
    }

    /// Bind a verified NEAR AI login to the unbound passkey account
    /// `(tenant, account)` (Z2 native passkey identity, slice S3).
    ///
    /// The anchor decides, through the same [`Self::provision_against_anchor`]
    /// race handling provisioning uses, with the passkey account's own tenant in
    /// the place of a freshly minted one:
    ///
    /// - **The anchor resolves to no tenant, or to this one: bind in place.**
    ///   One transaction in this tenant flips the binding row `unbound` ->
    ///   `bound`, claims the anchor for this account, and writes the device,
    ///   principal, provisioned-device row, session and audit row. If the
    ///   globally UNIQUE anchor was claimed meanwhile, the transaction rolls
    ///   back whole and the retry resolves the winner's tenant.
    /// - **The anchor resolves to another tenant (account X): refuse.** X is
    ///   provisioned by the very function the unauthenticated provisioning
    ///   uses, in X's tenant only, and then this account is closed in its own
    ///   tenant only. No transaction touches both tenants, and nothing of this
    ///   account moves to X. (The cross-tenant fold is the deferred S6.)
    pub(super) async fn near_ai_login_bind(
        &self,
        tenant: &str,
        account: Uuid,
        login: &crate::near_ai_login::VerifiedNearAiLogin,
        device_public_key: &[u8; 32],
        session: NewSession<'_>,
        identity: &NearAccountIdentity,
    ) -> Result<crate::account_onboarding::NearAiBindOutcome, DatabaseError> {
        if session.client_kind != crate::account_native_auth::NATIVE_SESSION_CLIENT_KIND {
            return Err(refused());
        }
        let material = NearAiAnchorMaterial::for_login(login, identity)?;
        let own = tenant.to_string();
        let (own_ref, material_ref, session_ref) = (&own, &material, &session);
        self.provision_against_anchor(
            &material.anchor_hash,
            || own.clone(),
            |resolved| async move {
                if resolved == *own_ref {
                    self.near_ai_bind_in_place(
                        own_ref,
                        &account,
                        login,
                        device_public_key,
                        session_ref,
                        material_ref,
                    )
                    .await
                } else {
                    self.near_ai_bind_refuse_existing(
                        own_ref,
                        &account,
                        resolved,
                        login,
                        device_public_key,
                        session_ref,
                        material_ref,
                    )
                    .await
                }
            },
        )
        .await
    }

    /// The in-place bind: one transaction in the passkey account's tenant.
    ///
    /// `Ok(None)` means the anchor was claimed by another tenant while this
    /// transaction waited on the advisory lock; dropping the transaction rolls
    /// back the state flip and everything else, so the account is exactly as
    /// unbound as before.
    async fn near_ai_bind_in_place(
        &self,
        tenant: &str,
        account: &Uuid,
        login: &crate::near_ai_login::VerifiedNearAiLogin,
        device_public_key: &[u8; 32],
        session: &NewSession<'_>,
        material: &NearAiAnchorMaterial,
    ) -> Result<Option<crate::account_onboarding::NearAiBindOutcome>, DatabaseError> {
        let provider = login.auth_provider().to_string();
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
            &[&material.anchor_hash],
        )
        .await?;
        // The state flip is also the guard: it matches only an open account
        // whose row is still `unbound`, and its row lock serializes two binds
        // of the same account. A legacy account (no row), a bound one, or a
        // closed one flips nothing and is refused before anything is written.
        let flipped = tx
            .execute(
                "UPDATE trace_account_bindings b SET state = 'bound', bound_at = now()
                  WHERE b.tenant_id = trace_current_tenant_id() AND b.account_id = $1
                    AND b.state = 'unbound'
                    AND EXISTS (SELECT 1 FROM trace_accounts a
                                 WHERE a.tenant_id = b.tenant_id AND a.account_id = b.account_id
                                   AND a.closed_at IS NULL)",
                &[account],
            )
            .await?;
        if flipped != 1 {
            return Err(named_refusal(BIND_ACCOUNT_NOT_UNBOUND));
        }
        let claimed = tx.execute("INSERT INTO trace_near_account_anchors(tenant_id,anchor_hash,account_id,sealed_account_name,index_pepper_ref,account_name_key_ref,identity_source,auth_provider) VALUES(trace_current_tenant_id(),$1,$2,$3,$4,$5,'near_ai_login',$6) ON CONFLICT (anchor_hash) DO NOTHING", &[&material.anchor_hash,account,&material.sealed_json,&material.pepper_ref,&material.key_ref,&provider]).await?;
        if claimed == 0 {
            return Ok(None);
        }
        let (device, _principal) = attach_near_ai_login_device(
            &tx,
            tenant,
            account,
            device_public_key,
            &material.anchor_hash,
            session,
        )
        .await?;
        let actor = crate::account_session::account_actor_ref(
            &crate::account_session::AccountId::from_uuid(*account),
        );
        tx.execute("INSERT INTO trace_account_audit(tenant_id,action,actor_ref,outcome,safe_metadata) VALUES(trace_current_tenant_id(),'account_bound',$1,'success',$2)", &[&actor,&serde_json::json!({"identity":"near_ai_login"})]).await?;
        tx.commit().await?;
        Ok(Some(crate::account_onboarding::NearAiBindOutcome::Bound(
            ProvisionedNearAccount {
                tenant_id: tenant.to_string(),
                account_id: *account,
                device_key_id: device,
                anchor_hash: material.anchor_hash.clone(),
            },
        )))
    }

    /// Enroll a further device into the `bound` account `(tenant, account)`: a
    /// second Mac signed in with that account's passkey, proving the near.ai
    /// login the account is already bound to.
    ///
    /// **One transaction, in the session's own tenant, and the comparison is
    /// inside it.** Under the anchor's advisory lock (the one provisioning and
    /// bind take, so no concurrent claim of this anchor interleaves), it reads
    /// the anchor row for this login under forced RLS and requires it to name
    /// this account, open and `bound`, with the binding row share-locked so a
    /// concurrent state change waits for this transaction. Only then does it
    /// write the device, principal, provisioned-device row, session and audit
    /// row, and commit. A mismatch returns before the first write; dropping
    /// the transaction leaves nothing behind.
    ///
    /// **It never resolves the anchor across tenants.** No login-resolver
    /// lookup, no mint, no claim: an anchor held by another account in another
    /// tenant is invisible here, exactly as an anchor nobody holds is, so the
    /// two are one refusal
    /// ([`crate::account_onboarding::NEAR_AI_ENROL_ACCOUNT_MISMATCH`]) and
    /// this path can never disclose, or create, another account.
    pub(super) async fn near_ai_login_enrol(
        &self,
        tenant: &str,
        account: Uuid,
        login: &crate::near_ai_login::VerifiedNearAiLogin,
        device_public_key: &[u8; 32],
        session: NewSession<'_>,
        identity: &NearAccountIdentity,
    ) -> Result<ProvisionedNearAccount, DatabaseError> {
        if session.client_kind != crate::account_native_auth::NATIVE_SESSION_CLIENT_KIND {
            return Err(refused());
        }
        let material = NearAiAnchorMaterial::for_login(login, identity)?;
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
            &[&material.anchor_hash],
        )
        .await?;
        let own = tx
            .query_opt(
                "SELECT 1 FROM trace_account_bindings b
                   JOIN trace_accounts a
                     ON a.tenant_id = b.tenant_id AND a.account_id = b.account_id
                   JOIN trace_near_account_anchors n
                     ON n.tenant_id = b.tenant_id AND n.account_id = b.account_id
                  WHERE b.tenant_id = trace_current_tenant_id() AND b.account_id = $1
                    AND b.state = 'bound' AND a.closed_at IS NULL
                    AND n.anchor_hash = $2 AND n.identity_source = 'near_ai_login'
                  FOR SHARE OF b",
                &[&account, &material.anchor_hash],
            )
            .await?;
        if own.is_none() {
            return Err(named_refusal(
                crate::account_onboarding::NEAR_AI_ENROL_ACCOUNT_MISMATCH,
            ));
        }
        let (device, _principal) = attach_near_ai_login_device(
            &tx,
            tenant,
            &account,
            device_public_key,
            &material.anchor_hash,
            &session,
        )
        .await?;
        let actor = crate::account_session::account_actor_ref(
            &crate::account_session::AccountId::from_uuid(account),
        );
        tx.execute("INSERT INTO trace_account_audit(tenant_id,action,actor_ref,outcome,safe_metadata) VALUES(trace_current_tenant_id(),'account_device_enrolled',$1,'success',$2)", &[&actor,&serde_json::json!({"identity":"near_ai_login"})]).await?;
        tx.commit().await?;
        Ok(ProvisionedNearAccount {
            tenant_id: tenant.to_string(),
            account_id: account,
            device_key_id: device,
            anchor_hash: material.anchor_hash,
        })
    }

    /// The existing-account branch: re-check the passkey account, provision X
    /// as provisioning would, then close the passkey account. Separate
    /// transactions, one per tenant, in that order: if the re-check fails,
    /// nothing is written anywhere; if X's fails, the passkey account is
    /// untouched; if the closure fails, the request fails and the passkey
    /// account stays `unbound` (the session minted for X is never returned, so
    /// its secret is gone).
    ///
    /// The re-check (#1135 review) is what stops a bind that lost a race --
    /// the passkey account closed or bound after the anchor resolved -- from
    /// provisioning X a device and a session nobody is handed. It narrows the
    /// window rather than closing it: holding the passkey account's row lock
    /// across X's transaction would close it, but would hold one pooled
    /// connection while waiting on a second, and the runtime pool has no wait
    /// timeout, so enough concurrent binds would starve it. What is left is a
    /// close landing between the re-check and the closure, which the closure's
    /// own `unbound` guard still refuses.
    #[allow(clippy::too_many_arguments)]
    async fn near_ai_bind_refuse_existing(
        &self,
        own_tenant: &str,
        account: &Uuid,
        existing_tenant: String,
        login: &crate::near_ai_login::VerifiedNearAiLogin,
        device_public_key: &[u8; 32],
        session: &NewSession<'_>,
        material: &NearAiAnchorMaterial,
    ) -> Result<Option<crate::account_onboarding::NearAiBindOutcome>, DatabaseError> {
        if !self
            .near_ai_bind_account_still_unbound(own_tenant, account)
            .await?
        {
            return Err(named_refusal(BIND_ACCOUNT_NOT_UNBOUND));
        }
        let Some(provisioned) = self
            .near_ai_login_provision_in_tenant(
                login,
                device_public_key,
                session,
                existing_tenant,
                &material.anchor_hash,
                &material.sealed_json,
                &material.pepper_ref,
                &material.key_ref,
            )
            .await?
        else {
            return Ok(None);
        };
        let (strong, binding_state) = self
            .near_ai_bind_existing_account_facts(&provisioned.tenant_id, &provisioned.account_id)
            .await?;
        let reason = if strong > 0 {
            crate::account_onboarding::BIND_REFUSED_ANCHOR_CLAIMED_STRONG
        } else {
            crate::account_onboarding::BIND_REFUSED_ANCHOR_CLAIMED
        };
        self.near_ai_bind_close_refused(own_tenant, account, reason)
            .await?;
        Ok(Some(
            crate::account_onboarding::NearAiBindOutcome::ExistingAccount {
                provisioned,
                binding_state,
            },
        ))
    }

    /// Whether the passkey account is still an open, `unbound` one: the same
    /// predicate the in-place state flip matches, read in its own tenant.
    async fn near_ai_bind_account_still_unbound(
        &self,
        tenant: &str,
        account: &Uuid,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                "SELECT 1 FROM trace_account_bindings b
                  WHERE b.tenant_id = trace_current_tenant_id() AND b.account_id = $1
                    AND b.state = 'unbound'
                    AND EXISTS (SELECT 1 FROM trace_accounts a
                                 WHERE a.tenant_id = b.tenant_id AND a.account_id = b.account_id
                                   AND a.closed_at IS NULL)",
                &[account],
            )
            .await?;
        tx.commit().await?;
        Ok(row.is_some())
    }

    /// X's active strong-authenticator count and binding state, read under X's
    /// own tenant. Read-only. The count is the shared
    /// [`super::active_strong_authenticator_count`] that
    /// `count_active_strong_authenticators` also reads, not a copy of its SQL.
    /// That method itself is not called: it first upserts the tenant row, a
    /// write this path neither needs nor holds a grant for.
    async fn near_ai_bind_existing_account_facts(
        &self,
        tenant: &str,
        account: &Uuid,
    ) -> Result<(i64, crate::account_binding::AccountBindingState), DatabaseError> {
        let strong = {
            let mut client = self.trace_pool().get().await?;
            let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
            let strong = super::active_strong_authenticator_count(&tx, account).await?;
            tx.commit().await?;
            strong
        };
        let binding = self
            .binding_state_for_account(tenant, *account)
            .await
            .map_err(|_| refused())?;
        Ok((strong, binding))
    }

    /// Close a passkey account that lost its bind to an existing account, in
    /// its own tenant: binding `closed`, every session and credential revoked,
    /// the account closed, and a label-only audit row. Refuses unless the row
    /// is still `unbound`, so it can never close a bound or legacy account.
    async fn near_ai_bind_close_refused(
        &self,
        tenant: &str,
        account: &Uuid,
        reason: &'static str,
    ) -> Result<(), DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let closed = tx
            .execute(
                "UPDATE trace_account_bindings SET state = 'closed'
                  WHERE tenant_id = trace_current_tenant_id() AND account_id = $1
                    AND state = 'unbound'",
                &[account],
            )
            .await?;
        if closed != 1 {
            return Err(named_refusal(BIND_ACCOUNT_NOT_UNBOUND));
        }
        tx.execute(
            "UPDATE trace_sessions SET revoked_at = now()
              WHERE tenant_id = trace_current_tenant_id() AND account_id = $1
                AND revoked_at IS NULL",
            &[account],
        )
        .await?;
        tx.execute(
            "UPDATE trace_webauthn_credentials SET revoked_at = now()
              WHERE tenant_id = trace_current_tenant_id() AND account_id = $1
                AND revoked_at IS NULL",
            &[account],
        )
        .await?;
        tx.execute(
            "UPDATE trace_accounts SET closed_at = now()
              WHERE tenant_id = trace_current_tenant_id() AND account_id = $1
                AND closed_at IS NULL",
            &[account],
        )
        .await?;
        let actor = crate::account_session::account_actor_ref(
            &crate::account_session::AccountId::from_uuid(*account),
        );
        tx.execute("INSERT INTO trace_account_audit(tenant_id,action,actor_ref,outcome,safe_metadata) VALUES(trace_current_tenant_id(),'account_binding_refused',$1,'denied',$2)", &[&actor,&serde_json::json!({"reason":reason})]).await?;
        tx.commit().await?;
        Ok(())
    }

    /// The provisioned admission anchor for one authenticated principal.
    ///
    /// Accepts either provisioning origin (#836): `near` is a wallet, `near_ai`
    /// a NEAR AI login. The two are separate identity systems and their anchors
    /// live in disjoint keyspaces, but both grant admission the same way, so
    /// this reads either. The caller's tenant prefix already decided which
    /// namespace the request is on, and a row is only ever written under the
    /// matching one.
    async fn near_provisioned_row_for_principal(
        &self,
        tenant: &str,
        principal: &str,
    ) -> Result<Option<(String, uuid::Uuid)>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx.query("SELECT n.anchor_hash, n.account_id FROM trace_near_provisioned_devices n JOIN device_keys d ON d.tenant_id=n.tenant_id AND d.device_key_id=n.device_key_id JOIN trace_account_principals p ON p.tenant_id=n.tenant_id AND p.account_id=n.account_id AND p.principal_ref=n.principal_ref JOIN trace_accounts a ON a.tenant_id=n.tenant_id AND a.account_id=n.account_id WHERE n.tenant_id=$1 AND n.principal_ref=$2 AND d.revoked_at IS NULL AND d.onboarding_origin IN ('near','near_ai') AND p.unlinked_at IS NULL AND a.closed_at IS NULL LIMIT 2", &[&tenant,&principal]).await?;
        tx.commit().await?;
        if rows.len() > 1 {
            return Err(DatabaseError::Query("near_provisioning_ambiguous".into()));
        }
        Ok(rows.into_iter().next().map(|r| (r.get(0), r.get(1))))
    }

    pub(super) async fn near_anchor_for_principal(
        &self,
        tenant: &str,
        principal: &str,
    ) -> Result<Option<String>, DatabaseError> {
        Ok(self
            .near_provisioned_row_for_principal(tenant, principal)
            .await?
            .map(|(anchor, _)| anchor))
    }

    pub(super) async fn near_account_for_principal(
        &self,
        tenant: &str,
        principal: &str,
    ) -> Result<Option<uuid::Uuid>, DatabaseError> {
        Ok(self
            .near_provisioned_row_for_principal(tenant, principal)
            .await?
            .map(|(_, account)| account))
    }
}
