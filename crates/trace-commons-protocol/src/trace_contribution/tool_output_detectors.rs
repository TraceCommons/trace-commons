//! Deterministic detectors for identifiers that tool output carries (#1362).
//!
//! Stub: detection not implemented yet.

use super::{RedactionReport, RedactionState};

pub(super) const GIT_AUTHOR: &str = "git_author";
pub(super) const IP_ADDRESS: &str = "ip_address";
pub(super) const PRIVATE_HOSTNAME: &str = "private_hostname";
pub(super) const OS_USERNAME: &str = "os_username";
pub(super) const PHONE_NUMBER: &str = "phone_number";

pub(super) fn redact_identity_shapes(
    input: &str,
    _state: &mut RedactionState,
    _report: &mut RedactionReport,
) -> String {
    input.to_string()
}

pub(super) fn redact_network_and_account_shapes(
    input: &str,
    _state: &mut RedactionState,
    _report: &mut RedactionReport,
) -> String {
    input.to_string()
}

#[cfg(test)]
mod tests {
    #![deny(dead_code)]

    use super::super::{
        ConsentMetadata, ConsentScope, DeterministicTraceRedactor, RawTraceCaptureToolCall,
        RawTraceCaptureTurn, RawTraceContribution, RecordedTraceContributionOptions,
        RedactionReport, ResidualPiiRisk, TRACE_CONTRIBUTION_POLICY_VERSION, TraceRedactor,
        label_bears_severity, rescrub_trace_envelope_with, residual_envelope_scan, residual_risk,
    };
    use super::*;

    const NEW_LABELS: [&str; 5] = [
        GIT_AUTHOR,
        IP_ADDRESS,
        PRIVATE_HOSTNAME,
        OS_USERNAME,
        PHONE_NUMBER,
    ];

    fn redactor() -> DeterministicTraceRedactor {
        DeterministicTraceRedactor::deterministic_only(Vec::new())
    }

    fn redact(input: &str) -> (String, RedactionReport) {
        redactor().redact_text(input)
    }

    fn count(report: &RedactionReport, label: &str) -> u32 {
        report.counts.get(label).copied().unwrap_or(0)
    }

    /// Asserts the text is untouched and none of the new labels fired.
    fn assert_untouched(input: &str) {
        let (out, report) = redact(input);
        for label in NEW_LABELS {
            assert_eq!(
                count(&report, label),
                0,
                "{label} fired on {input:?} -> {out:?}"
            );
        }
        assert_eq!(out, input, "text changed: {input:?}");
    }

    // --- git_author -------------------------------------------------------

    #[test]
    fn git_log_author_lines_keep_the_label_and_lose_the_name() {
        let input = "commit 3f2a9c1\nAuthor: Jane Q. Example <jane@example.com>\nDate:   Sat Oct 11 10:00:00 2026 +0000\n\n    Fix the build\n";
        let (out, report) = redact(input);
        assert_eq!(
            out,
            "commit 3f2a9c1\nAuthor: <PRIVATE_GIT_AUTHOR_1> <<PRIVATE_EMAIL_1>>\nDate:   Sat Oct 11 10:00:00 2026 +0000\n\n    Fix the build\n"
        );
        assert_eq!(count(&report, GIT_AUTHOR), 1);
        assert_eq!(count(&report, "private_email"), 1);
    }

    #[test]
    fn git_trailers_are_covered_case_insensitively_and_repeat_names_share_a_placeholder() {
        let input = "    Signed-off-by: Jane Example <jane@example.com>\n    Co-Authored-By: Sam Sample <sam@example.org>\n    co-authored-by: Jane Example <jane@example.com>\nCommitter: Sam Sample <sam@example.org>";
        let (out, report) = redact(input);
        assert_eq!(
            out,
            "    Signed-off-by: <PRIVATE_GIT_AUTHOR_1> <<PRIVATE_EMAIL_1>>\n    Co-Authored-By: <PRIVATE_GIT_AUTHOR_2> <<PRIVATE_EMAIL_2>>\n    co-authored-by: <PRIVATE_GIT_AUTHOR_1> <<PRIVATE_EMAIL_1>>\nCommitter: <PRIVATE_GIT_AUTHOR_2> <<PRIVATE_EMAIL_2>>"
        );
        assert_eq!(count(&report, GIT_AUTHOR), 4);
    }

