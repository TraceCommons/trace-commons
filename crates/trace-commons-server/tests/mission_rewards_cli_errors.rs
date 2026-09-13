// Copyright (C) 2026 K&Z Partners LLC
// SPDX-License-Identifier: AGPL-3.0-or-later
// INTEGRATION: proves the reward CLI fails safely before connecting without its dedicated URL.

use std::process::Command;

const MISSING_URL_MESSAGE: &str =
    "error: reward_database_url_missing: set TRACE_COMMONS_REWARDS_DATABASE_URL and retry\n";
const FALLBACK_MARKER: &str = "reward-cli-fallback-marker";

#[test]
fn reward_operator_refuses_remote_transport_with_a_safe_error() {
    let output = Command::new(env!("CARGO_BIN_EXE_trace-commons-reward-operator"))
        .args([
            "--tenant",
            "reward-cli-errors",
            "program-show",
            "--program",
            "00000000-0000-0000-0000-000000000001",
        ])
        .env(
            "TRACE_COMMONS_REWARDS_DATABASE_URL",
            "postgres://private-operator-marker@203.0.113.1/rewards",
        )
        .output()
        .expect("run reward CLI with remote transport");
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(
        String::from_utf8(output.stderr).expect("UTF-8 stderr"),
        "error: reward_transport_invalid: use a loopback address or Unix socket through a protected local connection\n"
    );
}

#[test]
fn reward_operator_requires_a_nonblank_dedicated_database_url() {
    for (case, reward_url) in [
        ("missing", None),
        ("empty", Some("")),
        ("whitespace", Some(" \t\n ")),
    ] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_trace-commons-reward-operator"));
        command
            .args([
                "--tenant",
                "reward-cli-errors",
                "program-show",
                "--program",
                "00000000-0000-0000-0000-000000000001",
            ])
            .env_remove("TRACE_COMMONS_REWARDS_DATABASE_URL")
            .env(
                "DATABASE_URL",
                format!("postgres://{FALLBACK_MARKER}@127.0.0.1:1/ignored"),
            );
        if let Some(reward_url) = reward_url {
            command.env("TRACE_COMMONS_REWARDS_DATABASE_URL", reward_url);
        }

        let output = command
            .output()
            .unwrap_or_else(|error| panic!("run reward CLI for {case}: {error}"));
        assert!(
            !output.status.success(),
            "{case} dedicated URL must fail before an operation runs"
        );
        assert!(
            output.stdout.is_empty(),
            "{case} dedicated URL must not emit JSON output"
        );
        let stderr = String::from_utf8(output.stderr)
            .unwrap_or_else(|error| panic!("{case} stderr must be UTF-8: {error}"));
        assert_eq!(stderr, MISSING_URL_MESSAGE, "{case} dedicated URL error");
        assert!(
            !stderr.contains(FALLBACK_MARKER),
            "{case} error must not leak the ignored DATABASE_URL fallback"
        );
    }
}
