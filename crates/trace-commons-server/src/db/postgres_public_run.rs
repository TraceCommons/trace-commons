// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! PostgreSQL persistence for reviewed public workflows.

use tokio_postgres::Row;
use trace_commons_protocol::public_run::{
    PublicRunEvidence, PublicRunLink, PublicRunReusePermission,
};
use trace_commons_protocol::trace_contribution::TaskSuccess;
use uuid::Uuid;

use crate::db::{
    PublicRunMutation, PublicRunOwnerData, PublicRunPageData, PublicRunRow, PublicRunSourceRow,
    PublicRunUnpublishMutation, PublicRunWrite,
};
use crate::error::DatabaseError;

use crate::db::postgres::PgBackend;

const PUBLIC_RUN_COLUMNS: &str = "tenant_id, publication_id, account_id, submission_id, slug, \
title, outcome_summary, correction_excerpt, workflow, reuse_permission, evidence_jsonb, \
task_success, contributed_version, approval_sha256, source_publication_id, version, \
published_at, updated_at, unpublished_at";
const PUBLIC_RUN_PROVENANCE_LOCK_CLASSID: i32 = 0x7075_6272;
const PUBLIC_RUN_PROVENANCE_LOCK_OBJID: i32 = 0x756e_7072;

fn permission_wire(permission: PublicRunReusePermission) -> &'static str {
    match permission {
        PublicRunReusePermission::CcBy40 => "cc_by_4_0",
        PublicRunReusePermission::Cc0_10 => "cc0_1_0",
    }
}

fn permission_from_wire(permission: &str) -> Result<PublicRunReusePermission, DatabaseError> {
    match permission {
        "cc_by_4_0" => Ok(PublicRunReusePermission::CcBy40),
        "cc0_1_0" => Ok(PublicRunReusePermission::Cc0_10),
        _ => Err(DatabaseError::Serialization(
            "public run reuse permission is invalid".to_string(),
        )),
    }
}

fn task_success_from_wire(value: &str) -> Result<TaskSuccess, DatabaseError> {
    match value {
        "success" => Ok(TaskSuccess::Success),
        "partial" => Ok(TaskSuccess::Partial),
        "failure" => Ok(TaskSuccess::Failure),
        "unknown" => Ok(TaskSuccess::Unknown),
        _ => Err(DatabaseError::Serialization(
            "public run task success is invalid".to_string(),
        )),
    }
}

fn task_success_wire(value: TaskSuccess) -> &'static str {
    match value {
        TaskSuccess::Success => "success",
        TaskSuccess::Partial => "partial",
        TaskSuccess::Failure => "failure",
        TaskSuccess::Unknown => "unknown",
    }
}

fn public_run_from_row(row: Row) -> Result<PublicRunRow, DatabaseError> {
    let permission_wire: String = row.get("reuse_permission");
    let evidence_value: serde_json::Value = row.get("evidence_jsonb");
    let evidence = serde_json::from_value(evidence_value)
        .map_err(|_| DatabaseError::Serialization("public run evidence is invalid".to_string()))?;
    let task_success_wire: String = row.get("task_success");
    Ok(PublicRunRow {
        tenant_id: row.get("tenant_id"),
        publication_id: row.get("publication_id"),
        account_id: row.get("account_id"),
        submission_id: row.get("submission_id"),
        slug: row.get("slug"),
        title: row.get("title"),
        outcome_summary: row.get("outcome_summary"),
        correction_excerpt: row.get("correction_excerpt"),
        workflow: row.get("workflow"),
        reuse_permission: permission_from_wire(&permission_wire)?,
        evidence,
        task_success: task_success_from_wire(&task_success_wire)?,
        contributed_version: row.get("contributed_version"),
        approval_sha256: row.get("approval_sha256"),
        source_publication_id: row.get("source_publication_id"),
        version: row.get("version"),
        published_at: row.get("published_at"),
        updated_at: row.get("updated_at"),
        unpublished_at: row.get("unpublished_at"),
    })
}

