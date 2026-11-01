// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! A contributor adds the `model_training` use to traces already sent
//! (V141, owner decisions 2026-10-28).
//!
//! The grant is add-only: it appends `model_training` to a trace's stored
//! consent scopes and allowed uses and removes nothing. Each grant is one
//! dated `trace_consent_grants` row, written in the same transaction as the
//! submission row it changes, under that row's lock.

use super::*;
use crate::db::{
    ModelTrainingConsentEligible, ModelTrainingConsentGrant, ModelTrainingConsentGrantRow,
    WithheldGateAward,
};
use chrono::{DateTime, Utc};
use trace_commons_protocol::model_training_consent::ModelTrainingConsentOutcome;

/// The stored value of the one scope, and the one use, this path adds.
const MODEL_TRAINING: &str = "model_training";

/// The withheld reason a grant re-runs: the award refused for lack of the
/// use (ingest's `policy_mismatch`).
const POLICY_MISMATCH: &str = "policy_mismatch";

/// The reason the award emitter records when its checks fail outright
/// (ingest's `CREDIT_CHECK_ERROR_LABEL`). A re-run that failed that way left
/// this on the row; matching it lets a repeat grant try again, and every
/// check reapplies.
const CREDIT_CHECK_ERROR: &str = "credit_check_error";

/// A trace (aliased `s`) that may take the use: `accepted`, and not revoked,
/// purged, expired or withdrawn -- by `withdrawn_at`, a `trace_withdrawals`
/// row, or a withdrawn source session.
macro_rules! live_accepted_submission_sql {
    () => {
        "s.status = 'accepted'
         AND s.revoked_at IS NULL AND s.purged_at IS NULL AND s.withdrawn_at IS NULL
         AND (s.expires_at IS NULL OR s.expires_at > now())
         AND NOT EXISTS (
             SELECT 1 FROM trace_withdrawals w
              WHERE w.tenant_id = s.tenant_id AND w.submission_id = s.submission_id)
         AND NOT EXISTS (
             SELECT 1 FROM trace_submission_sessions mapping
               JOIN trace_source_sessions source
                 ON source.tenant_id = mapping.tenant_id
                AND source.account_id = mapping.account_id
                AND source.session_digest = mapping.session_digest
              WHERE mapping.tenant_id = s.tenant_id
                AND mapping.submission_id = s.submission_id
                AND source.withdrawn_at IS NOT NULL)"
    };
}

impl PgBackend {
    pub(super) async fn model_training_consent_eligible(
        &self,
        tenant_id: &str,
        owner_principal_refs: &[String],
        limit: i64,
    ) -> Result<ModelTrainingConsentEligible, DatabaseError> {
        if owner_principal_refs.is_empty() || limit <= 0 {
            return Ok(ModelTrainingConsentEligible::default());
        }
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // One statement, one snapshot: the count and the ids agree.
        let rows = tx
            .query(
                concat!(
                    "SELECT s.submission_id, count(*) OVER () AS eligible_count
                       FROM trace_submissions s
                      WHERE s.tenant_id = trace_current_tenant_id()
                        AND s.auth_principal_ref = ANY($1)
                        AND NOT (s.allowed_uses ? 'model_training')
                        AND ",
                    live_accepted_submission_sql!(),
                    " ORDER BY s.received_at, s.submission_id
                      LIMIT $2"
                ),
                &[&owner_principal_refs, &limit],
            )
            .await?;
        tx.commit().await?;
        // `LIMIT` cuts the rows, not the window count, but with no row there
        // is no count to read: zero eligible.
        let eligible_count = rows
            .first()
            .map(|row| row.get::<_, i64>("eligible_count"))
            .unwrap_or(0);
        Ok(ModelTrainingConsentEligible {
            eligible_count: u64::try_from(eligible_count).unwrap_or(0),
            submission_ids: rows.iter().map(|row| row.get("submission_id")).collect(),
        })
    }

