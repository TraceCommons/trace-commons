//! Handle-free Insights bridge. No daemon or contributor account is created.
use super::*;
use trace_commons_contributor::insights::service::{MAX_REQUEST_BYTES, dispatch_json};

/// Return the complete fixed Insights UI vocabulary without opening a store.
/// The returned JSON string is owned and must be freed with `tc_string_free`.
#[unsafe(no_mangle)]
pub extern "C" fn tc_insights_copy_json() -> *mut c_char {
    guarded_string_no_err(|| {
        let json = serde_json::to_string(&trace_commons_contributor::insights::service::ui_copy())
            .unwrap_or_else(|_| "{}".to_string());
        Ok(to_owned_cstring(&json))
    })
}

/// Execute a local Insights request without starting a daemon or enrollment.
/// Returns owned JSON on success, NULL and an owned fixed error label on failure.
/// Free either owned string with `tc_string_free`. Clears `*err` on success.
///
/// The request is UTF-8 JSON, at most 65536 bytes, without a trailing NUL:
/// `{"store_dir":"/chosen/store","operation":{"type":"list"}}`.
/// Analyze: `{"type":"analyze","source":"codex|claude_code|trajectory","file":"/chosen/file","save":false}`.
/// Explain/delete: `{"type":"explain","id":"..."}` / `{"type":"delete","id":"..."}`.
/// User assessment: `{"type":"annotate","id":"...","category":"docs","outcome":"partial"}`.
/// Clear assessment: `{"type":"clear_annotation","id":"..."}`.
/// Native usage: `{"type":"usage","source":"claude_code","file":"/chosen/file"}`.
/// Saved-history summary: `{"type":"summary"}`. Reads derived observations only.
/// Shared UI vocabulary: `{"type":"copy"}`. List/summary create no absent store.
/// Deterministic question cards: `{"type":"question_cards","questions":[
/// "recorded_activity","episode_outcomes","observed_models","estimated_cost"],
/// "snapshot_ids":[],"episode_ids":[]}`. This read returns typed `result`
/// cards plus shared rendered `text`; empty selections create no absent store.
/// Token week over saved snapshots (feed S): `{"type":"week_overview",
/// "week_start":"2026-10-05","tz":3600}` and `{"type":"card_inputs","card":
/// "tokens|cache_share|sessions","week_start":"2026-10-05","tz":3600}`.
/// `week_start` is any date in the local ISO week (absent: the current week);
/// `tz` is the UTC offset in seconds east, refused as `insights_tz_invalid`
/// beyond 18 hours. Sessions are dated by their own records, never the import
/// time; nothing is compared with another week. Reads create no absent store.
/// Patterns over saved snapshots (feed S): `{"type":"patterns","week_start":
/// "2026-10-05","weeks":6,"tz":3600}` returns the four cards (token figure,
/// count, weekly bars where an absent week is `null`, never 0) and the
/// re-read table as letters and extensions; `weeks` is 1 to 6 (default 6),
/// refused as `insights_weeks_invalid`. `{"type":"pattern_sessions","pattern":
/// "repeated_reads|retried_calls|edit_fail_edit|long_context","week_start":
/// "2026-10-05","tz":3600}` lists the sessions behind one card. Neither
/// compares weeks under feed S, and neither returns a path or a digest.
/// Whole-snapshot episodes: `{"type":"episode_create","snapshot_ids":["..."]}`;
/// `episode_list`, and `episode_explain` with an episode UUID `id`.
/// Edits require `id` and `expected_revision`: `episode_replace_members` also
/// takes `snapshot_ids`; `episode_annotate` takes `category` and `outcome`;
/// `episode_clear_assessment` and `episode_delete` take no other fields.
/// Episode reads resolve saved observations without rereading source files.
/// Absent episode history creates no store. Revisions protect concurrent edits;
/// `insights_episode_revision_conflict` requires refresh and user review.
/// Analyze/delete return additive `mutation_effects.invalidated_episode_ids`;
/// those groups and assessments were atomically removed with a member snapshot.
/// Successful response JSON is capped at 16 MiB. Oversized responses return
/// `insights_response_too_large`; no truncated evidence is returned.
/// Only fixed typed episode errors cross this ABI. Unexpected execution errors
/// remain `insights-operation-failed`, without paths or parser/source details.
/// Omit store_dir to use the shared platform local-data Insights directory.
/// Runs synchronous bounded-source local IO: call off the UI thread; closing a
/// window does not cancel a started operation. Keep buffers alive until return.
///
/// # Safety
/// A non-null `request` must point to `request_len` readable bytes for the call.
/// `err`, if non-null, must point to writable pointer storage, with no unfreed
/// prior owned string. NULL requests and oversized lengths are refused before
/// any request memory is read.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn tc_insights_call(
    request: *const u8,
    request_len: usize,
    err: *mut *mut c_char,
) -> *mut c_char {
    guarded_string(err, || {
        if !err.is_null() {
            unsafe { *err = std::ptr::null_mut() };
        }
        // Every fallible operation inside this forwarding guard emits only a
        // fixed label. Source paths and parser details never cross the ABI.
        let result = guard_forwarding(|| {
            if request_len > MAX_REQUEST_BYTES {
                anyhow::bail!("insights-request-too-large");
            }
            if request.is_null() {
                anyhow::bail!("insights-request-null");
            }
            let bytes = unsafe { std::slice::from_raw_parts(request, request_len) };
            dispatch_json(bytes)
        });
        match result {
            Ok(json) => Ok(to_owned_cstring(&json)),
            Err(label) => {
                set_last_error(&label);
                if !err.is_null() {
                    unsafe { *err = to_owned_cstring(&label) };
                }
                Ok(std::ptr::null_mut())
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use trace_commons_contributor::insights::service::LocalInsightsResponse;

    #[test]
    fn stateless_copy_includes_store_routing_words() {
        let result = tc_insights_copy_json();
        assert!(!result.is_null());
        let copy: serde_json::Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        assert_eq!(copy["insights_store_title"], "Insights store");
        assert_eq!(
            copy["insights_store_unavailable"],
            "Insights store unavailable"
        );
        assert!(
            copy["insights_store_missing_path"]
                .as_str()
                .unwrap()
                .contains("absolute")
        );
        unsafe { tc_string_free(result) };
    }

    #[test]
    fn qualified_schema_two_result_crosses_the_ffi_dependency_boundary() {
        let response: LocalInsightsResponse = serde_json::from_slice(include_bytes!(
            "../../trace-commons-contributor/fixtures/insights/comparison-estimator/schema2-qualified/preview-response.json"
        ))
        .unwrap();
        let LocalInsightsResponse::ComparisonPreviewSpec {
            specification,
            result,
        } = response
        else {
            panic!("fixture must be a comparison preview response")
        };
        specification.validate().unwrap();
        result.validate_for_specification(&specification).unwrap();
        assert_eq!(result.schema_version, 2);
        assert!(result.exact_estimation.is_some());
    }

    /// A saved specification claiming the exact-categorical estimator can only
    /// come from a hand-edited store: `comparison_save_spec` always writes
    /// `NotYetCalibrated`. The frozen protocol sets
    /// `qualified_for_saved_specifications: false`, so the store refuses the
    /// edit by name instead of letting a shell render a simultaneous-coverage
    /// claim behind it.
    #[test]
    fn hand_granted_qualified_specification_is_refused_across_the_ffi() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        let saved = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_save_spec",
                "input":{
                    "evidence_cutoff":"2026-09-11T12:00:00Z",
                    "cohort_labels":["model-a","model-b"],
                    "date_start":"2026-09-01", "date_end":"2026-09-10",
                    "stratum":{
                        "project_id":"00000000-0000-4000-8000-000000000001",
                        "language":"rust", "configuration_fingerprint":"1".repeat(64)
                    }
                }
            }),
        )
        .unwrap();
        let old_id = saved["specification"]["id"].as_str().unwrap();
        assert_eq!(
            saved["specification"]["estimator_state"],
            serde_json::json!("not_yet_calibrated"),
            "production must never save a qualified estimator state"
        );
        let fixture: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../trace-commons-contributor/fixtures/insights/comparison-estimator/schema2-qualified/preview-response.json"
        ))
        .unwrap();
        let qualified = fixture["specification"].clone();
        let qualified_id = qualified["id"].as_str().unwrap().to_owned();
        let path = store.join("index.json");
        let mut index: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        index["comparison_specifications"]
            .as_object_mut()
            .unwrap()
            .remove(old_id);
        index["comparison_specifications"][&qualified_id] = qualified;
        std::fs::write(&path, serde_json::to_vec(&index).unwrap()).unwrap();

        assert_eq!(
            json_call(
                &store,
                serde_json::json!({"type":"comparison_evaluate","id":qualified_id}),
            )
            .unwrap_err(),
            "insights-comparison-estimator-not-qualified"
        );
        // The refusal is a property of the store, not of one operation: a
        // plain list of the same store must not succeed either.
        assert_eq!(
            json_call(&store, serde_json::json!({"type":"comparison_list_specs"})).unwrap_err(),
            "insights-comparison-estimator-not-qualified"
        );
    }

    /// The captured schema-10 store is a record of what the evaluator produces,
    /// not a grant. Reading it through the ABI must hit the same refusal.
    #[test]
    fn captured_qualified_store_is_refused_across_the_ffi() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        std::fs::create_dir(&store).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&store, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        std::fs::write(
            store.join("index.json"),
            include_bytes!(
                "../../trace-commons-contributor/fixtures/insights/comparison-estimator/schema2-supported-store/index.json"
            ),
        )
        .unwrap();
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({
                    "type":"comparison_evaluate",
                    "id":"0e86d56a-8117-4daa-b18f-8f4f8210c457"
                }),
            )
            .unwrap_err(),
            "insights-comparison-estimator-not-qualified"
        );
    }

    fn failure(bytes: *const u8, len: usize, expected: &str) {
        let mut error = std::ptr::null_mut();
        let response = unsafe { tc_insights_call(bytes, len, &mut error) };
        assert!(response.is_null());
        assert!(!error.is_null());
        assert_eq!(unsafe { CStr::from_ptr(error) }.to_str().unwrap(), expected);
        unsafe { tc_string_free(error) };
    }

    #[test]
    fn invalid_requests_are_bounded_and_owned_errors_are_freeable() {
        failure(std::ptr::null(), 0, "insights-request-null");
        // Even NULL cannot be dereferenced when the supplied bound is invalid.
        failure(
            std::ptr::null(),
            MAX_REQUEST_BYTES + 1,
            "insights-request-too-large",
        );
        failure([0xff].as_ptr(), 1, "insights-request-invalid-utf8");
        let malformed = br#"{"secret":"do-not-echo"}"#;
        failure(
            malformed.as_ptr(),
            malformed.len(),
            "insights-request-invalid",
        );
        assert!(unsafe { tc_insights_call(std::ptr::null(), 0, std::ptr::null_mut()) }.is_null());
    }

    #[test]
    fn account_free_list_returns_owned_json() {
        let temp = tempfile::tempdir().unwrap();
        let bytes = serde_json::to_vec(&serde_json::json!({"store_dir":temp.path().join("insights"),"operation":{"type":"list"}})).unwrap();
        let mut error = std::ptr::null_mut();
        let result = unsafe { tc_insights_call(bytes.as_ptr(), bytes.len(), &mut error) };
        assert!(error.is_null());
        assert!(!result.is_null());
        let value: serde_json::Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        assert_eq!(value, serde_json::json!({"type":"list","insights":[]}));
        unsafe { tc_string_free(result) };
        assert_eq!(std::fs::read_dir(temp.path()).unwrap().count(), 0);
    }

    fn json_call(
        store: &std::path::Path,
        operation: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        let request =
            serde_json::to_vec(&serde_json::json!({"store_dir":store,"operation":operation}))
                .unwrap();
        let mut error = std::ptr::null_mut();
        let result = unsafe { tc_insights_call(request.as_ptr(), request.len(), &mut error) };
        if result.is_null() {
            assert!(!error.is_null());
            let message = unsafe { CStr::from_ptr(error) }
                .to_str()
                .unwrap()
                .to_owned();
            unsafe { tc_string_free(error) };
            return Err(message);
        }
        assert!(error.is_null());
        let value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        unsafe { tc_string_free(result) };
        Ok(value)
    }

    /// A Codex rollout, which needs no digest key: a saved Claude snapshot
    /// would reach the OS keychain from this test binary.
    fn codex_rollout(path: &std::path::Path, final_input: u64) {
        let token = |time: &str, input: u64, cached: u64, output: u64| {
            serde_json::json!({
                "type":"event_msg", "timestamp":time,
                "payload":{"type":"token_count","info":{"total_token_usage":{
                    "input_tokens":input,"cached_input_tokens":cached,"output_tokens":output,
                    "reasoning_output_tokens":0,"total_tokens":input + output
                }}}
            })
        };
        let rows = [
            serde_json::json!({"type":"session_meta","timestamp":"2026-09-11T00:00:00Z","payload":{"id":"PRIVATE_SESSION_ID","model_provider":"openai"}}),
            serde_json::json!({"type":"turn_context","timestamp":"2026-09-11T00:00:00Z","payload":{"model":"fixture-model"}}),
            token("2026-09-11T00:00:01Z", 100, 20, 20),
            token("2026-09-11T00:00:03Z", final_input, 40, 30),
        ];
        std::fs::write(
            path,
            rows.iter()
                .map(|row| row.to_string())
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
    }

    #[test]
    fn week_overview_and_card_inputs_cross_the_abi_and_follow_mutations() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        let week = || serde_json::json!({"type":"week_overview","week_start":"2026-09-09","tz":0});
        let empty = json_call(&store, week()).unwrap();
        assert_eq!(empty["type"], "week_overview");
        assert_eq!(empty["overview"]["sessions"], 0);
        assert!(!store.exists());

        let source = temp.path().join("rollout.jsonl");
        codex_rollout(&source, 150);
        let analyze = serde_json::json!({
            "type":"analyze","source":"codex","file":source,"save":true
        });
        let saved = json_call(&store, analyze.clone()).unwrap();
        let first = json_call(&store, week()).unwrap();
        let overview = &first["overview"];
        assert_eq!(overview["feed"], "saved");
        assert_eq!(overview["week_start"], "2026-09-07");
        assert_eq!(overview["sources"][0]["tokens"], 60);
        assert_eq!(overview["sources"][0]["cache_share"]["permille"], 400);
        assert_eq!(overview["sources"][0]["change"], "needs_counter_pass");
        assert_eq!(overview["sources"][0]["best_week"], "needs_counter_pass");
        assert_eq!(overview["by_project"], "not_available_for_analyzed_files");
        assert_eq!(overview["weeks"], serde_json::json!(["2026-09-07"]));
        assert!(!first.to_string().contains("PRIVATE_SESSION_ID"));

        let inputs = json_call(
            &store,
            serde_json::json!({"type":"card_inputs","card":"sessions","week_start":"2026-09-09","tz":0}),
        )
        .unwrap();
        assert_eq!(inputs["type"], "card_inputs");
        assert_eq!(
            inputs["inputs"]["sessions"][0]["session_ref"],
            saved["insight"]["id"]
        );

        // Replace: new bytes at the same path.
        codex_rollout(&source, 250);
        json_call(&store, analyze).unwrap();
        let replaced = json_call(&store, week()).unwrap();
        assert_ne!(
            replaced["overview"]["generation"],
            first["overview"]["generation"]
        );
        assert_eq!(replaced["overview"]["sources"][0]["tokens"], 160);
        assert_eq!(replaced["overview"]["sessions"], 1);

        // Delete.
        let id =
            json_call(&store, serde_json::json!({"type":"list"})).unwrap()["insights"][0]["id"]
                .clone();
        json_call(&store, serde_json::json!({"type":"delete","id":id})).unwrap();
        let deleted = json_call(&store, week()).unwrap();
        assert_eq!(deleted["overview"]["sessions"], 0);
        assert_eq!(deleted["overview"]["sources"], serde_json::json!([]));

        assert_eq!(
            json_call(
                &store,
                serde_json::json!({"type":"week_overview","tz":-65_000})
            )
            .unwrap_err(),
            "insights_tz_invalid"
        );
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({"type":"card_inputs","card":"spend","tz":0})
            )
            .unwrap_err(),
            "insights-request-invalid"
        );
    }

    #[test]
    fn patterns_cross_the_abi_typed_and_codex_only_weeks_are_unknown() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        let patterns = || serde_json::json!({"type":"patterns","week_start":"2026-09-09","tz":0});
        let empty = json_call(&store, patterns()).unwrap();
        assert_eq!(empty["type"], "patterns");
        assert_eq!(empty["patterns"]["cards"].as_array().unwrap().len(), 4);
        assert!(!store.exists());

        let source = temp.path().join("rollout.jsonl");
        codex_rollout(&source, 150);
        json_call(
            &store,
            serde_json::json!({"type":"analyze","source":"codex","file":source,"save":true}),
        )
        .unwrap();
        let read = json_call(&store, patterns()).unwrap();
        let read = &read["patterns"];
        assert_eq!(read["sessions"], 1);
        assert_eq!(read["claude_sessions"], 0);
        assert_eq!(read["claude_only"], true);
        // Codex records no tool calls: unknown, never zero.
        for card in read["cards"].as_array().unwrap() {
            assert!(card["tokens"].is_null(), "{card}");
            assert_eq!(card["change_unavailable"], "needs_counter_pass");
        }
        assert!(!read.to_string().contains("PRIVATE_SESSION_ID"));

        let sessions = json_call(
            &store,
            serde_json::json!({"type":"pattern_sessions","pattern":"repeated_reads","week_start":"2026-09-09","tz":0}),
        )
        .unwrap();
        assert_eq!(
            sessions["pattern_sessions"]["sessions"],
            serde_json::json!([])
        );
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({"type":"patterns","weeks":9,"tz":0})
            )
            .unwrap_err(),
            "insights_weeks_invalid"
        );
    }

    #[test]
    fn the_analytics_words_cross_the_abi_marked_by_key() {
        let result = tc_insights_copy_json();
        let copy: serde_json::Value =
            serde_json::from_str(unsafe { CStr::from_ptr(result) }.to_str().unwrap()).unwrap();
        unsafe { tc_string_free(result) };
        assert_eq!(copy["analytics_tab_analyze"], "Analyze");
        assert_eq!(copy["analytics_later"], "Later");
        assert_eq!(
            copy["analytics_feed_saved"],
            "Only sessions you analyzed are counted."
        );
    }

    #[test]
    fn episode_abi_preserves_fixed_errors_no_state_and_revision_conflicts() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        assert_eq!(
            json_call(&store, serde_json::json!({"type":"episode_list"})).unwrap()["episodes"],
            serde_json::json!([])
        );
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({"type":"episode_explain","id":"PRIVATE_UUID"})
            )
            .unwrap_err(),
            "insights_episode_invalid"
        );
        assert_eq!(json_call(&store, serde_json::json!({"type":"episode_explain","id":"d0c18c96-6093-49f5-bb6f-6092ef0630b9"})).unwrap_err(), "insights_episode_not_found");
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({"type":"episode_create","snapshot_ids":["a".repeat(64)]})
            )
            .unwrap_err(),
            "insights_episode_missing_members"
        );
        assert!(!store.exists());
        let source = temp.path().join("source.jsonl");
        std::fs::write(&source, b"{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00Z\",\"content\":\"PRIVATE_BODY\"}\n").unwrap();
        let saved = json_call(
            &store,
            serde_json::json!({"type":"analyze","source":"trajectory","file":source,"save":true}),
        )
        .unwrap();
        let snapshot = saved["insight"]["id"].as_str().unwrap();
        let created = json_call(
            &store,
            serde_json::json!({"type":"episode_create","snapshot_ids":[snapshot]}),
        )
        .unwrap();
        let id = created["episode"]["id"].as_str().unwrap();
        let updated = json_call(&store, serde_json::json!({"type":"episode_annotate","id":id,"expected_revision":1,"category":"docs","outcome":"unknown"})).unwrap();
        assert_eq!(updated["episode"]["revision"], 2);
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({"type":"episode_delete","id":id,"expected_revision":1})
            )
            .unwrap_err(),
            "insights_episode_revision_conflict"
        );
        let deleted = json_call(
            &store,
            serde_json::json!({"type":"episode_delete","id":id,"expected_revision":2}),
        )
        .unwrap();
        assert_eq!(deleted["episode"]["id"], id);
        assert!(json_call(&store, serde_json::json!({"type":"explain","id":snapshot})).is_ok());
        std::fs::write(store.join("index.json"), b"PRIVATE_PARSER_CONTENT").unwrap();
        assert_eq!(
            json_call(&store, serde_json::json!({"type":"episode_list"})).unwrap_err(),
            "insights_store_invalid"
        );
    }

    #[test]
    fn claude_code_source_round_trips_through_the_native_json_abi() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        let source = temp.path().join("claude.jsonl");
        std::fs::write(
            &source,
            b"{\"type\":\"user\",\"message\":{\"content\":\"synthetic request\"}}\n",
        )
        .unwrap();

        let analyzed = json_call(
            &store,
            serde_json::json!({
                "type":"analyze", "source":"claude_code", "file":source, "save":true
            }),
        )
        .unwrap();
        assert_eq!(analyzed["insight"]["source_format"], "claude_code");
        assert_eq!(
            analyzed["insight"]["model_observations"]["schema_version"],
            3
        );
        assert_eq!(
            analyzed["insight"]["model_observations"]["source_format"],
            "claude_code"
        );
        // Claude usage evidence is saved, and unknown here: the source has
        // no assistant record. It is never a zero.
        assert_eq!(
            analyzed["insight"]["usage_evidence"]["source"],
            "claude_code"
        );
        assert_eq!(
            analyzed["insight"]["usage_evidence"]["aggregate_unavailable_reason"],
            "no_usage"
        );
        assert!(analyzed["insight"]["usage_evidence"]["aggregate_counts"].is_null());
        assert!(analyzed["insight"]["task_attribution"].is_null());
        let listed = json_call(&store, serde_json::json!({"type":"list"})).unwrap();
        assert_eq!(listed["insights"][0]["source_format"], "claude_code");
    }

    #[test]
    fn comparison_specification_abi_preserves_preview_and_detects_stale_results() {
        let dir = tempfile::tempdir().unwrap();
        let store = dir.path().join("insights");
        assert_eq!(
            json_call(&store, serde_json::json!({"type":"comparison_list_specs"})).unwrap()["specifications"],
            serde_json::json!([])
        );
        assert!(!store.exists());
        let episode = comparison_episode(&store, &dir.path().join("fixture.jsonl"));
        let task = json_call(
            &store,
            serde_json::json!({"type":"comparison_task_create","episode_ids":[episode]}),
        )
        .unwrap()["task"]
            .clone();
        let input = serde_json::json!({
            "evidence_cutoff":task["updated_at"],
            "cohort_labels":["fixture","second-fixture"],
            "date_start":"2026-09-01", "date_end":"2026-09-30",
            "stratum":{
                "project_id":"20c18c96-6093-49f5-bb6f-6092ef0630b9",
                "language":"rust", "configuration_fingerprint":"a".repeat(64)
            }
        });
        let before = std::fs::read(store.join("index.json")).unwrap();
        let preview = json_call(
            &store,
            serde_json::json!({"type":"comparison_preview_spec","input":input}),
        )
        .unwrap();
        assert_eq!(std::fs::read(store.join("index.json")).unwrap(), before);
        assert_eq!(
            preview["result"]["included_task_ids"],
            serde_json::json!([])
        );
        let specification = json_call(
            &store,
            serde_json::json!({"type":"comparison_save_spec","input":input}),
        )
        .unwrap()["specification"]
            .clone();
        assert_eq!(
            specification["provenance"],
            "retrospective_user_specification"
        );
        let id = &specification["id"];
        let result = json_call(
            &store,
            serde_json::json!({"type":"comparison_evaluate","id":id}),
        )
        .unwrap()["result"]
            .clone();
        assert_eq!(result["excluded_tasks"][0]["task_id"], task["id"]);
        assert!(
            result["excluded_tasks"][0]["reasons"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("source_attribution_unavailable"))
        );
        assert!(!result.to_string().contains("PRIVATE_COMPARISON_BODY"));
        let explanation = serde_json::json!({"type":"comparison_explain_result","specification_id":id,"audit_digest":result["audit_digest"]});
        assert_eq!(
            json_call(&store, explanation.clone()).unwrap()["result"],
            result
        );
        json_call(&store, serde_json::json!({"type":"comparison_task_delete","id":task["id"],"expected_revision":1})).unwrap();
        assert_eq!(
            json_call(&store, explanation).unwrap_err(),
            "insights-comparison-result-stale"
        );
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({"type":"comparison_get_spec","id":id})
            )
            .unwrap()["specification"],
            specification
        );
    }

    fn comparison_episode(store: &std::path::Path, source: &std::path::Path) -> String {
        std::fs::write(source, b"{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture\"}\n{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00Z\",\"content\":\"PRIVATE_COMPARISON_BODY\"}\n").unwrap();
        let saved = json_call(
            store,
            serde_json::json!({
                "type":"analyze", "source":"trajectory", "file":source, "save":true
            }),
        )
        .unwrap();
        let episode = json_call(
            store,
            serde_json::json!({
                "type":"episode_create", "snapshot_ids":[saved["insight"]["id"]]
            }),
        )
        .unwrap();
        episode["episode"]["id"].as_str().unwrap().to_owned()
    }

    fn comparison_context() -> serde_json::Value {
        serde_json::json!({
            "project_id":"20c18c96-6093-49f5-bb6f-6092ef0630b9",
            "category":"refactor", "task_date":"2026-09-11",
            "checkout_provenance":{"state":"unavailable"},
            "language":{"state":"known","value":"rust"},
            "configuration":{
                "harness_id":{"state":"known","value":"fixture"},
                "harness_version":{"state":"known","value":"1"},
                "reasoning_effort":"none",
                "tool_policy_id":{"state":"known","value":"read-only"},
                "tool_policy_version":{"state":"known","value":"1"},
                "prompt_template_digest":{"state":"known","digest":"a".repeat(64)}
            }
        })
    }

    #[test]
    fn comparison_task_abi_preserves_review_across_outcomes_and_invalidates_changed_context() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        let list = json_call(&store, serde_json::json!({"type":"comparison_task_list"})).unwrap();
        assert_eq!(list["tasks"], serde_json::json!([]));
        assert!(!store.exists());
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({
                    "type":"comparison_task_explain", "id":"20c18c96-6093-49f5-bb6f-6092ef0630b9"
                })
            )
            .unwrap_err(),
            "insights_comparison_task_not_found"
        );
        assert!(
            !store.exists(),
            "explaining a missing task must not create storage"
        );
        let episode = comparison_episode(&store, &temp.path().join("source.jsonl"));
        let created = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_create", "episode_ids":[episode]
            }),
        )
        .unwrap();
        let id = created["task"]["id"].as_str().unwrap();
        let context = comparison_context();
        let contextualized = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_set_context", "id":id, "expected_revision":1,
                "context":context
            }),
        )
        .unwrap();
        let material = &contextualized["task"]["material_digest"];
        let confirmed = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_reconfirm", "id":id, "expected_revision":2,
                "displayed_material_digest":material
            }),
        )
        .unwrap();
        let pending = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_set_outcome", "id":id, "expected_revision":3,
                "outcome":"pending"
            }),
        )
        .unwrap();
        assert_eq!(pending["task"]["outcome"]["value"], "pending");
        let accepted = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_set_outcome", "id":id, "expected_revision":4,
                "outcome":"accepted"
            }),
        )
        .unwrap();
        assert_eq!(accepted["task"]["material_digest"], *material);
        assert_eq!(
            accepted["task"]["independence_confirmation"],
            confirmed["task"]["independence_confirmation"]
        );
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({
                    "type":"comparison_task_delete", "id":id, "expected_revision":4
                })
            )
            .unwrap_err(),
            "insights_comparison_task_revision_conflict"
        );
        let mut changed_context = context;
        changed_context["task_date"] = serde_json::json!("2026-09-12");
        let changed = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_set_context", "id":id, "expected_revision":5,
                "context":changed_context
            }),
        )
        .unwrap();
        assert_ne!(changed["task"]["material_digest"], *material);
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({
                    "type":"comparison_task_reconfirm", "id":id, "expected_revision":6,
                    "displayed_material_digest":material
                })
            )
            .unwrap_err(),
            "insights_comparison_task_material_digest_conflict"
        );
        let detail = json_call(
            &store,
            serde_json::json!({"type":"comparison_task_explain","id":id}),
        )
        .unwrap();
        let reasons = detail["detail"]["stale_reasons"].as_array().unwrap();
        assert!(reasons.contains(&serde_json::json!("outcome_material_changed")));
        assert!(reasons.contains(&serde_json::json!("independence_material_changed")));
        assert!(reasons.contains(&serde_json::json!("attribution_pending_qualification")));
        assert!(!detail.to_string().contains("PRIVATE_COMPARISON_BODY"));
        json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_delete", "id":id, "expected_revision":6
            }),
        )
        .unwrap();
        assert_eq!(
            json_call(&store, serde_json::json!({"type":"comparison_task_list"})).unwrap()["tasks"],
            serde_json::json!([])
        );
        assert!(
            json_call(
                &store,
                serde_json::json!({"type":"episode_explain","id":episode})
            )
            .is_ok()
        );
    }

    #[test]
    fn comparison_task_abi_retains_missing_evidence_and_rejects_stale_reconfirmation() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        let episode = comparison_episode(&store, &temp.path().join("source.jsonl"));
        let created = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_create", "episode_ids":[episode]
            }),
        )
        .unwrap();
        let id = created["task"]["id"].as_str().unwrap();
        json_call(
            &store,
            serde_json::json!({"type":"episode_delete","id":episode,"expected_revision":1}),
        )
        .unwrap();
        let detail = json_call(
            &store,
            serde_json::json!({"type":"comparison_task_explain","id":id}),
        )
        .unwrap();
        assert_eq!(
            detail["detail"]["task"]["episodes"][0]["episode_id"],
            episode
        );
        assert!(
            detail["detail"]["stale_reasons"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("episode_missing"))
        );
        assert!(
            json_call(
                &store,
                serde_json::json!({
                    "type":"comparison_task_reconfirm", "id":id, "expected_revision":1,
                    "displayed_material_digest":created["task"]["material_digest"]
                })
            )
            .is_err()
        );
        let list = json_call(&store, serde_json::json!({"type":"comparison_task_list"})).unwrap();
        assert_eq!(list["tasks"].as_array().unwrap().len(), 1);
        json_call(
            &store,
            serde_json::json!({"type":"comparison_task_delete","id":id,"expected_revision":1}),
        )
        .unwrap();
    }

    #[test]
    fn comparison_task_abi_reports_upstream_and_overlap_changes_for_reconciliation() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        let episode = comparison_episode(&store, &temp.path().join("source.jsonl"));
        let first = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_create", "episode_ids":[episode]
            }),
        )
        .unwrap();
        let first_id = first["task"]["id"].as_str().unwrap();
        let second = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_create", "episode_ids":[episode]
            }),
        )
        .unwrap();
        assert!(
            second["mutation_effects"]["stale_comparison_task_ids"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(first_id))
        );
        let detail = json_call(
            &store,
            serde_json::json!({"type":"comparison_task_explain","id":first_id}),
        )
        .unwrap();
        assert_eq!(
            detail["detail"]["overlapping_task_ids"],
            serde_json::json!([second["task"]["id"]])
        );
        let deleted = json_call(
            &store,
            serde_json::json!({
                "type":"comparison_task_delete", "id":second["task"]["id"], "expected_revision":1
            }),
        )
        .unwrap();
        assert!(
            deleted["mutation_effects"]["stale_comparison_task_ids"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(first_id))
        );
        let detail = json_call(
            &store,
            serde_json::json!({"type":"comparison_task_explain","id":first_id}),
        )
        .unwrap();
        assert_eq!(
            detail["detail"]["overlapping_task_ids"],
            serde_json::json!([])
        );
        let annotated = json_call(
            &store,
            serde_json::json!({
                "type":"episode_annotate", "id":episode, "expected_revision":1,
                "category":"refactor", "outcome":"accepted"
            }),
        )
        .unwrap();
        assert!(
            annotated["mutation_effects"]["stale_comparison_task_ids"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(first_id))
        );
        let effects = annotated["mutation_effects"]["stale_comparison_tasks"]
            .as_array()
            .unwrap();
        let affected = effects
            .iter()
            .find(|effect| effect["task_id"] == first_id)
            .unwrap();
        assert!(
            affected["reasons"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("episode_revision_changed"))
        );
        let snapshot = &first["task"]["episodes"][0]["members"][0]["snapshot_id"];
        let removed =
            json_call(&store, serde_json::json!({"type":"delete","id":snapshot})).unwrap();
        assert!(
            removed["mutation_effects"]["stale_comparison_task_ids"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!(first_id))
        );
        let detail = json_call(
            &store,
            serde_json::json!({"type":"comparison_task_explain","id":first_id}),
        )
        .unwrap();
        assert!(
            detail["detail"]["stale_reasons"]
                .as_array()
                .unwrap()
                .contains(&serde_json::json!("snapshot_missing_or_replaced"))
        );
        assert_eq!(detail["detail"]["task"]["id"], first_id);
    }

    fn all_questions() -> serde_json::Value {
        serde_json::json!([
            "recorded_activity",
            "episode_outcomes",
            "observed_models",
            "estimated_cost"
        ])
    }

    #[test]
    fn question_cards_abi_reads_an_absent_store_without_creating_state() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        let response = json_call(
            &store,
            serde_json::json!({
                "type":"question_cards",
                "questions":all_questions(),
                "snapshot_ids":[],
                "episode_ids":[]
            }),
        )
        .unwrap();
        assert_eq!(response["type"], "question_cards");
        assert_eq!(response["result"]["schema_version"], 1);
        let cards = response["result"]["cards"].as_array().unwrap();
        assert_eq!(cards.len(), 4);
        assert_eq!(cards[0]["question"], "recorded_activity");
        assert_eq!(cards[1]["question"], "episode_outcomes");
        assert_eq!(cards[2]["question"], "observed_models");
        assert_eq!(cards[3]["question"], "estimated_cost");
        assert!(
            response["text"]
                .as_str()
                .unwrap()
                .contains("Recorded activity")
        );
        assert!(!store.exists());
    }

    #[test]
    fn question_cards_abi_returns_fixed_selection_errors() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({
                    "type":"question_cards",
                    "questions":["recorded_activity","recorded_activity"],
                    "snapshot_ids":[],
                    "episode_ids":[]
                })
            )
            .unwrap_err(),
            "insights_card_invalid_selection"
        );
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({
                    "type":"question_cards",
                    "questions":["recorded_activity"],
                    "snapshot_ids":["a".repeat(64)],
                    "episode_ids":[]
                })
            )
            .unwrap_err(),
            "insights_card_snapshot_not_found"
        );
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({
                    "type":"question_cards",
                    "questions":["episode_outcomes"],
                    "snapshot_ids":[],
                    "episode_ids":["00000000-0000-4000-8000-000000000001"]
                })
            )
            .unwrap_err(),
            "insights_card_episode_not_found"
        );
        assert!(!store.exists());
    }

    #[test]
    fn question_cards_abi_projects_a_saved_snapshot_and_shared_text() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        let source = temp.path().join("source.jsonl");
        std::fs::write(
            &source,
            b"{\"role\":\"meta\",\"source\":\"claude-code\",\"model\":\"fixture-model\"}\n{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00.123Z\",\"content\":\"PRIVATE_BODY\"}\n{\"role\":\"assistant\",\"timestamp\":\"2026-09-11T12:00:01.124Z\",\"content\":\"done\"}\n",
        )
        .unwrap();
        let saved = json_call(
            &store,
            serde_json::json!({"type":"analyze","source":"trajectory","file":source,"save":true}),
        )
        .unwrap();
        let snapshot = saved["insight"]["id"].as_str().unwrap();
        let response = json_call(
            &store,
            serde_json::json!({
                "type":"question_cards",
                "questions":all_questions(),
                "snapshot_ids":[snapshot],
                "episode_ids":[]
            }),
        )
        .unwrap();
        let result = &response["result"];
        assert_eq!(result["cards"].as_array().unwrap().len(), 4);
        assert_eq!(
            result["cards"][0]["evidence_ids"],
            serde_json::json!([snapshot])
        );
        assert_eq!(
            result["cards"][0]["rows"][10]["value"],
            serde_json::json!({"type":"milliseconds","value":1001})
        );
        assert_eq!(result["cards"][2]["rows"][0]["label"], "fixture-model");
        let text = response["text"].as_str().unwrap();
        assert!(text.contains("Recorded activity"));
        assert!(text.contains("Applicable versioned pricing evidence is not available."));
        assert!(!text.contains("PRIVATE_BODY"));
    }

    /// Every operational outcome a shell has to act on differently must arrive
    /// as its own label. Flattening them left "another window is writing" and
    /// "your store is damaged" indistinguishable on the far side of the ABI.
    #[test]
    fn each_store_outcome_keeps_its_own_label_across_the_abi() {
        let temp = tempfile::tempdir().unwrap();
        let store = temp.path().join("insights");
        let source = temp.path().join("source.jsonl");
        std::fs::write(&source, b"{\"role\":\"meta\",\"source\":\"fixture\"}\n{\"role\":\"user\",\"timestamp\":\"2026-09-11T12:00:00Z\",\"content\":\"PRIVATE_BODY\"}\n").unwrap();
        let saved = json_call(
            &store,
            serde_json::json!({"type":"analyze","source":"trajectory","file":source,"save":true}),
        )
        .unwrap();
        let id = saved["insight"]["id"].as_str().unwrap().to_owned();

        // not found
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({"type":"explain","id":"a".repeat(64)})
            )
            .unwrap_err(),
            "insights_not_found"
        );
        // evidence link not found
        assert_eq!(
            json_call(
                &store,
                serde_json::json!({"type":"unlink_evidence","id":id,"evidence_id":"b".repeat(64)})
            )
            .unwrap_err(),
            "insights_evidence_link_not_found"
        );
        // busy: another holder of the same store lock
        let contender = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(store.join("store.lock"))
            .unwrap();
        contender.try_lock().unwrap();
        assert_eq!(
            json_call(&store, serde_json::json!({"type":"list"})).unwrap_err(),
            "insights_store_busy"
        );
        contender.unlock().unwrap();
        drop(contender);
        // refused: the store location is a link, not a directory
        let link = temp.path().join("linked-store");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&store, &link).unwrap();
            assert_eq!(
                json_call(&link, serde_json::json!({"type":"list"})).unwrap_err(),
                "insights_store_symlink_refused"
            );
        }
        // io: a file standing where the store directory must be
        let occupied = temp.path().join("occupied");
        std::fs::write(&occupied, b"not a directory").unwrap();
        assert_eq!(
            json_call(
                &occupied,
                serde_json::json!({"type":"analyze","source":"trajectory","file":source,"save":true})
            )
            .unwrap_err(),
            "insights_store_unavailable"
        );
        // corrupt: an entry the store cannot read, named without its content
        let mut index: serde_json::Value =
            serde_json::from_slice(&std::fs::read(store.join("index.json")).unwrap()).unwrap();
        index["reports"][&id]["cost_unavailable_reason"] = "PRIVATE_INVENTED".into();
        std::fs::write(
            store.join("index.json"),
            serde_json::to_vec(&index).unwrap(),
        )
        .unwrap();
        assert_eq!(
            json_call(&store, serde_json::json!({"type":"explain","id":id})).unwrap_err(),
            "insights_store_invalid"
        );
        let listed = json_call(&store, serde_json::json!({"type":"list"})).unwrap();
        assert_eq!(listed["insights"], serde_json::json!([]));
        assert_eq!(
            listed["quarantined"]["snapshot_ids"],
            serde_json::json!([id])
        );
        // Nothing about the request or the stored bytes rides along.
        for label in [
            "insights_not_found",
            "insights_evidence_link_not_found",
            "insights_store_busy",
            "insights_store_invalid",
            "insights_store_symlink_refused",
            "insights_store_unavailable",
        ] {
            assert!(!label.contains("PRIVATE"));
        }
        let _ = link;
    }
}
