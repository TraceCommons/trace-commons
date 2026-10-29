// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The PostgreSQL referral code store (V142;
//! docs/superpowers/specs/2026-10-29-referral-rewards-design.md, sections 2
//! and 3).
//!
//! Every call opens a transaction in the auth-derived tenant's context
//! first, so forced RLS (`trace_current_tenant_id()`) bounds every
//! statement. Applying a code is the one call that touches two tenants:
//!
//! 1. Which tenant and account own a code index is read in a transaction of
//!    its own as `trace_referral_resolver` (`SET LOCAL ROLE`, which ends
//!    with that transaction on every path). That role reads six columns of
//!    `trace_referral_codes` and nothing else.
//! 2. The apply transaction then runs the checks of spec section 3 in order
//!    under the referee's tenant, holding an advisory lock on the referee
//!    account, and switches `trace_commons.trace_tenant_id` to the
//!    referrer's tenant only to re-read the code (still live, still that
//!    account's) and the referrer's anchors and device keys, and to write
//!    the referrer's rows. A refusal is always recorded under the referee's
//!    tenant.
//!
//! Audit rows are hash-only: the actor is `referral_actor_hash` of the
//! account in its own tenant, and the referee's `referee_ref_hash` in the
//! referrer's. No row names the other tenant's account.

use std::collections::BTreeSet;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use tokio_postgres::error::SqlState;
use trace_commons_protocol::payout_link::NEAR_AI_PAYOUT_RAIL_ID;
use trace_commons_protocol::referral::{MAX_LIVE_REFERRAL_CODES, ReferralAppliedVia};
use uuid::Uuid;

use super::postgres::PgBackend;
use super::{
    REFERRAL_REFUSAL_LIMIT, REFERRAL_REFUSAL_WINDOW_HOURS, ReferralAccountStatus, ReferralApply,
    ReferralApplyPolicy, ReferralAttribution, ReferralCodeCreate, ReferralCodeDisable,
    ReferralCodeRow, ReferralCodeWrite, ReferralCodesView, ReferralRefusal, ReferralStore,
};
use crate::error::DatabaseError;
use crate::referral_code::{referee_ref_hash, referral_actor_hash};

const ONE_CODE_INDEX: &str = "trace_referral_codes_one_index";

const LIVE: &str = "disabled_at IS NULL AND suspended_at IS NULL AND voided_at IS NULL";

fn unique_violation_of(error: &tokio_postgres::Error, constraint: &str) -> bool {
    error.as_db_error().is_some_and(|db| {
        db.code() == &SqlState::UNIQUE_VIOLATION && db.constraint() == Some(constraint)
    })
}

fn parse_signed(value: &str) -> Result<i128, DatabaseError> {
    value
        .parse::<i128>()
        .map_err(|_| DatabaseError::Query("referral_amount_out_of_range".into()))
}

fn parse_units(value: &str) -> Result<u128, DatabaseError> {
    value
        .parse::<u128>()
        .map_err(|_| DatabaseError::Query("referral_amount_out_of_range".into()))
}

async fn set_tenant(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
) -> Result<(), DatabaseError> {
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await?;
    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn insert_audit(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    action: &str,
    actor_ref_hash: &str,
    code_index: Option<&str>,
    referee_ref: Option<&str>,
    label: Option<&str>,
) -> Result<(), DatabaseError> {
    tx.execute(
        "INSERT INTO trace_referral_audit (
             tenant_id, audit_id, action, actor_ref_hash, code_index, referee_ref_hash, label
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
        &[
            &tenant,
            &Uuid::new_v4(),
            &action,
            &actor_ref_hash,
            &code_index,
            &referee_ref,
            &label,
        ],
    )
    .await?;
    Ok(())
}

/// Records `refusal` under the referee's tenant (the context `tx` is in)
/// and commits.
async fn refuse(
    tx: deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
    code_index: Option<&str>,
    refusal: ReferralRefusal,
) -> Result<ReferralApply, DatabaseError> {
    insert_audit(
        &tx,
        tenant,
        "referral_refused",
        &referral_actor_hash(account),
        code_index,
        Some(&referee_ref_hash(tenant, account)),
        Some(refusal.label()),
    )
    .await?;
    tx.commit().await?;
    Ok(ReferralApply::Refused(refusal))
}

/// The NEAR anchors (wallet and login) and device keys of `account` in the
/// tenant `tx` is in: what a self-referral shares.
async fn identity_marks(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
) -> Result<BTreeSet<String>, DatabaseError> {
    let mut marks = BTreeSet::new();
    for row in tx
        .query(
            "SELECT 'anchor:' || anchor_hash FROM trace_near_account_anchors
              WHERE tenant_id = $1 AND account_id = $2
             UNION ALL
             SELECT 'device:' || device_key_id FROM trace_near_provisioned_devices
              WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant, &account],
        )
        .await?
    {
        marks.insert(row.get::<_, String>(0));
    }
    Ok(marks)
}

