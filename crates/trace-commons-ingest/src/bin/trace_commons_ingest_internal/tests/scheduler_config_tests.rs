// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The shared scheduler-configuration helpers. Operators read these error
//! strings, so each one is pinned to the text the per-scheduler copies
//! produced before the helpers replaced them. The env-reading tests use
//! variable names that only this file reads, so they cannot race with a
//! test that builds `AppState` from the real environment.

use super::*;

#[test]
fn scheduler_config_error_names_each_scheduler_as_before() {
    for name in [
        "export job",
        "NEAR credit outbox",
        "retention maintenance",
        "vector index",
        "benchmark registry",
        "benchmark pipeline",
        "credit cycle",
        "credit settlement",
        "process evaluation",
        "revocation propagation",
    ] {
        let error = scheduler_config_error(name)(api_error_unlogged(
            StatusCode::FORBIDDEN,
            "worker_role_required",
        ));
        assert_eq!(
            error.to_string(),
            format!(
                "invalid Trace Commons {name} scheduler configuration: \
                 status=403 Forbidden, error=worker_role_required"
            )
        );
    }
}

const ENABLED: &str = "TRACE_COMMONS_TEST_ONLY_F1_SCHEDULER_ENABLED";
const TOKEN: &str = "TRACE_COMMONS_TEST_ONLY_F1_SCHEDULER_TOKEN";
const INTERVAL: &str = "TRACE_COMMONS_TEST_ONLY_F1_SCHEDULER_INTERVAL_SECONDS";
const PURPOSE: &str = "TRACE_COMMONS_TEST_ONLY_F1_SCHEDULER_PURPOSE";

/// Serializes the tests below; they share the four variables above.
static SCHEDULER_HELPER_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn set(var: &str, value: Option<&str>) {
    // SAFETY: the variables are read only by the tests in this file, which
    // hold SCHEDULER_HELPER_ENV_LOCK.
    unsafe {
        match value {
            Some(value) => std::env::set_var(var, value),
            None => std::env::remove_var(var),
        }
    }
}

#[test]
fn scheduler_worker_token_is_off_on_or_refused() {
    let _guard = SCHEDULER_HELPER_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    // Neither set: the scheduler is off.
    set(ENABLED, None);
    set(TOKEN, None);
    assert_eq!(
        scheduler_worker_token_from_env(ENABLED, TOKEN).expect("off"),
        None
    );

    // Enabled without a token: refused, naming both variables, exactly as
    // each scheduler's own copy did.
    set(ENABLED, Some("true"));
    let error = scheduler_worker_token_from_env(ENABLED, TOKEN).expect_err("refused");
    assert_eq!(
        error.to_string(),
        "TRACE_COMMONS_TEST_ONLY_F1_SCHEDULER_ENABLED=true requires \
         TRACE_COMMONS_TEST_ONLY_F1_SCHEDULER_TOKEN"
    );

    // A token alone switches the scheduler on, trimmed.
    set(ENABLED, None);
    set(TOKEN, Some("  worker-token  "));
    assert_eq!(
        scheduler_worker_token_from_env(ENABLED, TOKEN).expect("on"),
        Some("worker-token".to_string())
    );
    set(TOKEN, None);
}

#[test]
fn scheduler_interval_keeps_the_five_second_to_one_day_bounds() {
    let _guard = SCHEDULER_HELPER_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    set(INTERVAL, None);
    assert_eq!(
        scheduler_interval_from_env(INTERVAL, 300).expect("default"),
        StdDuration::from_secs(300)
    );
    for accepted in ["5", "86400"] {
        set(INTERVAL, Some(accepted));
        assert_eq!(
            scheduler_interval_from_env(INTERVAL, 300).expect("in range"),
            StdDuration::from_secs(accepted.parse().unwrap())
        );
    }
    for refused in ["4", "86401", "soon"] {
        set(INTERVAL, Some(refused));
        let helper = scheduler_interval_from_env(INTERVAL, 300).expect_err("refused");
        let direct = parse_optional_scheduler_u64_env(INTERVAL, 300, 5, 86_400)
            .expect_err("refused by the bound parser too");
        assert_eq!(helper.to_string(), direct.to_string());
    }
    set(INTERVAL, None);
}

#[test]
fn scheduler_text_defaults_then_validates() {
    let _guard = SCHEDULER_HELPER_ENV_LOCK
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    set(PURPOSE, None);
    assert_eq!(
        scheduler_text_from_env(
            PURPOSE,
            "scheduled vector index",
            validate_vector_index_scheduler_purpose
        )
        .expect("default"),
        "scheduled vector index"
    );

    set(PURPOSE, Some("x".repeat(1025).as_str()));
    let error = scheduler_text_from_env(
        PURPOSE,
        "scheduled vector index",
        validate_vector_index_scheduler_purpose,
    )
    .expect_err("too long");
    assert_eq!(
        error.to_string(),
        "vector index scheduler purpose is too long"
    );
    set(PURPOSE, None);
}
