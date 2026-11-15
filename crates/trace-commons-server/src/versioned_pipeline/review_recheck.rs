// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Immutable corrected privacy versions, separately retained from the first pass.

use super::*;
use trace_commons_protocol::trace_contribution::{
    PiiClassifyPolicy, ReviewCorrectionStatus, ReviewPrivacyRecheckEvidence,
};

static REVIEW_RECHECK_WORK: tokio::sync::Semaphore = tokio::sync::Semaphore::const_new(1);
pub const PIPELINE_REVIEW_RECHECK_BUSY_LABEL: &str = "pipeline_review_recheck_busy";

pub const PIPELINE_REVIEW_RECHECK_STALE_LABEL: &str = "pipeline_review_recheck_stale";
pub const PIPELINE_REVIEW_RECHECK_SOURCE_CHANGED_LABEL: &str =
    "pipeline_review_recheck_source_changed";
pub const PIPELINE_REVIEW_RECHECK_NOT_FOUND_LABEL: &str = "pipeline_review_recheck_not_found";
pub const PIPELINE_REVIEW_RECHECK_BINDING_REQUIRED_LABEL: &str = "pipeline_review_recheck_required";
pub const PIPELINE_REVIEW_RECHECK_UNRESOLVED_LABEL: &str = "pipeline_review_recheck_unresolved";

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineReviewRecheckBinding {
    pub recheck_id: Uuid,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PipelineReviewRecheck {
    pub recheck_id: Uuid,
    pub run_id: Uuid,
    pub submission_id: Uuid,
    pub ordinal: i64,
    pub parent_recheck_id: Option<Uuid>,
    pub source_object_ref_id: Uuid,
    pub source_content_hash: String,
    pub object_ref_id: Uuid,
    pub content_hash: String,
    pub evidence: ReviewPrivacyRecheckEvidence,
    pub residual_pii_risk: ResidualPiiRisk,
    #[serde(skip_serializing, default)]
    pub redaction_counts: BTreeMap<String, u32>,
    pub redaction_pipeline_version: String,
    pub created_at: DateTime<Utc>,
}

impl PipelineReviewRecheck {
    pub fn binding(&self) -> PipelineReviewRecheckBinding {
        PipelineReviewRecheckBinding {
            recheck_id: self.recheck_id,
            content_hash: self.content_hash.clone(),
        }
    }

    pub fn approval_evidence_complete(&self) -> bool {
        self.evidence.policy == PiiClassifyPolicy::AllEvents
            && self.evidence.coverage_complete
            && self.evidence.residual_scan_complete
            && self.evidence.correction_status != ReviewCorrectionStatus::Unresolved
            && !self
                .evidence
                .basis
                .iter()
                .any(|condition| condition.forces_high())
    }
}

#[derive(Debug, Clone)]
pub(super) struct ReviewRecheckSource {
    pub claim_expires_at: DateTime<Utc>,
    pub run: PipelineRunRecord,
    pub ordinal: i64,
    pub parent_recheck_id: Option<Uuid>,
    pub object_ref_id: Uuid,
    pub content_hash: String,
}

#[derive(Debug)]
pub struct PreparedReviewRecheck {
    pub(super) inner: PreparedReviewRecheckInner,
}

#[derive(Debug)]
pub(super) enum PreparedReviewRecheckInner {
    Existing(Box<PipelineReviewRecheck>),
    New(Box<PreparedReviewRecheckNew>),
}

#[derive(Debug)]
pub(super) struct PreparedReviewRecheckNew {
    claim: PipelineReviewClaim,
    source: ReviewRecheckSource,
    recheck_id: Uuid,
    receipt: EncryptedTraceArtifactReceipt,
    object_ref: TraceObjectRefWrite,
    content_hash: String,
    evidence: ReviewPrivacyRecheckEvidence,
    privacy: trace_commons_protocol::trace_contribution::PrivacyMetadata,
}

fn stale() -> DatabaseError {
    DatabaseError::Constraint(PIPELINE_REVIEW_RECHECK_STALE_LABEL.to_string())
}

pub(super) fn recheck_from_row(row: &Row) -> Result<PipelineReviewRecheck, DatabaseError> {
    Ok(PipelineReviewRecheck {
        recheck_id: row.get("recheck_id"),
        run_id: row.get("run_id"),
        submission_id: row.get("submission_id"),
        ordinal: row.get("ordinal"),
        parent_recheck_id: row.get("parent_recheck_id"),
        source_object_ref_id: row.get("source_object_ref_id"),
        source_content_hash: row.get("source_content_hash"),
        object_ref_id: row.get("object_ref_id"),
        content_hash: row.get("content_hash"),
        evidence: serde_json::from_value(row.get("evidence")).map_err(|_| {
            DatabaseError::Serialization("review recheck evidence is invalid".to_string())
        })?,
        residual_pii_risk: serde_json::from_value(serde_json::Value::String(
            row.get("residual_pii_risk"),
        ))
        .map_err(|_| DatabaseError::Serialization("review recheck risk is invalid".to_string()))?,
        redaction_counts: serde_json::from_value(row.get("redaction_counts")).map_err(|_| {
            DatabaseError::Serialization("review recheck counts are invalid".to_string())
        })?,
        redaction_pipeline_version: row.get("redaction_pipeline_version"),
        created_at: row.get("created_at"),
    })
}

pub(super) async fn latest_recheck_on_tx(
    tx: &Transaction<'_>,
    tenant_id: &str,
    run_id: Uuid,
) -> Result<Option<PipelineReviewRecheck>, DatabaseError> {
    tx.query_opt("SELECT * FROM pipeline_review_rechecks WHERE tenant_id=$1 AND run_id=$2 ORDER BY ordinal DESC LIMIT 1", &[&tenant_id, &run_id])
        .await?.as_ref().map(recheck_from_row).transpose()
}

async fn lock_review_recheck_source(
    tx: &Transaction<'_>,
    claim: &PipelineReviewClaim,
) -> Result<ReviewRecheckSource, DatabaseError> {
    let row = tx
        .query_opt(
            concat!(
                "SELECT p.* FROM pipeline_runs p WHERE p.tenant_id=$1 AND p.run_id=$2 AND (",
                run_waiting_for_review_sql!(),
                ") AND p.approved_revision_id IS NULL FOR UPDATE"
            ),
            &[&claim.tenant_id, &claim.run_id],
        )
        .await?
        .ok_or_else(stale)?;
    let run = pipeline_run_from_row(&row)?;
    let live_claim = tx.query_opt("SELECT lease_expires_at FROM pipeline_review_claims WHERE tenant_id=$1 AND run_id=$2 AND lease_token=$3 AND reviewer_principal_ref=$4 AND lease_expires_at>clock_timestamp() FOR UPDATE", &[&claim.tenant_id,&claim.run_id,&claim.lease_token,&claim.reviewer_principal_ref]).await?;
    let claim_expires_at = live_claim
        .as_ref()
        .map(|row| row.get::<_, DateTime<Utc>>(0));
    if claim_expires_at.is_none()
        || !review_submission_is_operable(tx, &claim.tenant_id, run.submission_id).await?
    {
        return Err(stale());
    }
    let assessed: bool=tx.query_one("SELECT EXISTS(SELECT 1 FROM pipeline_review_assessments WHERE tenant_id=$1 AND run_id=$2 AND phase='review')", &[&claim.tenant_id,&claim.run_id]).await?.get(0);
    if assessed {
        return Err(stale());
    }
    let latest = latest_recheck_on_tx(tx, &claim.tenant_id, claim.run_id).await?;
    let (ordinal, parent_recheck_id, object_ref_id, content_hash) = match latest {
        Some(candidate) => (
            candidate.ordinal.checked_add(1).ok_or_else(stale)?,
            Some(candidate.recheck_id),
            candidate.object_ref_id,
            candidate.content_hash,
        ),
        None => (
            1,
            None,
            run.privacy_pass_object_ref_id.ok_or_else(stale)?,
            run.privacy_pass_content_hash.clone().ok_or_else(stale)?,
        ),
    };
    let source_live=tx.query_opt("SELECT 1 FROM trace_object_refs WHERE tenant_id=$1 AND submission_id=$2 AND object_ref_id=$3 AND invalidated_at IS NULL AND deleted_at IS NULL FOR SHARE", &[&claim.tenant_id,&run.submission_id,&object_ref_id]).await?.is_some();
    if !source_live {
        return Err(stale());
    }
    let claim_still_live:bool=tx.query_one("SELECT lease_expires_at>clock_timestamp() FROM pipeline_review_claims WHERE tenant_id=$1 AND run_id=$2", &[&claim.tenant_id,&claim.run_id]).await?.get(0);
    if !claim_still_live {
        return Err(stale());
    }
    Ok(ReviewRecheckSource {
        claim_expires_at: claim_expires_at.ok_or_else(stale)?,
        run,
        ordinal,
        parent_recheck_id,
        object_ref_id,
        content_hash,
    })
}

fn source_changed() -> DatabaseError {
    DatabaseError::Constraint(PIPELINE_REVIEW_RECHECK_SOURCE_CHANGED_LABEL.to_string())
}

fn raw_fields_preserved(original: &serde_json::Value, supported: &serde_json::Value) -> bool {
    match (original, supported) {
        (serde_json::Value::Object(left), serde_json::Value::Object(right)) => {
            left.iter().all(|(key, value)| {
                right
                    .get(key)
                    .is_some_and(|other| raw_fields_preserved(value, other))
            })
        }
        (serde_json::Value::Array(left), serde_json::Value::Array(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(a, b)| raw_fields_preserved(a, b))
        }
        _ => original == supported,
    }
}

fn same_source(actual: &ReviewRecheckSource, expected: &ReviewRecheckSource) -> bool {
    actual.run.run_id == expected.run.run_id
        && actual.run.submission_id == expected.run.submission_id
        && actual.ordinal == expected.ordinal
        && actual.object_ref_id == expected.object_ref_id
        && actual.content_hash == expected.content_hash
        && actual.parent_recheck_id == expected.parent_recheck_id
}

impl PgPipelineStore {
    pub async fn get_review_recheck(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        recheck_id: Uuid,
    ) -> Result<Option<PipelineReviewRecheck>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::read_committed_tenant_transaction(&mut client, tenant_id).await?;
        let row=tx.query_opt("SELECT * FROM pipeline_review_rechecks WHERE tenant_id=$1 AND run_id=$2 AND recheck_id=$3", &[&tenant_id,&run_id,&recheck_id]).await?;
        let result = row.as_ref().map(recheck_from_row).transpose()?;
        tx.commit().await?;
        Ok(result)
    }

    pub async fn list_review_rechecks(
        &self,
        tenant_id: &str,
        run_id: Uuid,
        limit: usize,
    ) -> Result<Vec<PipelineReviewRecheck>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::read_committed_tenant_transaction(&mut client, tenant_id).await?;
        let rows=tx.query("SELECT * FROM pipeline_review_rechecks WHERE tenant_id=$1 AND run_id=$2 ORDER BY ordinal DESC LIMIT $3", &[&tenant_id,&run_id,&i64::try_from(limit.clamp(1,500)).unwrap_or(500)]).await?;
        let result = rows.iter().map(recheck_from_row).collect();
        tx.commit().await?;
        result
    }

    async fn review_recheck_source(
        &self,
        claim: &PipelineReviewClaim,
        recheck_id: Uuid,
        expected_source_hash: &str,
    ) -> Result<Result<ReviewRecheckSource, PipelineReviewRecheck>, DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::read_committed_tenant_transaction(&mut client, &claim.tenant_id).await?;
        let source = lock_review_recheck_source(&tx, claim).await?;
        let existing=tx.query_opt("SELECT * FROM pipeline_review_rechecks WHERE tenant_id=$1 AND run_id=$2 AND recheck_id=$3", &[&claim.tenant_id,&claim.run_id,&recheck_id]).await?;
        let result = if let Some(row) = existing {
            let candidate = recheck_from_row(&row)?;
            if candidate.source_content_hash != expected_source_hash {
                return Err(source_changed());
            }
            Err(candidate)
        } else {
            if source.content_hash != expected_source_hash {
                return Err(source_changed());
            }
            Ok(source)
        };
        tx.commit().await?;
        Ok(result)
    }

    async fn stage_review_recheck(
        &self,
        claim: &PipelineReviewClaim,
        source: &ReviewRecheckSource,
        recheck_id: Uuid,
        receipt: &EncryptedTraceArtifactReceipt,
    ) -> Result<(), DatabaseError> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::read_committed_tenant_transaction(&mut client, &claim.tenant_id).await?;
        let actual = lock_review_recheck_source(&tx, claim).await?;
        if !same_source(&actual, source) {
            return Err(source_changed());
        }
        let cleanup_after = actual.claim_expires_at + Duration::minutes(1);
        let inserted=tx.query_opt("INSERT INTO pipeline_attempt_artifacts(tenant_id,run_id,lease_token,artifact,object_key,ciphertext_sha256,cleanup_after) VALUES($1,$2,$3,'review-recheck',$4,$5,$6) ON CONFLICT (tenant_id,run_id,lease_token,artifact) DO NOTHING RETURNING object_key", &[&claim.tenant_id,&claim.run_id,&recheck_id,&receipt.object_key,&receipt.ciphertext_sha256,&cleanup_after]).await?;
        if inserted.is_none() {
            return Err(DatabaseError::Constraint(
                PIPELINE_REVIEW_RECHECK_STALE_LABEL.to_string(),
            ));
        }
        tx.commit().await?;
        Ok(())
    }

    async fn finish_review_recheck_record(
        &self,
        prepared: &PreparedReviewRecheckInner,
    ) -> Result<PipelineReviewRecheck, DatabaseError> {
        let PreparedReviewRecheckInner::New(prepared) = prepared else {
            return Err(stale());
        };
        let PreparedReviewRecheckNew {
            claim,
            source,
            recheck_id,
            receipt,
            object_ref,
            content_hash,
            evidence,
            privacy,
        } = prepared.as_ref();
        let mut client = self.backend.trace_pool().get().await?;
        let tx = Self::read_committed_tenant_transaction(&mut client, &claim.tenant_id).await?;
        let actual = lock_review_recheck_source(&tx, claim).await?;
        if let Some(row)=tx.query_opt("SELECT * FROM pipeline_review_rechecks WHERE tenant_id=$1 AND run_id=$2 AND recheck_id=$3", &[&claim.tenant_id,&claim.run_id,recheck_id]).await? {
            let existing=recheck_from_row(&row)?;
            if existing.source_content_hash!=source.content_hash || existing.content_hash!=*content_hash {return Err(stale());}
            tx.commit().await?;return Ok(existing);
        }
        if !same_source(&actual, source)
            || object_ref.tenant_id != claim.tenant_id
            || object_ref.submission_id != source.run.submission_id
        {
            return Err(stale());
        }
        let moved=tx.execute("UPDATE pipeline_attempt_artifacts SET state='committed', committed_at=clock_timestamp() WHERE tenant_id=$1 AND run_id=$2 AND lease_token=$3 AND artifact='review-recheck' AND state='staged' AND object_key=$4 AND ciphertext_sha256=$5 AND cleanup_after>clock_timestamp() AND EXISTS(SELECT 1 FROM pipeline_review_claims c WHERE c.tenant_id=$1 AND c.run_id=$2 AND c.lease_token=$6 AND c.lease_expires_at>clock_timestamp())", &[&claim.tenant_id,&claim.run_id,recheck_id,&receipt.object_key,&receipt.ciphertext_sha256,&claim.lease_token]).await?;
        if moved != 1 {
            return Err(DatabaseError::Constraint(
                PIPELINE_ATTEMPT_ARTIFACT_MISSING_LABEL.to_string(),
            ));
        }
        tx.execute("INSERT INTO trace_object_refs(tenant_id,submission_id,object_ref_id,artifact_kind,object_store,object_key,content_sha256,encryption_key_ref,size_bytes,compression,created_by_job_id) VALUES($1,$2,$3,'review_snapshot',$4,$5,$6,$7,$8,$9,$10)", &[&object_ref.tenant_id,&object_ref.submission_id,&object_ref.object_ref_id,&object_ref.object_store,&object_ref.object_key,&object_ref.content_sha256,&object_ref.encryption_key_ref,&object_ref.size_bytes,&object_ref.compression,&object_ref.created_by_job_id]).await?;
        let evidence_json = serde_json::to_value(evidence).map_err(|_| {
            DatabaseError::Serialization("review recheck evidence encode failed".to_string())
        })?;
        let risk = enum_string(&privacy.residual_pii_risk).map_err(|_| {
            DatabaseError::Serialization("review recheck risk encode failed".to_string())
        })?;
        let counts = serde_json::to_value(&privacy.redaction_counts).map_err(|_| {
            DatabaseError::Serialization("review recheck counts encode failed".to_string())
        })?;
        let row=tx.query_one("INSERT INTO pipeline_review_rechecks(tenant_id,run_id,recheck_id,submission_id,ordinal,parent_recheck_id,source_object_ref_id,source_content_hash,object_ref_id,content_hash,evidence,residual_pii_risk,reviewer_principal_ref,redaction_counts,redaction_pipeline_version) VALUES($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15) RETURNING *", &[&claim.tenant_id,&claim.run_id,recheck_id,&source.run.submission_id,&source.ordinal,&source.parent_recheck_id,&source.object_ref_id,&source.content_hash,&object_ref.object_ref_id,content_hash,&evidence_json,&risk,&claim.reviewer_principal_ref,&counts,&privacy.redaction_pipeline_version]).await?;
        let candidate = recheck_from_row(&row)?;
        // Fence an older REPEATABLE READ reviewer that waited on this run:
        // it must not continue on a snapshot from before the candidate existed.
        tx.execute("UPDATE pipeline_runs SET updated_at=clock_timestamp() WHERE tenant_id=$1 AND run_id=$2", &[&claim.tenant_id,&claim.run_id]).await?;
        tx.commit().await?;
        Ok(candidate)
    }
}

