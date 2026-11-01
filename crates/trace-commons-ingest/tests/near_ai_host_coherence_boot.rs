// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later

//! The ingest binary refuses to boot on a NEAR AI host from the other
//! channel (staging design, section 3.9).
//!
//! The unit tests cover the check itself; these run the built binary, so
//! they also hold the check's place in the boot: it must run, and it must
//! run before any database, token or network configuration is read. The
//! environment is cleared, so a binary that reached any of those would fail
//! on something else first, and the assertions below would not hold.

use std::process::{Command, Output};

const CONTROL: &str = "missing_control:near_ai_staging_host_coherence";
const SCORER_BASE: &str = "TRACE_COMMONS_NEAR_AI_BASE_URL";
const ATTESTATION_BASE: &str = "TRACE_COMMONS_NEAR_AI_ATTESTATION_BASE_URL";
const MARKER: &str = "boot-coherence-marker-q7";

/// A NEAR AI host from the channel this build does not belong to, with a
/// marker that must never appear in the refusal.
#[cfg(feature = "near-ai-staging")]
fn other_channel_base() -> String {
    format!("https://cloud-api.near.ai/v1?{MARKER}")
}

#[cfg(not(feature = "near-ai-staging"))]
fn other_channel_base() -> String {
    format!("https://cloud-stg-api.near.ai/v1?{MARKER}")
}

/// The settings every boot of this build needs to pass the check: a staging
/// build must name the attestation base, whose default is production.
fn coherent_env() -> Vec<(&'static str, String)> {
    if cfg!(feature = "near-ai-staging") {
        vec![(ATTESTATION_BASE, "https://cloud-stg-api.near.ai/v1".into())]
    } else {
        Vec::new()
    }
}

fn run_ingest(env: &[(&'static str, String)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_trace-commons-ingest"));
    command.env_clear();
    for (name, value) in env {
        command.env(name, value);
    }
    command.output().expect("run trace-commons-ingest")
}

#[test]
fn the_ingest_binary_refuses_a_near_ai_host_from_the_other_channel_at_boot() {
    let mut env = coherent_env();
    env.push((SCORER_BASE, other_channel_base()));
    let output = run_ingest(&env);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!output.status.success(), "boot must fail: {stderr}");
    assert!(stderr.contains(CONTROL), "stderr: {stderr}");
    assert!(stderr.contains(SCORER_BASE), "stderr: {stderr}");
    assert!(!stderr.contains(MARKER) && !stdout.contains(MARKER));
    assert!(!stderr.contains("near.ai/v1") && !stdout.contains("near.ai/v1"));
}

#[test]
fn the_ingest_binary_without_a_foreign_host_gets_past_the_check() {
    let output = run_ingest(&coherent_env());
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Nothing else is configured, so the boot still fails, but on a later
    // control: the refusal above came from the host, not from the empty env.
    assert!(!output.status.success(), "an unconfigured boot must fail");
    assert!(!stderr.contains(CONTROL), "stderr: {stderr}");
}

const SCORER_PRODUCTION_HOST: &str = "TRACE_COMMONS_NEAR_AI_SCORER_PRODUCTION_HOST";
const SCORER_PRODUCTION_HOST_CONTROL: &str = "near_ai_scorer_production_host";
const SCORER_KEY: &str = "TRACE_COMMONS_NEAR_AI_SCORER_API_KEY";
const KEY_MARKER: &str = "sk-boot-scorer-marker-c4";

/// Owner exception, 2026-10-24 (staging design 3.9): a staging build with
/// the opt-in flag gets past the check with its scorer on a production
/// direct-completions host, and says so in one label line that names
/// neither the host nor the key.
#[cfg(feature = "near-ai-staging")]
#[test]
fn a_staging_ingest_with_the_flag_boots_past_the_check_on_a_production_completions_scorer() {
    let mut env = coherent_env();
    env.push((
        SCORER_BASE,
        "https://boot-marker-q7.completions.near.ai/v1".into(),
    ));
    env.push((SCORER_PRODUCTION_HOST, "true".into()));
    env.push((SCORER_KEY, KEY_MARKER.into()));
    // The deployed env's level (deploy/pilot-gcp/ingest.env.template).
    env.push(("RUST_LOG", "info".into()));
    let output = run_ingest(&env);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!output.status.success(), "an unconfigured boot must fail");
    assert!(!stderr.contains(CONTROL), "stderr: {stderr}");
    assert!(
        stdout.contains(SCORER_PRODUCTION_HOST_CONTROL)
            || stderr.contains(SCORER_PRODUCTION_HOST_CONTROL),
        "the exception is logged: {stdout} {stderr}"
    );
    for leaked in ["boot-marker-q7", "completions.near.ai", KEY_MARKER] {
        assert!(
            !stdout.contains(leaked) && !stderr.contains(leaked),
            "{leaked}: {stdout} {stderr}"
        );
    }
}

/// A default build refuses the flag at boot, whatever the scorer host.
#[cfg(not(feature = "near-ai-staging"))]
#[test]
fn a_default_ingest_refuses_the_scorer_production_host_flag_at_boot() {
    let env = vec![
        (
            SCORER_BASE,
            "https://boot-marker-q7.completions.near.ai/v1".to_string(),
        ),
        (SCORER_PRODUCTION_HOST, "true".to_string()),
        (SCORER_KEY, KEY_MARKER.to_string()),
    ];
    let output = run_ingest(&env);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let stdout = String::from_utf8_lossy(&output.stdout);

    assert!(!output.status.success(), "boot must fail: {stderr}");
    assert!(
        stderr.contains(&format!("missing_control:{SCORER_PRODUCTION_HOST_CONTROL}"))
            && stderr.contains(SCORER_PRODUCTION_HOST),
        "stderr: {stderr}"
    );
    for leaked in ["boot-marker-q7", KEY_MARKER] {
        assert!(!stdout.contains(leaked) && !stderr.contains(leaked));
    }
}
