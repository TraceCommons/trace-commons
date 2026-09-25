// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use trace_commons_server::account_onboarding::{
    PROVISIONING_TTL_SECONDS, PendingNearProvisioning, ProvisioningAssertion,
};
use trace_commons_server::account_trust::{PolicyPeriod, parse_bounded_policy};
use trace_commons_server::config::NearConfig;

#[test]
fn bounded_policy_requires_explicit_reviewed_controls() {
    // These small values and the version name are test fixtures only.
    const FIXTURE: &str = r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":12,"period":{"mode":"lifetime"},"growth_rule":"none"}"#;
    let policy = parse_bounded_policy(FIXTURE, &["test-v1"]).unwrap();
    assert_eq!(policy.processing_cost_bound(), 3);
    assert_eq!(policy.bounded_allowance(), 12);
    assert_eq!(policy.period(), &PolicyPeriod::Lifetime);
    assert!(parse_bounded_policy(FIXTURE, &[]).is_err());

    for invalid in [
        r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":12,"period":{"mode":"lifetime","seconds":86400},"growth_rule":"none"}"#,
        r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":12,"period":{"mode":"lifetime","typo":true},"growth_rule":"none"}"#,
        r#"{"version":"test-v1","processing_cost_bound":0,"bounded_allowance":12,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
        r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":0,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
        r#"{"version":"test-v1","processing_cost_bound":13,"bounded_allowance":12,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
        r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":12,"growth_rule":"none"}"#,
        r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":12,"period":{"mode":"period"},"growth_rule":"none"}"#,
        r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":12,"period":{"mode":"fixed","seconds":0},"growth_rule":"none"}"#,
        r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":12,"period":{"mode":"lifetime"},"growth_rule":"quality"}"#,
        r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":12,"period":{"mode":"lifetime"}}"#,
        r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":9223372036854775807,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
        r#"{"version":"unknown","processing_cost_bound":3,"bounded_allowance":12,"period":{"mode":"lifetime"},"growth_rule":"none"}"#,
    ] {
        assert!(
            parse_bounded_policy(invalid, &["test-v1"]).is_err(),
            "{invalid}"
        );
    }
    let fixed = r#"{"version":"test-v1","processing_cost_bound":3,"bounded_allowance":12,"period":{"mode":"fixed","seconds":3600},"growth_rule":"none"}"#;
    assert_eq!(
        parse_bounded_policy(fixed, &["test-v1"]).unwrap().period(),
        &PolicyPeriod::Fixed { seconds: 3600 }
    );
}

fn config() -> NearConfig {
    // No test makes an RPC call: invalid signatures must be rejected first.
    NearConfig {
        rpc_url: "https://rpc.invalid".into(),
        network: "mainnet".into(),
        recipient: "app.tracecommons.test".into(),
    }
}

#[test]
fn issuance_rejects_noncanonical_accounts_and_missing_controls() {
    for account in [
        "",
        "a",
        "Alice.near",
        " alice.near",
        "alice.near\n",
        ".alice",
        "alice.",
        "alice..near",
        "alice-_near",
        "alice/near",
    ] {
        assert!(PendingNearProvisioning::issue(&config(), account, [1; 32], [2; 32], 100).is_err());
    }
    assert!(
        PendingNearProvisioning::issue(&config(), &"a".repeat(65), [1; 32], [2; 32], 100).is_err()
    );
    for account in ["alice.near", "alice_test.near", "ab", &"a".repeat(64)] {
        assert!(PendingNearProvisioning::issue(&config(), account, [1; 32], [2; 32], 100).is_ok());
    }
    assert!(
        PendingNearProvisioning::issue(&config(), "alice.near", [1; 32], [0; 32], 100).is_err()
    );
    assert!(
        PendingNearProvisioning::issue(&config(), "alice.near", [1; 32], [2; 32], i64::MAX)
            .is_err()
    );
    for field in ["network", "recipient", "rpc"] {
        let mut cfg = config();
        match field {
            "network" => cfg.network.clear(),
            "recipient" => cfg.recipient.clear(),
            _ => cfg.rpc_url.clear(),
        }
        assert!(PendingNearProvisioning::issue(&cfg, "alice.near", [1; 32], [2; 32], 100).is_err());
    }
}

#[tokio::test]
async fn invalid_finish_has_only_safe_error_and_no_provisioned_identity() {
    let cfg = config();
    let p = PendingNearProvisioning::issue(&cfg, "alice.near", [1; 32], [2; 32], 100).unwrap();
    let challenge = p.challenge();
    assert_eq!(challenge.expires_at, 100 + PROVISIONING_TTL_SECONDS);
    assert!(
        challenge
            .message
            .contains("trace_commons.near_provisioning.v1")
    );
    let result = p
        .verify(
            &cfg,
            ProvisioningAssertion {
                wallet_public_key: "invalid",
                wallet_signature: "invalid",
                device_signature: "invalid",
            },
            &[2; 32],
            101,
        )
        .await;
    let error = result.err().expect("must refuse before RPC");
    assert_eq!(error.to_string(), "near_provisioning_refused");
    assert!(!format!("{error:?}").contains("alice"));
}
