// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[tokio::test]
async fn export_eligibility_refuses_the_deadline_without_waiting_for_maintenance() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path().to_path_buf());
    let envelope = low_risk_envelope().await;
    let id = envelope.submission_id;
    let _ = test_submit(state, "token-a", envelope)
        .await
        .expect("submit");
    let mut record = read_submission_record(temp.path(), "tenant-a", id)
        .expect("read")
        .expect("record");
    let deadline = record.received_at + Duration::seconds(60);
    record.expires_at = Some(deadline);
    assert!(record.is_export_eligible_at(deadline - Duration::nanoseconds(1)));
    assert!(!record.is_export_eligible_at(deadline));
    assert!(!record.is_export_eligible_at(deadline + Duration::nanoseconds(1)));
    record.expires_at = None;
    assert!(record.is_export_eligible_at(deadline));
    for status in [
        TraceCorpusStatus::Quarantined,
        TraceCorpusStatus::Rejected,
        TraceCorpusStatus::Revoked,
        TraceCorpusStatus::Expired,
        TraceCorpusStatus::Purged,
    ] {
        record.status = status;
        assert!(
            !record.is_export_eligible_at(deadline),
            "{status:?} cannot export"
        );
    }
}

#[tokio::test]
async fn export_revalidation_refuses_a_source_that_expired_after_selection() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path().to_path_buf());
    let envelope = low_risk_envelope().await;
    let id = envelope.submission_id;
    let _ = test_submit(state.clone(), "token-a", envelope)
        .await
        .expect("submit");
    let auth = test_reviewer_auth("tenant-a");
    revalidate_db_export_sources(&state, &auth, &[id], false)
        .await
        .expect("live source");
    let mut record = read_submission_record(temp.path(), "tenant-a", id)
        .expect("read")
        .expect("record");
    record.expires_at = Some(Utc::now() - Duration::seconds(1));
    write_submission_record(temp.path(), &record).expect("deadline changed while work awaited");
    let error = revalidate_db_export_sources(&state, &auth, &[id], false)
        .await
        .expect_err("expired source cannot be published");
    assert!(error.to_string().contains("no longer"), "{error}");
}

#[tokio::test]
async fn ranker_export_excludes_expired_sources_without_utility_credit() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state(temp.path().to_path_buf());
    let mut envelope = low_risk_envelope().await;
    envelope.consent.scopes = vec![ConsentScope::RankingTraining];
    envelope.trace_card.consent_scope = ConsentScope::RankingTraining;
    envelope.trace_card.allowed_uses = vec![TraceAllowedUse::RankingModelTraining];
    let id = envelope.submission_id;
    let _ = test_submit(state.clone(), "token-a", envelope)
        .await
        .expect("submit");
    let mut record = read_submission_record(temp.path(), "tenant-a", id)
        .expect("read")
        .expect("record");
    assert_eq!(record.status, TraceCorpusStatus::Accepted);
    record.expires_at = Some(Utc::now() - Duration::seconds(1));
    write_submission_record(temp.path(), &record).expect("deadline");
    let Json(export) = ranker_training_candidates_handler(
        State(state),
        auth_headers("review-token-a"),
        Query(RankerTrainingExportQuery {
            status: Some(TraceCorpusStatus::Accepted),
            consent_scope: Some("ranking-training".to_string()),
            purpose: Some("expiry_regression".to_string()),
            ..test_ranker_query()
        }),
    )
    .await
    .expect("export without expired source");
    assert_eq!(export.item_count, 0);
    assert!(export.candidates.is_empty());
    assert!(
        !read_all_credit_events(temp.path(), "tenant-a")
            .expect("credits")
            .iter()
            .any(|event| event.submission_id == id
                && event.event_type == TraceCreditLedgerEventType::TrainingUtility)
    );
}

