// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use std::collections::BTreeMap;

use axum::http::HeaderMap;
use k256::ecdsa::SigningKey;
use secrecy::SecretString;
use sha2::{Digest, Sha256};
use sha3::Keccak256;
use trace_commons_protocol::trace_contribution::ResidualPiiRisk;
use trace_commons_protocol::witness_provenance::{
    AttestationClass, FinalCallAttestation, InferenceProvenance, inference_provenance_json,
};
use trace_commons_server::config::{DatabaseConfig, SslMode};
use trace_commons_server::db::{Database, postgres::PgBackend};
use trace_commons_server::redaction_witness::certificate::{
    CertificateDetails, WitnessCertificate,
};
use trace_commons_server::redaction_witness::request::{CERTIFICATE_HEADER, SIGNATURE_HEADER};
use trace_commons_server::redaction_witness::verification::{
    WitnessPin, verify_witness_certificate,
};
use trace_commons_server::trace_corpus_storage::{
    TraceCorpusStatus, TraceCorpusStore, TraceObjectArtifactKind, TraceObjectRefWrite,
    TraceSubmissionWrite, TraceWitnessCertificateEvidenceWrite, TraceWitnessEvidenceCoverage,
};
use uuid::Uuid;

const BODY: &[u8] = b"{\"witnessed\":true}";
const MEASUREMENT: &str = "c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1c1";

fn sample_submission(tenant_id: &str, submission_id: Uuid) -> TraceSubmissionWrite {
    TraceSubmissionWrite {
        tenant_id: tenant_id.into(),
        submission_id,
        trace_id: Uuid::new_v4(),
        auth_principal_ref: "principal:test".into(),
        contributor_pseudonym: None,
        submitted_tenant_scope_ref: None,
        schema_version: "v1".into(),
        consent_policy_version: "v1".into(),
        consent_scopes: vec![],
        allowed_uses: vec![],
        retention_policy_id: "standard".into(),
        status: TraceCorpusStatus::Accepted,
        privacy_risk: "low".into(),
        redaction_pipeline_version: "test".into(),
        redaction_counts: BTreeMap::new(),
        redaction_hash: "sha256:test".into(),
        canonical_summary_hash: None,
        submission_score: None,
        credit_points_pending: None,
        credit_points_final: None,
        expires_at: None,
        residual_risk_basis: None,
    }
}

fn signed_evidence(
    tenant: &str,
    submission: Uuid,
    class: AttestationClass,
    artifact: &str,
    legacy: bool,
) -> (TraceWitnessCertificateEvidenceWrite, Vec<u8>, Vec<u8>) {
    signed_evidence_at(
        tenant,
        submission,
        class,
        artifact,
        legacy,
        chrono::Utc::now().timestamp(),
    )
}

fn signed_evidence_at(
    tenant: &str,
    submission: Uuid,
    class: AttestationClass,
    artifact: &str,
    legacy: bool,
    issued: i64,
) -> (TraceWitnessCertificateEvidenceWrite, Vec<u8>, Vec<u8>) {
    signed_evidence_over(tenant, submission, class, artifact, legacy, issued, BODY)
}

