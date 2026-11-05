//! Explicit presentation contracts for routing, sources and preview facts.
//! Template probes call canonical renderers with synthetic arguments ONLY;
//! neither user content nor a completed incoming sentence is searched.

use super::{plural, text};
use crate::{first_run_copy, nudge_copy, preview_copy, routing_copy, source_copy, witness_copy};
use serde_json::{Value, json};

const PROBE: u64 = 1_048_573;

fn label(input: &Value, name: &str) -> String {
    input[name].as_str().unwrap_or_default().to_owned()
}

fn variant(value: &str, known: &[&str]) -> String {
    if known.contains(&value) && !value.is_empty() {
        value.replace('-', "_")
    } else {
        "other".into()
    }
}

fn error_variant(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_'))
    {
        value.replace('-', "_")
    } else {
        "other".into()
    }
}

fn fixed(operation: &str, variant: &str, english: &str) -> Value {
    text(
        &format!("message.{operation}.{variant}"),
        english,
        json!({}),
    )
}

fn counted(operation: &str, count: u64, render: impl Fn(u64) -> String) -> Value {
    plural(
        &format!("message.{operation}.count"),
        &render(1).replace('1', "{count}"),
        &render(PROBE).replace(&PROBE.to_string(), "{count}"),
        count,
        json!({}),
    )
}