    #[test]
    fn git_author_lines_inside_an_escaped_json_string_are_covered() {
        let input = r#"{"stdout":"commit abc\nAuthor: Jane Example <jane@example.com>\n"}"#;
        let (out, report) = redact(input);
        assert!(
            out.contains(r"\nAuthor: <PRIVATE_GIT_AUTHOR_1> <"),
            "{out}"
        );
        assert_eq!(count(&report, GIT_AUTHOR), 1);
    }

    #[test]
    fn git_blame_lines_lose_the_author_name() {
        let input =
            "^3f2a9c1 src/lib.rs (Jane Example 2026-10-11 10:00:00 +0000 12) fn main() {}";
        let (out, report) = redact(input);
        assert_eq!(
            out,
            "^3f2a9c1 src/lib.rs (<PRIVATE_GIT_AUTHOR_1> 2026-10-11 10:00:00 +0000 12) fn main() {}"
        );
        assert_eq!(count(&report, GIT_AUTHOR), 1);
    }

    #[test]
    fn prose_mentioning_an_author_is_not_a_git_line() {
        assert_untouched("The author: see the README for details.");
        assert_untouched("Author: unknown");
        assert_untouched("AuthorDate: Sat Oct 11 10:00:00 2026 +0000");
    }

    // --- ip_address -------------------------------------------------------

    #[test]
    fn public_and_private_ipv4_addresses_are_replaced() {
        let (out, report) = redact(
            "Connecting to 34.117.59.81:443 from 10.0.4.17 via 192.168.1.20 and 100.101.102.103, again 34.117.59.81",
        );
        assert_eq!(
            out,
            "Connecting to <PRIVATE_IP_ADDRESS_1>:443 from <PRIVATE_IP_ADDRESS_2> via <PRIVATE_IP_ADDRESS_3> and <PRIVATE_IP_ADDRESS_4>, again <PRIVATE_IP_ADDRESS_1>"
        );
        assert_eq!(count(&report, IP_ADDRESS), 5);
    }

    #[test]
    fn loopback_unspecified_link_local_documentation_and_masks_stay() {
        assert_untouched("listening on 127.0.0.1:8080 and 0.0.0.0:3000");
        assert_untouched("link-local 169.254.169.254 metadata");
        assert_untouched("examples 192.0.2.10, 198.51.100.7, 203.0.113.99");
        assert_untouched("netmask 255.255.255.0 broadcast 255.255.255.255 wildcard 0.0.0.255");
        assert_untouched("multicast 224.0.0.251");
        assert_untouched("resolvers 8.8.8.8 and 1.1.1.1");
    }

    #[test]
    fn version_strings_and_longer_dotted_runs_are_not_addresses() {
        assert_untouched("bumped to v1.2.3.4 today");
        assert_untouched("version 1.2.3.4");
        assert_untouched("pinned foo==1.2.3.4");
        assert_untouched("Mozilla/5.0 Chrome/120.0.0.0 Safari/537.36");
        assert_untouched("oid 1.3.6.1.4.1.311.21");
        assert_untouched("release 1.2.3.4-beta.1");
        assert_untouched("not an address 256.1.1.1 or 01.02.03.04");
        assert_untouched("rustc 1.82.0 (f6e511eec 2024-10-15)");
        assert_untouched("cache key ab/34.117.59.81/blob");
    }

    #[test]
    fn urls_with_an_address_host_have_the_host_replaced() {
        let (out, report) = redact("curl http://34.117.59.81/health and https://10.1.2.3:8443/x");
        assert_eq!(
            out,
            "curl http://<PRIVATE_IP_ADDRESS_1>/health and https://<PRIVATE_IP_ADDRESS_2>:8443/x"
        );
        assert_eq!(count(&report, IP_ADDRESS), 2);
    }

    #[test]
    fn global_and_unique_local_ipv6_addresses_are_replaced() {
        let (out, report) = redact(
            "inet6 2a01:4f8:c17:1234::1 prefixlen 64 and fd12:3456:789a::5 and [2606:4700:4700::1111]:443",
        );
        assert_eq!(
            out,
            "inet6 <PRIVATE_IP_ADDRESS_1> prefixlen 64 and <PRIVATE_IP_ADDRESS_2> and [<PRIVATE_IP_ADDRESS_3>]:443"
        );
        assert_eq!(count(&report, IP_ADDRESS), 3);
    }

    #[test]
    fn ipv6_lookalikes_and_non_global_ipv6_stay() {
        assert_untouched("loopback ::1 and fe80::1%en0 and 2001:db8::1");
        assert_untouched("at 12:30:45 the mac aa:bb:cc:dd:ee:ff changed");
        assert_untouched("use std::io::Error; serde::de::Deserialize");
        assert_untouched("let x = a::b::c; dead::beef");
        assert_untouched("sha 3f2a9c1e4b5d6f7a8b9c0d1e2f3a4b5c6d7e8f90");
    }

