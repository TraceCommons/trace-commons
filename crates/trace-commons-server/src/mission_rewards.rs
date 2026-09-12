// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Operator-reviewed mission and insight awards, independent of Trace Credit.
//!
//! PostgreSQL binds operator authority to the authenticated database login and
//! owns every transition. Participants are operator-asserted pseudonyms. These
//! program units have no redemption or automated completion-verification path.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio_postgres::types::ToSql;
use uuid::Uuid;

use crate::db::postgres::PgBackend;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RewardActivityKind {
    MissionCompletion,
    InsightContribution,
}

/// Immutable operator-published terms. Digests identify externally retained
/// rules and evidence; the ledger does not infer their truth or qualification.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RewardProgramTerms {
    pub schema_version: i32,
    pub activity_kind: RewardActivityKind,
    pub definition_hash: String,
    pub rubric_hash: String,
    pub evaluator_policy_hash: String,
    pub required_evidence_hash: String,
    pub rights_hash: String,
    pub challenge_policy_hash: String,
    pub sponsor_hash: String,
    pub award_units: i64,
    pub capacity_units: i64,
    pub participant_cap_units: i64,
    pub closes_at: DateTime<Utc>,
    pub reservation_ttl_seconds: i64,
}

/// Only these stable labels cross the database boundary. Database errors can
/// include input values or connection details and must never reach CLI output.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewardError {
    Unauthorized,
    RequestInvalid,
    NotFound,
    PayloadConflict,
    ProgramClosed,
    CapacityExhausted,
    ParticipantCap,
    WorkDuplicate,
    EvidenceDuplicate,
    EvidenceInvalidated,
    ReservationExpired,
    StateConflict,
    SelfReview,
    StoreUnavailable,
}

impl RewardError {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unauthorized => "reward_unauthorized",
            Self::RequestInvalid => "reward_request_invalid",
            Self::NotFound => "reward_not_found",
            Self::PayloadConflict => "reward_payload_conflict",
            Self::ProgramClosed => "reward_program_closed",
            Self::CapacityExhausted => "reward_capacity_exhausted",
            Self::ParticipantCap => "reward_participant_cap",
            Self::WorkDuplicate => "reward_work_duplicate",
            Self::EvidenceDuplicate => "reward_evidence_duplicate",
            Self::EvidenceInvalidated => "reward_evidence_invalidated",
            Self::ReservationExpired => "reward_reservation_expired",
            Self::StateConflict => "reward_state_conflict",
            Self::SelfReview => "reward_self_review",
            Self::StoreUnavailable => "reward_store_unavailable",
        }
    }

    fn from_postgres(error: tokio_postgres::Error) -> Self {
        // P0001 is an explicit application refusal. Constraint violations,
        // connection errors and arbitrary database messages stay opaque.
        let Some(db) = error.as_db_error() else {
            return Self::StoreUnavailable;
        };
        if db.code().code() != "P0001" {
            return Self::StoreUnavailable;
        }
        match db.message() {
            "reward_unauthorized" => Self::Unauthorized,
            "reward_request_invalid" => Self::RequestInvalid,
            "reward_not_found" => Self::NotFound,
            "reward_payload_conflict" => Self::PayloadConflict,
            "reward_program_closed" => Self::ProgramClosed,
            "reward_capacity_exhausted" => Self::CapacityExhausted,
            "reward_participant_cap" => Self::ParticipantCap,
            "reward_work_duplicate" => Self::WorkDuplicate,
            "reward_evidence_duplicate" => Self::EvidenceDuplicate,
            "reward_evidence_invalidated" => Self::EvidenceInvalidated,
            "reward_reservation_expired" => Self::ReservationExpired,
            "reward_state_conflict" => Self::StateConflict,
            "reward_self_review" => Self::SelfReview,
            _ => Self::StoreUnavailable,
        }
    }
}

impl std::fmt::Display for RewardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.label())
    }
}

impl std::error::Error for RewardError {}