fn signed_evidence_over(
    tenant: &str,
    submission: Uuid,
    class: AttestationClass,
    artifact: &str,
    legacy: bool,
    issued: i64,
    body: &[u8],
) -> (TraceWitnessCertificateEvidenceWrite, Vec<u8>, Vec<u8>) {
    let key = SigningKey::from_slice(&Keccak256::digest(b"z2 evidence test key")).unwrap();
    let point = key.verifying_key().to_encoded_point(false);
    let signer = format!(
        "0x{}",
        hex::encode(&Keccak256::digest(&point.as_bytes()[1..])[12..])
    );
    let pin = WitnessPin::new(&signer, [MEASUREMENT.to_string()]).unwrap();
    let provenance = if class == AttestationClass::Unattested {
        InferenceProvenance::Unattested
    } else {
        InferenceProvenance::Attested(
            FinalCallAttestation::new(class, Some("model".into()), "b".repeat(64)).unwrap(),
        )
    };
    let body_digest = hex::encode(Sha256::digest(body));
    let details = CertificateDetails {
        residual_risk_verdict: ResidualPiiRisk::Low,
        redaction_policy_version: "full-pipeline".into(),
        witness_measurement: MEASUREMENT.into(),
        timestamp: issued,
    };
    let certificate = if legacy {
        WitnessCertificate::from_wire(body_digest.clone(), details)
    } else {
        WitnessCertificate::from_wire_v2(body_digest.clone(), details, provenance.clone())
    };
    let mut hasher = Keccak256::new();
    hasher.update(b"\x19Ethereum Signed Message:\n");
    hasher.update(certificate.signing_bytes().len().to_string().as_bytes());
    hasher.update(certificate.signing_bytes());
    let digest: [u8; 32] = hasher.finalize().into();
    let (signature, recovery) = key.sign_prehash_recoverable(&digest).unwrap();
    let mut sig = signature.to_bytes().to_vec();
    sig.push(recovery.to_byte() + 27);
    let signature = format!("0x{}", hex::encode(sig));
    let base_json = format!(
        "{{\"redacted_sha256\":\"{body_digest}\",\"redaction_policy_version\":\"full-pipeline\",\"witness_measurement\":\"{MEASUREMENT}\",\"residual_risk_verdict\":\"low\",\"timestamp\":{issued}"
    );
    let cert_json = if legacy {
        format!("{base_json}}}")
    } else {
        format!(
            "{base_json},\"version\":2,\"inference_provenance\":{}}}",
            inference_provenance_json(&provenance)
        )
    };
    let mut headers = HeaderMap::new();
    headers.insert(CERTIFICATE_HEADER, cert_json.parse().unwrap());
    headers.insert(SIGNATURE_HEADER, signature.parse().unwrap());
    let verified = verify_witness_certificate(certificate, &signature, Some(&pin), body).unwrap();
    let evidence = TraceWitnessCertificateEvidenceWrite::from_verified(
        tenant, submission, &verified, &headers, body, artifact,
    )
    .unwrap();
    (evidence, cert_json.into_bytes(), signature.into_bytes())
}

async fn backend() -> Option<PgBackend> {
    let url = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL")
        .or_else(|_| std::env::var("DATABASE_URL"))
        .ok()?;
    let config = DatabaseConfig {
        url: SecretString::from(url),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    };
    Some(
        PgBackend::new(&config)
            .await
            .expect("connect test database"),
    )
}