impl PipelineService {
    pub async fn prepare_review_recheck(
        &self,
        claim: &PipelineReviewClaim,
        recheck_id: Uuid,
        expected_source_hash: &str,
    ) -> anyhow::Result<PreparedReviewRecheck> {
        let source = match self
            .store
            .review_recheck_source(claim, recheck_id, expected_source_hash)
            .await?
        {
            Ok(source) => source,
            Err(existing) => {
                return Ok(PreparedReviewRecheck {
                    inner: PreparedReviewRecheckInner::Existing(Box::new(existing)),
                });
            }
        };
        let _permit = REVIEW_RECHECK_WORK
            .try_acquire()
            .map_err(|_| anyhow::anyhow!(PIPELINE_REVIEW_RECHECK_BUSY_LABEL))?;
        let mut claim = claim.clone();
        claim.lease_expires_at = source.claim_expires_at;
        let bytes = self
            .load_object_bytes(&source.run, source.object_ref_id)
            .await?;
        anyhow::ensure!(
            sha256_prefixed(&bytes) == source.content_hash,
            PIPELINE_ARTIFACT_INTEGRITY_FAILED_LABEL
        );
        let mut envelope: TraceContributionEnvelope = serde_json::from_slice(&bytes)
            .map_err(|_| anyhow::anyhow!(PIPELINE_ARTIFACT_INTEGRITY_FAILED_LABEL))?;
        let raw: serde_json::Value = serde_json::from_slice(&bytes)?;
        let supported: serde_json::Value = serde_json::from_slice(&serde_json::to_vec(&envelope)?)?;
        anyhow::ensure!(
            raw_fields_preserved(&raw, &supported),
            "pipeline_review_recheck_unsupported_fields"
        );
        let identity = (
            envelope.submission_id,
            envelope.trace_id,
            envelope.contributor.clone(),
            envelope.consent.clone(),
            envelope.outcome.human_correction.clone(),
        );
        let privacy = self
            .privacy
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!(PIPELINE_PRIVACY_CONTROL_MISSING_LABEL))?;
        let remaining =
            claim.lease_expires_at.signed_duration_since(Utc::now()) - Duration::seconds(30);
        anyhow::ensure!(
            remaining > Duration::zero(),
            PIPELINE_REVIEW_RECHECK_STALE_LABEL
        );
        let timeout = self.privacy_pass_timeout().min(remaining.to_std()?);
        let evidence = tokio::time::timeout(timeout, privacy.recheck_for_review(&mut envelope))
            .await
            .map_err(|_| anyhow::anyhow!(PIPELINE_PRIVACY_CLASSIFICATION_FAILED_LABEL))??;
        anyhow::ensure!(
            envelope.submission_id == identity.0
                && envelope.trace_id == identity.1
                && envelope.contributor == identity.2
                && envelope.consent.scopes == identity.3.scopes
                && envelope.consent.policy_version == identity.3.policy_version
                && envelope.consent.revocable == identity.3.revocable
                && envelope.outcome.human_correction == identity.4,
            PIPELINE_ARTIFACT_INTEGRITY_FAILED_LABEL
        );
        let output = serde_json::to_vec(&envelope)?;
        let content_hash = sha256_prefixed(&output);
        let wrapper = encode_pipeline_artifact_bytes(&output)?;
        let object_id = pipeline_attempt_object_id(
            PipelineAttemptArtifact::ReviewRecheck.as_str(),
            claim.run_id,
            recheck_id,
        );
        let store = self.artifact_store.clone();
        let tenant = pipeline_tenant_storage_ref(&claim.tenant_id);
        let prepared = artifact_store_call(move || {
            store.prepare_serialized_json(
                tenant.as_str(),
                TraceArtifactKind::ContributionEnvelope,
                &object_id,
                &wrapper,
            )
        })
        .await?;
        self.store
            .stage_review_recheck(&claim, &source, recheck_id, prepared.receipt())
            .await?;
        let store = self.artifact_store.clone();
        let receipt = artifact_store_call(move || store.publish_serialized_json(&prepared)).await?;
        let mut object_ref =
            privacy_pass_object_ref(&source.run, &receipt, output.len(), &self.object_store_name);
        object_ref.object_ref_id = Uuid::new_v5(
            &Uuid::NAMESPACE_URL,
            format!(
                "tracecommons:pipeline-review-recheck-object:{}:{recheck_id}",
                claim.run_id
            )
            .as_bytes(),
        );
        Ok(PreparedReviewRecheck {
            inner: PreparedReviewRecheckInner::New(Box::new(PreparedReviewRecheckNew {
                claim: claim.clone(),
                source,
                recheck_id,
                receipt,
                object_ref,
                content_hash,
                evidence,
                privacy: envelope.privacy,
            })),
        })
    }

    pub async fn finish_review_recheck(
        &self,
        prepared: PreparedReviewRecheck,
    ) -> anyhow::Result<PipelineReviewRecheck> {
        if let PreparedReviewRecheckInner::Existing(existing) = prepared.inner {
            return Ok(*existing);
        }
        match self
            .store
            .finish_review_recheck_record(&prepared.inner)
            .await
        {
            Ok(candidate) => Ok(candidate),
            Err(error) => {
                if phase_commit_refused(&error) {
                    self.discard_review_recheck(&prepared).await?;
                }
                Err(error.into())
            }
        }
    }

    pub async fn discard_review_recheck(
        &self,
        prepared: &PreparedReviewRecheck,
    ) -> anyhow::Result<()> {
        if let PreparedReviewRecheckInner::New(prepared) = &prepared.inner {
            self.delete_attempt_objects(
                &prepared.claim.tenant_id,
                std::iter::once(&prepared.receipt),
                "review_recheck_discard_failed",
            )
            .await;
        }
        Ok(())
    }

    pub async fn load_review_recheck_content(
        &self,
        claim: &PipelineReviewClaim,
        recheck_id: Uuid,
    ) -> anyhow::Result<Vec<u8>> {
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::read_committed_tenant_transaction(&mut client, &claim.tenant_id)
            .await?;
        let snapshot = lock_review_recheck_source(&tx, claim).await?;
        let row=tx.query_opt("SELECT * FROM pipeline_review_rechecks WHERE tenant_id=$1 AND run_id=$2 AND recheck_id=$3", &[&claim.tenant_id,&claim.run_id,&recheck_id]).await?.ok_or_else(||DatabaseError::Constraint(PIPELINE_REVIEW_RECHECK_NOT_FOUND_LABEL.to_string()))?;
        let candidate = recheck_from_row(&row)?;
        tx.commit().await?;
        let bytes = self
            .load_object_bytes(&snapshot.run, candidate.object_ref_id)
            .await?;
        anyhow::ensure!(
            sha256_prefixed(&bytes) == candidate.content_hash,
            PIPELINE_ARTIFACT_INTEGRITY_FAILED_LABEL
        );
        let mut client = self.backend.trace_pool().get().await?;
        let tx = PgPipelineStore::read_committed_tenant_transaction(&mut client, &claim.tenant_id)
            .await?;
        lock_review_recheck_source(&tx, claim).await?;
        let live_ref=tx.query_opt("SELECT 1 FROM trace_object_refs WHERE tenant_id=$1 AND submission_id=$2 AND object_ref_id=$3 AND invalidated_at IS NULL AND deleted_at IS NULL FOR SHARE", &[&claim.tenant_id,&candidate.submission_id,&candidate.object_ref_id]).await?.is_some();
        if !live_ref {
            return Err(stale().into());
        }
        tx.commit().await?;
        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_candidate() -> PipelineReviewRecheck {
        PipelineReviewRecheck {
            recheck_id: Uuid::new_v4(),
            run_id: Uuid::new_v4(),
            submission_id: Uuid::new_v4(),
            ordinal: 1,
            parent_recheck_id: None,
            source_object_ref_id: Uuid::new_v4(),
            source_content_hash: format!("sha256:{}", "1".repeat(64)),
            object_ref_id: Uuid::new_v4(),
            content_hash: format!("sha256:{}", "2".repeat(64)),
            evidence: ReviewPrivacyRecheckEvidence {
                basis: vec![ResidualRiskCondition::ConsentContentFlag],
                coverage_complete: true,
                correction_status: ReviewCorrectionStatus::CheckedUnchanged,
                residual_scan_complete: true,
                useful_classifier_result: false,
                policy: PiiClassifyPolicy::AllEvents,
            },
            residual_pii_risk: ResidualPiiRisk::High,
            redaction_counts: BTreeMap::new(),
            redaction_pipeline_version: "review-test".to_string(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn complete_recheck_can_be_manually_assessed_without_resetting_prior_high() {
        let candidate = complete_candidate();
        assert!(candidate.approval_evidence_complete());
        assert_eq!(candidate.residual_pii_risk, ResidualPiiRisk::High);
        assert!(!candidate.evidence.useful_classifier_result);
    }

    #[test]
    fn partial_coverage_survivors_and_unresolved_corrections_never_authorize_approval() {
        let mut candidate = complete_candidate();
        candidate.evidence.policy = PiiClassifyPolicy::ProseOnly;
        assert!(!candidate.approval_evidence_complete());
        candidate = complete_candidate();
        candidate.evidence.coverage_complete = false;
        assert!(!candidate.approval_evidence_complete());
        candidate = complete_candidate();
        candidate.evidence.residual_scan_complete = false;
        assert!(!candidate.approval_evidence_complete());
        candidate = complete_candidate();
        candidate.evidence.correction_status = ReviewCorrectionStatus::Unresolved;
        assert!(!candidate.approval_evidence_complete());
        for finding in [
            ResidualRiskCondition::KeyFinding,
            ResidualRiskCondition::CoverageIncomplete,
            ResidualRiskCondition::ResidualSurvivor,
            ResidualRiskCondition::ResidualScanUnavailable,
        ] {
            candidate = complete_candidate();
            candidate.evidence.basis.push(finding);
            assert!(!candidate.approval_evidence_complete());
        }
    }

    #[test]
    fn typed_wire_coverage_refuses_unknown_content_and_changed_numbers() {
        let original = serde_json::json!({"events":[{"structured_payload":{"private_field":"must be examined"}}],"quality":0.73});
        let dropped = serde_json::json!({"events":[{"structured_payload":{}}],"quality":0.73});
        assert!(!raw_fields_preserved(&original, &dropped));
        let mut changed = original.clone();
        changed["quality"] = serde_json::json!(0.72);
        assert!(!raw_fields_preserved(&original, &changed));
        let mut with_defaults = original.clone();
        with_defaults["new_default"] = serde_json::json!(false);
        assert!(raw_fields_preserved(&original, &with_defaults));
    }
}