fn public_run_page_from_row(row: &Row) -> Result<PublicRunPageData, DatabaseError> {
    let permission_wire: String = row.get("reuse_permission");
    let evidence = serde_json::from_value::<Vec<PublicRunEvidence>>(row.get("evidence"))
        .map_err(|_| DatabaseError::Serialization("public run evidence is invalid".into()))?;
    let source = row
        .get::<_, Option<serde_json::Value>>("source")
        .map(serde_json::from_value::<PublicRunLink>)
        .transpose()
        .map_err(|_| DatabaseError::Serialization("public run source is invalid".into()))?;
    let variations = serde_json::from_value::<Vec<PublicRunLink>>(row.get("variations"))
        .map_err(|_| DatabaseError::Serialization("public run variations are invalid".into()))?;
    let stored_version: i32 = row.get("version");
    let version = u32::try_from(stored_version)
        .map_err(|_| DatabaseError::Serialization("public run version is invalid".to_string()))?;
    if version == 0 {
        return Err(DatabaseError::Serialization(
            "public run version is invalid".to_string(),
        ));
    }
    let task_success_wire: String = row.get("task_success");
    Ok(PublicRunPageData {
        slug: row.get("slug"),
        title: row.get("title"),
        outcome_summary: row.get("outcome_summary"),
        correction_excerpt: row.get("correction_excerpt"),
        workflow: row.get("workflow"),
        reuse_permission: permission_from_wire(&permission_wire)?,
        evidence,
        task_success: task_success_from_wire(&task_success_wire)?,
        contributed_version: row.get("contributed_version"),
        version,
        published_at: row.get("published_at"),
        source,
        source_unavailable: row.get("source_unavailable"),
        variations,
    })
}

