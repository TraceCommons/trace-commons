// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

use super::*;

#[test]
fn multitenant_settlement_explicit_config_needs_no_tenant_bearer() {
    const CHILD: &str = "TC_MULTITENANT_SETTLEMENT_CONFIG_TEST";
    if std::env::var_os(CHILD).is_some() {
        assert!(
            parse_trace_credit_settlement_scheduler_config_from_env()
                .unwrap()
                .is_some()
        );
        return;
    }
    let status = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("--exact")
        .arg("ingest::app::tests::multitenant_settlement_tests::multitenant_settlement_explicit_config_needs_no_tenant_bearer")
        .arg("--nocapture")
        .env(CHILD, "1")
        .env("TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_SCOPE", "all_tenants")
        .env(TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_ENABLED, "true")
        .env(TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_POLICY_VERSION, "staging-2026-10")
        .env_remove(TRACE_COMMONS_CREDIT_SETTLEMENT_SCHEDULER_TOKEN)
        .status().unwrap();
    assert!(status.success());
}
