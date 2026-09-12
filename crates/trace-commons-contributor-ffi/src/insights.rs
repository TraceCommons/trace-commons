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
            "insights-operation-failed"
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
        assert!(analyzed["insight"]["usage_evidence"].is_null());
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
}