#[tokio::test]
async fn pg_verified_evidence_is_immutable_tenant_scoped_and_active_artifact_bound() {
    let Some(db) = backend().await else {
        eprintln!("skipping PostgreSQL: no test URL");
        return;
    };
    db.run_migrations().await.expect("migrate");
    let tenant_a = format!("z2-a-{}", Uuid::new_v4());
    let tenant_b = format!("z2-b-{}", Uuid::new_v4());
    let tenant_c = format!("z2-c-{}", Uuid::new_v4());
    let submission = Uuid::new_v4();
    let artifact = "e".repeat(64);
    let (evidence, original_cert, original_sig) = signed_evidence(
        &tenant_a,
        submission,
        AttestationClass::ProviderTeeFinalCall,
        &artifact,
        false,
    );
    db.upsert_trace_submission_with_witness(
        sample_submission(&tenant_a, submission),
        Some(evidence.clone()),
    )
    .await
    .expect("atomic insert");
    db.upsert_trace_submission_with_witness(
        sample_submission(&tenant_a, submission),
        Some(evidence),
    )
    .await
    .expect("exact retry");
    assert_eq!(
        db.witness_retry_identity_matches(
            &tenant_a,
            submission,
            Some(&original_cert),
            Some(&original_sig),
            BODY,
        )
        .await
        .unwrap(),
        Some(true),
    );
    assert_eq!(
        db.witness_retry_identity_matches(&tenant_a, submission, None, None, BODY)
            .await
            .unwrap(),
        Some(true),
        "an exact-body receipt read can omit expired witness headers",
    );
    assert_eq!(
        db.witness_retry_identity_matches(&tenant_a, submission, Some(&original_cert), None, BODY)
            .await
            .unwrap(),
        Some(false),
        "a partial offered witness pair cannot claim the old evidence",
    );
    assert_eq!(
        db.witness_retry_identity_matches(&tenant_a, submission, None, None, b"changed")
            .await
            .unwrap(),
        Some(false),
        "even a headerless receipt read must match the original raw body",
    );
    assert_eq!(
        db.witness_retry_identity_matches(
            &tenant_a,
            submission,
            Some(&original_cert),
            Some(&original_sig),
            b"changed",
        )
        .await
        .unwrap(),
        Some(false),
    );
    assert_eq!(
        db.witness_retry_identity_matches(
            &tenant_b,
            submission,
            Some(&original_cert),
            Some(&original_sig),
            BODY,
        )
        .await
        .unwrap(),
        None,
    );
    let claim = db
        .get_verified_witness_evidence(&tenant_a, submission, &artifact)
        .await
        .unwrap();
    assert_eq!(claim.class, AttestationClass::ProviderTeeFinalCall);
    assert_eq!(claim.coverage, TraceWitnessEvidenceCoverage::VerifiedV2);
    let issued = serde_json::from_slice::<serde_json::Value>(&original_cert).unwrap()["timestamp"]
        .as_i64()
        .unwrap();
    let (same_signed_source_new_ciphertext, cert_again, sig_again) = signed_evidence_at(
        &tenant_a,
        submission,
        AttestationClass::ProviderTeeFinalCall,
        &"d".repeat(64),
        false,
        issued,
    );
    assert_eq!(cert_again, original_cert);
    assert_eq!(sig_again, original_sig);
    db.upsert_trace_submission_with_witness(
        sample_submission(&tenant_a, submission),
        Some(same_signed_source_new_ciphertext),
    )
    .await
    .expect("same signed source with new ciphertext digest is a valid retry");
    assert_eq!(
        db.get_verified_witness_evidence(&tenant_a, submission, &artifact)
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::ArtifactMismatch,
    );
    assert_eq!(
        db.get_verified_witness_evidence(&tenant_a, submission, &"d".repeat(64))
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::VerifiedV2,
    );
    assert_eq!(
        db.get_current_verified_witness_evidence(&tenant_a, submission)
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::ArtifactMismatch,
        "without a selected DB object ref there is no current-object claim",
    );
    let mut current_object = TraceObjectRefWrite {
        object_ref_id: Uuid::new_v4(),
        tenant_id: tenant_a.clone(),
        submission_id: submission,
        artifact_kind: TraceObjectArtifactKind::SubmittedEnvelope,
        object_store: "test-encrypted".into(),
        object_key: "first".into(),
        content_sha256: format!("sha256:{}", "d".repeat(64)),
        encryption_key_ref: "tenant:test".into(),
        size_bytes: 10,
        compression: None,
        created_by_job_id: None,
    };
    db.append_trace_object_ref(current_object.clone())
        .await
        .unwrap();
    assert_eq!(
        db.get_current_verified_witness_evidence(&tenant_a, submission)
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::VerifiedV2,
    );
    current_object.object_key = "rescrubbed".into();
    current_object.content_sha256 = format!("sha256:{}", "f".repeat(64));
    db.append_trace_object_ref(current_object).await.unwrap();
    assert_eq!(
        db.get_current_verified_witness_evidence(&tenant_a, submission)
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::ArtifactMismatch,
        "rescrubbed current object cannot inherit historical source class",
    );
    assert_eq!(
        db.get_verified_witness_evidence(&tenant_b, submission, &artifact)
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::Missing
    );
    assert_eq!(
        db.get_verified_witness_evidence(&tenant_a, submission, &"f".repeat(64))
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::ArtifactMismatch
    );
    let (conflict, _, _) = signed_evidence(
        &tenant_a,
        submission,
        AttestationClass::GatewayFinalCall,
        &artifact,
        false,
    );
    let mut changed_submission = sample_submission(&tenant_a, submission);
    changed_submission.status = TraceCorpusStatus::Revoked;
    assert!(
        db.upsert_trace_submission_with_witness(changed_submission, Some(conflict))
            .await
            .is_err()
    );
    assert_eq!(
        db.get_trace_submission(&tenant_a, submission)
            .await
            .unwrap()
            .unwrap()
            .status,
        TraceCorpusStatus::Accepted,
        "conflict must roll back submission update"
    );
    let mut revoked = sample_submission(&tenant_a, submission);
    revoked.status = TraceCorpusStatus::Revoked;
    db.upsert_trace_submission(revoked).await.unwrap();
    assert_eq!(
        db.get_verified_witness_evidence(&tenant_a, submission, &artifact)
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::Inactive
    );
    assert_eq!(
        db.get_current_verified_witness_evidence(&tenant_a, submission)
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::Inactive,
        "revocation must defeat even a previously matching current-object ref",
    );

    for (class, legacy, expected_class, coverage) in [
        (
            AttestationClass::GatewayFinalCall,
            false,
            AttestationClass::GatewayFinalCall,
            TraceWitnessEvidenceCoverage::VerifiedV2,
        ),
        (
            AttestationClass::Unattested,
            false,
            AttestationClass::Unattested,
            TraceWitnessEvidenceCoverage::ExplicitUnattested,
        ),
        (
            AttestationClass::Unattested,
            true,
            AttestationClass::Unattested,
            TraceWitnessEvidenceCoverage::LegacyV1,
        ),
    ] {
        let id = Uuid::new_v4();
        let (evidence, _, _) = signed_evidence(&tenant_a, id, class, &artifact, legacy);
        db.upsert_trace_submission_with_witness(sample_submission(&tenant_a, id), Some(evidence))
            .await
            .unwrap();
        let claim = db
            .get_verified_witness_evidence(&tenant_a, id, &artifact)
            .await
            .unwrap();
        assert_eq!(claim.class, expected_class);
        assert_eq!(claim.coverage, coverage);
    }

    let (tenant_b_evidence, _, _) = signed_evidence(
        &tenant_b,
        submission,
        AttestationClass::GatewayFinalCall,
        &artifact,
        false,
    );
    db.upsert_trace_submission_with_witness(
        sample_submission(&tenant_b, submission),
        Some(tenant_b_evidence),
    )
    .await
    .unwrap();
    assert_eq!(
        db.get_verified_witness_evidence(&tenant_b, submission, &artifact)
            .await
            .unwrap()
            .class,
        AttestationClass::GatewayFinalCall
    );
    assert_eq!(
        db.get_verified_witness_evidence(&tenant_a, submission, &artifact)
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::Inactive
    );

    let pool = db.raw_pool_for_tests_and_diagnostics();
    let mut client = pool.get().await.unwrap();
    let stored = client
        .query_one(
            "SELECT certificate_json, signature_header, raw_body_sha256
         FROM trace_witness_certificate_evidence WHERE tenant_id=$1 AND submission_id=$2",
            &[&tenant_a, &submission],
        )
        .await
        .unwrap();
    assert_eq!(stored.get::<_, Vec<u8>>(0), original_cert);
    assert_eq!(stored.get::<_, Vec<u8>>(1), original_sig);
    assert_eq!(
        stored.get::<_, String>(2),
        hex::encode(Sha256::digest(BODY))
    );
    let row = client
        .query_one(
            "SELECT c.relrowsecurity, c.relforcerowsecurity,
                has_table_privilege('trace_witness_evidence_runtime', c.oid, 'SELECT') AS can_read,
                has_column_privilege('trace_witness_evidence_runtime', c.oid, 'artifact_sha256', 'UPDATE') AS can_rebind,
                has_column_privilege('trace_witness_evidence_runtime', c.oid, 'certificate_json', 'UPDATE') AS can_rewrite_certificate
         FROM pg_class c WHERE c.oid = 'trace_witness_certificate_evidence'::regclass",
            &[],
        )
        .await
        .unwrap();
    assert!(row.get::<_, bool>(0) && row.get::<_, bool>(1));
    assert!(row.get::<_, bool>(2) && row.get::<_, bool>(3));
    assert!(!row.get::<_, bool>(4));
    let insert_target = Uuid::new_v4();
    db.upsert_trace_submission(sample_submission(&tenant_a, insert_target))
        .await
        .unwrap();
    let (_, insert_cert, insert_sig) = signed_evidence(
        &tenant_a,
        insert_target,
        AttestationClass::ProviderTeeFinalCall,
        &artifact,
        false,
    );
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE trace_witness_evidence_runtime")
        .await
        .unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_a],
    )
    .await
    .unwrap();
    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM trace_witness_certificate_evidence WHERE tenant_id=$1",
            &[&tenant_a],
        )
        .await
        .unwrap()
        .get(0);
    assert!(
        count > 0,
        "restricted role must see its own tenant's evidence"
    );
    tx.rollback().await.unwrap();
    let tx = client.transaction().await.unwrap();
    tx.batch_execute("SET LOCAL ROLE trace_witness_evidence_runtime")
        .await
        .unwrap();
    tx.execute(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant_c],
    )
    .await
    .unwrap();
    let count: i64 = tx
        .query_one(
            "SELECT count(*) FROM trace_witness_certificate_evidence",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert_eq!(count, 0, "tenant C must not see A or B's exact headers");
    let denied = tx
        .execute(
            "INSERT INTO trace_witness_certificate_evidence (
            tenant_id, submission_id, certificate_json, signature_header, raw_body_sha256,
            artifact_sha256, certificate_version, inference_class, receipt_signer,
            issued_at
         ) VALUES ($1,$2,$3,$4,$5,$6,2,'provider_tee_final_call',$7,NOW())",
            &[
                &tenant_a,
                &insert_target,
                &insert_cert,
                &insert_sig,
                &hex::encode(Sha256::digest(BODY)),
                &artifact,
                &"b".repeat(64),
            ],
        )
        .await;
    assert!(
        denied.is_err(),
        "tenant C role cannot insert tenant A evidence"
    );
    tx.rollback().await.unwrap();
}

