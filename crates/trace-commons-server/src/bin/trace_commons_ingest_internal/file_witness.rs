// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Private file-store evidence. This is a historical verified source proof,
//! never an assertion that the server's transformed envelope was witness-signed.

use super::*;
use trace_commons_protocol::witness_provenance::{AttestationClass, InferenceProvenance};
use trace_commons_server::redaction_witness::request::{CERTIFICATE_HEADER, SIGNATURE_HEADER};
use trace_commons_server::trace_corpus_storage::{
    TraceWitnessCertificateEvidenceWrite, TraceWitnessEvidenceClaim, TraceWitnessEvidenceCoverage,
};

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Evidence {
    certificate_json: Vec<u8>,
    signature_header: Vec<u8>,
    raw_body_sha256: String,
    certificate_version: i16,
    class: AttestationClass,
    receipt_sha256: Option<String>,
    object_key: String,
    artifact_sha256: String,
}

impl std::fmt::Debug for Evidence {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FileWitnessEvidence")
            .field("certificate_version", &self.certificate_version)
            .field("class", &self.class)
            .finish_non_exhaustive()
    }
}

impl Evidence {
    pub(super) fn retry_matches(&self, headers: &HeaderMap, body: &[u8]) -> bool {
        self.raw_body_sha256 == hex::encode(Sha256::digest(body))
            && match (
                headers.get(CERTIFICATE_HEADER),
                headers.get(SIGNATURE_HEADER),
            ) {
                (None, None) => true,
                (Some(cert), Some(sig)) => {
                    self.certificate_json == cert.as_bytes()
                        && self.signature_header == sig.as_bytes()
                }
                _ => false,
            }
    }

    fn same_source(&self, other: &Self) -> bool {
        self.raw_body_sha256 == other.raw_body_sha256
            && self.certificate_json == other.certificate_json
            && self.signature_header == other.signature_header
    }
}

/// Ownership is cross-process and crash-released. Keep the lock file permanently:
/// unlinking it would let different processes lock different inodes for one ID.
/// Distinct namespaces avoid recursively taking the submit lock at metadata commit.
pub(super) fn lock(
    root: &Path,
    tenant_id: &str,
    id: Uuid,
    namespace: &str,
) -> anyhow::Result<std::fs::File> {
    let directory = root
        .join("tenants")
        .join(tenant_storage_key(tenant_id))
        .join(namespace);
    std::fs::create_dir_all(&directory)?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(directory.join(format!("{id}.lock")))?;
    file.try_lock()
        .map_err(|_| anyhow::anyhow!("submission_file_lock_unavailable"))?;
    Ok(file)
}

/// Preserve the first proof through remediation. A headerless changed-body
/// remediation can proceed, but can never associate that proof with its new object.
pub(super) fn for_submission(
    envelope: &TraceContributionEnvelope,
    record: &TraceCommonsSubmissionRecord,
    prior: Option<&TraceCommonsSubmissionRecord>,
    verified: Option<&VerifiedWitnessCertificate>,
    headers: &HeaderMap,
    body: &[u8],
) -> anyhow::Result<Option<Evidence>> {
    // Bind only this handler's prepared object, never an unrelated writer's
    // replacement observed on disk after store_envelope. The reader checks the
    // actual object against this identity before it exposes a current claim.
    let prepared_digest = || -> anyhow::Result<String> {
        match &record.artifact_receipt {
            Some(receipt) => Ok(receipt.ciphertext_sha256.clone()),
            None => Ok(hex::encode(Sha256::digest(serde_json::to_vec_pretty(
                envelope,
            )?))),
        }
    };
    if let Some(evidence) = prior.and_then(|prior| prior.witness_evidence.as_ref()) {
        let mut evidence = evidence.clone();
        if evidence.retry_matches(headers, body) {
            evidence.object_key = record.object_key.clone();
            evidence.artifact_sha256 = prepared_digest()?;
        }
        return Ok(Some(evidence));
    }
    let Some(verified) = verified else {
        return Ok(None);
    };
    let digest = prepared_digest()?;
    // Reuse the trusted boundary's exact header/body validation. No caller can
    // synthesize verified provenance from an envelope field or parsed headers.
    let source = TraceWitnessCertificateEvidenceWrite::from_verified(
        &record.tenant_id,
        record.submission_id,
        verified,
        headers,
        body,
        &digest,
    )?;
    let (class, receipt_sha256) = match verified.inference_provenance() {
        InferenceProvenance::Unattested => (AttestationClass::Unattested, None),
        InferenceProvenance::Attested(call) => {
            (call.class(), Some(call.receipt_sha256().to_string()))
        }
    };
    Ok(Some(Evidence {
        certificate_json: headers[CERTIFICATE_HEADER].as_bytes().to_vec(),
        signature_header: headers[SIGNATURE_HEADER].as_bytes().to_vec(),
        raw_body_sha256: source.raw_body_sha256().to_string(),
        certificate_version: source.certificate_version(),
        class,
        receipt_sha256,
        object_key: record.object_key.clone(),
        artifact_sha256: digest,
    }))
}

