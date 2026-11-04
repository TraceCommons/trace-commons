// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! F10's database half: the change ceremony row, the start check and the
//! one-transaction change. Spec:
//! `docs/superpowers/specs/2026-11-03-f10-change-account-design.md`, 4.3.
//!
//! Every statement runs in the session's own tenant under forced RLS
//! (`begin_trace_tenant_transaction`); nothing here reads another tenant.

use super::*;
use crate::account_login_change::{
    NearAiRebindPending, REBIND_COOLDOWN_DAYS, RebindCurrent, RebindOutcome, RebindPrecheck,
    RebindSession,
};
use trace_commons_protocol::account_login_change::IdentityKind;

/// The payout audit label for a link a change unlinks (V147's).
const PAYOUT_LABEL: &str = "near_ai_login_required";

/// Read everything start and finish both require, in `tx`. `lock` takes the
/// account row `FOR UPDATE`, so two changes of one account serialize.
async fn session_check(
    tx: &deadpool_postgres::Transaction<'_>,
    session: &RebindSession,
    lock: bool,
) -> Result<Result<IdentityKind, RebindPrecheck>, DatabaseError> {
    if !trace_commons_protocol::admission::is_anchored_tenant(&session.tenant_id) {
        return Ok(Err(RebindPrecheck::Refused("tenant_not_anchored")));
    }
    let account_sql = if lock {
        "SELECT 1 FROM trace_accounts WHERE tenant_id=$1 AND account_id=$2 AND closed_at IS NULL FOR UPDATE"
    } else {
        "SELECT 1 FROM trace_accounts WHERE tenant_id=$1 AND account_id=$2 AND closed_at IS NULL"
    };
    if tx
        .query_opt(account_sql, &[&session.tenant_id, &session.account_id])
        .await?
        .is_none()
    {
        return Ok(Err(RebindPrecheck::Refused("account_not_open")));
    }
    // The session is a live native session of this account, minted for
    // this device.
    let live_session = tx
        .query_opt(
            "SELECT 1 FROM trace_sessions
              WHERE tenant_id=$1 AND token_hash=$2 AND account_id=$3
                AND client_kind='native' AND device_key_id=$4
                AND revoked_at IS NULL AND expires_at > clock_timestamp()",
            &[
                &session.tenant_id,
                &session.session_token_hash,
                &session.account_id,
                &session.device_key_id,
            ],
        )
        .await?;
    if live_session.is_none() {
        return Ok(Err(RebindPrecheck::Refused("session_device_mismatch")));
    }
    // FOR SHARE: a revoke holds the row until it commits, so this sees it.
    let device = tx
        .query_opt(
            "SELECT onboarding_origin, revoked_at IS NULL FROM device_keys
              WHERE tenant_id=$1 AND device_key_id=$2 FOR SHARE",
            &[&session.tenant_id, &session.device_key_id],
        )
        .await?;
    let Some(device) = device else {
        return Ok(Err(RebindPrecheck::Refused("device_unknown")));
    };
    if !device.get::<_, bool>(1) {
        return Ok(Err(RebindPrecheck::Refused("device_revoked")));
    }
    if !matches!(device.get::<_, String>(0).as_str(), "near" | "near_ai") {
        return Ok(Err(RebindPrecheck::Refused("device_origin")));
    }
    // Decision 8: a device that joined by code does not start a change.
    let joined_by_code = tx
        .query_opt(
            "SELECT 1 FROM trace_account_device_codes
              WHERE tenant_id=$1 AND redeemed_device_key_id=$2",
            &[&session.tenant_id, &session.device_key_id],
        )
        .await?
        .is_some();
    if joined_by_code {
        return Ok(Err(RebindPrecheck::Refused("device_joined_by_code")));
    }
    let live = tx
        .query(
            "SELECT identity_source FROM trace_near_account_anchors
              WHERE tenant_id=$1 AND account_id=$2 AND retired_at IS NULL",
            &[&session.tenant_id, &session.account_id],
        )
        .await?;
    let kind = match live.as_slice() {
        [row] => match row.get::<_, String>(0).as_str() {
            "wallet" => IdentityKind::Wallet,
            "near_ai_login" => IdentityKind::NearAiLogin,
            _ => return Ok(Err(RebindPrecheck::Refused("anchor_source_unknown"))),
        },
        _ => return Ok(Err(RebindPrecheck::Refused("no_single_live_anchor"))),
    };
    let recent = tx
        .query_opt(
            "SELECT 1 FROM trace_account_login_changes
              WHERE tenant_id=$1 AND account_id=$2
                AND changed_at > clock_timestamp() - make_interval(days => $3::int)",
            &[
                &session.tenant_id,
                &session.account_id,
                &(REBIND_COOLDOWN_DAYS as i32),
            ],
        )
        .await?
        .is_some();
    if recent {
        return Ok(Err(RebindPrecheck::TooSoon));
    }
    Ok(Ok(kind))
}

