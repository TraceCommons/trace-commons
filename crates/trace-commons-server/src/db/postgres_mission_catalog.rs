// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! INTEGRATION: Root must declare `mod mission_catalog;` beside the
//! other PostgreSQL reward adapters after the mission protocol modules land.

use serde_json::Value;
use trace_commons_protocol::mission_catalog::{
    MissionCatalogPage, MissionCatalogQuery, MissionPublication,
};
use trace_commons_protocol::mission_evaluation::{
    MISSION_EVALUATION_PACKAGE_MAX_BYTES, MissionEvaluationPackage,
};
use uuid::Uuid;

use crate::db::postgres::PgBackend;
use crate::mission_rewards::RewardError;

fn decode_publication(value: Value) -> Result<MissionPublication, RewardError> {
    let bytes = serde_json::to_vec(&value).map_err(|_| RewardError::StoreUnavailable)?;
    MissionPublication::parse(&bytes).map_err(|_| RewardError::StoreUnavailable)
}

fn decode_page(
    value: Value,
    query: &MissionCatalogQuery,
) -> Result<MissionCatalogPage, RewardError> {
    let bytes = serde_json::to_vec(&value).map_err(|_| RewardError::StoreUnavailable)?;
    MissionCatalogPage::parse(&bytes, query).map_err(|_| RewardError::StoreUnavailable)
}

fn require_publication_mission(
    publication: &MissionPublication,
    mission: Uuid,
) -> Result<(), RewardError> {
    if publication
        .package()
        .map_err(|_| RewardError::StoreUnavailable)?
        .mission_id
        != mission
    {
        return Err(RewardError::StoreUnavailable);
    }
    Ok(())
}

impl PgBackend {
    /// Publishes one immutable mission package through the issuer-authorized DB function.
    pub async fn reward_mission_publish(
        &self,
        tenant: &str,
        program: Uuid,
        package: &MissionEvaluationPackage,
    ) -> Result<MissionPublication, RewardError> {
        if program.is_nil() || package.validate().is_err() || package.program_id != program {
            return Err(RewardError::RequestInvalid);
        }
        let package_json =
            serde_json::to_string(package).map_err(|_| RewardError::RequestInvalid)?;
        if package_json.len() > MISSION_EVALUATION_PACKAGE_MAX_BYTES {
            return Err(RewardError::RequestInvalid);
        }
        let package_sha256 = package
            .package_sha256()
            .map_err(|_| RewardError::RequestInvalid)?;
        let publication = decode_publication(
            self.reward_query(
                tenant,
                "SELECT public.trace_reward_mission_publish($1,$2,$3)",
                &[&tenant, &program, &package_json],
            )
            .await?,
        )?;
        if publication.package_json != package_json || publication.package_sha256 != package_sha256
        {
            return Err(RewardError::StoreUnavailable);
        }
        Ok(publication)
    }