    // --- private_hostname -------------------------------------------------

    #[test]
    fn private_suffix_hostnames_are_replaced() {
        let (out, report) = redact(
            "ping Janes-MacBook-Pro.local; ssh build01.corp; curl http://grafana.internal:3000/ and nas.home.arpa, db.lan",
        );
        assert_eq!(
            out,
            "ping <PRIVATE_HOSTNAME_1>; ssh <PRIVATE_HOSTNAME_2>; curl http://<PRIVATE_HOSTNAME_3>:3000/ and <PRIVATE_HOSTNAME_4>, <PRIVATE_HOSTNAME_5>"
        );
        assert_eq!(count(&report, PRIVATE_HOSTNAME), 5);
    }

    #[test]
    fn well_known_internal_names_and_local_files_stay() {
        assert_untouched("host.docker.internal and metadata.google.internal");
        assert_untouched("edit .env.local and Makefile.local and settings.local.json");
        assert_untouched("see config/app.local/readme and localhost:8080");
        assert_untouched("github.com and docs.rs and example.com");
    }

    #[test]
    fn an_address_at_a_private_suffix_stays_an_email() {
        let (out, report) = redact("mail jane@corp.internal");
        assert_eq!(out, "mail <PRIVATE_EMAIL_1>");
        assert_eq!(count(&report, PRIVATE_HOSTNAME), 0);
    }

    // --- shell prompts, ssh targets and os_username ----------------------

    #[test]
    fn shell_prompts_lose_user_and_host() {
        let (out, report) = redact(
            "jdoe@devbox:~/src$ ls\njdoe@Janes-MacBook-Pro trace-commons % git status\n[jdoe@fedora ~]$ id",
        );
        assert_eq!(
            out,
            "<PRIVATE_OS_USERNAME_1>@<PRIVATE_HOSTNAME_1>:~/src$ ls\n<PRIVATE_OS_USERNAME_1>@<PRIVATE_HOSTNAME_2> trace-commons % git status\n[<PRIVATE_OS_USERNAME_1>@<PRIVATE_HOSTNAME_3> ~]$ id"
        );
        assert_eq!(count(&report, OS_USERNAME), 3);
        assert_eq!(count(&report, PRIVATE_HOSTNAME), 3);
    }

    #[test]
    fn a_prompt_at_a_local_host_is_not_an_email() {
        let (out, report) = redact("jdoe@devbox.local:~$ make");
        assert_eq!(out, "<PRIVATE_OS_USERNAME_1>@<PRIVATE_HOSTNAME_1>:~$ make");
        assert_eq!(count(&report, "private_email"), 0);
    }

    #[test]
    fn generic_accounts_keep_their_name() {
        let (out, report) = redact("root@buildhost:/# ls");
        assert_eq!(out, "root@<PRIVATE_HOSTNAME_1>:/# ls");
        assert_eq!(count(&report, OS_USERNAME), 0);
    }

    #[test]
    fn ssh_family_targets_lose_user_and_bare_host() {
        let (out, report) =
            redact("ssh -p 2222 jdoe@buildbox uptime\nscp out.tar ubuntu@34.117.59.81:/srv/");
        assert_eq!(
            out,
            "ssh -p 2222 <PRIVATE_OS_USERNAME_1>@<PRIVATE_HOSTNAME_1> uptime\nscp out.tar ubuntu@<PRIVATE_IP_ADDRESS_1>:/srv/"
        );
        assert_eq!(count(&report, OS_USERNAME), 1);
    }

    #[test]
    fn package_and_action_references_are_not_user_at_host() {
        assert_untouched("npm install lodash@latest react@18 @types/node");
        assert_untouched("uses: actions/checkout@v6");
        assert_untouched("git rev-parse HEAD@{upstream}");
    }

