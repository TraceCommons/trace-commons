// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! Effective target scope regressions: an empty set means unrestricted.
use super::*;

fn scoped_admin() -> TenantAuth {
    let mut auth = test_reviewer_auth("tenant-a");
    auth.role = TokenRole::Admin;
    auth.allowed_consent_scopes = BTreeSet::from([ConsentScope::DebuggingEvaluation]);
    auth.allowed_uses = BTreeSet::from([TraceAllowedUse::Evaluation]);
    auth
}

#[tokio::test]
async fn scoped_admin_cannot_write_an_unrestricted_policy() {
    let temp = tempfile::tempdir().expect("tempdir");
    let admin = scoped_admin();
    let state = test_state_with_tokens(
        temp.path().to_path_buf(),
        BTreeMap::from([("scoped-admin".to_string(), admin)]),
    );
    let error = put_tenant_policy_handler(
        State(state),
        auth_headers("scoped-admin"),
        Json(TraceTenantPolicyRequest {
            policy_version: "scope-regression".to_string(),
            allowed_consent_scopes: vec![],
            allowed_uses: vec![],
        }),
    )
    .await
    .expect_err("unrestricted target exceeds the caller before any DB write");
    assert_eq!(error.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn scoped_admin_cannot_create_an_unrestricted_grant() {
    let temp = tempfile::tempdir().expect("tempdir");
    let state = test_state_with_tokens(
        temp.path().to_path_buf(),
        BTreeMap::from([("scoped-admin".to_string(), scoped_admin())]),
    );
    let request = serde_json::from_value(serde_json::json!({
        "principal_ref": "principal-target",
        "role": "contributor",
        "reason": "scope regression"
    }))
    .expect("omitted scope fields deserialize as unrestricted");
    let error = create_tenant_access_grant_handler(
        State(state),
        auth_headers("scoped-admin"),
        Json(request),
    )
    .await
    .expect_err("unrestricted grant exceeds the caller before any DB write");
    assert_eq!(error.0, StatusCode::FORBIDDEN);
}

#[test]
fn target_guard_compares_effective_permissions_against_each_ceiling() {
    let scopes = [
        BTreeSet::new(),
        BTreeSet::from([ConsentScope::DebuggingEvaluation]),
        BTreeSet::from([ConsentScope::ModelTraining]),
        BTreeSet::from([
            ConsentScope::DebuggingEvaluation,
            ConsentScope::ModelTraining,
        ]),
    ];
    let uses = [
        BTreeSet::new(),
        BTreeSet::from([TraceAllowedUse::Evaluation]),
        BTreeSet::from([TraceAllowedUse::ModelTraining]),
        BTreeSet::from([TraceAllowedUse::Evaluation, TraceAllowedUse::ModelTraining]),
    ];
    for target_scopes in &scopes {
        for target_uses in &uses {
            for caller_scopes in &scopes {
                for caller_uses in &uses {
                    for policy in [
                        None,
                        Some(TenantSubmissionPolicy {
                            allowed_consent_scopes: scopes[1].clone(),
                            allowed_uses: uses[1].clone(),
                        }),
                        Some(TenantSubmissionPolicy::default()),
                    ] {
                        let mut admin = scoped_admin();
                        admin.allowed_consent_scopes = caller_scopes.clone();
                        admin.allowed_uses = caller_uses.clone();
                        // Compare represented permissions over the finite domain,
                        // rather than mirroring the implementation's set formula.
                        let admits = |ceiling: &BTreeSet<_>, value| {
                            ceiling.is_empty() || ceiling.contains(&value)
                        };
                        let consent_ok = [
                            ConsentScope::DebuggingEvaluation,
                            ConsentScope::BenchmarkOnly,
                            ConsentScope::RankingTraining,
                            ConsentScope::ModelTraining,
                            ConsentScope::PublicAttribution,
                        ]
                        .into_iter()
                        .filter(|value| admits(target_scopes, *value))
                        .all(|value| {
                            admits(caller_scopes, value)
                                && policy
                                    .as_ref()
                                    .is_none_or(|p| admits(&p.allowed_consent_scopes, value))
                        });
                        let use_ok = [
                            TraceAllowedUse::Debugging,
                            TraceAllowedUse::Evaluation,
                            TraceAllowedUse::BenchmarkGeneration,
                            TraceAllowedUse::RankingModelTraining,
                            TraceAllowedUse::ModelTraining,
                            TraceAllowedUse::AggregateAnalytics,
                        ]
                        .into_iter()
                        .filter(|value| target_uses.is_empty() || target_uses.contains(value))
                        .all(|value| {
                            (caller_uses.is_empty() || caller_uses.contains(&value))
                                && policy.as_ref().is_none_or(|p| {
                                    p.allowed_uses.is_empty() || p.allowed_uses.contains(&value)
                                })
                        });
                        let result = ensure_action_target_matches_privileged_action_policy_abac(
                            target_scopes,
                            target_uses,
                            &admin,
                            policy.as_ref(),
                            "scope regression",
                        );
                        assert_eq!(
                            result.is_ok(),
                            consent_ok && use_ok,
                            "target={target_scopes:?}/{target_uses:?}, caller={caller_scopes:?}/{caller_uses:?}, policy={policy:?}"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn unrestricted_targets_remain_unrestricted_when_all_ceilings_are_unrestricted() {
    let mut auth = scoped_admin();
    auth.allowed_consent_scopes.clear();
    auth.allowed_uses.clear();
    let empty = TenantSubmissionPolicy::default();
    ensure_action_target_matches_privileged_action_policy_abac(
        &empty.allowed_consent_scopes,
        &empty.allowed_uses,
        &auth,
        Some(&empty),
        "unrestricted admin",
    )
    .expect("an unrestricted caller and tenant may create an unrestricted target");
    restrict_effective_allowlist(
        &mut auth.allowed_consent_scopes,
        BTreeSet::new(),
        "scope refused",
    )
    .expect("empty grant preserves runtime semantics");
    restrict_effective_allowlist(&mut auth.allowed_uses, BTreeSet::new(), "use refused")
        .expect("empty grant preserves runtime semantics");
    enforce_scoped_token_requested_scope(&auth, "grant regression", ConsentScope::ModelTraining)
        .expect("unrestricted claim still permits training scope");
    enforce_scoped_token_required_use(&auth, "grant regression", TraceAllowedUse::ModelTraining)
        .expect("unrestricted claim still permits training use");
}