fn current_object_digest(
    state: &AppState,
    record: &TraceCommonsSubmissionRecord,
) -> anyhow::Result<String> {
    if let Some(receipt) = record.artifact_receipt.as_ref() {
        let store = state
            .artifact_store
            .as_ref()
            .context("witness_artifact_store_unavailable")?;
        anyhow::ensure!(
            trace_record_artifact_receipt_matches_store(
                record,
                receipt,
                store.object_store_name()
            )?,
            "witness_artifact_store_mismatch"
        );
        // get_json verifies the actual ciphertext, receipt binding and decryption.
        let _: TraceContributionEnvelope = read_envelope_by_record(state, record)?;
        Ok(receipt.ciphertext_sha256.clone())
    } else {
        Ok(hex::encode(Sha256::digest(std::fs::read(
            state.root.join(&record.object_key),
        )?)))
    }
}

/// Serialize only private durable metadata. Atomic replacement keeps a crash
/// from publishing a partial proof/record pair. Merge under a separate lock so
/// stale non-submit writers cannot erase a proof first committed meanwhile.
pub(super) fn write_record(
    root: &Path,
    record: &TraceCommonsSubmissionRecord,
) -> anyhow::Result<()> {
    let _lock = lock(
        root,
        &record.tenant_id,
        record.submission_id,
        "metadata-locks",
    )?;
    let mut record = record.clone();
    if let Some(existing) = read_submission_record(root, &record.tenant_id, record.submission_id)?
        && let Some(proof) = existing.witness_evidence
    {
        match &record.witness_evidence {
            Some(incoming) => {
                anyhow::ensure!(proof.same_source(incoming), "witness_evidence_conflict")
            }
            None => record.witness_evidence = Some(proof),
        }
    }
    let path = submission_metadata_path(root, &record.tenant_id, record.submission_id);
    let parent = path
        .parent()
        .context("submission_metadata_parent_missing")?;
    std::fs::create_dir_all(parent)?;
    let mut temporary = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut temporary, &record)?;
    temporary.as_file().sync_all()?;
    temporary
        .persist(&path)
        .map_err(|_| anyhow::anyhow!("submission_metadata_replace_failed"))?;
    // Windows does not support opening directories with File::open. Replacement
    // there is atomic via tempfile; Unix additionally syncs the rename itself.
    #[cfg(unix)]
    std::fs::File::open(parent)?.sync_all()?;
    Ok(())
}

/// Future gate/credit/export seam; this slice does not activate a policy consumer.
/// Auth-derived tenant and ownership, durable status/tombstones, and the actual
/// current object are all read here. No arbitrary caller digest can qualify proof.
#[allow(dead_code)]
pub(super) fn current_claim(
    state: &AppState,
    tenant: &TenantCtx,
    id: Uuid,
) -> anyhow::Result<TraceWitnessEvidenceClaim> {
    use TraceWitnessEvidenceCoverage as Coverage;
    let mut claim = TraceWitnessEvidenceClaim {
        class: AttestationClass::Unattested,
        coverage: Coverage::Missing,
        raw_body_sha256: None,
        receipt_sha256: None,
    };
    anyhow::ensure!(
        state.db_mirror.is_none(),
        "file_witness_read_requires_file_only_store"
    );
    let Some(record) = tenant.read_submission_record(&state.root, id)? else {
        return Ok(claim);
    };
    if !tenant.can_access_submission(&record) {
        return Ok(claim);
    }
    let Some(evidence) = record.witness_evidence.as_ref() else {
        return Ok(claim);
    };
    claim.raw_body_sha256 = Some(evidence.raw_body_sha256.clone());
    claim.receipt_sha256 = evidence.receipt_sha256.clone();
    let tombstones = read_all_revocations(&state.root, tenant.tenant_id())?;
    let derived = tenant.read_derived_record(&state.root, id)?;
    let revoked = tombstones.iter().any(|t| {
        t.submission_id == id
            || t.canonical_summary_hash.as_ref().is_some_and(|hash| {
                derived
                    .as_ref()
                    .is_some_and(|d| &d.canonical_summary_hash == hash)
            })
            || t.redaction_hash.as_ref().is_some_and(|hash| {
                redaction_hash_for_record(state, &record).as_ref() == Some(hash)
            })
    });
    claim.coverage = if record.status != TraceCorpusStatus::Accepted
        || record.purged_at.is_some()
        || record.is_expired_at(Utc::now())
        || revoked
    {
        Coverage::Inactive
    } else if evidence.certificate_version == 1 {
        Coverage::LegacyV1
    } else if evidence.certificate_version != 2 || evidence.class == AttestationClass::Unattested {
        Coverage::ExplicitUnattested
    } else if evidence.object_key != record.object_key
        || current_object_digest(state, &record).ok().as_ref() != Some(&evidence.artifact_sha256)
    {
        Coverage::ArtifactMismatch
    } else {
        claim.class = evidence.class;
        Coverage::VerifiedV2
    };
    Ok(claim)
}
