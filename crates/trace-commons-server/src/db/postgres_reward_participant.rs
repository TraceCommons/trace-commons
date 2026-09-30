// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! INTEGRATION: V71 functions own identity, ownership and atomic reservation rules.

use serde::de::DeserializeOwned;
use serde_json::Value;
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::mission_rewards::{RewardError, RewardProgramTerms};
use crate::reward_participant::{
    RewardHistoryQuery, RewardOffer, RewardOfferManifest, RewardParticipantHistory,
    RewardReservation, RewardReservationRequest,
};

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, RewardError> {
    serde_json::from_value(value).map_err(|_| RewardError::StoreUnavailable)
}

impl PgBackend {
    /// Issuer-only publication, deliberately outside the participant Database facade.
    pub async fn reward_offer_publish(
        &self,
        tenant: &str,
        program: Uuid,
        terms: &RewardProgramTerms,
        manifest: &RewardOfferManifest,
    ) -> Result<RewardOffer, RewardError> {
        let terms = serde_json::to_value(terms).map_err(|_| RewardError::RequestInvalid)?;
        let manifest = serde_json::to_value(manifest).map_err(|_| RewardError::RequestInvalid)?;
        decode(
            self.reward_query(
                tenant,
                "SELECT public.trace_reward_offer_publish($1,$2,$3,$4)",
                &[&tenant, &program, &terms, &manifest],
            )
            .await?,
        )
    }

    pub async fn reward_offer_suspend(
        &self,
        tenant: &str,
        program: Uuid,
        suspended: bool,
    ) -> Result<RewardOffer, RewardError> {
        decode(
            self.reward_query(
                tenant,
                "SELECT public.trace_reward_offer_suspend($1,$2,$3)",
                &[&tenant, &program, &suspended],
            )
            .await?,
        )
    }

    pub(crate) async fn participant_reward_offer(
        &self,
        program: Uuid,
    ) -> Result<RewardOffer, RewardError> {
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| RewardError::StoreUnavailable)?;
        // The dedicated reader function resolves only published program UUIDs.
        // No request-controlled tenant context enters this public lookup.
        let row = client
            .query_one("SELECT public.trace_reward_offer_get($1)", &[&program])
            .await
            .map_err(RewardError::from_postgres)?;
        decode(row.try_get(0).map_err(|_| RewardError::StoreUnavailable)?)
    }

    pub(crate) async fn participant_reward_reserve(
        &self,
        tenant: &str,
        account: Uuid,
        program: Uuid,
        request: &RewardReservationRequest,
    ) -> Result<RewardReservation, RewardError> {
        request.validate()?;
        decode(
            self.reward_query(
                tenant,
                "SELECT public.trace_reward_participant_reserve($1,$2,$3,$4,$5)",
                &[
                    &tenant,
                    &account,
                    &program,
                    &request.reservation_id,
                    &request.offer_version_hash,
                ],
            )
            .await?,
        )
    }

    pub(crate) async fn participant_reward_reservation(
        &self,
        tenant: &str,
        account: Uuid,
        reservation: Uuid,
    ) -> Result<RewardReservation, RewardError> {
        decode(
            self.reward_query(
                tenant,
                "SELECT public.trace_reward_participant_get($1,$2,$3)",
                &[&tenant, &account, &reservation],
            )
            .await?,
        )
    }

    pub(crate) async fn participant_reward_history(
        &self,
        tenant: &str,
        account: Uuid,
        query: &RewardHistoryQuery,
    ) -> Result<RewardParticipantHistory, RewardError> {
        query.validate()?;
        decode(
            self.reward_query(
                tenant,
                "SELECT public.trace_reward_participant_history($1,$2,$3,$4)",
                &[&tenant, &account, &query.limit, &query.before],
            )
            .await?,
        )
    }
}