pub fn message(operation: &str, input: &Value) -> Option<Value> {
    let state = label(input, "state");
    let reason = label(input, "label");
    Some(match operation {
        "channel_mismatch_line" => fixed(operation, "value", crate::health_copy::CHANNEL_MISMATCH),
        "channel_window_label" => fixed(
            operation,
            if crate::health_copy::window_label().is_some() {
                "staging"
            } else {
                "production"
            },
            crate::health_copy::window_label().unwrap_or(""),
        ),
        "state_dir_override_refused_line" => fixed(
            operation,
            "value",
            crate::health_copy::STATE_DIR_OVERRIDE_REFUSED,
        ),
        "routing_tool_word" => {
            let mode = label(input, "source_mode");
            let wiring = match input["wiring"].as_i64() {
                Some(0) => routing_copy::ToolWiring::Wired,
                Some(1) => routing_copy::ToolWiring::NotWired,
                _ => routing_copy::ToolWiring::Unknown,
            };
            let tag = if mode == "off" {
                "off".to_owned()
            } else {
                format!(
                    "wiring_{}",
                    match input["wiring"].as_i64() {
                        Some(0) => 0,
                        Some(1) => 1,
                        _ => 2,
                    }
                )
            };
            fixed(operation, &tag, routing_copy::tool_word(&mode, wiring))
        }
        "routing_state_line" => fixed(
            operation,
            &variant(
                if state.is_empty() {
                    "not_declared"
                } else {
                    &state
                },
                &[
                    "awaiting_rows",
                    "rows_seen",
                    "token_unreadable",
                    "not_declared",
                ],
            ),
            routing_copy::ironwire_state_line(&state),
        ),
        "routing_folder_note" => {
            let folder = crate::daemon::settings::ironwire_default_token_dir();
            match folder.as_deref().and_then(std::path::Path::to_str) {
                Some(folder) => text(
                    "message.routing_folder_note.folder",
                    &routing_copy::ironwire_folder_note(Some("{folder}")),
                    json!({"folder": folder}),
                ),
                None => fixed(
                    operation,
                    "missing",
                    &routing_copy::ironwire_folder_note(None),
                ),
            }
        }
        "routing_token_line" => match input["path"].as_str() {
            Some(path) => text(
                "message.routing_token_line.path",
                &routing_copy::ironwire_token_line(Some("{path}")),
                json!({"path": path}),
            ),
            None => fixed(
                operation,
                "missing",
                &routing_copy::ironwire_token_line(None),
            ),
        },
        "routing_unreachable_line" | "routing_discovery_line" => {
            let render = if operation == "routing_unreachable_line" {
                routing_copy::ironwire_unreachable_line
            } else {
                routing_copy::ironwire_discovery_line
            };
            let port = input["port"]
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .filter(|n| *n > 0);
            match port {
                // A port is a protocol identifier: leave its digits unchanged.
                Some(port) => text(
                    &format!("message.{operation}.port"),
                    &render(Some(u16::MAX)).replace("65535", "{port}"),
                    json!({"port": port.to_string()}),
                ),
                None => fixed(operation, "missing", &render(None)),
            }
        }
        "routing_last_checked" => {
            let when = input["when"].as_str().filter(|v| !v.is_empty())?;
            text(
                "message.routing_last_checked.time",
                &routing_copy::last_checked_line("{when}"),
                json!({"when": when}),
            )
        }
        "source_check_line" => {
            let tool = label(input, "tool");
            let source = source_copy::SourceTool::from_key(&tool)?;
            let mode = label(input, "source_mode");
            fixed(
                operation,
                &format!("{tool}.{}", variant(&mode, &["watch", "off", "unset"])),
                &source_copy::source_check_line(source, &mode),
            )
        }
        "witness_state_line" => {
            let code = i32::try_from(input["state"].as_i64()?).ok()?;
            let state = crate::witness::status::WitnessTrustState::from_abi_code(code)?;
            fixed(
                operation,
                &format!("state_{code}"),
                witness_copy::witness_state_line(state),
            )
        }
        "witness_review_failure_json" => {
            let mut view = input["view"].as_object()?.clone();
            let review = witness_copy::witness_copy().review;
            match view.get("state").and_then(Value::as_str) {
                Some("Busy") => {
                    view.insert(
                        "message".into(),
                        fixed(operation, "busy", review.failed_busy),
                    );
                    view.insert(
                        "retry_label".into(),
                        fixed(operation, "retry_label", review.busy_retry_at),
                    );
                }
                Some("Refused") => {
                    let reason = input["label"].as_str();
                    let known = reason.filter(|label| WITNESS_REFUSALS.contains(label));
                    view.insert(
                        "message".into(),
                        fixed(
                            operation,
                            known.unwrap_or("unknown"),
                            witness_copy::witness_refusal_line(known),
                        ),
                    );
                }
                _ => {}
            }
            Value::Object(view)
        }
        "witness_pinned_count_line" => {
            let count = input["count"].as_u64()?;
            if count == 0 {
                fixed(
                    operation,
                    "zero",
                    &witness_copy::witness_pinned_count_line(0),
                )
            } else {
                counted(operation, count, |n| {
                    witness_copy::witness_pinned_count_line(n as usize)
                })
            }
        }
        "witness_last_result_line" => {
            use crate::witness::status::WitnessLastResult;
            let result = if input.get("outcome").is_some() {
                match input["outcome"].as_str()? {
                    "not_observed" => WitnessLastResult::NotObserved,
                    "local_redaction" => WitnessLastResult::LocalRedaction,
                    "certified" => WitnessLastResult::Certified {
                        n_of_m: match input.get("n_of_m").filter(|v| !v.is_null()) {
                            Some(count) => Some(crate::witness::status::InferenceReceiptCount {
                                n: u32::try_from(count["n"].as_u64()?).ok()?,
                                m: u32::try_from(count["m"].as_u64()?).ok()?,
                            }),
                            None => None,
                        },
                    },
                    "refused" => WitnessLastResult::Refused {
                        label: String::new(),
                        certificate_obtained: input["certificate_obtained"].as_bool()?,
                    },
                    _ => return None,
                }
            } else {
                crate::witness::status::last_result()
            };
            match &result {
                WitnessLastResult::Certified {
                    n_of_m: Some(count),
                } => {
                    let render = |n| {
                        witness_copy::witness_last_result_line(&WitnessLastResult::Certified {
                            n_of_m: Some(crate::witness::status::InferenceReceiptCount {
                                n: PROBE as u32,
                                m: n,
                            }),
                        })
                    };
                    // n and m are independent; only m selects the plural.
                    let one = render(1)
                        .replace(&PROBE.to_string(), "{receipts}")
                        .replace("of 1 model", "of {count} model");
                    let other = render(PROBE as u32 + 1)
                        .replace(&(PROBE + 1).to_string(), "{count}")
                        .replace(&PROBE.to_string(), "{receipts}");
                    plural(
                        "message.witness_last_result_line.receipts",
                        &one,
                        &other,
                        u64::from(count.m),
                        json!({"receipts": count.n}),
                    )
                }
                WitnessLastResult::Certified { .. } => fixed(
                    operation,
                    "certified",
                    &witness_copy::witness_last_result_line(&result),
                ),
                WitnessLastResult::NotObserved => fixed(
                    operation,
                    "not_observed",
                    &witness_copy::witness_last_result_line(&result),
                ),
                WitnessLastResult::LocalRedaction => fixed(
                    operation,
                    "local",
                    &witness_copy::witness_last_result_line(&result),
                ),
                WitnessLastResult::Refused {
                    certificate_obtained,
                    ..
                } => fixed(
                    operation,
                    if *certificate_obtained {
                        "mismatch"
                    } else {
                        "refused"
                    },
                    &witness_copy::witness_last_result_line(&result),
                ),
            }
        }
        "decisions_owed_text" => match input["count"].as_u64() {
            None => fixed(
                operation,
                "unknown",
                &preview_copy::decisions_owed_text(None),
            ),
            Some(0) => fixed(operation, "zero", ""),
            Some(count) => counted(operation, count, |n| {
                preview_copy::decisions_owed_text(Some(n))
            }),
        },
        "first_run_estimate_line" => {
            let line = first_run_copy::traces_estimate_line_json(&input.to_string())?;
            fixed(
                operation,
                if input["earns_credit"].as_bool() == Some(false) {
                    "could"
                } else {
                    "earns"
                },
                &line,
            )
        }
        "nudge_digest_help_json" => {
            let evening = input["digest_schedule"]["mode"].as_str() == Some("evening");
            let interval = input["digest_interval_secs"].as_u64();
            let Some(_) = crate::nudge_render::digest_help(evening, interval) else {
                return Some(json!({}));
            };
            let help = if evening {
                fixed(
                    operation,
                    "evening",
                    nudge_copy::SETTING_DIGEST_HELP_EVENING,
                )
            } else {
                plural(
                    "message.nudge_digest_help_json.hours",
                    nudge_copy::SETTING_DIGEST_HELP_INTERVAL_ONE,
                    &nudge_copy::SETTING_DIGEST_HELP_INTERVAL.replace("{hours}", "{count}"),
                    interval? / 3600,
                    json!({}),
                )
            };
            json!({"digest_help": help})
        }
        "nudge_entry_tags_json" => {
            use crate::nudge_render::{EntryUsdEstimate, entry_tags, usd_text};
            let estimate = input.get("settlement_estimate").and_then(|e| {
                Some(EntryUsdEstimate {
                    low_usd_micros: e["low_usd_micros"].as_u64()?,
                    high_usd_micros: e["high_usd_micros"].as_u64()?,
                    tier: e["tier"].as_str().map(str::to_owned),
                    drawn: e["drawn"].as_bool().unwrap_or(false),
                })
            });
            let tags = entry_tags(input["mission_fit"].as_u64(), estimate.as_ref());
            let mut tree = json!({});
            if let Some(line) = tags.mission_fit {
                tree["mission_fit"] = fixed(operation, "mission_fit", &line);
            }
            if tags.estimate_band.is_some() {
                let e = estimate.as_ref()?;
                let amount = |micros| {
                    let english = usd_text(micros);
                    json!({"currency": english.trim_start_matches('$'), "code":"USD", "english":english})
                };
                tree["estimate_band"] = text(
                    &format!(
                        "message.{operation}.band_{}",
                        if e.low_usd_micros == 0 {
                            "up_to"
                        } else {
                            "range"
                        }
                    ),
                    if e.low_usd_micros == 0 {
                        nudge_copy::ENTRY_ESTIMATE_BAND_UP_TO
                    } else {
                        nudge_copy::ENTRY_ESTIMATE_BAND
                    },
                    if e.low_usd_micros == 0 {
                        json!({"high":amount(e.high_usd_micros)})
                    } else {
                        json!({"low":amount(e.low_usd_micros),"high":amount(e.high_usd_micros)})
                    },
                );
            }
            if let Some(line) = tags.estimate_tier {
                tree["estimate_tier"] = fixed(
                    operation,
                    &format!("tier_{}", estimate.as_ref()?.tier.as_deref()?),
                    &line,
                );
            }
            if let Some(line) = tags.estimate_explainer {
                tree["estimate_explainer"] = fixed(operation, "explainer", &line);
            }
            tree
        }
        "second_look_line_text" => fixed(
            operation,
            &error_variant(&reason),
            preview_copy::second_look_line_or_fallback(&reason),
        ),
        "session_detail_error_line" => fixed(
            operation,
            &error_variant(&reason),
            crate::public_run::session_detail_error_line(&reason),
        ),
        "public_run_error_line" => fixed(
            operation,
            &error_variant(&reason),
            crate::public_run::publication_error_line(&reason),
        ),
        "skill_learning_error_line" => fixed(
            operation,
            &error_variant(&reason),
            crate::skill_loop::skill_learning_error_line(&reason),
        ),
        _ => return None,
    })
}