#[test]
fn witness_evidence_schema_is_forced_tenant_scoped_and_body_free() {
    let sql = include_str!("../../../migrations/V76__trace_witness_certificate_evidence.sql");
    assert!(sql.contains("FOREIGN KEY (tenant_id, submission_id)"));
    assert!(sql.contains("certificate_json BYTEA NOT NULL"));
    assert!(sql.contains("signature_header BYTEA NOT NULL"));
    assert!(sql.contains("ENABLE ROW LEVEL SECURITY"));
    assert!(sql.contains("FORCE ROW LEVEL SECURITY"));
    assert!(sql.contains("trace_current_tenant_id()"));
    assert!(sql.contains("BEFORE UPDATE ON trace_witness_certificate_evidence"));
    assert!(!sql.contains("receipt_sha256"));
    assert!(!sql.contains("transcript_json"));
    assert!(!sql.contains("raw_body BYTEA"));
}

const REMEDIATED_BODY: &[u8] = b"{\"witnessed\":true,\"remediated\":true}";

fn quarantined_submission(tenant_id: &str, submission_id: Uuid) -> TraceSubmissionWrite {
    let mut submission = sample_submission(tenant_id, submission_id);
    submission.status = TraceCorpusStatus::Quarantined;
    submission
}

/// Quarantine remediation replaces a submission's body under the same id, so
/// the prior body's evidence no longer describes the submission. It is
/// replaced by the new body's evidence, or removed when the remediation is
/// unwitnessed, in the same transaction as the submission update.
#[tokio::test]
async fn pg_quarantine_remediation_replaces_witness_evidence() {
    let Some(db) = backend().await else {
        eprintln!("skipping PostgreSQL: no test URL");
        return;
    };
    db.run_migrations().await.expect("migrate");
    let tenant = format!("z2-remediate-{}", Uuid::new_v4());
    let artifact = "e".repeat(64);

    // Witnessed quarantined submission, remediated with a new certificate.
    let witnessed = Uuid::new_v4();
    let (original, _, _) = signed_evidence(
        &tenant,
        witnessed,
        AttestationClass::ProviderTeeFinalCall,
        &artifact,
        false,
    );
    db.upsert_trace_submission_with_witness(
        quarantined_submission(&tenant, witnessed),
        Some(original),
    )
    .await
    .expect("quarantined witnessed insert");
    let (replacement, replacement_cert, replacement_sig) = signed_evidence_over(
        &tenant,
        witnessed,
        AttestationClass::GatewayFinalCall,
        &"d".repeat(64),
        false,
        chrono::Utc::now().timestamp(),
        REMEDIATED_BODY,
    );
    db.remediate_trace_submission_with_witness(
        sample_submission(&tenant, witnessed),
        Some(replacement),
    )
    .await
    .expect("remediation with a new certificate replaces the evidence");
    assert_eq!(
        db.witness_retry_identity_matches(
            &tenant,
            witnessed,
            Some(&replacement_cert),
            Some(&replacement_sig),
            REMEDIATED_BODY,
        )
        .await
        .unwrap(),
        Some(true),
        "a retry of the remediated body matches the replacement evidence",
    );
    assert_eq!(
        db.witness_retry_identity_matches(&tenant, witnessed, None, None, REMEDIATED_BODY)
            .await
            .unwrap(),
        Some(true),
    );
    assert_eq!(
        db.witness_retry_identity_matches(&tenant, witnessed, None, None, BODY)
            .await
            .unwrap(),
        Some(false),
        "the pre-remediation body no longer matches",
    );
    assert_eq!(
        db.get_verified_witness_evidence(&tenant, witnessed, &"d".repeat(64))
            .await
            .unwrap()
            .class,
        AttestationClass::GatewayFinalCall,
    );

    // Witnessed quarantined submission, remediated without witness headers.
    let unwitnessed = Uuid::new_v4();
    let (original, _, _) = signed_evidence(
        &tenant,
        unwitnessed,
        AttestationClass::ProviderTeeFinalCall,
        &artifact,
        false,
    );
    db.upsert_trace_submission_with_witness(
        quarantined_submission(&tenant, unwitnessed),
        Some(original),
    )
    .await
    .expect("quarantined witnessed insert");
    db.remediate_trace_submission_with_witness(sample_submission(&tenant, unwitnessed), None)
        .await
        .expect("unwitnessed remediation succeeds");
    assert_eq!(
        db.witness_retry_identity_matches(&tenant, unwitnessed, None, None, REMEDIATED_BODY)
            .await
            .unwrap(),
        None,
        "evidence for the replaced body must not survive an unwitnessed remediation",
    );
    assert_eq!(
        db.get_current_verified_witness_evidence(&tenant, unwitnessed)
            .await
            .unwrap()
            .coverage,
        TraceWitnessEvidenceCoverage::Missing,
    );

    // Only a quarantined row may have its evidence replaced.
    let accepted = Uuid::new_v4();
    let (original, original_cert, original_sig) = signed_evidence(
        &tenant,
        accepted,
        AttestationClass::ProviderTeeFinalCall,
        &artifact,
        false,
    );
    db.upsert_trace_submission_with_witness(sample_submission(&tenant, accepted), Some(original))
        .await
        .expect("accepted witnessed insert");
    let (other, _, _) = signed_evidence_over(
        &tenant,
        accepted,
        AttestationClass::GatewayFinalCall,
        &artifact,
        false,
        chrono::Utc::now().timestamp(),
        REMEDIATED_BODY,
    );
    assert!(
        db.remediate_trace_submission_with_witness(
            sample_submission(&tenant, accepted),
            Some(other)
        )
        .await
        .is_err(),
        "evidence of a non-quarantined submission is never replaced",
    );
    db.remediate_trace_submission_with_witness(sample_submission(&tenant, accepted), None)
        .await
        .expect("an unwitnessed write to a non-quarantined row leaves evidence alone");
    assert_eq!(
        db.witness_retry_identity_matches(
            &tenant,
            accepted,
            Some(&original_cert),
            Some(&original_sig),
            BODY,
        )
        .await
        .unwrap(),
        Some(true),
    );
}

