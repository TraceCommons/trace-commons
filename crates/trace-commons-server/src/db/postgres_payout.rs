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

use std::collections::BTreeSet;

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio_postgres::error::SqlState;
use trace_commons_gate_api::pipeline::AtomicUnits;
use trace_commons_gate_api::{RailId, RailRate, RecipientKind};
use uuid::Uuid;

use super::postgres::PgBackend;
use super::{
    PayoutAccrualKind, PayoutAccrualWrite, PayoutAuditAction, PayoutControl, PayoutLine,
    PayoutLineResolution, PayoutLineResolve, PayoutLineState, PayoutLineWrite, PayoutLink,
    PayoutLinkAction, PayoutLinkInsert, PayoutLinkRequest, PayoutLinkRequestInsert,
    PayoutLinkRequestWrite, PayoutLinkState, PayoutLinkWrite, PayoutOwed, PayoutRateEpoch,
    PayoutRateEpochRecord, PayoutStore, PendingPayoutLineInsert, SealedRecipient,
};
use crate::error::DatabaseError;

const ONE_LINK_PER_STATE: [&str; 3] = [
    "trace_payout_links_one_active",
    "trace_payout_links_one_waiting",
    "trace_payout_links_one_closing",
];

/// The audit row's hash of a link id. Hash-only: the link id itself is not
/// written to the audit table.
fn link_id_hash(link_id: Uuid) -> String {
    format!(
        "sha256:{:x}",
        Sha256::digest(format!("payout-link-v1:{link_id}").as_bytes())
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

fn parse_recipient_kind(value: String) -> Result<RecipientKind, DatabaseError> {
    RecipientKind::new(value)
        .map_err(|_| DatabaseError::Query("payout_recipient_kind_unknown".into()))
}

#[allow(clippy::too_many_arguments)]
async fn insert_audit(
    tx: &deadpool_postgres::Transaction<'_>,
    tenant: &str,
    rail: &str,
    action: PayoutAuditAction,
    actor_ref_hash: &str,
    link_id: Option<Uuid>,
    prior: Option<PayoutLinkState>,
    new: Option<PayoutLinkState>,
    label: Option<&str>,
) -> Result<(), DatabaseError> {
    tx.execute(
        "INSERT INTO trace_payout_audit (
             tenant_id, audit_id, rail_id, action, actor_ref_hash, link_id_hash,
             prior_state, new_state, label
         ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        &[
            &tenant,
            &Uuid::new_v4(),
            &rail,
            &action.as_str(),
            &actor_ref_hash,
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
    program, final_paid::TEXT, unlink_requested_at, recipient_unavailable_at, version";

fn link_from_row(row: &tokio_postgres::Row) -> Result<PayoutLink, DatabaseError> {
    let final_paid: Option<String> = row.get(11);
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
        },
        program: row.get(10),
        final_paid: final_paid
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
    delta::TEXT, state, receipt_hash, attempts, last_error_label";

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
                  RETURNING rail_id",
                &[&tenant, &credit_account_hash, &account],
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
                     index_pepper_ref, subject_index, program
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
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
        final_paid: Option<AtomicUnits>,
        actor_ref_hash: &str,
        label: Option<&str>,
    ) -> Result<bool, DatabaseError> {
        use PayoutLinkState::{Active, Closed, Closing, Waiting};
        if !matches!(
            (from, to),
            (Waiting, Active) | (Active, Closing) | (Closing, Closed)
        ) {
            return Err(DatabaseError::Constraint(
                "payout_link_transition_invalid".into(),
            ));
        }
        if (to == Closed) != final_paid.is_some() {
            return Err(DatabaseError::Constraint(
                "payout_link_final_paid_required".into(),
            ));
        }
        let final_paid = final_paid.map(|units| units.get().to_string());
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                "UPDATE trace_payout_links
                    SET state = $4, final_paid = $5::TEXT::NUMERIC,
                        version = version + 1, updated_at = clock_timestamp()
                  WHERE tenant_id = $1 AND link_id = $2 AND state = $3
                  RETURNING rail_id",
                &[&tenant, &link_id, &from.as_str(), &to.as_str(), &final_paid],
            )
            .await
            .map_err(|error| match violated_constraint(&error) {
                Some(constraint) if ONE_LINK_PER_STATE.contains(&constraint.as_str()) => {
                    DatabaseError::Constraint("payout_link_state_taken".into())
                }
                _ => DatabaseError::Postgres(error),
            })?;
        let Some(row) = row else {
            return Ok(false);
        };
        let rail: String = row.get(0);
        insert_audit(
            &tx,
            tenant,
            &rail,
            PayoutAuditAction::for_link_state(to),
            actor_ref_hash,
            Some(link_id),
            Some(from),
            Some(to),
            label,
        )
        .await?;
        tx.commit().await?;
        Ok(true)
    }

    async fn insert_pending_payout_line(
        &self,
        tenant: &str,
        line: &PayoutLineWrite,
    ) -> Result<PendingPayoutLineInsert, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let inserted = tx
            .execute(
                "INSERT INTO trace_payout_lines (
                     tenant_id, line_id, account_id, rail_id, link_id, target_total,
                     delta, owed_through_accrual_count, state
                 ) VALUES ($1, $2, $3, $4, $5, $6::TEXT::NUMERIC, $7::TEXT::NUMERIC, $8,
                           'pending')",
                &[
                    &tenant,
                    &line.line_id,
                    &line.account_id,
                    &line.rail_id.as_str(),
                    &line.link_id,
                    &line.target_total.get().to_string(),
                    &line.delta.get().to_string(),
                    &line.owed_through_accrual_count,
                ],
            )
            .await;
        if let Err(error) = &inserted {
            if violated_constraint(error).as_deref() == Some("trace_payout_lines_one_pending") {
                return Ok(PendingPayoutLineInsert::LinkHasPendingLine);
            }
        }
        inserted?;
        tx.commit().await?;
        Ok(PendingPayoutLineInsert::Inserted)
    }

    async fn pending_payout_line_for_link(
        &self,
        tenant: &str,
        link_id: Uuid,
    ) -> Result<Option<PayoutLine>, DatabaseError> {
        let mut client = self.payout_tenant_client().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant).await?;
        let row = tx
            .query_opt(
                &format!(
                    "SELECT {LINE_COLUMNS} FROM trace_payout_lines
                      WHERE tenant_id = $1 AND link_id = $2 AND state = 'pending'"
                ),
                &[&tenant, &link_id],
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
            None,
            None,
            None,
            Some(label),
        )
        .await?;
        tx.commit().await?;
        Ok(true)
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
                     index_pepper_ref, subject_index, expires_at
                 ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
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
                            subject_index, expires_at",
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
                  ORDER BY changed_at DESC, change_id DESC
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
}