impl PgBackend {
    pub(super) async fn near_ai_rebind_store_ceremony(
        &self,
        hash: &str,
        pending: &NearAiRebindPending,
        expires_at: i64,
    ) -> Result<(), DatabaseError> {
        self.store_ceremony_payload(hash, pending, expires_at).await
    }

    pub(super) async fn near_ai_rebind_take_ceremony(
        &self,
        hash: &str,
    ) -> Result<Option<NearAiRebindPending>, DatabaseError> {
        self.take_ceremony_payload(hash).await
    }

    pub(super) async fn near_ai_rebind_check(
        &self,
        session: &RebindSession,
    ) -> Result<RebindPrecheck, DatabaseError> {
        if !trace_commons_protocol::admission::is_anchored_tenant(&session.tenant_id) {
            return Ok(RebindPrecheck::Refused("tenant_not_anchored"));
        }
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &session.tenant_id).await?;
        let checked = session_check(&tx, session, false).await?;
        tx.commit().await?;
        Ok(match checked {
            Ok(current_kind) => RebindPrecheck::Ready { current_kind },
            Err(refusal) => refusal,
        })
    }

    pub(super) async fn near_ai_rebind_commit(
        &self,
        session: &RebindSession,
        current: &RebindCurrent,
        new_login: &crate::near_ai_login::VerifiedNearAiLogin,
        identity: &crate::near_account_identity::NearAccountIdentity,
    ) -> Result<RebindOutcome, DatabaseError> {
        if !trace_commons_protocol::admission::is_anchored_tenant(&session.tenant_id) {
            return Ok(RebindOutcome::Refused("tenant_not_anchored"));
        }
        let super::account_onboarding::NearAiAnchorMaterial {
            anchor_hash: new_anchor,
            sealed_json,
            pepper_ref,
            key_ref,
        } = super::account_onboarding::NearAiAnchorMaterial::for_login(new_login, identity)?;
        let old_anchor = current.anchor_hash();
        if new_anchor == old_anchor {
            return Ok(RebindOutcome::Refused("same_identity"));
        }
        let tenant = session.tenant_id.as_str();
        let account = session.account_id;
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;

        // The account row first, then both anchors' provisioning locks in
        // sorted order: a concurrent change waits on the row, a concurrent
        // sign-up with either login waits on its anchor.
        let kind = match session_check(&tx, session, true).await? {
            Ok(kind) => kind,
            Err(RebindPrecheck::TooSoon) => return Ok(RebindOutcome::TooSoon),
            Err(RebindPrecheck::Refused(label)) => return Ok(RebindOutcome::Refused(label)),
            Err(RebindPrecheck::Ready { .. }) => unreachable!("session_check never answers Ready"),
        };
        if kind != current.kind() {
            return Ok(RebindOutcome::Refused("current_kind_mismatch"));
        }
        let mut locks = [old_anchor, new_anchor.as_str()];
        locks.sort_unstable();
        for anchor in locks {
            tx.execute(
                "SELECT pg_advisory_xact_lock(hashtextextended($1,0))",
                &[&anchor],
            )
            .await?;
        }
        // The proven identity is this account's live anchor of that kind.
        let current_is_live = tx
            .query_opt(
                "SELECT 1 FROM trace_near_account_anchors
                  WHERE tenant_id=$1 AND account_id=$2 AND anchor_hash=$3
                    AND identity_source=$4 AND retired_at IS NULL",
                &[&tenant, &account, &old_anchor, &kind.as_str()],
            )
            .await?
            .is_some();
        if !current_is_live {
            return Ok(RebindOutcome::Refused("current_identity_mismatch"));
        }
        // No evidence-ledger lease may straddle two anchors.
        let Some(old_hex) = old_anchor.strip_prefix("sha256:") else {
            return Ok(RebindOutcome::Refused("anchor_shape"));
        };
        let busy: bool = tx
            .query_one(
                "SELECT trace_admission_anchor_busy($1,$2)",
                &[&tenant, &old_hex],
            )
            .await?
            .get(0);
        if busy {
            return Ok(RebindOutcome::Refused("admission_busy"));
        }

        // Payouts (spec 5.1). A wallet-origin account holds no proved link
        // after #1862 and V147; a waiting one is left for an operator, and an
        // active one is unlinked as V147 would have.
        let mut unlinked = Vec::new();
        if kind == IdentityKind::Wallet {
            let waiting = tx
                .query_opt(
                    "SELECT 1 FROM trace_payout_links
                      WHERE tenant_id=$1 AND account_id=$2 AND state='waiting'",
                    &[&tenant, &account],
                )
                .await?
                .is_some();
            if waiting {
                return Ok(RebindOutcome::PayoutReviewRequired);
            }
            for row in tx
                .query(
                    "UPDATE trace_payout_links
                        SET unlink_requested_at = clock_timestamp(), version = version + 1,
                            updated_at = clock_timestamp()
                      WHERE tenant_id=$1 AND account_id=$2 AND state='active'
                        AND unlink_requested_at IS NULL
                      RETURNING rail_id, link_id",
                    &[&tenant, &account],
                )
                .await?
            {
                unlinked.push((row.get::<_, String>(0), row.get::<_, uuid::Uuid>(1)));
            }
        }
        // A request proved by the old login must not confirm after it.
        tx.execute(
            "DELETE FROM trace_payout_link_requests WHERE tenant_id=$1 AND account_id=$2",
            &[&tenant, &account],
        )
        .await?;

        // Retire first: at most one live anchor per account (V150).
        tx.execute(
            "UPDATE trace_near_account_anchors SET retired_at = clock_timestamp()
              WHERE tenant_id=$1 AND account_id=$2 AND anchor_hash=$3 AND retired_at IS NULL",
            &[&tenant, &account, &old_anchor],
        )
        .await?;
        let provider = new_login.auth_provider().to_string();
        let inserted = tx
            .execute(
                "INSERT INTO trace_near_account_anchors(tenant_id,anchor_hash,account_id,sealed_account_name,index_pepper_ref,account_name_key_ref,identity_source,auth_provider)
                 VALUES($1,$2,$3,$4,$5,$6,'near_ai_login',$7) ON CONFLICT (anchor_hash) DO NOTHING",
                &[&tenant, &new_anchor, &account, &sealed_json, &pepper_ref, &key_ref, &provider],
            )
            .await?;
        let (switched_back, auth_provider) = if inserted == 1 {
            (false, provider)
        } else {
            // The login is anchored already. Only a retired anchor of this
            // same account is ours to revive; RLS hides every other tenant's
            // row, so anything else is a login in use.
            let revived = tx
                .query_opt(
                    "UPDATE trace_near_account_anchors SET retired_at = NULL
                      WHERE tenant_id=$1 AND account_id=$2 AND anchor_hash=$3
                        AND identity_source='near_ai_login' AND retired_at IS NOT NULL
                      RETURNING auth_provider",
                    &[&tenant, &account, &new_anchor],
                )
                .await?;
            match revived {
                Some(row) => (true, row.get::<_, Option<String>>(0).unwrap_or_default()),
                None => return Ok(RebindOutcome::InUse),
            }
        };
        tx.execute(
            "UPDATE trace_near_provisioned_devices SET anchor_hash=$3
              WHERE tenant_id=$1 AND account_id=$2 AND anchor_hash <> $3",
            &[&tenant, &account, &new_anchor],
        )
        .await?;
        // Decision 3: the wallet stops signing in to the account.
        if let RebindCurrent::Wallet {
            near_account_id, ..
        } = current
        {
            tx.execute(
                "UPDATE trace_near_identities SET revoked_at = clock_timestamp()
                  WHERE tenant_id=$1 AND account_id=$2 AND near_account_id=$3
                    AND revoked_at IS NULL",
                &[&tenant, &account, near_account_id],
            )
            .await?;
        }
        tx.execute(
            "INSERT INTO trace_account_login_changes(tenant_id,account_id,change_id,from_source,switched_back)
             VALUES($1,$2,$3,$4,$5)",
            &[&tenant, &account, &uuid::Uuid::new_v4(), &kind.as_str(), &switched_back],
        )
        .await?;
        let actor = super::onboarding_device_principal_ref(tenant, &session.device_key_id);
        tx.execute(
            "INSERT INTO trace_account_audit(tenant_id,action,actor_ref,outcome,safe_metadata)
             VALUES($1,'near_ai_login_rebound',$2,'success',$3)",
            &[
                &tenant,
                &actor,
                &serde_json::json!({
                    "identity": "near_ai_login",
                    "from": kind.as_str(),
                    "switched_back": switched_back,
                }),
            ],
        )
        .await?;
        let actor_hash = format!(
            "sha256:{}",
            hex::encode(<sha2::Sha256 as sha2::Digest>::digest(
                format!("payout-link-actor-v1:{account}").as_bytes()
            ))
        );
        for (rail, link) in unlinked {
            crate::db::postgres_payout::insert_audit(
                &tx,
                tenant,
                &rail,
                crate::db::PayoutAuditAction::UnlinkRequested,
                &actor_hash,
                Some(account),
                Some(link),
                Some(crate::db::PayoutLinkState::Active),
                Some(crate::db::PayoutLinkState::Active),
                Some(PAYOUT_LABEL),
            )
            .await?;
        }
        tx.commit().await?;
        Ok(RebindOutcome::Rebound {
            anchor_hash: new_anchor,
            auth_provider,
            switched_back,
        })
    }
}
