// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The PostgreSQL payout rail store (V132).
//!
//! Every tenant-scoped call opens a transaction in the auth-derived tenant's
//! context first, so forced RLS (`trace_current_tenant_id()`) bounds every
//! statement. The deployment-wide writes and the cross-tenant enumeration
//! take their narrow role with `SET LOCAL ROLE`, which ends with the
//! transaction on every path, commit, rollback or drop.
//!
//! Amounts cross the wire as text (`$n::TEXT::NUMERIC` in, `col::TEXT` out)
//! and are parsed as `u128`: no decimal crate, and a value that does not fit
//! is refused, never truncated.

use std::collections::{BTreeSet, HashMap};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio_postgres::error::SqlState;
use trace_commons_gate_api::pipeline::{AtomicUnits, InstrumentId};
use trace_commons_gate_api::{RailId, RailRate, RecipientKind};
use uuid::Uuid;

use super::PayoutAccountStanding;
use super::postgres::PgBackend;
use super::{
    PayoutAccountLinkView, PayoutAccrual, PayoutAccrualKind, PayoutAccrualWrite, PayoutAuditAction,
    PayoutControl, PayoutHold, PayoutLine, PayoutLineResolution, PayoutLineResolve,
    PayoutLineState, PayoutLineWrite, PayoutLink, PayoutLinkAction, PayoutLinkClose,
    PayoutLinkConfirm, PayoutLinkConfirmWrite, PayoutLinkInsert, PayoutLinkRequest,
    PayoutLinkRequestInsert, PayoutLinkRequestWrite, PayoutLinkState, PayoutLinkTransition,
    PayoutLinkWrite, PayoutOwed, PayoutRateEpoch, PayoutRateEpochRecord, PayoutReferralOwed,
    PayoutStore, PayoutStream, PendingPayoutLineInsert, SealedRecipient,
};
use crate::error::DatabaseError;

const ONE_LINK_PER_STATE: [&str; 2] = [
    "trace_payout_links_one_live",
    "trace_payout_links_one_waiting",
];

/// The audit row's hash of a link id. Hash-only: the link id itself is not
/// written to the audit table. The operator release route names a link by
/// this hash.
pub fn link_id_hash(link_id: Uuid) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("payout-link-v1:{link_id}").as_bytes())
    )
}

/// The audit row's hash of an account id, domain-separated from the link
/// hash. Account ids are random UUIDs, so, like the link hash, it joins audit
/// rows to an account only for someone who already holds the account id.
/// The operator hold routes name an account by this hash.
pub fn account_id_hash(account: Uuid) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("payout-account-v1:{account}").as_bytes())
    )
}

fn violated_constraint(error: &tokio_postgres::Error) -> Option<String> {
    let db = error.as_db_error()?;
    if db.code() != &SqlState::UNIQUE_VIOLATION {
        return None;
    }
    db.constraint().map(str::to_owned)
}

fn parse_units(value: &str) -> Result<u128, DatabaseError> {
    value
        .parse::<u128>()
        .map_err(|_| DatabaseError::Query("payout_amount_out_of_range".into()))
}

fn parse_rail(value: String) -> Result<RailId, DatabaseError> {
    RailId::new(value).map_err(|_| DatabaseError::Query("payout_rail_unknown".into()))
}

fn parse_link_state(value: &str) -> Result<PayoutLinkState, DatabaseError> {
    PayoutLinkState::parse(value).ok_or_else(|| DatabaseError::Query("payout_state_unknown".into()))
}

fn parse_line_state(value: &str) -> Result<PayoutLineState, DatabaseError> {
    PayoutLineState::parse(value).ok_or_else(|| DatabaseError::Query("payout_state_unknown".into()))
}

fn parse_stream(value: &str) -> Result<PayoutStream, DatabaseError> {
    PayoutStream::parse(value).ok_or_else(|| DatabaseError::Query("payout_stream_unknown".into()))
}

/// A signed NUMERIC sum as text: `None` below zero, the magnitude refused
/// above `u128::MAX` like every other amount.
fn parse_signed_units(value: &str) -> Result<Option<u128>, DatabaseError> {
    match value.strip_prefix('-') {
        Some(magnitude) => {
            parse_units(magnitude)?;
            Ok(None)
        }
        None => parse_units(value).map(Some),
    }
}

fn parse_recipient_kind(value: String) -> Result<RecipientKind, DatabaseError> {
    RecipientKind::new(value)
        .map_err(|_| DatabaseError::Query("payout_recipient_kind_unknown".into()))
}

/// The account's link version on a rail ([`PayoutAccountLinkView::version`]):
/// links ever written plus unlinks confirmed. Only the person's changes
/// move it; the pass activating, closing or marking a link does not.
async fn account_link_version(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    account: Uuid,
    rail: &str,
) -> Result<i64, DatabaseError> {
    Ok(tx
        .query_one(
            "SELECT (count(*) + count(unlink_requested_at))::BIGINT
               FROM trace_payout_links
              WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3",
            &[&tenant, &account, &rail],
        )
        .await?
        .get(0))
}

#[allow(clippy::too_many_arguments)]
async fn insert_audit(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    rail: &str,
    action: PayoutAuditAction,
    actor_ref_hash: &str,
    account: Option<Uuid>,
    link_id: Option<Uuid>,
    prior: Option<PayoutLinkState>,
    new: Option<PayoutLinkState>,
    label: Option<&str>,
) -> Result<(), DatabaseError> {
    tx.execute(
        "INSERT INTO trace_payout_audit (
             tenant_id, audit_id, rail_id, action, actor_ref_hash, account_id_hash,
             link_id_hash, prior_state, new_state, label
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        &[
            &tenant,
            &Uuid::new_v4(),
            &rail,
            &action.as_str(),
            &actor_ref_hash,
            &account.map(account_id_hash),
            &link_id.map(link_id_hash),
            &prior.map(PayoutLinkState::as_str),
            &new.map(PayoutLinkState::as_str),
            &label,
        ],
    )
    .await?;
    Ok(())
}

impl PgBackend {
    async fn payout_tenant_client(&self) -> Result<deadpool_postgres::Client, DatabaseError> {
        Ok(self.trace_pool().get().await?)
    }
}

const LINK_COLUMNS: &str = "account_id, rail_id, link_id, recipient_kind, state,
    sealed_recipient, recipient_index, recipient_key_ref, index_pepper_ref, subject_index,
    program, final_paid::TEXT, unlink_requested_at, recipient_unavailable_at, version,
    recipient_ref, referral_program, final_paid_referral::TEXT";

fn link_from_row(row: &tokio_postgres::Row) -> Result<PayoutLink, DatabaseError> {
    let final_paid: Option<String> = row.get(11);
    let final_paid_referral: Option<String> = row.get(17);
    Ok(PayoutLink {
        account_id: row.get(0),
        rail_id: parse_rail(row.get(1))?,
        link_id: row.get(2),
        recipient_kind: parse_recipient_kind(row.get(3))?,
        state: parse_link_state(row.get(4))?,
        recipient: SealedRecipient {
            sealed: row.get(5),
            recipient_index: row.get(6),
            recipient_key_ref: row.get(7),
            index_pepper_ref: row.get(8),
            subject_index: row.get(9),
            recipient_ref: row.get(15),
        },
        program: row.get(10),
        final_paid: final_paid
            .as_deref()
            .map(parse_units)
            .transpose()?
            .map(AtomicUnits::from_raw),
        referral_program: row.get(16),
        final_paid_referral: final_paid_referral
            .as_deref()
            .map(parse_units)
            .transpose()?
            .map(AtomicUnits::from_raw),
        unlink_requested_at: row.get(12),
        recipient_unavailable_at: row.get(13),
        version: row.get(14),
    })
}