/// The immutability of the signed source must hold for every role, including
/// the table owner a single-login deployment connects as. Only the derived
/// `artifact_sha256` link may move.
#[tokio::test]
async fn pg_witness_evidence_signed_source_is_immutable_even_for_the_owner() {
    let Some(db) = backend().await else {
        eprintln!("skipping PostgreSQL: no test URL");
        return;
    };
    db.run_migrations().await.expect("migrate");
    let tenant = format!("z2-immutable-{}", Uuid::new_v4());
    let submission = Uuid::new_v4();
    let (evidence, _, _) = signed_evidence(
        &tenant,
        submission,
        AttestationClass::ProviderTeeFinalCall,
        &"e".repeat(64),
        false,
    );
    db.upsert_trace_submission_with_witness(sample_submission(&tenant, submission), Some(evidence))
        .await
        .expect("insert");
    let pool = db.raw_pool_for_tests_and_diagnostics();
    let client = pool.get().await.unwrap();
    for assignment in [
        "certificate_json = '\\x00'::bytea",
        "signature_header = '\\x00'::bytea",
        "raw_body_sha256 = repeat('0', 64)",
        "certificate_version = 1, inference_class = 'unattested', bound_model = NULL, receipt_signer = NULL",
        "bound_model = 'other-model'",
        "issued_at = issued_at - interval '1 day'",
        "received_at = received_at - interval '1 day'",
    ] {
        let result = client
            .execute(
                &format!(
                    "UPDATE trace_witness_certificate_evidence SET {assignment}
                     WHERE tenant_id = $1 AND submission_id = $2"
                ),
                &[&tenant, &submission],
            )
            .await;
        assert!(
            result.is_err(),
            "owner update must be refused: {assignment}"
        );
    }
    let rebound = client
        .execute(
            "UPDATE trace_witness_certificate_evidence SET artifact_sha256 = $3
             WHERE tenant_id = $1 AND submission_id = $2",
            &[&tenant, &submission, &"f".repeat(64)],
        )
        .await
        .expect("the derived artifact link may move");
    assert_eq!(rebound, 1);
}