#[tokio::test]
async fn replay_revalidation_follows_metadata_authority_not_optional_mirror_presence() {
    let temp = tempfile::tempdir().expect("tempdir");
    let mut state = test_state(temp.path().to_path_buf());
    let envelope = low_risk_envelope().await;
    let id = envelope.submission_id;
    let _ = test_submit(state.clone(), "token-a", envelope)
        .await
        .expect("submit");
    revalidate_derived_export_sources(&state, &test_reviewer_auth("tenant-a"), &[id], true)
        .await
        .expect_err("required DB object references cannot use a no-DB file fallback");
    // This optional mirror has no submission implementation. File-authoritative
    // reads must not silently become DB-authoritative merely because it exists.
    Arc::get_mut(&mut state).expect("unique").db_mirror =
        Some(Arc::new(NativeAuthTestDb::default()));
    let auth = test_reviewer_auth("tenant-a");
    revalidate_replay_export_sources(&state, &auth, &[id])
        .await
        .expect("live file source");
    Arc::get_mut(&mut state)
        .expect("unique")
        .db_replay_export_reads = true;
    assert!(
        revalidate_replay_export_sources(&state, &auth, &[id])
            .await
            .is_err(),
        "DB-authoritative validation must not fall back to the live file"
    );
}

#[tokio::test]
async fn derived_export_revalidation_refuses_authoritative_file_changes_hidden_by_a_stale_mirror() {
    let Some(backend) = required_postgres_backend_for_ingest_test().await else {
        return;
    };
    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
    let temp = tempfile::tempdir().expect("tempdir");
    let mut state = test_state_with_db(temp.path().to_path_buf(), backend.clone());
    let envelope = low_risk_envelope().await;
    let id = envelope.submission_id;
    let _ = test_submit(state.clone(), "token-a", envelope)
        .await
        .expect("submit");
    let auth = test_reviewer_auth("tenant-a");
    revalidate_derived_export_sources(&state, &auth, &[id], false)
        .await
        .expect("live source");
    let mut file = read_submission_record(temp.path(), "tenant-a", id)
        .expect("read")
        .expect("record");
    let original = file.clone();
    for revoked in [false, true] {
        file = original.clone();
        if revoked {
            file.status = TraceCorpusStatus::Revoked;
        } else {
            file.expires_at = Some(Utc::now() - Duration::seconds(1));
        }
        write_submission_record(temp.path(), &file)
            .expect("authoritative file changed after selection");
        // The old publication check alone still sees an eligible mirror. This
        // positive control pins the disagreement that caused the review finding.
        revalidate_db_export_sources(&state, &auth, &[id], false)
            .await
            .expect("mirror remains live");
        revalidate_derived_export_sources(&state, &auth, &[id], false)
            .await
            .expect_err("file-authoritative derived export must refuse the stale mirror");
    }
    Arc::get_mut(&mut state)
        .expect("unique state")
        .db_reviewer_reads = true;
    revalidate_derived_export_sources(&state, &auth, &[id], false)
        .await
        .expect("DB-authoritative mode follows the live DB instead of falling back to the file");
    // File authority does not discard required object-reference validation.
    Arc::get_mut(&mut state)
        .expect("unique state")
        .db_reviewer_reads = false;
    write_submission_record(temp.path(), &original).expect("restore live file");
    let refs = revalidate_derived_export_sources(&state, &auth, &[id], true)
        .await
        .expect("required object reference is initially available");
    assert!(refs.contains_key(&id));
    let mut invalidated = 0;
    for kind in [
        StorageTraceObjectArtifactKind::SubmittedEnvelope,
        StorageTraceObjectArtifactKind::RescrubbedEnvelope,
    ] {
        invalidated += backend
            .invalidate_trace_object_refs_by_kind("tenant-a", id, kind)
            .await
            .expect("invalidate the actual envelope references");
    }
    assert!(invalidated > 0);
    revalidate_derived_export_sources(&state, &auth, &[id], true)
        .await
        .expect_err("a live file must not bypass missing required object references");
    cleanup_pg_trace_tenant(backend.as_ref(), "tenant-a").await;
}
