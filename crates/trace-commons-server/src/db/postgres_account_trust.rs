// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Transactional account invite redemption. The invite cache is never consulted.

use sha2::{Digest, Sha256};
use uuid::Uuid;

use super::PgBackend;
use crate::db::AccountInviteRedemption;
use crate::error::DatabaseError;

impl PgBackend {
    pub(super) async fn redeem_account_invite_in_tx(
        &self,
        tenant: &str,
        account: Uuid,
        invite_hash: &str,
        idempotency_key: Uuid,
    ) -> Result<AccountInviteRedemption, DatabaseError> {
        let request_digest = format!(
            "sha256:{}",
            hex::encode(Sha256::digest(invite_hash.as_bytes()))
        );
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await?;
        // V42/V50 policies expose only the presented code's durable row.
        tx.execute(
            "SELECT set_config('trace_commons.invite_subject', $1, true)",
            &[&invite_hash],
        )
        .await?;

        // The account lock serializes repeat requests and idempotency-key reuse
        // from every device/session attached to this account.
        let active = tx
            .query_opt(
                "SELECT 1 FROM trace_accounts a
                  WHERE a.tenant_id = $1 AND a.account_id = $2
                    AND a.closed_at IS NULL
                    AND EXISTS (
                        SELECT 1 FROM trace_near_account_anchors n
                         WHERE n.tenant_id = a.tenant_id
                           AND n.account_id = a.account_id
                    ) FOR UPDATE OF a",
                &[&tenant, &account],
            )
            .await?;
        if active.is_none() {
            return Ok(AccountInviteRedemption::AccountIneligible);
        }

        if let Some(previous) = tx
            .query_opt(
                "SELECT request_digest, trust_version FROM trace_account_trust_events
                  WHERE tenant_id = $1 AND account_id = $2 AND source_event_id = $3",
                &[&tenant, &account, &idempotency_key],
            )
            .await?
        {
            if previous.get::<_, String>(0) != request_digest {
                return Ok(AccountInviteRedemption::IdempotencyConflict);
            }
            return Ok(AccountInviteRedemption::Invited {
                trust_version: previous.get(1),
            });
        }

        // FOR UPDATE also serializes the final use among distinct accounts.
        // The row is read from the durable registry, never from its cache.
        let invite = tx
            .query_opt(
                "SELECT consumed_uses, max_uses, tenant_mode, fixed_tenant_id FROM onboarding_invite_grants
                  WHERE invite_subject_hash = $1 AND revoked_at IS NULL FOR UPDATE",
                &[&invite_hash],
            )
            .await?;
        let Some(invite) = invite else {
            return Ok(AccountInviteRedemption::InvalidInvite);
        };
        // `now()` is fixed at transaction start. The lock above may have
        // blocked until after expiration, so evaluate wall-clock time in a
        // separate statement only after the durable row is locked.
        let live: bool = tx
            .query_one(
                "SELECT revoked_at IS NULL
                        AND (expires_at IS NULL OR expires_at > clock_timestamp())
                   FROM onboarding_invite_grants WHERE invite_subject_hash = $1",
                &[&invite_hash],
            )
            .await?
            .get(0);
        let tenant_mode: String = invite.get(2);
        let fixed_tenant: Option<String> = invite.get(3);
        if !live || (tenant_mode == "fixed" && fixed_tenant.as_deref() != Some(tenant)) {
            return Ok(AccountInviteRedemption::InvalidInvite);
        }
        let consumed: i32 = invite.get(0);
        let maximum: i32 = invite.get(1);
        if consumed >= maximum {
            let already_redeemed: bool = tx
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM trace_account_invite_grants
                  WHERE tenant_id=$1 AND account_id=$2 AND invite_subject_hash=$3)",
                    &[&tenant, &account, &invite_hash],
                )
                .await?
                .get(0);
            if !already_redeemed {
                return Ok(AccountInviteRedemption::InvalidInvite);
            }
        }
        // Validate the presented grant before short-circuiting. An account
        // already elevated to invited has no further authority to gain, so it
        // must not consume another cohort's allowance or advance its version.
        let existing = tx
            .query_opt(
                "SELECT trust_version FROM trace_account_trust
                  WHERE tenant_id = $1 AND account_id = $2 AND authority = 'invited'",
                &[&tenant, &account],
            )
            .await?;
        let version = if let Some(existing) = existing {
            existing.get::<_, i64>(0)
        } else {
            if consumed >= maximum {
                return Ok(AccountInviteRedemption::InvalidInvite);
            }
            let next_version = tx
                .query_one(
                    "INSERT INTO trace_account_trust
                        (tenant_id, account_id, authority, trust_version)
                     VALUES ($1, $2, 'invited', 1)
                     ON CONFLICT (tenant_id, account_id) DO UPDATE
                       SET authority = 'invited',
                           trust_version = trace_account_trust.trust_version + 1,
                           updated_at = now()
                     RETURNING trust_version",
                    &[&tenant, &account],
                )
                .await?
                .get::<_, i64>(0);
            tx.execute(
                "INSERT INTO trace_account_invite_grants
                    (tenant_id, account_id, invite_subject_hash, trust_version)
                 VALUES ($1, $2, $3, $4)",
                &[&tenant, &account, &invite_hash, &next_version],
            )
            .await?;
            let spent = tx
                .query_opt(
                    "UPDATE onboarding_invite_grants
                    SET consumed_uses = consumed_uses + 1, updated_at = now()
                  WHERE invite_subject_hash = $1 AND revoked_at IS NULL
                    AND consumed_uses < max_uses
                    AND (expires_at IS NULL OR expires_at > clock_timestamp())
                  RETURNING consumed_uses",
                    &[&invite_hash],
                )
                .await?;
            if spent.is_none() {
                // Returning before commit rolls back the preceding trust and
                // grant inserts if the clock crossed expiry during this call.
                return Ok(AccountInviteRedemption::InvalidInvite);
            }
            let actor_ref = crate::account_session::account_actor_ref(
                &crate::account_session::AccountId::from_uuid(account),
            );
            tx.execute(
                "INSERT INTO trace_account_audit
                    (tenant_id, action, actor_ref, outcome, safe_metadata)
                 VALUES ($1, 'account_invite_redeemed', $2, 'invited', $3)",
                &[
                    &tenant,
                    &actor_ref,
                    &serde_json::json!({
                        "invite_subject_hash": invite_hash,
                        "trust_version": next_version,
                    }),
                ],
            )
            .await?;
            next_version
        };
        tx.execute(
            "INSERT INTO trace_account_trust_events
                (tenant_id, account_id, source_event_id, request_digest,
                 invite_subject_hash, trust_version)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &tenant,
                &account,
                &idempotency_key,
                &request_digest,
                &invite_hash,
                &version,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(AccountInviteRedemption::Invited {
            trust_version: version,
        })
    }
}