    pub(super) async fn model_training_consent_grant(
        &self,
        grant: ModelTrainingConsentGrant<'_>,
    ) -> Result<Vec<ModelTrainingConsentGrantRow>, DatabaseError> {
        let mut results = Vec::with_capacity(grant.submission_ids.len());
        if grant.owner_principal_refs.is_empty() {
            results.extend(
                grant
                    .submission_ids
                    .iter()
                    .map(|id| ModelTrainingConsentGrantRow {
                        submission_id: *id,
                        outcome: ModelTrainingConsentOutcome::NotEligible,
                        granted_at: None,
                    }),
            );
            return Ok(results);
        }
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, grant.tenant_id).await?;
        for submission_id in grant.submission_ids {
            // Lock the row first, so a withdrawal or revocation racing this
            // grant either lands before (and the trace reads not eligible)
            // or waits for this transaction.
            let row = tx
                .query_opt(
                    concat!(
                        "SELECT s.auth_principal_ref = ANY($2) AS owned,
                                s.allowed_uses ? 'model_training' AS has_use,
                                (",
                        live_accepted_submission_sql!(),
                        ") AS live
                           FROM trace_submissions s
                          WHERE s.tenant_id = trace_current_tenant_id()
                            AND s.submission_id = $1
                          FOR UPDATE OF s"
                    ),
                    &[submission_id, &grant.owner_principal_refs],
                )
                .await?;
            let outcome = match row {
                Some(row) if row.get::<_, bool>("owned") => {
                    if row.get::<_, bool>("has_use") {
                        ModelTrainingConsentOutcome::AlreadyGranted
                    } else if row.get::<_, bool>("live") {
                        ModelTrainingConsentOutcome::Granted
                    } else {
                        ModelTrainingConsentOutcome::NotEligible
                    }
                }
                _ => ModelTrainingConsentOutcome::NotEligible,
            };
            let granted_at = match outcome {
                ModelTrainingConsentOutcome::NotEligible => None,
                ModelTrainingConsentOutcome::AlreadyGranted => {
                    recorded_grant_time(&tx, *submission_id).await?
                }
                ModelTrainingConsentOutcome::Granted => {
                    // Append, never rewrite: what the trace carried stays,
                    // in order, and the scope is added only when missing.
                    tx.execute(
                        "UPDATE trace_submissions
                            SET consent_scopes = CASE
                                    WHEN consent_scopes ? $2 THEN consent_scopes
                                    ELSE consent_scopes || jsonb_build_array($2::text)
                                END,
                                allowed_uses = allowed_uses || jsonb_build_array($2::text),
                                updated_at = now()
                          WHERE tenant_id = trace_current_tenant_id()
                            AND submission_id = $1",
                        &[submission_id, &MODEL_TRAINING],
                    )
                    .await?;
                    tx.execute(
                        "INSERT INTO trace_consent_grants
                             (tenant_id, submission_id, consent_scope, allowed_use,
                              actor_principal_ref, granted_at)
                         VALUES (trace_current_tenant_id(), $1, $2, $2, $3, $4)
                         ON CONFLICT (tenant_id, submission_id, consent_scope) DO NOTHING",
                        &[
                            submission_id,
                            &MODEL_TRAINING,
                            &grant.actor_principal_ref,
                            &grant.granted_at,
                        ],
                    )
                    .await?;
                    recorded_grant_time(&tx, *submission_id).await?
                }
            };
            results.push(ModelTrainingConsentGrantRow {
                submission_id: *submission_id,
                outcome,
                granted_at,
            });
        }
        tx.commit().await?;
        Ok(results)
    }

    pub(super) async fn model_training_withheld_gate_award(
        &self,
        tenant_id: &str,
        submission_id: Uuid,
    ) -> Result<Option<WithheldGateAward>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        // The latest decision decides, as every latest-decision read does,
        // with `decision_id` as the final tiebreaker.
        let row = tx
            .query_opt(
                "SELECT decision_id, gate_policy_version, gate_version_hash,
                        vector_entry_id, chunks_capped, perplexity_passed,
                        novelty_passed, credit_withheld_reason, source
                   FROM trace_gate_decisions
                  WHERE tenant_id = trace_current_tenant_id()
                    AND submission_id = $1
                  ORDER BY decided_at DESC, decision_id DESC
                  LIMIT 1",
                &[&submission_id],
            )
            .await?;
        tx.commit().await?;
        let Some(row) = row else {
            return Ok(None);
        };
        let withheld_for_the_use = row.get::<_, bool>("perplexity_passed")
            && row.get::<_, bool>("novelty_passed")
            && matches!(
                row.get::<_, Option<String>>("credit_withheld_reason")
                    .as_deref(),
                Some(POLICY_MISMATCH | CREDIT_CHECK_ERROR)
            )
            && row.get::<_, String>("source") == "legacy_gate";
        if !withheld_for_the_use {
            return Ok(None);
        }
        Ok(Some(WithheldGateAward {
            decision_id: row.get("decision_id"),
            gate_policy_version: row.get("gate_policy_version"),
            gate_version_hash: row.get("gate_version_hash"),
            vector_entry_id: row.get("vector_entry_id"),
            chunks_capped: row.get::<_, Option<bool>>("chunks_capped").unwrap_or(false),
        }))
    }
}

/// The recorded time of the trace's model-training grant, if one was made
/// through this path (a trace sent with the use has none).
async fn recorded_grant_time(
    tx: &deadpool_postgres::Transaction<'_>,
    submission_id: Uuid,
) -> Result<Option<DateTime<Utc>>, DatabaseError> {
    Ok(tx
        .query_opt(
            "SELECT granted_at FROM trace_consent_grants
              WHERE tenant_id = trace_current_tenant_id()
                AND submission_id = $1
                AND consent_scope = 'model_training'",
            &[&submission_id],
        )
        .await?
        .map(|row| row.get("granted_at")))
}