    pub(crate) async fn public_mission_get(
        &self,
        mission: Uuid,
    ) -> Result<MissionPublication, RewardError> {
        if mission.is_nil() {
            return Err(RewardError::RequestInvalid);
        }
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| RewardError::StoreUnavailable)?;
        let row = client
            .query_one("SELECT public.trace_reward_mission_get($1)", &[&mission])
            .await
            .map_err(RewardError::from_postgres)?;
        let value = row.try_get(0).map_err(|_| RewardError::StoreUnavailable)?;
        let publication = decode_publication(value)?;
        require_publication_mission(&publication, mission)?;
        Ok(publication)
    }

    pub(crate) async fn public_mission_list(
        &self,
        query: &MissionCatalogQuery,
    ) -> Result<MissionCatalogPage, RewardError> {
        query.validate().map_err(|_| RewardError::RequestInvalid)?;
        let client = self
            .trace_pool()
            .get()
            .await
            .map_err(|_| RewardError::StoreUnavailable)?;
        let row = client
            .query_one(
                "SELECT public.trace_reward_mission_list($1,$2)",
                &[&query.before, &query.limit],
            )
            .await
            .map_err(RewardError::from_postgres)?;
        let value = row.try_get(0).map_err(|_| RewardError::StoreUnavailable)?;
        decode_page(value, query)
    }
}

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use sha2::{Digest, Sha256};
    use trace_commons_protocol::mission_catalog::{
        MISSION_CATALOG_SCHEMA_VERSION, MISSION_PUBLICATION_MAX_BYTES, MissionCatalogEntry,
        MissionCatalogError,
    };
    use trace_commons_protocol::mission_evaluation::{
        MISSION_EVALUATION_MAX_CONCURRENCY, MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
        MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS, MISSION_EVALUATION_REQUIRED_MODEL_OWNER,
        MISSION_EVALUATION_SCHEMA_VERSION, MISSION_EVALUATION_TOTAL_REQUESTS, MISSION_EVALUATOR_ID,
        MissionEvaluationPolicy, SkillDraft, render_skill,
    };

    use crate::db::postgres::mission_catalog::*;

    fn package() -> MissionEvaluationPackage {
        let skill = SkillDraft {
            name: "repair-generated-sources".into(),
            description: "Repair generated files at their authoritative source.".into(),
            procedure: "# Repair\n\nUpdate the source, then regenerate outputs.".into(),
        };
        MissionEvaluationPackage {
            schema_version: MISSION_EVALUATION_SCHEMA_VERSION,
            mission_id: Uuid::from_u128(100),
            program_id: Uuid::from_u128(200),
            offer_version_hash: format!("sha256:{}", "a".repeat(64)),
            task: "Repair the generated manifest from its source schema.".into(),
            skill_sha256: hex::encode(Sha256::digest(render_skill(&skill).as_bytes())),
            skill,
            evaluator_id: MISSION_EVALUATOR_ID.into(),
            evaluation_contract_hash: "b".repeat(64),
            execution: MissionEvaluationPolicy {
                required_model_owner: MISSION_EVALUATION_REQUIRED_MODEL_OWNER.into(),
                total_requests: MISSION_EVALUATION_TOTAL_REQUESTS,
                output_token_limit: MISSION_EVALUATION_OUTPUT_TOKEN_LIMIT,
                request_timeout_seconds: MISSION_EVALUATION_REQUEST_TIMEOUT_SECONDS,
                max_concurrency: MISSION_EVALUATION_MAX_CONCURRENCY,
            },
        }
    }

    fn publication() -> MissionPublication {
        let package = package();
        MissionPublication {
            schema_version: MISSION_CATALOG_SCHEMA_VERSION,
            package_json: serde_json::to_string(&package).expect("package serializes"),
            package_sha256: package.package_sha256().expect("package hashes"),
            published_at: DateTime::UNIX_EPOCH,
        }
    }

    #[test]
    fn publication_decoder_refuses_malformed_oversize_stale_and_mismatched_packages() {
        assert_eq!(
            decode_publication(serde_json::json!({"unexpected": true})),
            Err(RewardError::StoreUnavailable)
        );

        let oversize = Value::String("x".repeat(MISSION_PUBLICATION_MAX_BYTES + 1));
        assert_eq!(
            decode_publication(oversize),
            Err(RewardError::StoreUnavailable)
        );

        let mut changed = package();
        changed.task.push('!');
        let mut stale = publication();
        stale.package_json = serde_json::to_string(&changed).expect("package serializes");
        assert_eq!(
            decode_publication(serde_json::to_value(stale).expect("publication serializes")),
            Err(RewardError::StoreUnavailable)
        );

        let mut mismatched = publication();
        mismatched.package_json = serde_json::to_string(&MissionEvaluationPackage {
            mission_id: Uuid::from_u128(101),
            ..package()
        })
        .expect("package serializes");
        mismatched.package_sha256 = hex::encode(Sha256::digest(mismatched.package_json.as_bytes()));
        assert_eq!(
            require_publication_mission(
                &decode_publication(
                    serde_json::to_value(mismatched).expect("publication serializes")
                )
                .expect("publication decodes"),
                Uuid::from_u128(100),
            ),
            Err(RewardError::StoreUnavailable)
        );
    }

    #[test]
    fn page_decoder_refuses_malformed_cursor_projection() {
        let query = MissionCatalogQuery {
            limit: 1,
            before: Some(Uuid::from_u128(20)),
        };
        let page = MissionCatalogPage {
            schema_version: MISSION_CATALOG_SCHEMA_VERSION,
            entries: vec![MissionCatalogEntry {
                mission_id: Uuid::from_u128(20),
                program_id: Uuid::from_u128(200),
                package_sha256: "c".repeat(64),
                offer_version_hash: format!("sha256:{}", "d".repeat(64)),
                task_preview: "Repair the source manifest.".into(),
                published_at: DateTime::UNIX_EPOCH,
            }],
            next_cursor: Some(Uuid::from_u128(20)),
        };
        assert_eq!(
            decode_page(serde_json::to_value(page).expect("page serializes"), &query),
            Err(RewardError::StoreUnavailable)
        );
        assert_eq!(
            MissionCatalogPage::parse(
                br#"{"schema_version":1,"entries":[],"next_cursor":"not-a-uuid"}"#,
                &query
            ),
            Err(MissionCatalogError::MalformedJson)
        );
    }
}
