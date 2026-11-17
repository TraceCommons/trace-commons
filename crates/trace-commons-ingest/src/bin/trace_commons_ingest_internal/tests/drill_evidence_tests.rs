// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Drill evidence. Promotion gates compare evidence hashes across runs, so
//! the hash functions whose inputs are plain values are pinned here, and the
//! shared recorder is checked to store exactly what a drill hands it.

use super::*;

fn pinned_tenant() -> TenantAuth {
    test_admin_auth("tenant-a")
}

#[test]
fn rollback_drill_evidence_hash_is_pinned() {
    let hash = rollback_drill_evidence_hash(
        &pinned_tenant(),
        &["TRACE_COMMONS_DB_DUAL_WRITE".to_string()],
        3,
        2,
        5,
        4,
        1,
        0,
        &["db_submission_count_mismatch".to_string()],
    );
    assert_eq!(
        hash,
        "sha256:b1da7102627ae128da3f09acd1585d98ca7fea077e9c7249d3a39c37a6a658f5"
    );
}

#[test]
fn per_check_drill_evidence_hashes_are_pinned() {
    let tenant = pinned_tenant();
    assert_eq!(
        canary_read_drill_check_evidence_hash(&tenant, "sha256:aggregate", "submit_status", true),
        "sha256:7c920da52d3b44a494cd98702ce8776a829748ccb1b1e48667bc47a7c6ffc2dc"
    );
    assert_eq!(
        revocation_effects_drill_check_evidence_hash(
            &tenant,
            "sha256:aggregate",
            "export_excluded",
            false
        ),
        "sha256:a52259b4d4cd8d600286af6e030f085f4f6028bc390e56193207e9e0aaf9e065"
    );
}

#[tokio::test]
async fn recorded_drill_evidence_carries_what_the_drill_passed() {
    let temp = tempfile::tempdir().expect("temp dir");
    let state = test_state(temp.path().to_path_buf());
    let tenant = pinned_tenant();

    for (ready, status) in [
        (true, TraceRolloutSmokeEvidenceStatus::Passed),
        (false, TraceRolloutSmokeEvidenceStatus::Failed),
    ] {
        let evidence = record_rollout_smoke_evidence(
            state.as_ref(),
            &tenant,
            "rollback_flag_drill",
            ready,
            "sha256:evidence".to_string(),
            Some("sha256:purpose".to_string()),
        )
        .await
        .expect("evidence is recorded");
        assert_eq!(evidence.status, status);
        assert_eq!(evidence.check_name, "rollback_flag_drill");
        assert_eq!(evidence.evidence_hash, "sha256:evidence");
        assert_eq!(
            evidence.evidence_ref_hash.as_deref(),
            Some("sha256:purpose")
        );
        assert_eq!(evidence.tenant_id, tenant.tenant_id);
        assert_eq!(
            evidence.tenant_storage_ref,
            tenant_storage_ref(&tenant.tenant_id)
        );
        assert_eq!(evidence.actor_principal_ref, tenant.principal_ref);
    }
}
