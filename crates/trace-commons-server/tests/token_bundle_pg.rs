// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use chrono::Utc;
use secrecy::SecretString;
use std::collections::BTreeMap;
use trace_commons_protocol::token_distribution::*;
use trace_commons_server::{
    config::{DatabaseConfig, SslMode},
    db::{Database, postgres::PgBackend},
    token_bundle_store::StoredTokenBundle,
    trace_corpus_storage::TraceCorpusStore,
};

#[tokio::test]
async fn bundle_staging_is_immutable_and_owner_scoped() {
    let Ok(url) = std::env::var("TRACE_COMMONS_PG_TEST_DATABASE_URL") else {
        return;
    };
    let db = PgBackend::new(&DatabaseConfig {
        url: SecretString::from(url.clone()),
        pool_size: 4,
        ssl_mode: SslMode::Prefer,
        login_resolver_url: None,
        gate_driver_url: None,
        pii_backstop_driver_url: None,
        invite_registry_url: None,
    })
    .await
    .unwrap();
    db.run_migrations().await.unwrap();
    let tenant = format!("bundle-test-{}", uuid::Uuid::new_v4());
    let submission = uuid::Uuid::new_v4();
    let bundle = StoredTokenBundle {
        tenant_id: tenant.clone(),
        submission_id: submission,
        revision: "revision".into(),
        owner_ref: "owner".into(),
        manifest: ContributionBundleManifest {
            version: 1,
            usage_profile: TokenUsageProfile::RestrictedResearch,
            submission_id: submission.to_string(),
            bundle_revision: "revision".into(),
            envelope_digest: ContentDigest::of(b"envelope"),
            consent_digest: ContentDigest::of(b"consent"),
            policy_version: "policy".into(),
            attachments: vec![AttachmentDescriptor {
                artifact_id: "tokens".into(),
                event_id: "event".into(),
                content_digest: ContentDigest::of(b"tokens"),
                size_bytes: 6,
            }],
        },
        witness_headers: BTreeMap::new(),
        state: "staging".into(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
        receipt: None,
        attachments: Vec::new(),
    };
    let (first, retry) = tokio::join!(
        db.begin_token_bundle(bundle.clone()),
        db.begin_token_bundle(bundle.clone())
    );
    first.unwrap();
    retry.unwrap();
    // Exercise actual RLS under a non-superuser role, not just application predicates.
    let (mut raw, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let tx = raw.transaction().await.unwrap();
    let role = format!("tc_bundle_test_{}", uuid::Uuid::new_v4().simple());
    tx.batch_execute(&format!(
        "CREATE ROLE {role} NOLOGIN NOSUPERUSER NOBYPASSRLS NOINHERIT;
        GRANT SELECT, INSERT ON trace_token_bundles, trace_token_attachments TO {role};
        SET LOCAL ROLE {role};"
    ))
    .await
    .unwrap();
    let bypass: bool = tx
        .query_one(
            "SELECT rolsuper OR rolbypassrls FROM pg_roles WHERE rolname=current_user",
            &[],
        )
        .await
        .unwrap()
        .get(0);
    assert!(!bypass);
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id', $1, true)",
        &[&tenant],
    )
    .await
    .unwrap();
    let own: i64 = tx
        .query_one("SELECT count(*) FROM trace_token_bundles", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(own, 1);
    tx.query_one(
        "SELECT set_config('trace_commons.trace_tenant_id', 'other-bundle-tenant', true)",
        &[],
    )
    .await
    .unwrap();
    let other: i64 = tx
        .query_one("SELECT count(*) FROM trace_token_bundles", &[])
        .await
        .unwrap()
        .get(0);
    assert_eq!(other, 0);
    let error = tx.execute("INSERT INTO trace_token_bundles (tenant_id,submission_id,revision,owner_ref,manifest_digest,witness_headers,manifest,state,expires_at) VALUES ($1,$2,'rls','owner',$3,'{}','{}','staging',NOW())", &[&tenant,&uuid::Uuid::new_v4(),&bundle.manifest.digest().unwrap().as_str()]).await.unwrap_err();
    assert_eq!(
        error.code(),
        Some(&tokio_postgres::error::SqlState::INSUFFICIENT_PRIVILEGE)
    );
    tx.rollback().await.unwrap(); // Also removes the disposable role and grants.

    assert!(
        db.get_token_bundle(&tenant, submission, "revision", "other-owner")
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        db.get_token_bundle("other-tenant", submission, "revision", "owner")
            .await
            .unwrap()
            .is_none()
    );
    let mut changed = bundle.clone();
    changed.manifest.envelope_digest = ContentDigest::of(b"different");
    assert!(db.begin_token_bundle(changed).await.is_err());
    let receipt = DurableBundleReceipt {
        version: 1,
        server_id: "server".into(),
        tenant_id: tenant.clone(),
        account_id: "owner".into(),
        submission_id: submission.to_string(),
        bundle_revision: "revision".into(),
        manifest_digest: bundle.manifest.digest().unwrap(),
        committed_at_unix: Utc::now().timestamp() as u64,
        retain_until_unix: Utc::now().timestamp() as u64 + 3600,
        retention_policy_version: "policy".into(),
    };
    assert!(
        db.commit_token_bundle(&tenant, submission, "revision", "owner", receipt.clone())
            .await
            .is_err()
    );
    // Quotas also apply before a parent submission is admitted.
    for index in 1..16 {
        let mut next = bundle.clone();
        next.submission_id = uuid::Uuid::new_v4();
        next.manifest.submission_id = next.submission_id.to_string();
        next.revision = format!("revision-{index}");
        next.manifest.bundle_revision = next.revision.clone();
        db.begin_token_bundle(next).await.unwrap();
    }
    let mut excess = bundle.clone();
    excess.submission_id = uuid::Uuid::new_v4();
    excess.manifest.submission_id = excess.submission_id.to_string();
    assert!(db.begin_token_bundle(excess).await.is_err());
    db.begin_token_bundle(bundle.clone()).await.unwrap();
    qualify_publication_commit_and_withdrawal(&db, &url, bundle, receipt).await;
}

async fn qualify_publication_commit_and_withdrawal(
    db: &PgBackend,
    url: &str,
    bundle: StoredTokenBundle,
    receipt: DurableBundleReceipt,
) {
    use trace_commons_server::secrets::SecretsCrypto;
    use trace_commons_server::token_bundle_store::StoredTokenObject;
    use trace_commons_server::trace_artifact_store::*;
    use trace_commons_server::trace_corpus_storage::{TraceCorpusStatus, TraceSubmissionWrite};
    let tenant = &bundle.tenant_id;
    let submission = bundle.submission_id;
    db.upsert_trace_submission(TraceSubmissionWrite {
        tenant_id: tenant.clone(),
        submission_id: submission,
        trace_id: uuid::Uuid::new_v4(),
        auth_principal_ref: "owner".into(),
        contributor_pseudonym: None,
        submitted_tenant_scope_ref: Some(tenant.clone()),
        schema_version: "ironclaw.trace_contribution.v1".into(),
        consent_policy_version: "policy".into(),
        consent_scopes: vec!["debugging_evaluation".into()],
        allowed_uses: vec!["debugging".into()],
        retention_policy_id: "private_corpus_revocable".into(),
        status: TraceCorpusStatus::Accepted,
        privacy_risk: "low".into(),
        redaction_pipeline_version: "policy".into(),
        redaction_counts: BTreeMap::new(),
        redaction_hash: "hash".into(),
        canonical_summary_hash: None,
        submission_score: None,
        credit_points_pending: None,
        credit_points_final: None,
        expires_at: None,
        residual_risk_basis: None,
    })
    .await
    .unwrap();
    let crypto = || SecretsCrypto::new(SecretString::from("11".repeat(32))).unwrap();
    let objects = tempfile::tempdir().unwrap();
    let store = ServiceOwnedTraceArtifactStore::new(
        TraceArtifactProviderConfig::service_owned_remote("bundle-test").unwrap(),
        crypto(),
        trace_commons_server::trace_artifact_kek::LocalMasterKeyWrapper::new(crypto(), "test-kek"),
        FileRemoteTraceArtifactProvider::new(objects.path()),
    );
    let scope = TraceArtifactScope {
        tenant_storage_ref: tenant.clone(),
        submission_storage_ref: submission.to_string(),
    };
    for (artifact, kind, bytes) in [
        (
            "tokens",
            TraceArtifactKind::TokenDistribution,
            b"tokens".as_slice(),
        ),
        (
            "envelope",
            TraceArtifactKind::ContributionEnvelope,
            b"envelope".as_slice(),
        ),
    ] {
        let prepared = store
            .prepare_bundle_bytes(&scope, kind, artifact, bytes)
            .unwrap();
        db.stage_token_object(
            tenant,
            submission,
            "revision",
            "owner",
            StoredTokenObject {
                artifact_id: artifact.into(),
                object_ref: prepared.object_ref.clone(),
                deleted: false,
                ready: false,
                prepared: Some(serde_json::to_vec(&prepared).unwrap()),
            },
        )
        .await
        .unwrap();
        // Crash boundary: ciphertext exists, but no ready marker or receipt.
        store.publish_bundle_bytes(&scope, &prepared).unwrap();
        assert!(
            db.commit_token_bundle(tenant, submission, "revision", "owner", receipt.clone())
                .await
                .is_err()
        );
        db.publish_token_object(tenant, submission, "revision", "owner", artifact, &store)
            .await
            .unwrap();
        db.publish_token_object(tenant, submission, "revision", "owner", artifact, &store)
            .await
            .unwrap();
    }
    let (first, retry) = tokio::join!(
        db.commit_token_bundle(tenant, submission, "revision", "owner", receipt.clone()),
        db.commit_token_bundle(tenant, submission, "revision", "owner", receipt.clone())
    );
    assert_eq!(
        first.unwrap().manifest_digest,
        retry.unwrap().manifest_digest
    );
    let stored = db
        .get_token_bundle(tenant, submission, "revision", "owner")
        .await
        .unwrap()
        .unwrap();
    assert!(
        stored
            .attachments
            .iter()
            .all(|a| a.ready && a.prepared.is_none())
    );
    // Parent revocation blocks access before object deletion and prevents a
    // publication retry or a new revision from resurrecting the contribution.
    let (mut client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .unwrap();
    tokio::spawn(async move {
        let _ = connection.await;
    });
    let tx = client.transaction().await.unwrap();
    tx.query_one("SELECT submission_id FROM trace_submissions WHERE tenant_id=$1 AND submission_id=$2 FOR UPDATE", &[tenant, &submission]).await.unwrap();
    let mut racing = bundle.clone();
    racing.revision = "racing-revision".into();
    racing.manifest.bundle_revision = racing.revision.clone();
    let beginning = db.begin_token_bundle(racing);
    tokio::pin!(beginning);
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut beginning)
            .await
            .is_err(),
        "begin must wait for the parent lifecycle lock"
    );
    tx.execute("UPDATE trace_submissions SET status='revoked',withdrawn_at=NOW() WHERE tenant_id=$1 AND submission_id=$2", &[tenant, &submission]).await.unwrap();
    tx.commit().await.unwrap();
    assert!(
        beginning.await.is_err(),
        "withdrawal prevents a racing new revision"
    );
    assert_eq!(
        db.get_token_bundle(tenant, submission, "revision", "owner")
            .await
            .unwrap()
            .unwrap()
            .state,
        "revoked"
    );
    assert!(
        db.publish_token_object(tenant, submission, "revision", "owner", "tokens", &store)
            .await
            .is_err()
    );
    assert!(db.begin_token_bundle(bundle.clone()).await.is_err());
    db.delete_token_objects(tenant, submission, "revision", &store)
        .await
        .unwrap();
    db.delete_token_objects(tenant, submission, "revision", &store)
        .await
        .unwrap();
    assert!(
        db.get_token_bundle(tenant, submission, "revision", "owner")
            .await
            .unwrap()
            .unwrap()
            .attachments
            .iter()
            .all(|a| a.deleted && a.prepared.is_none())
    );
    for object in stored.attachments {
        assert!(store.read_bundle_bytes(&scope, &object.object_ref).is_err());
    }
}