impl PgBackend {
    // Only static statements below reach this helper. Each SQL function checks
    // session_user's tenant grant itself; setting the tenant GUC is not auth.
    async fn reward_query(
        &self,
        tenant: &str,
        statement: &'static str,
        parameters: &[&(dyn ToSql + Sync)],
    ) -> Result<Value, RewardError> {
        let mut client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| RewardError::StoreUnavailable)?;
        let transaction = Self::begin_trace_tenant_transaction(&mut client, tenant)
            .await
            .map_err(|_| RewardError::StoreUnavailable)?;
        let result = transaction
            .query_one(statement, parameters)
            .await
            .map_err(RewardError::from_postgres)?
            .try_get(0)
            .map_err(|_| RewardError::StoreUnavailable)?;
        transaction
            .commit()
            .await
            .map_err(|_| RewardError::StoreUnavailable)?;
        Ok(result)
    }

    pub async fn reward_program_create(
        &self,
        tenant: &str,
        program: Uuid,
        terms: &RewardProgramTerms,
    ) -> Result<Value, RewardError> {
        let terms = serde_json::to_value(terms).map_err(|_| RewardError::RequestInvalid)?;
        self.reward_query(
            tenant,
            "SELECT public.trace_reward_program_create($1,$2,$3)",
            &[&tenant, &program, &terms],
        )
        .await
    }

    pub async fn reward_program_show(
        &self,
        tenant: &str,
        program: Uuid,
    ) -> Result<Value, RewardError> {
        self.reward_query(
            tenant,
            "SELECT public.trace_reward_program_show($1,$2)",
            &[&tenant, &program],
        )
        .await
    }

    pub async fn reward_reserve(
        &self,
        tenant: &str,
        program: Uuid,
        reservation: Uuid,
        participant_hash: &str,
        work_hash: &str,
        consent_hash: &str,
    ) -> Result<Value, RewardError> {
        self.reward_query(
            tenant,
            "SELECT public.trace_reward_reserve($1,$2,$3,$4,$5,$6)",
            &[
                &tenant,
                &program,
                &reservation,
                &participant_hash,
                &work_hash,
                &consent_hash,
            ],
        )
        .await
    }

    pub async fn reward_claim_submit(
        &self,
        tenant: &str,
        reservation: Uuid,
        evidence_hash: &str,
        evaluation_hash: &str,
    ) -> Result<Value, RewardError> {
        self.reward_query(
            tenant,
            "SELECT public.trace_reward_claim_submit($1,$2,$3,$4)",
            &[&tenant, &reservation, &evidence_hash, &evaluation_hash],
        )
        .await
    }

    pub async fn reward_review(
        &self,
        tenant: &str,
        reservation: Uuid,
        decision: Uuid,
        accept: bool,
        reason: &str,
    ) -> Result<Value, RewardError> {
        self.reward_query(
            tenant,
            "SELECT public.trace_reward_review($1,$2,$3,$4,$5)",
            &[&tenant, &reservation, &decision, &accept, &reason],
        )
        .await
    }

    pub async fn reward_cancel(
        &self,
        tenant: &str,
        reservation: Uuid,
        decision: Uuid,
    ) -> Result<Value, RewardError> {
        self.reward_query(
            tenant,
            "SELECT public.trace_reward_cancel($1,$2,$3)",
            &[&tenant, &reservation, &decision],
        )
        .await
    }

    pub async fn reward_invalidate(
        &self,
        tenant: &str,
        evidence_hash: &str,
        decision: Uuid,
    ) -> Result<Value, RewardError> {
        self.reward_query(
            tenant,
            "SELECT public.trace_reward_invalidate($1,$2,$3)",
            &[&tenant, &evidence_hash, &decision],
        )
        .await
    }

    /// Read the first page, preserving the original three-argument history API.
    pub async fn reward_history(
        &self,
        tenant: &str,
        participant_hash: &str,
        limit: i32,
    ) -> Result<Value, RewardError> {
        self.reward_history_page(tenant, participant_hash, limit, None)
            .await
    }

    /// Continue bounded history with the reservation cursor from the prior page.
    pub async fn reward_history_page(
        &self,
        tenant: &str,
        participant_hash: &str,
        limit: i32,
        before: Option<Uuid>,
    ) -> Result<Value, RewardError> {
        self.reward_query(
            tenant,
            "SELECT public.trace_reward_history($1,$2,$3,$4)",
            &[&tenant, &participant_hash, &limit, &before],
        )
        .await
    }
}