pub fn templates() -> Vec<Value> {
    let mut messages = Vec::new();
    let mut add = |operation, input| {
        if let Some(value) = message(operation, &input) {
            messages.push(value);
        }
    };
    for operation in [
        "channel_mismatch_line",
        "channel_window_label",
        "state_dir_override_refused_line",
    ] {
        add(operation, json!({}));
    }
    for mode in ["off", "watch"] {
        for wiring in 0..=2 {
            add(
                "routing_tool_word",
                json!({"source_mode":mode,"wiring":wiring}),
            );
        }
    }
    for state in [
        "",
        "awaiting_rows",
        "rows_seen",
        "token_unreadable",
        "not_declared",
        "future",
    ] {
        add("routing_state_line", json!({"state":state}));
    }
    messages.push(text(
        "message.routing_folder_note.folder",
        &routing_copy::ironwire_folder_note(Some("{folder}")),
        json!({"folder": "synthetic"}),
    ));
    messages.push(fixed(
        "routing_folder_note",
        "missing",
        &routing_copy::ironwire_folder_note(None),
    ));
    let mut add = |operation, input| {
        if let Some(value) = message(operation, &input) {
            messages.push(value);
        }
    };
    for path in [Value::Null, json!("/synthetic/{count}")] {
        add("routing_token_line", json!({"path":path}));
    }
    for operation in ["routing_unreachable_line", "routing_discovery_line"] {
        for port in [0, 8463] {
            add(operation, json!({"port":port}));
        }
    }
    add("routing_last_checked", json!({"when":"synthetic"}));
    for tool in ["claude", "codex", "gemini", "cline", "opencode"] {
        for mode in ["watch", "off", "unset", "future"] {
            add("source_check_line", json!({"tool":tool,"source_mode":mode}));
        }
    }
    for state in -2..=4 {
        add("witness_state_line", json!({"state":state}));
    }
    add(
        "witness_review_failure_json",
        json!({"view":{"state":"Busy"}}),
    );
    for label in WITNESS_REFUSALS.iter().copied().chain(["unknown"]) {
        add(
            "witness_review_failure_json",
            json!({"label":label,"view":{"state":"Refused"}}),
        );
    }
    for outcome in ["not_observed", "local_redaction", "certified"] {
        add("witness_last_result_line", json!({"outcome":outcome}));
    }
    add(
        "witness_last_result_line",
        json!({"outcome":"certified","n_of_m":{"n":3,"m":7}}),
    );
    for certificate_obtained in [false, true] {
        add(
            "witness_last_result_line",
            json!({"outcome":"refused","certificate_obtained":certificate_obtained}),
        );
    }
    for count in [Value::Null, json!(0), json!(2)] {
        add("decisions_owed_text", json!({"count":count}));
    }
    for count in [0, 2] {
        add("witness_pinned_count_line", json!({"count":count}));
    }
    for earns_credit in [false, true] {
        add(
            "first_run_estimate_line",
            json!({"traces":2,"earns_credit":earns_credit}),
        );
    }
    for mode in ["evening", "interval"] {
        add(
            "nudge_digest_help_json",
            json!({"digest_schedule":{"mode":mode},"digest_interval_secs":7200}),
        );
    }
    for low in [0, 1_500_000] {
        for tier in ["lower", "middle", "higher"] {
            add(
                "nudge_entry_tags_json",
                json!({"mission_fit":2,"settlement_estimate":{"low_usd_micros":low,"high_usd_micros":2_000_000,"tier":tier,"drawn":true}}),
            );
        }
    }
    for operation in [
        "second_look_line_text",
        "session_detail_error_line",
        "public_run_error_line",
        "skill_learning_error_line",
    ] {
        for label in [
            "",
            "nothing-matched",
            "looks-unsure",
            "trimmed-to-fit",
            "account-session-required",
            "session-detail-not-found",
            "public-run-conflict",
            "public-run-trace-not-found",
            "public-run-source-not-found",
            "public-run-invalid",
            "commons_credential_storage_unavailable",
            "skill-correction-required",
            "skill-session-not-accepted",
            "skill-source-lineage-invalid",
            "skill-workflow-capacity",
            "skill-candidate-unknown",
            "skill-review-unknown",
            "skill-evaluation-unknown",
            "skill-family-not-supported",
            "skill-name-invalid",
            "skill-description-required",
            "skill-procedure-required",
            "skill-description-too-long",
            "skill-procedure-too-long",
            "skill-control-character",
            "skill-sensitive-text",
            "skill-evaluation-credential-required",
            "skill-evaluation-private-model-unavailable",
            "skill-evaluation-provider-rejected",
            "skill-evaluation-funding-required",
            "skill-evaluation-model-changed",
            "skill-evaluation-in-progress",
            "skill-evaluation-busy",
            "skill-evaluation-credential-changed",
            "skill-evaluation-did-not-pass",
            "skill-install-occupied",
            "skill-install-plan-changed",
            "skill-review-changed",
            "skill-install-plan-unknown",
            "skill-install-modified",
            "skill-install-extra-files",
            "skill-install-multiple-matches",
            "skill-install-scan-limit",
            "skill-install-not-owned",
            "skill-install-unknown",
            "skill-home-unavailable",
            "skill-install-root-invalid",
            "skill-install-write-failed",
            "skill-install-rollback-incomplete",
        ] {
            add(operation, json!({"label":label}));
        }
    }
    messages
}

const WITNESS_REFUSALS: &[&str] = &[
    "witness_host_not_allowed",
    crate::witness::WITNESS_EXPECTED_MEASUREMENT_CONTROL,
    "witness_attestation_unavailable",
    "witness_collateral_unavailable",
    "witness_response_malformed",
    "witness_timeout",
    "witness_unreachable",
    "witness_quote_unverified",
    "witness_quote_replayed",
    "witness_signer_unexpected",
    "witness_certificate_mismatched",
    "witness_certificate_unverified",
    "witness_body_not_stripped",
    "witness_payload_too_large",
    "witness_claim_unavailable",
    "admission_evidence_refused",
    trace_commons_protocol::witness_pacing::WITNESS_SATURATED_ERROR,
];
