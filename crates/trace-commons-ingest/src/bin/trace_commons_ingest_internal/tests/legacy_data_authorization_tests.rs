// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Historical ownerless records fail closed for ordinary contributors.
//! Exact credential aliases and tenant-scoped reviewer recovery remain available.
use super::*;
use axum::{body::Body, http::Request};
use tower::ServiceExt;

fn remove_historical_owner(root: &Path, submission_id: Uuid) {
    let path = submission_metadata_path(root, "tenant-a", submission_id);
    let mut stored: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&path).expect("metadata reads"))
            .expect("metadata decodes");
    assert!(
        stored
            .as_object_mut()
            .expect("record")
            .remove("auth_principal_ref")
            .is_some()
    );
    std::fs::write(
        &path,
        serde_json::to_vec(&stored).expect("metadata encodes"),
    )
    .expect("old-schema metadata persists");
    let record = read_submission_record(root, "tenant-a", submission_id)
        .expect("old-schema metadata reads")
        .expect("record exists");
    assert_eq!(record.auth_principal_ref, legacy_principal_ref());
}

async fn delete_as_other(state: Arc<AppState>, submission_id: Uuid) -> StatusCode {
    app(state)
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/v1/traces/{submission_id}"))
                .header(AUTHORIZATION, "Bearer token-a-2")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router response")
        .status()
}

#[tokio::test]
async fn legacy_unrelated_contributor_cannot_delete_an_old_schema_trace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path().to_path_buf());
    let envelope = low_risk_envelope().await;
    let id = envelope.submission_id;
    let _ = test_submit(state.clone(), "token-a", envelope)
        .await
        .expect("owner submits");
    let modern = read_submission_record(temp.path(), "tenant-a", id)
        .expect("metadata")
        .expect("record");
    assert!(
        !state
            .tokens
            .get("token-a-2")
            .expect("other token")
            .matches_stored_principal(&modern.auth_principal_ref)
    );
    assert_eq!(
        delete_as_other(state.clone(), id).await,
        StatusCode::NOT_FOUND
    );
    assert!(
        read_revocation(temp.path(), "tenant-a", id)
            .expect("lookup")
            .is_none()
    );

    // Exactly the historical serde default, not a forged attacker request.
    remove_historical_owner(temp.path(), id);
    assert_eq!(delete_as_other(state, id).await, StatusCode::NOT_FOUND);
    assert!(
        read_revocation(temp.path(), "tenant-a", id)
            .expect("lookup")
            .is_none()
    );
    assert_eq!(
        read_submission_record(temp.path(), "tenant-a", id)
            .expect("metadata")
            .expect("record")
            .status,
        TraceCorpusStatus::Accepted
    );
}

#[tokio::test]
async fn legacy_unrelated_contributor_cannot_replace_an_old_schema_quarantine() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path().to_path_buf());
    let mut original = low_risk_envelope().await;
    original.consent.message_text_included = true;
    original.consent.tool_payloads_included = true;
    original.privacy.residual_pii_risk = ResidualPiiRisk::Medium;
    let id = original.submission_id;
    let Json(receipt) = test_submit(state.clone(), "token-a", original.clone())
        .await
        .expect("owner submits quarantine");
    assert_eq!(receipt.status, "quarantined");
    let mut replacement = original;
    make_metadata_only_low_risk(&mut replacement);
    let error = test_submit(state.clone(), "token-a-2", replacement.clone())
        .await
        .expect_err("modern ownership denies replacement");
    assert_eq!(error.0, StatusCode::CONFLICT);

    remove_historical_owner(temp.path(), id);
    let error = test_submit(state.clone(), "token-a-2", replacement)
        .await
        .expect_err("an unknown historical owner cannot authorize remediation");
    assert_eq!(error.0, StatusCode::CONFLICT);
    let record = read_submission_record(temp.path(), "tenant-a", id)
        .expect("metadata")
        .expect("record");
    assert_eq!(record.status, TraceCorpusStatus::Quarantined);
    assert_eq!(record.auth_principal_ref, legacy_principal_ref());
    let stored = read_envelope_by_record(state.as_ref(), &record).expect("replacement body reads");
    assert!(stored.consent.message_text_included);
    assert!(stored.consent.tool_payloads_included);
}

#[tokio::test]
async fn legacy_unrelated_contributor_cannot_read_old_schema_status_metadata() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path().to_path_buf());
    let envelope = low_risk_envelope().await;
    let id = envelope.submission_id;
    let _ = test_submit(state.clone(), "token-a", envelope)
        .await
        .expect("owner submits");
    let request = || {
        Json(TraceSubmissionStatusRequest {
            submission_ids: vec![id],
        })
    };
    let Json(modern) =
        submission_status_handler(State(state.clone()), auth_headers("token-a-2"), request())
            .await
            .expect("status route");
    assert!(
        modern.is_empty(),
        "modern row is invisible to unrelated principal"
    );
    remove_historical_owner(temp.path(), id);
    let Json(legacy) =
        submission_status_handler(State(state), auth_headers("token-a-2"), request())
            .await
            .expect("status route");
    assert!(
        legacy.is_empty(),
        "an unknown owner does not grant metadata visibility"
    );
}