impl PgBackend {
    pub(super) async fn public_run_upsert(
        &self,
        write: PublicRunWrite,
    ) -> Result<PublicRunMutation, DatabaseError> {
        self.ensure_trace_tenant(&write.tenant_id).await?;
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, &write.tenant_id).await?;
        let account_is_open = tx
            .query_opt(
                "SELECT 1 FROM trace_accounts
                 WHERE tenant_id = $1 AND account_id = $2 AND closed_at IS NULL
                 FOR UPDATE",
                &[&write.tenant_id, &write.account_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .is_some();
        if !account_is_open {
            return Err(DatabaseError::Constraint(
                crate::db::PUBLIC_RUN_ACCOUNT_CLOSED.to_string(),
            ));
        }
        let source_status = tx
            .query_opt(
                "SELECT status FROM trace_submissions
                 WHERE tenant_id = $1 AND submission_id = $2
                 FOR UPDATE",
                &[&write.tenant_id, &write.submission_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .map(|row| row.get::<_, String>("status"));
        if source_status.as_deref() != Some("accepted") {
            return Err(DatabaseError::Constraint(
                crate::db::PUBLIC_RUN_SOURCE_NOT_ACCEPTED.to_string(),
            ));
        }
        let effective_publication_id = tx
            .query_opt(
                "SELECT publication_id FROM trace_public_runs
                 WHERE tenant_id = $1 AND submission_id = $2",
                &[&write.tenant_id, &write.submission_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .map_or(write.publication_id, |row| row.get("publication_id"));
        if let Some(source_publication_id) = write.source_publication_id {
            tx.execute(
                "SELECT pg_advisory_xact_lock($1, $2)",
                &[
                    &PUBLIC_RUN_PROVENANCE_LOCK_CLASSID,
                    &PUBLIC_RUN_PROVENANCE_LOCK_OBJID,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
            let would_cycle: bool = tx
                .query_one(
                    "SELECT trace_public_run_would_cycle($1, $2) AS would_cycle",
                    &[&effective_publication_id, &source_publication_id],
                )
                .await
                .map_err(DatabaseError::Postgres)?
                .get("would_cycle");
            if would_cycle {
                return Err(DatabaseError::Constraint(
                    crate::db::PUBLIC_RUN_PROVENANCE_CYCLE.to_string(),
                ));
            }
        }
        let evidence = serde_json::to_value(&write.evidence)
            .map_err(|_| DatabaseError::Serialization("public run evidence is invalid".into()))?;
        let permission = permission_wire(write.reuse_permission);
        let task_success = task_success_wire(write.task_success);
        let sql = format!(
            "INSERT INTO trace_public_runs (
                tenant_id, publication_id, account_id, submission_id, slug, title,
                outcome_summary, correction_excerpt, workflow, reuse_permission,
                evidence_jsonb, task_success, contributed_version, approval_sha256,
                source_publication_id
             ) VALUES (
                $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15
             )
             ON CONFLICT (tenant_id, submission_id) DO UPDATE SET
                account_id = EXCLUDED.account_id,
                title = EXCLUDED.title,
                outcome_summary = EXCLUDED.outcome_summary,
                correction_excerpt = EXCLUDED.correction_excerpt,
                workflow = EXCLUDED.workflow,
                reuse_permission = EXCLUDED.reuse_permission,
                evidence_jsonb = EXCLUDED.evidence_jsonb,
                task_success = EXCLUDED.task_success,
                contributed_version = EXCLUDED.contributed_version,
                approval_sha256 = EXCLUDED.approval_sha256,
                source_publication_id = EXCLUDED.source_publication_id,
                version = trace_public_runs.version + 1,
                updated_at = NOW(),
                unpublished_at = NULL
             WHERE trace_public_runs.version = $16
             RETURNING {PUBLIC_RUN_COLUMNS}"
        );
        let Some(row) = tx
            .query_opt(
                &sql,
                &[
                    &write.tenant_id,
                    &write.publication_id,
                    &write.account_id,
                    &write.submission_id,
                    &write.slug,
                    &write.title,
                    &write.outcome_summary,
                    &write.correction_excerpt,
                    &write.workflow,
                    &permission,
                    &evidence,
                    &task_success,
                    &write.contributed_version,
                    &write.approval_sha256,
                    &write.source_publication_id,
                    &write.expected_publication_version,
                ],
            )
            .await
            .map_err(DatabaseError::Postgres)?
        else {
            return Err(DatabaseError::Constraint(
                crate::db::PUBLIC_RUN_VERSION_CONFLICT.to_string(),
            ));
        };
        let stored: PublicRunRow = public_run_from_row(row)?;
        if write.expected_publication_version != 0 && stored.version == 1 {
            return Err(DatabaseError::Constraint(
                crate::db::PUBLIC_RUN_VERSION_CONFLICT.to_string(),
            ));
        }
        let actor_ref = crate::account_session::account_actor_ref(
            &crate::account_session::AccountId::from_uuid(write.account_id),
        );
        let safe_metadata = serde_json::json!({"approval_sha256": write.approval_sha256});
        tx.execute(
            "INSERT INTO trace_account_audit (tenant_id, action, actor_ref, outcome, safe_metadata)
             VALUES ($1, 'public_run_publish', $2, 'success', $3)",
            &[&write.tenant_id, &actor_ref, &safe_metadata],
        )
        .await
        .map_err(DatabaseError::Postgres)?;
        let page_row = tx
            .query_opt(
                "SELECT * FROM trace_public_run_page($1, $2)",
                &[&stored.slug, &12_i32],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .ok_or_else(|| {
                DatabaseError::Serialization("public run projection is unavailable".into())
            })?;
        let page = public_run_page_from_row(&page_row)?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(PublicRunMutation { row: stored, page })
    }

    pub(super) async fn public_run_owner_state(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        submission_id: Uuid,
    ) -> Result<PublicRunOwnerData, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let sql = format!(
            "SELECT {PUBLIC_RUN_COLUMNS} FROM trace_public_runs
             WHERE tenant_id = $1 AND account_id = $2 AND submission_id = $3
             FOR SHARE"
        );
        let row = tx
            .query_opt(&sql, &[&tenant_id, &account_id, &submission_id])
            .await
            .map_err(DatabaseError::Postgres)?
            .map(public_run_from_row)
            .transpose()?;
        let retained_source_slug = if row
            .as_ref()
            .is_some_and(|row| row.source_publication_id.is_some())
        {
            tx.query_opt(
                "SELECT retained_source_slug
                 FROM trace_public_run_retained_source($1, $2, $3)",
                &[&tenant_id, &account_id, &submission_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?
            .map(|source| source.get("retained_source_slug"))
        } else {
            None
        };
        let page = match row.as_ref().filter(|row| row.unpublished_at.is_none()) {
            Some(row) => tx
                .query_opt(
                    "SELECT * FROM trace_public_run_page($1, $2)",
                    &[&row.slug, &12_i32],
                )
                .await
                .map_err(DatabaseError::Postgres)?
                .map(|page| public_run_page_from_row(&page))
                .transpose()?,
            None => None,
        };
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(PublicRunOwnerData {
            row,
            page,
            retained_source_slug,
        })
    }

    pub(super) async fn public_run_unpublish(
        &self,
        tenant_id: &str,
        account_id: Uuid,
        submission_id: Uuid,
    ) -> Result<PublicRunUnpublishMutation, DatabaseError> {
        let mut client = self.trace_pool().get().await?;
        let tx = Self::begin_trace_tenant_transaction(&mut client, tenant_id).await?;
        let changed_version = tx
            .query_opt(
                "UPDATE trace_public_runs SET unpublished_at = NOW(), updated_at = NOW(),
                    version = version + 1
                 WHERE tenant_id = $1 AND account_id = $2 AND submission_id = $3
                   AND unpublished_at IS NULL
                 RETURNING version",
                &[&tenant_id, &account_id, &submission_id],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let unpublished = changed_version.is_some();
        if unpublished {
            let actor_ref = crate::account_session::account_actor_ref(
                &crate::account_session::AccountId::from_uuid(account_id),
            );
            tx.execute(
                "INSERT INTO trace_account_audit
                    (tenant_id, action, actor_ref, outcome, safe_metadata)
                 VALUES ($1, 'public_run_unpublish', $2, 'success', '{}'::jsonb)",
                &[&tenant_id, &actor_ref],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        }
        let version = match changed_version {
            Some(row) => row.get::<_, i32>("version"),
            None => tx
                .query_opt(
                    "SELECT version FROM trace_public_runs
                     WHERE tenant_id = $1 AND account_id = $2 AND submission_id = $3",
                    &[&tenant_id, &account_id, &submission_id],
                )
                .await
                .map_err(DatabaseError::Postgres)?
                .map_or(0, |row| row.get::<_, i32>("version")),
        };
        let expected_publication_version = u32::try_from(version).map_err(|_| {
            DatabaseError::Serialization("public run version is invalid".to_string())
        })?;
        tx.commit().await.map_err(DatabaseError::Postgres)?;
        Ok(PublicRunUnpublishMutation {
            unpublished,
            expected_publication_version,
        })
    }

    pub(super) async fn public_run_page(
        &self,
        slug: &str,
        variation_limit: i64,
    ) -> Result<Option<PublicRunPageData>, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let variation_limit = variation_limit.clamp(1, 20) as i32;
        let row = client
            .query_opt(
                "SELECT * FROM trace_public_run_page($1, $2)",
                &[&slug, &variation_limit],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        let Some(row) = row else {
            return Ok(None);
        };
        public_run_page_from_row(&row).map(Some)
    }

    pub(super) async fn public_run_source(
        &self,
        slug: &str,
    ) -> Result<Option<PublicRunSourceRow>, DatabaseError> {
        let client = self.trace_pool().get().await?;
        let row = client
            .query_opt(
                "SELECT * FROM trace_resolve_public_run_source($1)",
                &[&slug],
            )
            .await
            .map_err(DatabaseError::Postgres)?;
        Ok(row.map(|row| PublicRunSourceRow {
            publication_id: row.get("publication_id"),
            slug: row.get("slug"),
            title: row.get("title"),
        }))
    }
}