const LINE_COLUMNS: &str = "line_id, link_id, account_id, rail_id, target_total::TEXT,
    delta::TEXT, state, receipt_hash, attempts, last_error_label, stream";

fn line_from_row(row: &tokio_postgres::Row) -> Result<PayoutLine, DatabaseError> {
    let target: String = row.get(4);
    let delta: String = row.get(5);
    Ok(PayoutLine {
        line_id: row.get(0),
        link_id: row.get(1),
        account_id: row.get(2),
        rail_id: parse_rail(row.get(3))?,
        target_total: AtomicUnits::from_raw(parse_units(&target)?),
        delta: AtomicUnits::from_raw(parse_units(&delta)?),
        state: parse_line_state(row.get(6))?,
        receipt_hash: row.get(7),
        attempts: row.get(8),
        last_error_label: row.get(9),
        stream: parse_stream(row.get(10))?,
    })
}

#[async_trait]
impl PayoutStore for PgBackend {
    async fn insert_payout_accruals(
        &self,
        tenant: &str,
        accruals: &[PayoutAccrualWrite],
    ) -> Result<u64, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let mut inserted = 0;
        for accrual in accruals {
            let sign = match accrual.kind {
                PayoutAccrualKind::Credit => "",
                PayoutAccrualKind::Reversal => "-",
            };
            let award = format!("{sign}{}", accrual.award_units.get());
            let units = format!("{sign}{}", accrual.rail_units.get());
            // A conflict on either partial unique index (credit or reversal
            // key) means this rail already priced this line or event.
            inserted += tx
                .execute(
                    "INSERT INTO trace_payout_accruals (
                         tenant_id, accrual_id, rail_id, kind, settlement_batch_id,
                         credit_account_hash, reversal_event_id, account_id,
                         award_units, rail_units, rate_version, instrument_id
                     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8,
                               $9::TEXT::NUMERIC, $10::TEXT::NUMERIC, $11, $12)
                     ON CONFLICT DO NOTHING",
                    &[
                        &tenant,
                        &Uuid::new_v4(),
                        &accrual.rail_id.as_str(),
                        &accrual.kind.as_str(),
                        &accrual.settlement_batch_id,
                        &accrual.credit_account_hash,
                        &accrual.reversal_event_id,
                        &accrual.account_id,
                        &award,
                        &units,
                        &accrual.rate_version,
                        &accrual.instrument_id.as_str(),
                    ],
                )
                .await?;
        }
        tx.commit().await?;
        Ok(inserted)
    }

    async fn claim_payout_accruals(
        &self,
        tenant: &str,
        rail: &RailId,
        credit_account_hash: &str,
        account: Uuid,
        actor_ref_hash: &str,
    ) -> Result<u64, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "UPDATE trace_payout_accruals
                    SET account_id = $3, claimed_at = clock_timestamp()
                  WHERE tenant_id = $1 AND credit_account_hash = $2 AND account_id IS NULL
                    AND rail_id = $4
                  RETURNING rail_id",
                &[&tenant, &credit_account_hash, &account, &rail.as_str()],
            )
            .await?;
        let rails: BTreeSet<String> = rows.iter().map(|row| row.get(0)).collect();
        for rail in &rails {
            insert_audit(
                &tx,
                tenant,
                rail,
                PayoutAuditAction::AccrualClaimed,
                actor_ref_hash,
                Some(account),
                None,
                None,
                None,
                None,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(rows.len() as u64)
    }

    async fn payout_database_now(&self) -> Result<chrono::DateTime<chrono::Utc>, DatabaseError> {
        let client = self.payout_tenant_client().await?;
        let row = client.query_one("SELECT clock_timestamp()", &[]).await?;
        Ok(row.get(0))
    }

    async fn payout_principal_accounts(
        &self,
        tenant: &str,
        principals: &[String],
    ) -> Result<HashMap<String, Uuid>, DatabaseError> {
        if principals.is_empty() {
            return Ok(HashMap::new());
        }
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT principal_ref, account_id FROM trace_account_owned_principals
                  WHERE tenant_id = trace_current_tenant_id()
                    AND principal_ref = ANY($1)",
                &[&principals],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .iter()
            .map(|row| (row.get::<_, String>(0), row.get::<_, Uuid>(1)))
            .collect())
    }

    async fn payout_accruals(
        &self,
        tenant: &str,
        rail: &RailId,
    ) -> Result<Vec<PayoutAccrual>, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT kind, settlement_batch_id, credit_account_hash, reversal_event_id,
                        account_id, abs(award_units)::TEXT, abs(rail_units)::TEXT,
                        rate_version, instrument_id
                   FROM trace_payout_accruals
                  WHERE tenant_id = $1 AND rail_id = $2
                  ORDER BY created_at, accrual_id",
                &[&tenant, &rail.as_str()],
            )
            .await?;
        tx.commit().await?;
        rows.iter()
            .map(|row| {
                let kind: String = row.get(0);
                let kind = match kind.as_str() {
                    "credit" => PayoutAccrualKind::Credit,
                    "reversal" => PayoutAccrualKind::Reversal,
                    _ => return Err(DatabaseError::Query("payout_state_unknown".into())),
                };
                let award: String = row.get(5);
                let units: String = row.get(6);
                let instrument: String = row.get(8);
                Ok(PayoutAccrual {
                    rail_id: rail.clone(),
                    kind,
                    settlement_batch_id: row.get(1),
                    credit_account_hash: row.get(2),
                    reversal_event_id: row.get(3),
                    account_id: row.get(4),
                    award_units: AtomicUnits::from_raw(parse_units(&award)?),
                    rail_units: AtomicUnits::from_raw(parse_units(&units)?),
                    rate_version: row.get(7),
                    instrument_id: InstrumentId::new(instrument)
                        .map_err(|_| DatabaseError::Query("payout_instrument_unknown".into()))?,
                })
            })
            .collect()
    }

    async fn payout_owed(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<PayoutOwed, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_one(
                "SELECT COALESCE(SUM(rail_units) FILTER (WHERE kind = 'credit'), 0)::TEXT,
                        COALESCE(-SUM(rail_units) FILTER (WHERE kind = 'reversal'), 0)::TEXT,
                        count(*)
                   FROM trace_payout_accruals
                  WHERE tenant_id = $1 AND rail_id = $2 AND account_id = $3",
                &[&tenant, &rail.as_str(), &account],
            )
            .await?;
        tx.commit().await?;
        let credited: String = row.get(0);
        let reversed: String = row.get(1);
        Ok(PayoutOwed {
            credited: parse_units(&credited)?,
            reversed: parse_units(&reversed)?,
            accrual_count: row.get(2),
        })
    }

    async fn payout_closed_links_final_paid(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<u128, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_one(
                "SELECT COALESCE(SUM(final_paid), 0)::TEXT
                   FROM trace_payout_links
                  WHERE tenant_id = $1 AND rail_id = $2 AND account_id = $3
                    AND state = 'closed'",
                &[&tenant, &rail.as_str(), &account],
            )
            .await?;
        tx.commit().await?;
        let total: String = row.get(0);
        parse_units(&total)
    }

    async fn payout_live_link(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<Option<PayoutLink>, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                &format!(
                    "SELECT {LINK_COLUMNS} FROM trace_payout_links
                      WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                        AND state = 'active'"
                ),
                &[&tenant, &account, &rail.as_str()],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(link_from_row).transpose()
    }

    async fn payout_links(
        &self,
        tenant: &str,
        rail: &RailId,
        states: &[PayoutLinkState],
    ) -> Result<Vec<PayoutLink>, DatabaseError> {
        let states: Vec<&str> = states.iter().map(|state| state.as_str()).collect();
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                &format!(
                    "SELECT {LINK_COLUMNS} FROM trace_payout_links
                      WHERE tenant_id = $1 AND rail_id = $2 AND state = ANY($3::TEXT[])
                      ORDER BY account_id, created_at, link_id"
                ),
                &[&tenant, &rail.as_str(), &states],
            )
            .await?;
        tx.commit().await?;
        rows.iter().map(link_from_row).collect()
    }

    async fn mark_payout_link_unlink_requested(
        &self,
        tenant: &str,
        link_id: Uuid,
        actor_ref_hash: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                "UPDATE trace_payout_links
                    SET unlink_requested_at = clock_timestamp(), version = version + 1,
                        updated_at = clock_timestamp()
                  WHERE tenant_id = $1 AND link_id = $2 AND state = 'active'
                    AND unlink_requested_at IS NULL
                  RETURNING rail_id, account_id",
                &[&tenant, &link_id],
            )
            .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let rail: String = row.get(0);
        let account: Uuid = row.get(1);
        insert_audit(
            &tx,
            tenant,
            &rail,
            PayoutAuditAction::UnlinkRequested,
            actor_ref_hash,
            Some(account),
            Some(link_id),
            Some(PayoutLinkState::Active),
            Some(PayoutLinkState::Active),
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn mark_payout_link_recipient_unavailable(
        &self,
        tenant: &str,
        link_id: Uuid,
        actor_ref_hash: &str,
        label: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                "UPDATE trace_payout_links
                    SET recipient_unavailable_at = clock_timestamp(), version = version + 1,
                        updated_at = clock_timestamp()
                  WHERE tenant_id = $1 AND link_id = $2 AND state IN ('active', 'closing')
                    AND recipient_unavailable_at IS NULL
                  RETURNING rail_id, state, account_id",
                &[&tenant, &link_id],
            )
            .await?;
        let Some(row) = row else {
            return Ok(false);
        };
        let rail: String = row.get(0);
        let state = parse_link_state(row.get(1))?;
        let account: Uuid = row.get(2);
        insert_audit(
            &tx,
            tenant,
            &rail,
            PayoutAuditAction::LinkRecipientUnavailable,
            actor_ref_hash,
            Some(account),
            Some(link_id),
            Some(state),
            Some(state),
            label,
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn insert_payout_link(
        &self,
        tenant: &str,
        link: &PayoutLinkWrite,
    ) -> Result<PayoutLinkInsert, DatabaseError> {
        if !matches!(
            link.state,
            PayoutLinkState::Waiting | PayoutLinkState::Active
        ) {
            return Err(DatabaseError::Constraint(
                "payout_link_insert_state_invalid".into(),
            ));
        }
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let inserted = tx
            .execute(
                "INSERT INTO trace_payout_links (
                     tenant_id, account_id, rail_id, link_id, recipient_kind, state,
                     sealed_recipient, recipient_index, recipient_key_ref,
                     index_pepper_ref, subject_index, program, recipient_ref
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
                &[
                    &tenant,
                    &link.account_id,
                    &link.rail_id.as_str(),
                    &link.link_id,
                    &link.recipient_kind.as_str(),
                    &link.state.as_str(),
                    &link.recipient.sealed,
                    &link.recipient.recipient_index,
                    &link.recipient.recipient_key_ref,
                    &link.recipient.index_pepper_ref,
                    &link.recipient.subject_index,
                    &link.program,
                    &link.recipient.recipient_ref,
                ],
            )
            .await;
        if let Err(error) = &inserted {
            if violated_constraint(error)
                .is_some_and(|constraint| ONE_LINK_PER_STATE.contains(&constraint.as_str()))
            {
                return Ok(PayoutLinkInsert::StateTaken);
            }
        }
        inserted?;
        insert_audit(
            &tx,
            tenant,
            link.rail_id.as_str(),
            PayoutAuditAction::LinkCreated,
            &link.actor_ref_hash,
            Some(link.account_id),
            Some(link.link_id),
            None,
            Some(link.state),
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(PayoutLinkInsert::Inserted)
    }

    async fn transition_payout_link(
        &self,
        tenant: &str,
        link_id: Uuid,
        from: PayoutLinkState,
        to: PayoutLinkState,
        action: PayoutAuditAction,
        actor_ref_hash: &str,
        label: Option<&str>,
    ) -> Result<PayoutLinkTransition, DatabaseError> {
        if !action.fits_link_transition(from, to) {
            return Err(DatabaseError::Constraint(
                "payout_link_transition_invalid".into(),
            ));
        }
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        // A waiting link goes active only while the account has no active or
        // closing link on the rail: both states share the `one_live` index,
        // which refuses the move at the row, whatever races it.
        let row = tx
            .query_opt(
                "UPDATE trace_payout_links AS l
                    SET state = $4, version = l.version + 1, updated_at = clock_timestamp()
                  WHERE l.tenant_id = $1 AND l.link_id = $2 AND l.state = $3
                  RETURNING l.rail_id, l.account_id",
                &[&tenant, &link_id, &from.as_str(), &to.as_str()],
            )
            .await;
        let row = match row {
            Ok(row) => row,
            Err(error)
                if violated_constraint(&error).is_some_and(|constraint| {
                    ONE_LINK_PER_STATE.contains(&constraint.as_str())
                }) =>
            {
                return Ok(PayoutLinkTransition::LiveLinkTaken);
            }
            Err(error) => return Err(error.into()),
        };
        let Some(row) = row else {
            return Ok(PayoutLinkTransition::NotInState);
        };
        let rail: String = row.get(0);
        let account: Uuid = row.get(1);
        insert_audit(
            &tx,
            tenant,
            &rail,
            action,
            actor_ref_hash,
            Some(account),
            Some(link_id),
            Some(from),
            Some(to),
            label,
        )
        .await?;
        tx.commit().await?;
        Ok(PayoutLinkTransition::Moved)
    }

    async fn close_payout_link_streams(
        &self,
        tenant: &str,
        link_id: Uuid,
        final_paid: AtomicUnits,
        final_paid_referral: Option<AtomicUnits>,
        successor: Option<Uuid>,
        actor_ref_hash: &str,
        label: Option<&str>,
    ) -> Result<PayoutLinkClose, DatabaseError> {
        use PayoutLinkState::{Active, Closed, Closing, Waiting};
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let closed = tx
            .query_opt(
                "UPDATE trace_payout_links AS l
                    SET state = 'closed', final_paid = $3::TEXT::NUMERIC,
                        final_paid_referral = $4::TEXT::NUMERIC,
                        version = l.version + 1, updated_at = clock_timestamp()
                  WHERE l.tenant_id = $1 AND l.link_id = $2 AND l.state = 'closing'
                    AND (l.referral_program IS NULL) = ($4::TEXT IS NULL)
                    AND NOT EXISTS (
                        SELECT 1 FROM trace_payout_lines p
                         WHERE p.tenant_id = l.tenant_id AND p.link_id = l.link_id
                           AND p.state = 'pending')
                  RETURNING l.account_id, l.rail_id",
                &[
                    &tenant,
                    &link_id,
                    &final_paid.get().to_string(),
                    &final_paid_referral.map(|total| total.get().to_string()),
                ],
            )
            .await?;
        let Some(closed) = closed else {
            // Nothing changed; say why. The transaction rolls back on drop.
            let closing = tx
                .query_opt(
                    "SELECT referral_program IS NOT NULL FROM trace_payout_links
                      WHERE tenant_id = $1 AND link_id = $2 AND state = 'closing'",
                    &[&tenant, &link_id],
                )
                .await?;
            return Ok(match closing {
                None => PayoutLinkClose::NotClosing,
                Some(row) if row.get::<_, bool>(0) != final_paid_referral.is_some() => {
                    PayoutLinkClose::ReferralTotalMismatch
                }
                Some(_) => PayoutLinkClose::PendingLineOpen,
            });
        };
        let account: Uuid = closed.get(0);
        let rail: String = closed.get(1);
        insert_audit(
            &tx,
            tenant,
            &rail,
            PayoutAuditAction::LinkClosed,
            actor_ref_hash,
            Some(account),
            Some(link_id),
            Some(Closing),
            Some(Closed),
            label,
        )
        .await?;
        if let Some(successor) = successor {
            let activated = tx
                .execute(
                    "UPDATE trace_payout_links
                        SET state = 'active', version = version + 1,
                            updated_at = clock_timestamp()
                      WHERE tenant_id = $1 AND link_id = $2 AND state = 'waiting'
                        AND account_id = $3 AND rail_id = $4",
                    &[&tenant, &successor, &account, &rail],
                )
                .await?;
            if activated == 0 {
                return Ok(PayoutLinkClose::SuccessorNotWaiting);
            }
            insert_audit(
                &tx,
                tenant,
                &rail,
                PayoutAuditAction::LinkActivated,
                actor_ref_hash,
                Some(account),
                Some(successor),
                Some(Waiting),
                Some(Active),
                label,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(PayoutLinkClose::Closed)
    }

    async fn insert_pending_payout_line(
        &self,
        tenant: &str,
        line: &PayoutLineWrite,
    ) -> Result<PendingPayoutLineInsert, DatabaseError> {
        // An earned line names no program; a referral line names the link's
        // referral program, which is set here on the link's first one.
        match (line.stream, line.referral_program.as_deref()) {
            (PayoutStream::Earned, None) | (PayoutStream::Referral, Some(_)) => {}
            _ => return Err(DatabaseError::Query("payout_line_stream_invalid".into())),
        }
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        if let Some(program) = line.referral_program.as_deref() {
            // FOR UPDATE: the program is set once, under the link row's lock,
            // in the same transaction as the line it was set for.
            let current = tx
                .query_opt(
                    "SELECT referral_program FROM trace_payout_links
                      WHERE tenant_id = $1 AND link_id = $2 AND account_id = $3
                        AND rail_id = $4 AND state = 'active'
                        FOR UPDATE",
                    &[
                        &tenant,
                        &line.link_id,
                        &line.account_id,
                        &line.rail_id.as_str(),
                    ],
                )
                .await?;
            let Some(current) = current else {
                return Ok(PendingPayoutLineInsert::LinkNotActive);
            };
            match current.get::<_, Option<String>>(0) {
                Some(current) if current == program => {}
                Some(_) => return Ok(PendingPayoutLineInsert::ReferralProgramMismatch),
                None => {
                    tx.execute(
                        "UPDATE trace_payout_links SET referral_program = $3
                          WHERE tenant_id = $1 AND link_id = $2 AND referral_program IS NULL",
                        &[&tenant, &line.link_id, &program],
                    )
                    .await?;
                }
            }
        }
        // Only onto the account's active link on the line's rail: never a
        // waiting, closing or closed one. FOR SHARE holds the link row until
        // commit, so a concurrent close either waits for this line (and the
        // V132 link trigger then sees it pending and refuses) or commits
        // first (and this re-check finds the link no longer active).
        let inserted = tx
            .execute(
                "INSERT INTO trace_payout_lines (
                     tenant_id, line_id, account_id, rail_id, link_id, target_total,
                     delta, owed_through_accrual_count, state, stream
                 )
                 SELECT l.tenant_id, $2::UUID, l.account_id, l.rail_id, l.link_id,
                        $6::TEXT::NUMERIC, $7::TEXT::NUMERIC, $8::BIGINT, 'pending', $9::TEXT
                   FROM trace_payout_links l
                  WHERE l.tenant_id = $1::TEXT AND l.link_id = $5::UUID
                    AND l.account_id = $3::UUID AND l.rail_id = $4::TEXT
                    AND l.state = 'active'
                    FOR SHARE OF l",
                &[
                    &tenant,
                    &line.line_id,
                    &line.account_id,
                    &line.rail_id.as_str(),
                    &line.link_id,
                    &line.target_total.get().to_string(),
                    &line.delta.get().to_string(),
                    &line.owed_through_accrual_count,
                    &line.stream.as_str(),
                ],
            )
            .await;
        let inserted = match inserted {
            Ok(inserted) => inserted,
            Err(error)
                if violated_constraint(&error).as_deref()
                    == Some("trace_payout_lines_one_pending") =>
            {
                return Ok(PendingPayoutLineInsert::LinkHasPendingLine);
            }
            Err(error) => return Err(error.into()),
        };
        if inserted == 0 {
            return Ok(PendingPayoutLineInsert::LinkNotActive);
        }
        tx.commit().await?;
        Ok(PendingPayoutLineInsert::Inserted)
    }

    async fn pending_payout_line_for_stream(
        &self,
        tenant: &str,
        link_id: Uuid,
        stream: PayoutStream,
    ) -> Result<Option<PayoutLine>, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                &format!(
                    "SELECT {LINE_COLUMNS} FROM trace_payout_lines
                      WHERE tenant_id = $1 AND link_id = $2 AND state = 'pending'
                        AND stream = $3"
                ),
                &[&tenant, &link_id, &stream.as_str()],
            )
            .await?;
        tx.commit().await?;
        row.as_ref().map(line_from_row).transpose()
    }

    async fn resolve_payout_line(
        &self,
        tenant: &str,
        line_id: Uuid,
        resolution: &PayoutLineResolution,
    ) -> Result<PayoutLineResolve, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let updated = match resolution {
            PayoutLineResolution::Retry { label } => {
                tx.execute(
                    "UPDATE trace_payout_lines
                        SET attempts = attempts + 1, last_error_label = $3,
                            updated_at = clock_timestamp()
                      WHERE tenant_id = $1 AND line_id = $2 AND state = 'pending'",
                    &[&tenant, &line_id, label],
                )
                .await
            }
            PayoutLineResolution::Confirmed {
                receipt_hash,
                delta,
            } => {
                tx.execute(
                    "UPDATE trace_payout_lines
                        SET state = 'confirmed', receipt_hash = $3,
                            delta = $4::TEXT::NUMERIC, last_error_label = NULL,
                            updated_at = clock_timestamp(), resolved_at = clock_timestamp()
                      WHERE tenant_id = $1 AND line_id = $2 AND state = 'pending'",
                    &[&tenant, &line_id, receipt_hash, &delta.get().to_string()],
                )
                .await
            }
            PayoutLineResolution::NoOp => {
                tx.execute(
                    "UPDATE trace_payout_lines
                        SET state = 'no_op', updated_at = clock_timestamp(),
                            resolved_at = clock_timestamp()
                      WHERE tenant_id = $1 AND line_id = $2 AND state = 'pending'",
                    &[&tenant, &line_id],
                )
                .await
            }
            PayoutLineResolution::Held { label } | PayoutLineResolution::Failed { label } => {
                let state = if matches!(resolution, PayoutLineResolution::Held { .. }) {
                    PayoutLineState::Held
                } else {
                    PayoutLineState::Failed
                };
                tx.execute(
                    "UPDATE trace_payout_lines
                        SET state = $3, last_error_label = $4,
                            updated_at = clock_timestamp(), resolved_at = clock_timestamp()
                      WHERE tenant_id = $1 AND line_id = $2 AND state = 'pending'",
                    &[&tenant, &line_id, &state.as_str(), label],
                )
                .await
            }
        };
        let updated = match updated {
            Ok(updated) => updated,
            Err(error)
                if violated_constraint(&error).as_deref()
                    == Some("trace_payout_lines_one_receipt") =>
            {
                return Ok(PayoutLineResolve::ReceiptAlreadyClaimed);
            }
            Err(error) => return Err(error.into()),
        };
        if updated == 0 {
            return Ok(PayoutLineResolve::NotPending);
        }
        tx.commit().await?;
        Ok(PayoutLineResolve::Resolved)
    }

    async fn payout_receipt_is_claimed(
        &self,
        tenant: &str,
        rail: &RailId,
        receipt_hash: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM trace_payout_lines
                      WHERE tenant_id = $1 AND rail_id = $2 AND receipt_hash = $3)",
                &[&tenant, &rail.as_str(), &receipt_hash],
            )
            .await?;
        tx.commit().await?;
        Ok(row.get(0))
    }

    async fn open_payout_hold(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        label: &str,
        actor_ref_hash: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let opened = tx
            .execute(
                "INSERT INTO trace_payout_holds (tenant_id, hold_id, account_id, rail_id, label)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (tenant_id, account_id, rail_id, label)
                     WHERE cleared_at IS NULL DO NOTHING",
                &[&tenant, &Uuid::new_v4(), &account, &rail.as_str(), &label],
            )
            .await?;
        if opened == 0 {
            return Ok(false);
        }
        insert_audit(
            &tx,
            tenant,
            rail.as_str(),
            PayoutAuditAction::HoldSet,
            actor_ref_hash,
            Some(account),
            None,
            None,
            None,
            Some(label),
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn open_payout_reversal_hold(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        reversal_event_id: Uuid,
        label: &str,
        actor_ref_hash: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let decided = tx
            .execute(
                "INSERT INTO trace_payout_reversal_hold_decisions
                     (tenant_id, rail_id, reversal_event_id)
                 VALUES ($1, $2, $3)
                 ON CONFLICT (tenant_id, rail_id, reversal_event_id) DO NOTHING",
                &[&tenant, &rail.as_str(), &reversal_event_id],
            )
            .await?;
        if decided == 0 {
            // Decided on an earlier run: the hold is whatever it is now.
            return Ok(false);
        }
        let opened = tx
            .execute(
                "INSERT INTO trace_payout_holds (tenant_id, hold_id, account_id, rail_id, label)
                 VALUES ($1, $2, $3, $4, $5)
                 ON CONFLICT (tenant_id, account_id, rail_id, label)
                     WHERE cleared_at IS NULL DO NOTHING",
                &[&tenant, &Uuid::new_v4(), &account, &rail.as_str(), &label],
            )
            .await?;
        if opened > 0 {
            insert_audit(
                &tx,
                tenant,
                rail.as_str(),
                PayoutAuditAction::HoldSet,
                actor_ref_hash,
                Some(account),
                None,
                None,
                None,
                Some(label),
            )
            .await?;
        }
        // The decision commits even when the hold was already open, so this
        // event never reopens it after a clear.
        tx.commit().await?;
        Ok(opened > 0)
    }

    async fn clear_payout_hold(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        label: &str,
        actor_ref_hash: &str,
    ) -> Result<bool, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let cleared = tx
            .execute(
                "UPDATE trace_payout_holds SET cleared_at = clock_timestamp()
                  WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3 AND label = $4
                    AND cleared_at IS NULL",
                &[&tenant, &account, &rail.as_str(), &label],
            )
            .await?;
        if cleared == 0 {
            return Ok(false);
        }
        insert_audit(
            &tx,
            tenant,
            rail.as_str(),
            PayoutAuditAction::HoldCleared,
            actor_ref_hash,
            Some(account),
            None,
            None,
            None,
            Some(label),
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn open_payout_hold_labels(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<Vec<String>, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT label FROM trace_payout_holds
                  WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                    AND cleared_at IS NULL
                  ORDER BY label",
                &[&tenant, &account, &rail.as_str()],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    async fn open_payout_holds(
        &self,
        tenant: &str,
        rail: &RailId,
    ) -> Result<Vec<PayoutHold>, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT account_id, label FROM trace_payout_holds
                  WHERE tenant_id = $1 AND rail_id = $2 AND cleared_at IS NULL
                  ORDER BY account_id, label",
                &[&tenant, &rail.as_str()],
            )
            .await?;
        tx.commit().await?;
        Ok(rows
            .iter()
            .map(|row| PayoutHold {
                account_id: row.get(0),
                label: row.get(1),
            })
            .collect())
    }

    async fn insert_payout_link_request(
        &self,
        tenant: &str,
        request: &PayoutLinkRequestWrite,
    ) -> Result<PayoutLinkRequestInsert, DatabaseError> {
        let (kind, recipient) = match &request.recipient {
            Some((kind, recipient)) => (Some(kind.as_str()), Some(recipient)),
            None => (None, None),
        };
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let inserted = tx
            .execute(
                "INSERT INTO trace_payout_link_requests (
                     tenant_id, account_id, rail_id, idempotency_key, handle_hash,
                     request_digest, action, expected_current_version, recipient_kind,
                     sealed_recipient, recipient_index, recipient_key_ref,
                     index_pepper_ref, subject_index, expires_at, recipient_ref
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
                 ON CONFLICT (tenant_id, account_id, rail_id, idempotency_key) DO NOTHING",
                &[
                    &tenant,
                    &request.account_id,
                    &request.rail_id.as_str(),
                    &request.idempotency_key,
                    &request.handle_hash,
                    &request.request_digest,
                    &request.action.as_str(),
                    &request.expected_current_version,
                    &kind,
                    &recipient.map(|recipient| &recipient.sealed),
                    &recipient.map(|recipient| recipient.recipient_index.as_str()),
                    &recipient.map(|recipient| recipient.recipient_key_ref.as_str()),
                    &recipient.map(|recipient| recipient.index_pepper_ref.as_str()),
                    &recipient.map(|recipient| recipient.subject_index.as_str()),
                    &request.expires_at,
                    &recipient.and_then(|recipient| recipient.recipient_ref.as_deref()),
                ],
            )
            .await?;
        if inserted == 0 {
            let prior: String = tx
                .query_one(
                    "SELECT request_digest FROM trace_payout_link_requests
                      WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                        AND idempotency_key = $4",
                    &[
                        &tenant,
                        &request.account_id,
                        &request.rail_id.as_str(),
                        &request.idempotency_key,
                    ],
                )
                .await?
                .get(0);
            tx.commit().await?;
            return Ok(if prior == request.request_digest {
                PayoutLinkRequestInsert::Replayed
            } else {
                PayoutLinkRequestInsert::IdempotencyConflict
            });
        }
        let action = match request.action {
            PayoutLinkAction::Link => PayoutAuditAction::LinkRequested,
            PayoutLinkAction::Unlink => PayoutAuditAction::UnlinkRequested,
        };
        insert_audit(
            &tx,
            tenant,
            request.rail_id.as_str(),
            action,
            &request.actor_ref_hash,
            Some(request.account_id),
            None,
            None,
            None,
            None,
        )
        .await?;
        tx.commit().await?;
        Ok(PayoutLinkRequestInsert::Inserted)
    }

    async fn take_payout_link_request(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        handle_hash: &str,
    ) -> Result<Option<PayoutLinkRequest>, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                "DELETE FROM trace_payout_link_requests
                  WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                    AND handle_hash = $4
                  RETURNING account_id, rail_id, idempotency_key, action,
                            expected_current_version, recipient_kind, sealed_recipient,
                            recipient_index, recipient_key_ref, index_pepper_ref,
                            subject_index, expires_at, recipient_ref",
                &[&tenant, &account, &rail.as_str(), &handle_hash],
            )
            .await?;
        tx.commit().await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let action: String = row.get(3);
        let action = match action.as_str() {
            "link" => PayoutLinkAction::Link,
            "unlink" => PayoutLinkAction::Unlink,
            _ => return Err(DatabaseError::Query("payout_link_action_unknown".into())),
        };
        let kind: Option<String> = row.get(5);
        let recipient = match kind {
            Some(kind) => Some((
                parse_recipient_kind(kind)?,
                SealedRecipient {
                    sealed: row.get(6),
                    recipient_index: row.get(7),
                    recipient_key_ref: row.get(8),
                    index_pepper_ref: row.get(9),
                    subject_index: row.get(10),
                    recipient_ref: row.get(12),
                },
            )),
            None => None,
        };
        Ok(Some(PayoutLinkRequest {
            account_id: row.get(0),
            rail_id: parse_rail(row.get(1))?,
            idempotency_key: row.get(2),
            action,
            expected_current_version: row.get(4),
            recipient,
            expires_at: row.get(11),
        }))
    }

    async fn purge_expired_payout_link_requests(&self, tenant: &str) -> Result<u64, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let purged = tx
            .execute(
                "DELETE FROM trace_payout_link_requests
                  WHERE tenant_id = $1 AND expires_at <= clock_timestamp()",
                &[&tenant],
            )
            .await?;
        tx.commit().await?;
        Ok(purged)
    }

    async fn payout_account_link_view(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<PayoutAccountLinkView, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let version = account_link_version(&tx, tenant, account, rail.as_str()).await?;
        let newest = tx
            .query_opt(
                &format!(
                    "SELECT {LINK_COLUMNS} FROM trace_payout_links
                      WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                        AND state <> 'closed'
                      ORDER BY (state = 'waiting') DESC, created_at DESC
                      LIMIT 1"
                ),
                &[&tenant, &account, &rail.as_str()],
            )
            .await?
            .as_ref()
            .map(link_from_row)
            .transpose()?;
        let pending_request: bool = tx
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM trace_payout_link_requests
                      WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                        AND expires_at > clock_timestamp())",
                &[&tenant, &account, &rail.as_str()],
            )
            .await?
            .get(0);
        tx.commit().await?;
        Ok(PayoutAccountLinkView {
            version,
            newest,
            pending_request,
        })
    }

    async fn payout_account_login_anchors(
        &self,
        tenant: &str,
        account: Uuid,
    ) -> Result<Vec<String>, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT anchor_hash FROM trace_near_account_anchors
                  WHERE tenant_id = $1 AND account_id = $2
                    AND identity_source = 'near_ai_login'",
                &[&tenant, &account],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    async fn confirm_payout_link_request(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        handle_hash: &str,
        confirm: &PayoutLinkConfirmWrite,
    ) -> Result<PayoutLinkConfirm, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        // One confirm at a time per account and rail, so the version read
        // below and the write after it see the same links. The pass never
        // creates a link or an unlink, so it does not need this lock.
        tx.execute(
            "SELECT pg_advisory_xact_lock(hashtextextended(
                 'payout-link-confirm-v1:' || $1 || ':' || $2::UUID::TEXT || ':' || $3, 0))",
            &[&tenant, &account, &rail.as_str()],
        )
        .await?;
        let Some(row) = tx
            .query_opt(
                "DELETE FROM trace_payout_link_requests
                  WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                    AND handle_hash = $4
                  RETURNING action, expected_current_version, recipient_kind,
                            sealed_recipient, recipient_index, recipient_key_ref,
                            index_pepper_ref, subject_index, recipient_ref,
                            expires_at <= clock_timestamp(), created_at",
                &[&tenant, &account, &rail.as_str(), &handle_hash],
            )
            .await?
        else {
            tx.commit().await?;
            return Ok(PayoutLinkConfirm::NotFound);
        };
        let expired: bool = row.get(9);
        if expired {
            tx.commit().await?;
            return Ok(PayoutLinkConfirm::Expired);
        }
        let created_at: chrono::DateTime<chrono::Utc> = row.get(10);
        let superseded: bool = tx
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM trace_payout_link_requests
                      WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                        AND created_at > $4 AND expires_at > clock_timestamp())",
                &[&tenant, &account, &rail.as_str(), &created_at],
            )
            .await?
            .get(0);
        if superseded {
            tx.commit().await?;
            return Ok(PayoutLinkConfirm::NotFound);
        }
        let expected: Option<i64> = row.get(1);
        let version = account_link_version(&tx, tenant, account, rail.as_str()).await?;
        if expected.unwrap_or(0) != version {
            tx.commit().await?;
            return Ok(PayoutLinkConfirm::VersionConflict);
        }
        let action: String = row.get(0);
        let action = match action.as_str() {
            "link" => PayoutLinkAction::Link,
            "unlink" => PayoutLinkAction::Unlink,
            _ => return Err(DatabaseError::Query("payout_link_action_unknown".into())),
        };
        let waiting: bool = tx
            .query_one(
                "SELECT EXISTS (
                     SELECT 1 FROM trace_payout_links
                      WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                        AND state = 'waiting')",
                &[&tenant, &account, &rail.as_str()],
            )
            .await?
            .get(0);
        match action {
            PayoutLinkAction::Link => {
                if waiting {
                    tx.commit().await?;
                    return Ok(PayoutLinkConfirm::ChangePending);
                }
                let kind: Option<String> = row.get(2);
                let kind =
                    parse_recipient_kind(kind.ok_or_else(|| {
                        DatabaseError::Query("payout_link_request_shape".into())
                    })?)?;
                let recipient_ref: Option<String> = row.get(8);
                tx.execute(
                    "INSERT INTO trace_payout_links (
                         tenant_id, account_id, rail_id, link_id, recipient_kind, state,
                         sealed_recipient, recipient_index, recipient_key_ref,
                         index_pepper_ref, subject_index, program, recipient_ref
                     ) VALUES ($1, $2, $3, $4, $5, 'waiting', $6, $7, $8, $9, $10, $11, $12)",
                    &[
                        &tenant,
                        &account,
                        &rail.as_str(),
                        &confirm.link_id,
                        &kind.as_str(),
                        &row.get::<_, Option<serde_json::Value>>(3),
                        &row.get::<_, Option<String>>(4),
                        &row.get::<_, Option<String>>(5),
                        &row.get::<_, Option<String>>(6),
                        &row.get::<_, Option<String>>(7),
                        &confirm.program,
                        &recipient_ref,
                    ],
                )
                .await?;
                for audit in [
                    PayoutAuditAction::LinkCreated,
                    PayoutAuditAction::LinkConfirmed,
                ] {
                    insert_audit(
                        &tx,
                        tenant,
                        rail.as_str(),
                        audit,
                        &confirm.actor_ref_hash,
                        Some(account),
                        Some(confirm.link_id),
                        None,
                        Some(PayoutLinkState::Waiting),
                        Some(action.as_str()),
                    )
                    .await?;
                }
            }
            PayoutLinkAction::Unlink => {
                let active = tx
                    .query_opt(
                        "UPDATE trace_payout_links
                            SET unlink_requested_at = clock_timestamp(),
                                version = version + 1, updated_at = clock_timestamp()
                          WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                            AND state = 'active' AND unlink_requested_at IS NULL
                          RETURNING link_id",
                        &[&tenant, &account, &rail.as_str()],
                    )
                    .await?;
                let Some(active) = active else {
                    let marked: bool = tx
                        .query_one(
                            "SELECT EXISTS (
                                 SELECT 1 FROM trace_payout_links
                                  WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                                    AND state = 'active' AND unlink_requested_at IS NOT NULL)",
                            &[&tenant, &account, &rail.as_str()],
                        )
                        .await?
                        .get(0);
                    tx.commit().await?;
                    return Ok(if waiting || marked {
                        PayoutLinkConfirm::ChangePending
                    } else {
                        PayoutLinkConfirm::NotLinked
                    });
                };
                let link_id: Uuid = active.get(0);
                insert_audit(
                    &tx,
                    tenant,
                    rail.as_str(),
                    PayoutAuditAction::LinkConfirmed,
                    &confirm.actor_ref_hash,
                    Some(account),
                    Some(link_id),
                    Some(PayoutLinkState::Active),
                    Some(PayoutLinkState::Active),
                    Some(action.as_str()),
                )
                .await?;
            }
        }
        let version = account_link_version(&tx, tenant, account, rail.as_str()).await?;
        tx.commit().await?;
        Ok(PayoutLinkConfirm::Confirmed { action, version })
    }

    async fn payout_audit_actions(
        &self,
        tenant: &str,
        rail: &RailId,
    ) -> Result<Vec<PayoutAuditAction>, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let rows = tx
            .query(
                "SELECT action FROM trace_payout_audit
                  WHERE tenant_id = $1 AND rail_id = $2
                  ORDER BY created_at, audit_id",
                &[&tenant, &rail.as_str()],
            )
            .await?;
        tx.commit().await?;
        rows.iter()
            .map(|row| {
                let action: String = row.get(0);
                PayoutAuditAction::parse(&action)
                    .ok_or_else(|| DatabaseError::Query("payout_audit_action_unknown".into()))
            })
            .collect()
    }

    async fn record_payout_rate_epoch(
        &self,
        rail: &RailId,
        rate: &RailRate,
        pricing_label: Option<&str>,
    ) -> Result<PayoutRateEpochRecord, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL ROLE trace_payout_config_writer")
            .await?;
        let numerator = rate.numerator().to_string();
        let denominator = rate.denominator().to_string();
        let inserted = tx
            .execute(
                "INSERT INTO trace_payout_rate_epochs
                     (rail_id, version, numerator, denominator, pricing_label)
                 VALUES ($1, $2, $3::TEXT::NUMERIC, $4::TEXT::NUMERIC, $5)
                 ON CONFLICT (rail_id, version) DO NOTHING",
                &[
                    &rail.as_str(),
                    &rate.version(),
                    &numerator,
                    &denominator,
                    &pricing_label,
                ],
            )
            .await?;
        let outcome = if inserted == 1 {
            PayoutRateEpochRecord::Recorded
        } else {
            let row = tx
                .query_one(
                    "SELECT numerator::TEXT, denominator::TEXT FROM trace_payout_rate_epochs
                      WHERE rail_id = $1 AND version = $2",
                    &[&rail.as_str(), &rate.version()],
                )
                .await?;
            let stored: (String, String) = (row.get(0), row.get(1));
            if stored == (numerator, denominator) {
                PayoutRateEpochRecord::AlreadyRecorded
            } else {
                PayoutRateEpochRecord::Conflict
            }
        };
        tx.commit().await?;
        Ok(outcome)
    }

    async fn payout_rate_epochs(
        &self,
        rail: &RailId,
    ) -> Result<Vec<PayoutRateEpoch>, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let rows = client
            .query(
                "SELECT version, numerator::TEXT, denominator::TEXT, pricing_label, observed_at
                   FROM trace_payout_rate_epochs
                  WHERE rail_id = $1
                  ORDER BY observed_at, version",
                &[&rail.as_str()],
            )
            .await?;
        rows.iter()
            .map(|row| {
                let version: String = row.get(0);
                let numerator: String = row.get(1);
                let denominator: String = row.get(2);
                let rate = RailRate::new(
                    version,
                    parse_units(&numerator)?,
                    parse_units(&denominator)?,
                )
                .map_err(|_| DatabaseError::Query("payout_rate_epoch_invalid".into()))?;
                Ok(PayoutRateEpoch {
                    rate,
                    pricing_label: row.get(3),
                    observed_at: row.get(4),
                })
            })
            .collect()
    }

    async fn payout_control(&self, rail: &RailId) -> Result<PayoutControl, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let row = client
            .query_opt(
                "SELECT halted, label, changed_at FROM trace_payout_control
                  WHERE rail_id = $1
                  ORDER BY seq DESC
                  LIMIT 1",
                &[&rail.as_str()],
            )
            .await?;
        Ok(match row {
            Some(row) => PayoutControl {
                halted: row.get(0),
                label: row.get(1),
                changed_at: Some(row.get(2)),
            },
            None => PayoutControl::default(),
        })
    }

    async fn set_payout_control(
        &self,
        rail: &RailId,
        halted: bool,
        label: Option<&str>,
        actor_ref_hash: &str,
    ) -> Result<(), DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL ROLE trace_payout_config_writer")
            .await?;
        tx.execute(
            "INSERT INTO trace_payout_control (rail_id, change_id, halted, label, actor_ref_hash)
             VALUES ($1, $2, $3, $4, $5)",
            &[
                &rail.as_str(),
                &Uuid::new_v4(),
                &halted,
                &label,
                &actor_ref_hash,
            ],
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    async fn tenants_with_live_payout_links(
        &self,
        rail: &RailId,
    ) -> Result<Vec<String>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.batch_execute("SET LOCAL ROLE trace_payout_enumerator")
            .await?;
        let rows = tx
            .query(
                "SELECT DISTINCT tenant_id FROM trace_payout_links
                  WHERE rail_id = $1 AND state IN ('waiting', 'active', 'closing')
                  ORDER BY tenant_id",
                &[&rail.as_str()],
            )
            .await?;
        tx.commit().await?;
        Ok(rows.iter().map(|row| row.get(0)).collect())
    }

    async fn payout_account_standing(
        &self,
        tenant: &str,
        account: Uuid,
    ) -> Result<PayoutAccountStanding, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        // A credit hold names a settlement key (`account:{id}`) or a
        // principal; either, if it is open and points at this account,
        // holds every rail (spec section 4, "Holds").
        // Use the same UUID parser as settlement, including compact, braced,
        // URN and uppercase spellings accepted by that boundary.
        let row = tx
            .query_one(
                "SELECT
                     EXISTS (SELECT 1 FROM trace_accounts
                              WHERE tenant_id = $1 AND account_id = $2),
                     EXISTS (SELECT 1 FROM trace_accounts
                              WHERE tenant_id = $1 AND account_id = $2
                                AND closed_at IS NOT NULL),
                     EXISTS (SELECT 1 FROM trace_credit_holds h
                              WHERE h.tenant_id = $1 AND h.released_at IS NULL
                                AND (lower(h.credit_account_ref) = 'account:' || $2::UUID::TEXT
                                     OR h.credit_account_ref IN (
                                         SELECT p.principal_ref FROM trace_account_owned_principals p
                                          WHERE p.tenant_id = $1 AND p.account_id = $2)
                                     OR EXISTS (
                                         SELECT 1 FROM trace_payout_accruals accrued
                                          WHERE accrued.tenant_id = $1 AND accrued.account_id = $2
                                            AND accrued.credit_account_hash = 'sha256:' ||
                                                encode(sha256(convert_to(h.credit_account_ref, 'UTF8')), 'hex')))),
                     ARRAY(SELECT credit_account_ref FROM trace_credit_holds
                            WHERE tenant_id = $1 AND released_at IS NULL)",
                &[&tenant, &account],
            )
            .await?;
        tx.commit().await?;
        Ok(PayoutAccountStanding {
            exists: row.get(0),
            closed: row.get(1),
            credit_held: row.get::<_, bool>(2)
                || row
                    .get::<_, Vec<String>>(3)
                    .iter()
                    .any(|reference| super::settlement_key_account_id(reference) == Some(account)),
        })
    }

    async fn payout_link_stream_confirmed_total(
        &self,
        tenant: &str,
        link_id: Uuid,
        stream: PayoutStream,
    ) -> Result<u128, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_one(
                "SELECT COALESCE(SUM(delta), 0)::TEXT FROM trace_payout_lines
                  WHERE tenant_id = $1 AND link_id = $2 AND state = 'confirmed'
                    AND stream = $3",
                &[&tenant, &link_id, &stream.as_str()],
            )
            .await?;
        tx.commit().await?;
        let total: String = row.get(0);
        parse_units(&total)
    }

    async fn payout_account_stream_sent(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
        stream: PayoutStream,
    ) -> Result<u128, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_one(
                "SELECT COALESCE(SUM(delta), 0)::TEXT FROM trace_payout_lines
                  WHERE tenant_id = $1 AND account_id = $2 AND rail_id = $3
                    AND state = 'confirmed' AND stream = $4",
                &[&tenant, &account, &rail.as_str(), &stream.as_str()],
            )
            .await?;
        tx.commit().await?;
        let total: String = row.get(0);
        parse_units(&total)
    }

    async fn payout_referral_owed(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<PayoutReferralOwed, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_one(
                "SELECT COALESCE(SUM(bonus_units), 0)::TEXT, count(*)
                   FROM trace_referral_bonus_accruals
                  WHERE tenant_id = $1 AND rail_id = $2 AND account_id = $3",
                &[&tenant, &rail.as_str(), &account],
            )
            .await?;
        tx.commit().await?;
        let net: String = row.get(0);
        Ok(PayoutReferralOwed {
            net: parse_signed_units(&net)?,
            bonus_count: row.get(1),
        })
    }

    async fn payout_closed_links_final_paid_referral(
        &self,
        tenant: &str,
        account: Uuid,
        rail: &RailId,
    ) -> Result<u128, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_one(
                "SELECT COALESCE(SUM(final_paid_referral), 0)::TEXT
                   FROM trace_payout_links
                  WHERE tenant_id = $1 AND rail_id = $2 AND account_id = $3
                    AND state = 'closed'",
                &[&tenant, &rail.as_str(), &account],
            )
            .await?;
        tx.commit().await?;
        let total: String = row.get(0);
        parse_units(&total)
    }

    async fn try_acquire_payout_pass_lock(
        &self,
        rail: &RailId,
    ) -> Result<Option<PayoutPassLock>, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let acquired = client
            .query_one(PAYOUT_PASS_TRY_LOCK_SQL, &[&rail.as_str()])
            .await;
        match acquired {
            Ok(row) if row.get::<_, bool>(0) => Ok(Some(PayoutPassLock {
                client: Some(client),
                rail: rail.as_str().to_owned(),
            })),
            // Held by another pass: the connection holds nothing, so it
            // goes back to the pool.
            Ok(_) => Ok(None),
            Err(error) => {
                // Whether the lock was taken is unknown; close the session
                // rather than return it to the pool possibly holding it.
                drop(deadpool_postgres::Object::take(client));
                Err(error.into())
            }
        }
    }
}