impl PgBackend {
    /// Which tenant and account own the live code `code_index`, read as
    /// `trace_referral_resolver` in a transaction of its own. Not offered
    /// outside the apply: the answer names another tenant.
    async fn resolve_referral_code_owner(
        &self,
        code_index: &str,
    ) -> Result<Option<(String, Uuid)>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL ROLE trace_referral_resolver")
            .await?;
        let row = tx
            .query_opt(
                &format!(
                    "SELECT tenant_id, account_id FROM trace_referral_codes
                      WHERE code_index = $1 AND {LIVE}"
                ),
                &[&code_index],
            )
            .await?;
        tx.commit().await?;
        Ok(row.map(|row| (row.get(0), row.get(1))))
    }
}

#[async_trait]
impl ReferralStore for PgBackend {
    async fn create_referral_code(
        &self,
        tenant: &str,
        account: Uuid,
        code: &ReferralCodeWrite,
    ) -> Result<ReferralCodeCreate, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        // One create at a time per account, so the count below and the
        // insert after it see the same codes.
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended(
                 'referral-codes-v1:' || $1 || ':' || $2::UUID::TEXT, 0))",
            &[&tenant, &account],
        )
        .await?;
        let open: Option<bool> = tx
            .query_opt(
                "SELECT closed_at IS NULL FROM trace_accounts
                  WHERE tenant_id = $1 AND account_id = $2",
                &[&tenant, &account],
            )
            .await?
            .map(|row| row.get(0));
        if open != Some(true) {
            tx.commit().await?;
            return Ok(ReferralCodeCreate::AccountIneligible);
        }
        let live: i64 = tx
            .query_one(
                &format!(
                    "SELECT count(*) FROM trace_referral_codes
                      WHERE tenant_id = $1 AND account_id = $2 AND {LIVE}"
                ),
                &[&tenant, &account],
            )
            .await?
            .get(0);
        if live >= i64::from(MAX_LIVE_REFERRAL_CODES) {
            tx.commit().await?;
            return Ok(ReferralCodeCreate::LimitReached);
        }
        let inserted = tx
            .query_one(
                "INSERT INTO trace_referral_codes (
                     tenant_id, code_id, account_id, code_index, sealed_code,
                     code_key_ref, index_pepper_ref
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7)
                 RETURNING created_at",
                &[
                    &tenant,
                    &code.code_id,
                    &account,
                    &code.code_index,
                    &code.sealed_code,
                    &code.code_key_ref,
                    &code.index_pepper_ref,
                ],
            )
            .await;
        let created_at: DateTime<Utc> = match inserted {
            Ok(row) => row.get(0),
            // The index belongs to a code somewhere, possibly in a tenant
            // this transaction cannot see. Dropping the transaction rolls
            // it back; the caller draws another code.
            Err(error) if unique_violation_of(&error, ONE_CODE_INDEX) => {
                return Ok(ReferralCodeCreate::IndexTaken);
            }
            Err(error) => return Err(error.into()),
        };
        insert_audit(
            &tx,
            tenant,
            "code_created",
            &referral_actor_hash(account),
            Some(&code.code_index),
            None,
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(ReferralCodeCreate::Created { created_at })
    }

    async fn referral_codes(
        &self,
        tenant: &str,
        account: Uuid,
    ) -> Result<ReferralCodesView, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT c.code_id, c.code_index, c.sealed_code, c.code_key_ref,
                        c.index_pepper_ref, c.created_at, c.disabled_at, c.suspended_at,
                        c.voided_at,
                        (SELECT count(*) FROM trace_referral_referees r
                          WHERE r.tenant_id = c.tenant_id AND r.account_id = c.account_id
                            AND r.code_index = c.code_index AND r.voided_at IS NULL)::BIGINT,
                        (SELECT COALESCE(SUM(b.bonus_units), 0)::TEXT
                           FROM trace_referral_bonus_accruals b
                           JOIN trace_referral_referees r
                             ON r.tenant_id = b.tenant_id
                            AND r.referee_ref_hash = b.referee_ref_hash
                          WHERE b.tenant_id = c.tenant_id AND b.account_id = c.account_id
                            AND b.rail_id = $3 AND r.code_index = c.code_index)
                   FROM trace_referral_codes c
                  WHERE c.tenant_id = $1 AND c.account_id = $2
                  ORDER BY c.created_at, c.code_id",
                &[&tenant, &account, &NEAR_AI_PAYOUT_RAIL_ID],
            )
            .await?;
        let mut codes = Vec::with_capacity(rows.len());
        for row in &rows {
            let referees: i64 = row.get(9);
            let owed: String = row.get(10);
            codes.push(ReferralCodeRow {
                code_id: row.get(0),
                code_index: row.get(1),
                sealed_code: row.get(2),
                code_key_ref: row.get(3),
                index_pepper_ref: row.get(4),
                created_at: row.get(5),
                disabled_at: row.get(6),
                suspended_at: row.get(7),
                voided_at: row.get(8),
                referee_count: u64::try_from(referees).unwrap_or(0),
                bonus_owed: parse_signed(&owed)?,
            });
        }
        let owed: String = tx
            .query_one(
                "SELECT COALESCE(SUM(bonus_units), 0)::TEXT FROM trace_referral_bonus_accruals
                  WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3",
                &[&tenant, &account, &NEAR_AI_PAYOUT_RAIL_ID],
            )
            .await?
            .get(0);
        let sent: String = tx
            .query_one(
                "SELECT COALESCE(SUM(delta), 0)::TEXT FROM trace_payout_lines
                  WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                    AND stream = 'referral' AND state = 'confirmed'",
                &[&tenant, &account, &NEAR_AI_PAYOUT_RAIL_ID],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(ReferralCodesView {
            codes,
            bonus_owed: parse_signed(&owed)?,
            bonus_sent: parse_units(&sent)?,
        })
    }

    async fn disable_referral_code(
        &self,
        tenant: &str,
        account: Uuid,
        code_id: Uuid,
    ) -> Result<ReferralCodeDisable, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let disabled = tx
            .query_opt(
                "UPDATE trace_referral_codes SET disabled_at = clock_timestamp()
                  WHERE tenant_id = $1 AND account_id = $2 AND code_id = $3
                    AND disabled_at IS NULL
                  RETURNING code_index",
                &[&tenant, &account, &code_id],
            )
            .await?;
        let Some(row) = disabled else {
            let exists: bool = tx
                .query_one(
                    "SELECT EXISTS (SELECT 1 FROM trace_referral_codes
                                     WHERE tenant_id = $1 AND account_id = $2
                                       AND code_id = $3)",
                    &[&tenant, &account, &code_id],
                )
                .await?
                .get(0);
            tx.commit().await?;
            return Ok(if exists {
                ReferralCodeDisable::AlreadyDisabled
            } else {
                ReferralCodeDisable::NotFound
            });
        };
        let code_index: String = row.get(0);
        insert_audit(
            &tx,
            tenant,
            "code_disabled",
            &referral_actor_hash(account),
            Some(&code_index),
            None,
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(ReferralCodeDisable::Disabled)
    }

    async fn apply_referral_code(
        &self,
        tenant: &str,
        account: Uuid,
        code_index: Option<&str>,
        applied_via: ReferralAppliedVia,
        policy: &ReferralApplyPolicy,
    ) -> Result<ReferralApply, DatabaseError> {
        // The cross-tenant read comes first, in its own transaction on its
        // own connection, so the apply below never holds two connections.
        // Its answer is acted on only at check 3, and re-read at check 6.
        let owner = match code_index {
            Some(index) => self.resolve_referral_code_owner(index).await?,
            None => None,
        };

        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        // One apply at a time per account: the refusal count, the "already
        // referred" read and the attribution insert see the same rows.
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended(
                 'referral-apply-v1:' || $1 || ':' || $2::UUID::TEXT, 0))",
            &[&tenant, &account],
        )
        .await?;
        let now: DateTime<Utc> = tx.query_one("SELECT clock_timestamp()", &[]).await?.get(0);

        // 1. The program is open.
        let Some(program_start) = policy.program_start.filter(|start| now >= *start) else {
            return refuse(
                tx,
                tenant,
                account,
                code_index,
                ReferralRefusal::Unavailable,
            )
            .await;
        };
        if policy.program_end.is_some_and(|end| now >= end) {
            return refuse(
                tx,
                tenant,
                account,
                code_index,
                ReferralRefusal::Unavailable,
            )
            .await;
        }

        // 2. Not too many refusals in the window.
        let refused: i64 = tx
            .query_one(
                "SELECT count(*) FROM trace_referral_audit
                  WHERE tenant_id = $1 AND action = 'referral_refused'
                    AND referee_ref_hash = $2
                    AND created_at > clock_timestamp() - make_interval(hours => $3::INT)",
                &[
                    &tenant,
                    &referee_ref_hash(tenant, account),
                    &(REFERRAL_REFUSAL_WINDOW_HOURS as i32),
                ],
            )
            .await?
            .get(0);
        if refused >= REFERRAL_REFUSAL_LIMIT {
            return refuse(
                tx,
                tenant,
                account,
                code_index,
                ReferralRefusal::RateLimited,
            )
            .await;
        }

        // 3. A live code has this index.
        let (Some(index), Some((referrer_tenant, referrer_account))) = (code_index, owner) else {
            return refuse(
                tx,
                tenant,
                account,
                code_index,
                ReferralRefusal::CodeUnknown,
            )
            .await;
        };

        // 4. No referral yet, ever.
        let referred: bool = tx
            .query_one(
                "SELECT EXISTS (SELECT 1 FROM trace_referral_attributions
                                 WHERE tenant_id = $1 AND account_id = $2)",
                &[&tenant, &account],
            )
            .await?
            .get(0);
        if referred {
            return refuse(
                tx,
                tenant,
                account,
                code_index,
                ReferralRefusal::AlreadyReferred,
            )
            .await;
        }

        // 5. Created within the program, and recently enough. A missing or
        // closed account is refused here too: it can be no one's referee.
        let created_at = tx
            .query_opt(
                "SELECT created_at, closed_at IS NOT NULL FROM trace_accounts
                  WHERE tenant_id = $1 AND account_id = $2",
                &[&tenant, &account],
            )
            .await?
            .and_then(|row| (!row.get::<_, bool>(1)).then(|| row.get::<_, DateTime<Utc>>(0)));
        let late = Duration::days(i64::from(policy.late_days));
        let Some(created_at) =
            created_at.filter(|created| *created >= program_start && *created + late >= now)
        else {
            return refuse(
                tx,
                tenant,
                account,
                code_index,
                ReferralRefusal::AccountTooOld,
            )
            .await;
        };

        // 6. Not oneself: another tenant, no shared NEAR anchor or device
        // key, and the code still live and still that account's.
        if referrer_tenant == tenant {
            return refuse(
                tx,
                tenant,
                account,
                code_index,
                ReferralRefusal::SelfReferral,
            )
            .await;
        }
        let referee_marks = identity_marks(&tx, tenant, account).await?;
        set_tenant(&tx, &referrer_tenant).await?;
        let still_live = tx
            .query_opt(
                &format!(
                    "SELECT 1 FROM trace_referral_codes
                      WHERE tenant_id = $1 AND code_index = $2 AND account_id = $3 AND {LIVE}
                      FOR SHARE"
                ),
                &[&referrer_tenant, &index, &referrer_account],
            )
            .await?
            .is_some();
        if !still_live {
            set_tenant(&tx, tenant).await?;
            return refuse(
                tx,
                tenant,
                account,
                code_index,
                ReferralRefusal::CodeUnknown,
            )
            .await;
        }
        let referrer_marks = identity_marks(&tx, &referrer_tenant, referrer_account).await?;
        if !referee_marks.is_disjoint(&referrer_marks) {
            set_tenant(&tx, tenant).await?;
            return refuse(
                tx,
                tenant,
                account,
                code_index,
                ReferralRefusal::SelfReferral,
            )
            .await;
        }

        // Applied: the attribution under the referee's tenant, then the
        // mirror under the referrer's, in this one transaction.
        let referee_ref = referee_ref_hash(tenant, account);
        set_tenant(&tx, tenant).await?;
        let applied_at: DateTime<Utc> = tx
            .query_one(
                "INSERT INTO trace_referral_attributions (
                     tenant_id, account_id, code_index, referrer_tenant_id,
                     referrer_account_id, referee_ref_hash, referee_account_created_at,
                     applied_via
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                 RETURNING applied_at",
                &[
                    &tenant,
                    &account,
                    &index,
                    &referrer_tenant,
                    &referrer_account,
                    &referee_ref,
                    &created_at,
                    &applied_via.as_str(),
                ],
            )
            .await?
            .get(0);
        insert_audit(
            &tx,
            tenant,
            "referral_applied",
            &referral_actor_hash(account),
            Some(index),
            Some(&referee_ref),
            None,
        )
        .await?;
        set_tenant(&tx, &referrer_tenant).await?;
        tx.execute(
            "INSERT INTO trace_referral_referees (
                 tenant_id, referee_ref_hash, account_id, code_index,
                 referee_account_created_at, applied_at
             ) VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &referrer_tenant,
                &referee_ref,
                &referrer_account,
                &index,
                &created_at,
                &applied_at,
            ],
        )
        .await?;
        // In the referrer's tenant the referee is named by its hash only.
        insert_audit(
            &tx,
            &referrer_tenant,
            "referral_applied",
            &referee_ref,
            Some(index),
            Some(&referee_ref),
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(ReferralApply::Applied {
            applied_at,
            referee_account_created_at: created_at,
        })
    }

    async fn referral_status(
        &self,
        tenant: &str,
        account: Uuid,
    ) -> Result<ReferralAccountStatus, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let attribution = tx
            .query_opt(
                "SELECT applied_at, referee_account_created_at, applied_via,
                        voided_at IS NOT NULL
                   FROM trace_referral_attributions
                  WHERE tenant_id = $1 AND account_id = $2",
                &[&tenant, &account],
            )
            .await?
            .map(|row| {
                let via: String = row.get(2);
                Ok::<_, DatabaseError>(ReferralAttribution {
                    applied_at: row.get(0),
                    referee_account_created_at: row.get(1),
                    applied_via: ReferralAppliedVia::parse(&via).ok_or_else(|| {
                        DatabaseError::Query("referral_applied_via_unknown".into())
                    })?,
                    voided: row.get(3),
                })
            })
            .transpose()?;
        let account_created_at = tx
            .query_opt(
                "SELECT created_at FROM trace_accounts WHERE tenant_id = $1 AND account_id = $2",
                &[&tenant, &account],
            )
            .await?
            .map(|row| row.get(0));
        tx.commit().await?;
        Ok(ReferralAccountStatus {
            attribution,
            account_created_at,
        })
    }

    async fn record_referral_network_mark(
        &self,
        tenant: &str,
        account: Uuid,
        mark: &str,
        retention_days: u32,
    ) -> Result<bool, DatabaseError> {
        let retention_days = i32::try_from(retention_days).unwrap_or(i32::MAX);
        let refresh_minutes =
            i32::try_from(crate::referral_network::REFERRAL_NETWORK_MARK_REFRESH_MINUTES)
                .unwrap_or(i32::MAX);
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        tx.execute(
            "DELETE FROM trace_referral_network_marks
              WHERE tenant_id = $1 AND account_id = $2
                AND last_seen_at < clock_timestamp() - make_interval(days => $3)",
            &[&tenant, &account, &retention_days],
        )
        .await?;
        // Only an account in the program: it owns a code, or a code was
        // applied to it. Everyone else leaves no mark.
        let written = tx
            .execute(
                "INSERT INTO trace_referral_network_marks (tenant_id, account_id, mark)
                 SELECT $1, $2, $3
                  WHERE EXISTS (SELECT 1 FROM trace_referral_codes
                                 WHERE tenant_id = $1 AND account_id = $2)
                     OR EXISTS (SELECT 1 FROM trace_referral_attributions
                                 WHERE tenant_id = $1 AND account_id = $2)
                 ON CONFLICT (tenant_id, account_id, mark) DO UPDATE
                    SET last_seen_at = clock_timestamp()
                  WHERE trace_referral_network_marks.last_seen_at
                        < clock_timestamp() - make_interval(mins => $4)",
                &[&tenant, &account, &mark, &refresh_minutes],
            )
            .await?;
        tx.commit().await?;
        Ok(written > 0)
    }
}