#[tokio::test]
async fn exact_historical_credential_alias_retains_owner_status_and_deletion() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path().to_path_buf());
    let envelope = low_risk_envelope().await;
    let id = envelope.submission_id;
    let _ = test_submit(state.clone(), "token-a", envelope)
        .await
        .expect("owner submits");
    let mut record = read_submission_record(temp.path(), "tenant-a", id)
        .expect("metadata")
        .expect("record");
    record.auth_principal_ref = state
        .tokens
        .get("token-a")
        .expect("owner")
        .legacy_principal_ref
        .clone()
        .expect("credential alias");
    assert_ne!(record.auth_principal_ref, legacy_principal_ref());
    write_submission_record(temp.path(), &record).expect("historical alias persists");
    for (token, visible) in [
        ("token-a", true),
        ("token-a-2", false),
        ("token-b", false),
        ("review-token-a", true),
    ] {
        let Json(statuses) = submission_status_handler(
            State(state.clone()),
            auth_headers(token),
            Json(TraceSubmissionStatusRequest {
                submission_ids: vec![id],
            }),
        )
        .await
        .expect("status route");
        assert_eq!(!statuses.is_empty(), visible, "{token}");
    }
    assert_eq!(
        revoke_submission(&state, &auth_headers("token-a"), id, None)
            .await
            .expect("alias owner self-revokes"),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn ownerless_recovery_requires_tenant_reviewer_authority_and_reason() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path().to_path_buf());
    let envelope = low_risk_envelope().await;
    let id = envelope.submission_id;
    let _ = test_submit(state.clone(), "token-a", envelope)
        .await
        .expect("owner submits");
    remove_historical_owner(temp.path(), id);
    for token in ["token-a", "token-a-2", "token-b", "review-token-b"] {
        let error = revoke_submission(&state, &auth_headers(token), id, Some("legacy recovery"))
            .await
            .expect_err("ownership cannot be recovered from the sentinel");
        assert_eq!(error.0, StatusCode::NOT_FOUND, "{token}");
    }
    let error = revoke_submission(&state, &auth_headers("review-token-a"), id, None)
        .await
        .expect_err("reviewer recovery is privileged, not self-revocation");
    assert_eq!(error.0, StatusCode::BAD_REQUEST);
    let mut restricted = state.clone();
    Arc::make_mut(&mut restricted).tenant_policies = Arc::new(BTreeMap::from([(
        "tenant-a".to_string(),
        TenantSubmissionPolicy {
            allowed_consent_scopes: BTreeSet::from([ConsentScope::BenchmarkOnly]),
            allowed_uses: BTreeSet::from([TraceAllowedUse::BenchmarkGeneration]),
        },
    )]));
    let error = revoke_submission(
        &restricted,
        &auth_headers("review-token-a"),
        id,
        Some("legacy recovery"),
    )
    .await
    .expect_err("unknown owner cannot bypass reviewer policy");
    assert_eq!(error.0, StatusCode::FORBIDDEN);
    assert!(
        read_revocation(temp.path(), "tenant-a", id)
            .expect("lookup")
            .is_none()
    );
    assert_eq!(
        revoke_submission(
            &state,
            &auth_headers("review-token-a"),
            id,
            Some("legacy recovery")
        )
        .await
        .expect("reviewer recovers same-tenant record"),
        StatusCode::NO_CONTENT
    );
}

#[test]
fn file_and_storage_ownership_require_an_attributable_principal() {
    let mut tokens = BTreeMap::new();
    insert_token(&mut tokens, "tenant-a", "owner", TokenRole::Contributor);
    let owner = tokens.get("owner").expect("owner");
    let reviewer = test_reviewer_auth("tenant-a");
    for (principal, owns) in [
        (owner.principal_ref.clone(), true),
        (
            owner
                .legacy_principal_ref
                .clone()
                .expect("credential alias"),
            true,
        ),
        (legacy_principal_ref(), false),
        ("principal-unrelated".to_string(), false),
    ] {
        let mut record = submission_record_with_principal(&principal);
        record.status = TraceCorpusStatus::Quarantined;
        let mut stored = seeded_storage_submission_record(
            "tenant-a",
            record.submission_id,
            StorageTraceCorpusStatus::Quarantined,
        );
        stored.auth_principal_ref = principal;
        assert_eq!(principal_owns_submission(owner, &record), owns);
        assert_eq!(
            can_access_submission_ref(owner, &record.auth_principal_ref),
            owns
        );
        assert_eq!(principal_can_self_revoke_submission(owner, &record), owns);
        assert_eq!(principal_can_remediate_quarantined(owner, &record), owns);
        assert_eq!(principal_owns_storage_submission(owner, &stored), owns);
        assert_eq!(
            principal_can_self_revoke_storage_submission(owner, &stored),
            owns
        );
        let event = TraceCommonsCreditLedgerRecord {
            auth_principal_ref: record.auth_principal_ref.clone(),
            ..withdrawal_credit_event(
                record.submission_id,
                TraceCreditLedgerEventType::NoveltyUtility,
                1.0,
            )
        };
        assert_eq!(can_access_credit_event(owner, &event), owns);
        assert!(can_access_credit_event(&reviewer, &event));
        assert!(can_access_submission(&reviewer, &record));
        assert!(can_access_storage_submission(&reviewer, &stored));
        assert!(!principal_can_self_revoke_submission(&reviewer, &record));
        assert!(!principal_can_self_revoke_storage_submission(
            &reviewer, &stored
        ));
    }
}