    #[test]
    fn id_whoami_env_and_ls_output_lose_the_account_name() {
        let input = "$ id\nuid=501(jdoe) gid=20(staff) groups=20(staff),12(everyone),501(jdoe)\n$ whoami\njdoe\nUSER=jdoe\nLOGNAME=jdoe\n-rw-r--r--@  1 jdoe  staff  1204 Oct 11 10:00 notes.txt\ndrwxr-xr-x   4 jdoe  jdoe    128 Oct 11 10:00 src\n-rw-r--r--   1 root  wheel    10 Oct 11 10:00 hosts";
        let (out, report) = redact(input);
        assert_eq!(
            out,
            "$ id\nuid=501(<PRIVATE_OS_USERNAME_1>) gid=20(staff) groups=20(staff),12(everyone),501(<PRIVATE_OS_USERNAME_1>)\n$ whoami\n<PRIVATE_OS_USERNAME_1>\nUSER=<PRIVATE_OS_USERNAME_1>\nLOGNAME=<PRIVATE_OS_USERNAME_1>\n-rw-r--r--@  1 <PRIVATE_OS_USERNAME_1>  staff  1204 Oct 11 10:00 notes.txt\ndrwxr-xr-x   4 <PRIVATE_OS_USERNAME_1>  <PRIVATE_OS_USERNAME_1>    128 Oct 11 10:00 src\n-rw-r--r--   1 root  wheel    10 Oct 11 10:00 hosts"
        );
        assert_eq!(count(&report, OS_USERNAME), 8);
    }

    #[test]
    fn a_home_directory_name_is_already_a_path() {
        let (out, report) = redact("cd /home/jdoe/src && ls /Users/jdoe");
        assert_eq!(
            out,
            "cd <PRIVATE_LOCAL_PATH_1> && ls <PRIVATE_LOCAL_PATH_2>"
        );
        assert_eq!(count(&report, OS_USERNAME), 0);
    }

    // --- phone_number -----------------------------------------------------

    #[test]
    fn e164_and_national_phone_numbers_are_replaced() {
        let (out, report) = redact(
            "call +1 415 555 0132 or +44 20 7946 0958 or +14155550132, (415) 555-0132, 415-555-0132, 07700 900123",
        );
        assert_eq!(
            out,
            "call <PRIVATE_PHONE_NUMBER_1> or <PRIVATE_PHONE_NUMBER_2> or <PRIVATE_PHONE_NUMBER_3>, <PRIVATE_PHONE_NUMBER_4>, <PRIVATE_PHONE_NUMBER_5>, <PRIVATE_PHONE_NUMBER_6>"
        );
        assert_eq!(count(&report, PHONE_NUMBER), 6);
    }

    #[test]
    fn numbers_that_are_not_phone_numbers_stay() {
        assert_untouched("ts 1760176800 pid 48213 port 5432");
        assert_untouched("date 2026-10-11 and 2026-10-11T10:00:00Z");
        assert_untouched("src/lib.rs:123:45: error");
        assert_untouched("+1760176800\n+    let x = 4155550132;");
        assert_untouched("ssn-shaped 123-45-6789 and short 555-0132");
        assert_untouched("call 115-555-0132 (area codes never start with 1)");
        assert_untouched("uuid 123e4567-e89b-12d3-a456-426614174000");
        assert_untouched("bytes 1+234+567+8901");
    }

    // --- whole-pipeline properties ----------------------------------------

    const MIXED: &str = "jdoe@devbox:~$ git log -1\ncommit 3f2a9c1\nAuthor: Jane Example <jane@example.com>\n\nuid=501(jdoe) gid=20(staff)\nping Janes-MacBook-Pro.local (10.0.4.17): 34.117.59.81 2a01:4f8:c17:1234::1 +1 415 555 0132";

    #[test]
    fn running_twice_changes_nothing_and_finds_nothing() {
        let (once, first) = redact(MIXED);
        for label in NEW_LABELS {
            assert!(
                count(&first, label) > 0,
                "{label} did not fire on the mixed sample"
            );
        }
        let (twice, second) = redact(&once);
        assert_eq!(twice, once);
        for label in NEW_LABELS {
            assert_eq!(
                count(&second, label),
                0,
                "{label} fired on its own output: {once}"
            );
        }
        assert!(
            !once.contains("<<PRIVATE_GIT"),
            "double placeholder in {once}"
        );
        assert!(
            !once.contains("<PRIVATE_PRIVATE"),
            "nested placeholder in {once}"
        );
    }

    #[test]
    fn severity_follows_the_person_versus_machine_split() {
        assert!(label_bears_severity(GIT_AUTHOR));
        assert!(label_bears_severity(PHONE_NUMBER));
        assert!(!label_bears_severity(IP_ADDRESS));
        assert!(!label_bears_severity(PRIVATE_HOSTNAME));
        assert!(!label_bears_severity(OS_USERNAME));
        // The existing exemption and its neighbour are untouched.
        assert!(!label_bears_severity("local_path"));
        assert!(label_bears_severity("private_email"));
    }