const PAYOUT_PASS_TRY_LOCK_SQL: &str =
    "SELECT pg_try_advisory_lock(hashtextextended('payout-pass-v1:' || $1::TEXT, 0))";
const PAYOUT_PASS_UNLOCK_SQL: &str =
    "SELECT pg_advisory_unlock(hashtextextended('payout-pass-v1:' || $1::TEXT, 0))";

/// One rail's single-flight pass lock: a session-level advisory lock on a
/// pooled connection pinned for the whole pass (spec section 4, "One pass,
/// per rail", step 0). Release it with [`PayoutPassLock::release`]. Dropped
/// without that (a panic, a cancelled task), it closes its connection
/// instead of returning it to the pool, so the database ends the session and
/// the lock with it; a pooled connection must never keep a pass lock.
pub struct PayoutPassLock {
    client: Option<deadpool_postgres::Object>,
    rail: String,
}

impl std::fmt::Debug for PayoutPassLock {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PayoutPassLock")
            .field("rail", &self.rail)
            .field("held", &self.client.is_some())
            .finish()
    }
}

impl PayoutPassLock {
    /// Unlocks on the connection that took the lock, then returns the
    /// connection to the pool. On any doubt (the unlock fails, or reports the
    /// lock was not held) the connection is closed instead.
    pub async fn release(mut self) -> Result<(), DatabaseError> {
        let Some(client) = self.client.take() else {
            return Ok(());
        };
        match client
            .query_one(PAYOUT_PASS_UNLOCK_SQL, &[&self.rail.as_str()])
            .await
        {
            Ok(row) if row.get::<_, bool>(0) => Ok(()),
            Ok(_) => {
                drop(deadpool_postgres::Object::take(client));
                Err(DatabaseError::Query("payout_pass_lock_not_held".into()))
            }
            Err(error) => {
                drop(deadpool_postgres::Object::take(client));
                Err(error.into())
            }
        }
    }
}

impl Drop for PayoutPassLock {
    fn drop(&mut self) {
        if let Some(client) = self.client.take() {
            tracing::warn!(
                rail = %self.rail,
                "payout pass lock dropped without release; closing its connection"
            );
            drop(deadpool_postgres::Object::take(client));
        }
    }
}