#[tokio::test]
async fn ownerless_metadata_and_credit_are_hidden_from_contributors() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path().to_path_buf());
    let envelope = low_risk_envelope().await;
    let id = envelope.submission_id;
    let _ = test_submit(state.clone(), "token-a", envelope)
        .await
        .expect("owner submits");
    remove_historical_owner(temp.path(), id);
    let event = TraceCommonsCreditLedgerRecord {
        auth_principal_ref: legacy_principal_ref(),
        ..withdrawal_credit_event(id, TraceCreditLedgerEventType::NoveltyUtility, 1.0)
    };
    append_credit_event(temp.path(), "tenant-a", &event).expect("credit fixture persists");
    let batch: TraceCreditSettlementBatchRecord = serde_json::from_value(serde_json::json!({
        "settlement_batch_id": Uuid::new_v4(),
        "tenant_id": "tenant-a",
        "tenant_storage_ref": tenant_storage_ref("tenant-a"),
        "policy_version": "legacy-regression",
        "status": "finalized",
        "reason_hash": sha256_prefixed("legacy recovery"),
        "source_credit_event_ids": [event.event_id],
        "source_submission_ids": [id],
        "source_list_hash": sha256_prefixed("legacy-source"),
        "settled_credit_points": 1.0,
        "settled_credit_micros": 1_000_000,
        "line_items": [{
            "credit_account_ref": legacy_principal_ref(),
            "credit_account_hash": sha256_prefixed(&legacy_principal_ref()),
            "settled_credit_delta_micros": 1_000_000,
            "source_credit_event_ids": [event.event_id],
            "source_submission_ids": [id],
            "source_list_hash": sha256_prefixed("legacy-source"),
            "near_status": "pending"
        }],
        "actor_principal_ref": static_token_principal_ref("admin-token-a"),
        "created_at": Utc::now()
    }))
    .expect("settlement fixture");
    append_credit_settlement_batch(temp.path(), "tenant-a", &batch).expect("settlement persists");
    for (token, visible) in [
        ("token-a", false),
        ("token-a-2", false),
        ("token-b", false),
        ("review-token-a", true),
        ("admin-token-a", true),
    ] {
        let Json(credit) = credit_handler(State(state.clone()), auth_headers(token))
            .await
            .expect("credit route");
        assert_eq!(credit.accepted, usize::from(visible), "{token}");
        assert_eq!(
            credit.credit_points_settled,
            if visible { 1.0 } else { 0.0 },
            "{token}"
        );
        let Json(events) = credit_events_handler(State(state.clone()), auth_headers(token))
            .await
            .expect("event route");
        assert_eq!(!events.is_empty(), visible, "{token}");
    }
}

#[tokio::test]
async fn legacy_export_excludes_expired_but_not_yet_maintained_trace() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path().to_path_buf());
    let mut envelope = low_risk_envelope().await;
    envelope.trace_card.allowed_uses = vec![TraceAllowedUse::Evaluation];
    let id = envelope.submission_id;
    let _ = test_submit(state.clone(), "token-a", envelope)
        .await
        .expect("owner submits");
    let mut record = read_submission_record(temp.path(), "tenant-a", id)
        .expect("metadata")
        .expect("record");
    record.expires_at = Some(Utc::now() - Duration::seconds(1));
    assert!(record.is_expired_at(Utc::now()));
    assert_eq!(record.status, TraceCorpusStatus::Accepted);
    write_submission_record(temp.path(), &record).expect("past deadline persists");

    let Json(export) = dataset_replay_handler(
        State(state),
        auth_headers("review-token-a"),
        Query(test_dataset_query()),
    )
    .await
    .expect("export succeeds without including the expired source");
    assert!(export.items.is_empty());
    assert_eq!(export.item_count, 0);
}