    fn no_content_consent() -> ConsentMetadata {
        ConsentMetadata {
            policy_version: TRACE_CONTRIBUTION_POLICY_VERSION.to_string(),
            scopes: vec![ConsentScope::DebuggingEvaluation],
            message_text_included: false,
            tool_payloads_included: false,
            correction_included: false,
            routing_metadata_included: false,
            revocable: true,
        }
    }

    #[test]
    fn machine_identifiers_alone_do_not_raise_the_tier() {
        let (_, report) = redact("ping Janes-MacBook-Pro.local (10.0.4.17) as jdoe@devbox:~$");
        assert!(count(&report, IP_ADDRESS) > 0, "{report:?}");
        assert_eq!(
            residual_risk(&no_content_consent(), &report),
            ResidualPiiRisk::Low
        );
        let (_, report) = redact("Author: Jane Example <jane@example.com>");
        assert!(count(&report, GIT_AUTHOR) > 0, "{report:?}");
        assert_eq!(
            residual_risk(&no_content_consent(), &report),
            ResidualPiiRisk::Medium
        );
        let (_, report) = redact("call +44 20 7946 0958");
        assert!(count(&report, PHONE_NUMBER) > 0, "{report:?}");
        assert_eq!(
            residual_risk(&no_content_consent(), &report),
            ResidualPiiRisk::Medium
        );
    }

    fn tool_heavy_raw() -> RawTraceContribution {
        let started = chrono::Utc::now();
        RawTraceContribution::from_capture_turns(
            &[RawTraceCaptureTurn {
                user_input: "why is the build failing on jdoe@devbox:~$ make".to_string(),
                response: Some(
                    "The last commit changed the host to grafana.internal.".to_string(),
                ),
                tool_calls: vec![RawTraceCaptureToolCall {
                    name: "shell".to_string(),
                    id: Some("call_1".to_string()),
                    arguments: Some(
                        serde_json::json!({"command": "git log -1 && id && ifconfig"}),
                    ),
                    result_preview: Some(MIXED.to_string()),
                    error: None,
                    rationale: None,
                }],
                started_at: started,
                completed_at: Some(started + chrono::Duration::milliseconds(10)),
                state: Some("Completed".to_string()),
            }],
            RecordedTraceContributionOptions {
                include_message_text: true,
                include_tool_payloads: true,
                ..RecordedTraceContributionOptions::default()
            },
        )
    }

    /// The residual scan runs every detector over EVERY string leaf of the
    /// finished envelope, typed fields included. A new label showing up there
    /// would make `residual_clean()` false and block a downgrade for good.
    #[tokio::test]
    async fn a_redacted_envelope_leaves_no_residual_finding_for_the_new_labels() {
        let redactor = redactor();
        let envelope = redactor
            .redact_trace(tool_heavy_raw())
            .await
            .expect("redacts");
        for label in NEW_LABELS {
            assert!(
                envelope
                    .privacy
                    .redaction_counts
                    .get(label)
                    .copied()
                    .unwrap_or(0)
                    > 0,
                "{label} absent from {:?}",
                envelope.privacy.redaction_counts
            );
        }
        let residual = residual_envelope_scan(&redactor, &envelope).expect("scan runs");
        for label in NEW_LABELS {
            assert_eq!(
                residual.counts.get(label),
                None,
                "{label} survived into the finished envelope"
            );
        }
        let serialized = serde_json::to_string(&envelope).unwrap();
        for raw in [
            "jdoe",
            "Jane Example",
            "34.117.59.81",
            "Janes-MacBook-Pro",
            "415 555 0132",
        ] {
            assert!(!serialized.contains(raw), "{raw} reached the envelope");
        }
    }

    /// The server re-runs the same pass over a client-redacted envelope. It
    /// must find nothing new and change no event content.
    #[tokio::test]
    async fn the_server_rescrub_is_idempotent_over_a_client_redacted_envelope() {
        let redactor = redactor();
        let mut envelope = redactor
            .redact_trace(tool_heavy_raw())
            .await
            .expect("redacts");
        let before_events = serde_json::to_value(&envelope.events).unwrap();
        let before_counts = envelope.privacy.redaction_counts.clone();
        rescrub_trace_envelope_with(&redactor, &mut envelope);
        assert_eq!(
            serde_json::to_value(&envelope.events).unwrap(),
            before_events
        );
        for label in NEW_LABELS {
            assert_eq!(
                envelope.privacy.redaction_counts.get(label),
                before_counts.get(label),
                "{label} counted again by the server pass"
            );
        }
    }
}
