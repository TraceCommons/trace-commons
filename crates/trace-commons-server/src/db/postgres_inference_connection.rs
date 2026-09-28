// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Account-scoped, durable selection of an operator-owned inference connection.

use sha2::{Digest, Sha256};
use trace_commons_protocol::inference_connection::{
    SelectInferenceConnection, SelectedInferenceConnection,
};
use uuid::Uuid;

use super::postgres::PgBackend;
use crate::error::DatabaseError;
use crate::inference_connection::OperatorInferenceConnection;

#[derive(Clone, PartialEq, Eq, serde::Serialize)]
pub struct InferenceConnectionStatus {
    pub connection_id: Uuid,
    pub state_version: i64,
    pub offer_id: String,
    pub revision: String,
    pub config_digest: String,
    pub disclosure_version: String,
    pub reselection_required: bool,
}

impl std::fmt::Debug for InferenceConnectionStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("InferenceConnectionStatus")
            .field("connection_id", &"[redacted]")
            .field("state_version", &self.state_version)
            .field("offer_id", &self.offer_id)
            .field("revision", &"[redacted]")
            .field("config_digest", &"[redacted]")
            .field("disclosure_version", &self.disclosure_version)
            .field("reselection_required", &self.reselection_required)
            .finish()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InferenceSelectionOutcome {
    Selected(SelectedInferenceConnection),
    AccountIneligible,
    InvalidSelection,
    VersionConflict,
    IdempotencyConflict,
    ReselectionRequired,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InferenceDisconnectOutcome {
    Revoked { state_version: i64 },
    AccountIneligible,
    NotFound,
}

fn request_digest(request: &SelectInferenceConnection) -> String {
    let mut bytes = b"trace-commons/inference-connection-request/v1\0".to_vec();
    for value in [
        request.offer_id.as_str(),
        request.revision.as_str(),
        request.config_digest.as_str(),
        request.disclosure_version.as_str(),
    ] {
        bytes.extend_from_slice(&(value.len() as u32).to_be_bytes());
        bytes.extend_from_slice(value.as_bytes());
    }
    match request.expected_current_version {
        Some(version) => {
            bytes.push(1);
            bytes.extend_from_slice(&version.to_be_bytes());
        }
        None => bytes.push(0),
    }
    format!("sha256:{}", hex::encode(Sha256::digest(bytes)))
}

impl PgBackend {
    pub async fn select_inference_connection(
        &self,
        tenant: &str,
        account: Uuid,
        request: &SelectInferenceConnection,
        catalog: &OperatorInferenceConnection,
    ) -> Result<InferenceSelectionOutcome, DatabaseError> {
        if request.validate().is_err() {
            return Ok(InferenceSelectionOutcome::InvalidSelection);
        }
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await?;
        let eligible = tx
            .query_opt(
                "SELECT 1 FROM trace_accounts a WHERE a.tenant_id = $1 AND a.account_id = $2
                AND a.closed_at IS NULL AND EXISTS (
                    SELECT 1 FROM trace_near_account_anchors n
                    WHERE n.tenant_id = a.tenant_id AND n.account_id = a.account_id)
                FOR UPDATE OF a",
                &[&tenant, &account],
            )
            .await?;
        if eligible.is_none() {
            return Ok(InferenceSelectionOutcome::AccountIneligible);
        }

        let digest = request_digest(request);
        if let Some(prior) = tx
            .query_opt(
                "SELECT request_digest, connection_id, state_version
             FROM trace_account_inference_connection_requests
             WHERE tenant_id = $1 AND account_id = $2 AND idempotency_key = $3",
                &[&tenant, &account, &request.idempotency_key],
            )
            .await?
        {
            if prior.get::<_, String>(0) != digest {
                return Ok(InferenceSelectionOutcome::IdempotencyConflict);
            }
            let connection_id: Uuid = prior.get(1);
            let version: i64 = prior.get(2);
            let row = tx.query_one(
                "SELECT revoked_at IS NULL, revision, config_digest FROM trace_account_inference_connections
                 WHERE tenant_id = $1 AND account_id = $2 AND connection_id = $3",
                &[&tenant, &account, &connection_id]).await?;
            if !row.get::<_, bool>(0)
                || !catalog.matches_selection(request)
                || row.get::<_, String>(1) != request.revision
                || row.get::<_, String>(2) != request.config_digest
            {
                return Ok(InferenceSelectionOutcome::ReselectionRequired);
            }
            return Ok(InferenceSelectionOutcome::Selected(
                catalog.selected(connection_id, version),
            ));
        }
        if !catalog.matches_selection(request) {
            return Ok(if catalog.offer().offer_id == request.offer_id {
                InferenceSelectionOutcome::ReselectionRequired
            } else {
                InferenceSelectionOutcome::InvalidSelection
            });
        }
        let current = tx
            .query_opt(
                "SELECT connection_id, state_version FROM trace_account_inference_connections
             WHERE tenant_id = $1 AND account_id = $2 AND revoked_at IS NULL",
                &[&tenant, &account],
            )
            .await?;
        let current_version = current.as_ref().map(|row| row.get::<_, i64>(1));
        if request.expected_current_version != current_version {
            return Ok(InferenceSelectionOutcome::VersionConflict);
        }
        // A replacement is two account transitions: revoke the old selection,
        // then select the new one. Give each event its own monotonic version.
        let transition_version: i64 = tx.query_one(
            "SELECT COALESCE(MAX(state_version), 0) + 1 FROM trace_account_inference_connections
             WHERE tenant_id = $1 AND account_id = $2",
            &[&tenant, &account]).await?.get::<_, i64>(0);
        let next_version = if current.is_some() {
            transition_version + 1
        } else {
            transition_version
        };
        if let Some(current) = current {
            let old: Uuid = current.get(0);
            tx.execute(
                "UPDATE trace_account_inference_connections SET revoked_at = now(), state_version = $4
                 WHERE tenant_id = $1 AND account_id = $2 AND connection_id = $3",
                &[&tenant, &account, &old, &transition_version],
            )
            .await?;
            tx.execute(
                "INSERT INTO trace_account_inference_connection_events
                  (tenant_id, account_id, connection_id, event_type, state_version, revision, config_digest)
                 SELECT tenant_id, account_id, connection_id, 'inference_connection_revoked',
                        state_version, revision, config_digest
                 FROM trace_account_inference_connections
                 WHERE tenant_id = $1 AND account_id = $2 AND connection_id = $3",
                &[&tenant, &account, &old]).await?;
        }
        let id = Uuid::new_v4();
        let offer = catalog.offer();
        tx.execute(
            "INSERT INTO trace_account_inference_connections
              (tenant_id, account_id, connection_id, offer_id, provider_id, revision,
               config_digest, disclosure_version, state_version)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
            &[
                &tenant,
                &account,
                &id,
                &offer.offer_id,
                &offer.provider_id,
                &offer.revision,
                &offer.config_digest,
                &offer.disclosure_version,
                &next_version,
            ],
        )
        .await?;
        tx.execute(
            "INSERT INTO trace_account_inference_connection_requests
              (tenant_id, account_id, idempotency_key, request_digest, connection_id, state_version)
             VALUES ($1, $2, $3, $4, $5, $6)",
            &[
                &tenant,
                &account,
                &request.idempotency_key,
                &digest,
                &id,
                &next_version,
            ],
        )
        .await?;
        tx.execute(
            "INSERT INTO trace_account_inference_connection_events
              (tenant_id, account_id, connection_id, event_type, state_version, revision, config_digest)
             VALUES ($1, $2, $3, 'inference_connection_selected', $4, $5, $6)",
            &[&tenant, &account, &id, &next_version, &offer.revision, &offer.config_digest]).await?;
        tx.commit().await?;
        Ok(InferenceSelectionOutcome::Selected(
            catalog.selected(id, next_version),
        ))
    }

    pub async fn current_inference_connection(
        &self,
        tenant: &str,
        account: Uuid,
        catalog: &[OperatorInferenceConnection],
    ) -> Result<Option<InferenceConnectionStatus>, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await?;
        let row = tx
            .query_opt(
                "SELECT c.connection_id, c.state_version, c.offer_id, c.revision,
                    c.config_digest, c.disclosure_version
             FROM trace_account_inference_connections c JOIN trace_accounts a
               ON a.tenant_id = c.tenant_id AND a.account_id = c.account_id
             WHERE c.tenant_id = $1 AND c.account_id = $2 AND c.revoked_at IS NULL
               AND a.closed_at IS NULL AND EXISTS (
                   SELECT 1 FROM trace_near_account_anchors n
                   WHERE n.tenant_id = a.tenant_id AND n.account_id = a.account_id)",
                &[&tenant, &account],
            )
            .await?;
        let result = row.map(|row| {
            let offer_id: String = row.get(2);
            let revision: String = row.get(3);
            let config_digest: String = row.get(4);
            let disclosure_version: String = row.get(5);
            let reselection_required = !catalog.iter().any(|entry| {
                let offer = entry.offer();
                offer.offer_id == offer_id
                    && offer.revision == revision
                    && offer.config_digest == config_digest
                    && offer.disclosure_version == disclosure_version
            });
            InferenceConnectionStatus {
                connection_id: row.get(0),
                state_version: row.get(1),
                offer_id,
                revision,
                config_digest,
                disclosure_version,
                reselection_required,
            }
        });
        tx.commit().await?;
        Ok(result)
    }

    pub async fn disconnect_inference_connection(
        &self,
        tenant: &str,
        account: Uuid,
        connection_id: Uuid,
    ) -> Result<InferenceDisconnectOutcome, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = client.transaction().await?;
        tx.execute(
            "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
            &[&tenant],
        )
        .await?;
        let eligible = tx
            .query_opt(
                "SELECT 1 FROM trace_accounts a WHERE a.tenant_id = $1 AND a.account_id = $2
             AND a.closed_at IS NULL AND EXISTS (
                 SELECT 1 FROM trace_near_account_anchors n
                 WHERE n.tenant_id = a.tenant_id AND n.account_id = a.account_id)
             FOR UPDATE OF a",
                &[&tenant, &account],
            )
            .await?;
        if eligible.is_none() {
            return Ok(InferenceDisconnectOutcome::AccountIneligible);
        }
        let row = tx
            .query_opt(
                "SELECT state_version, revoked_at IS NOT NULL, revision, config_digest
             FROM trace_account_inference_connections
             WHERE tenant_id = $1 AND account_id = $2 AND connection_id = $3",
                &[&tenant, &account, &connection_id],
            )
            .await?;
        let Some(row) = row else {
            return Ok(InferenceDisconnectOutcome::NotFound);
        };
        let version: i64 = row.get(0);
        if row.get::<_, bool>(1) {
            return Ok(InferenceDisconnectOutcome::Revoked {
                state_version: version,
            });
        }
        let new_version = version + 1;
        tx.execute(
            "UPDATE trace_account_inference_connections SET revoked_at = now(), state_version = $4
             WHERE tenant_id = $1 AND account_id = $2 AND connection_id = $3",
            &[&tenant, &account, &connection_id, &new_version],
        )
        .await?;
        let revision: String = row.get(2);
        let config_digest: String = row.get(3);
        tx.execute(
            "INSERT INTO trace_account_inference_connection_events
              (tenant_id, account_id, connection_id, event_type, state_version, revision, config_digest)
             VALUES ($1, $2, $3, 'inference_connection_revoked', $4, $5, $6)",
            &[&tenant, &account, &connection_id, &new_version, &revision, &config_digest]).await?;
        tx.commit().await?;
        Ok(InferenceDisconnectOutcome::Revoked {
            state_version: new_version,
        })
    }
}
