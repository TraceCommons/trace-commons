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
        url: SecretString::from(url),
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
    db.begin_token_bundle(bundle.clone()).await.unwrap();
    db.begin_token_bundle(bundle.clone()).await.unwrap();
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
        db.commit_token_bundle(&tenant, submission, "revision", "owner", receipt)
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
    db.begin_token_bundle(bundle).await.unwrap();
}
