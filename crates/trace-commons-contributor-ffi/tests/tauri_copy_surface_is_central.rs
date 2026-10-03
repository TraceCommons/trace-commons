//! Keep Tauri's safety copy on the contributor core's shared tables and make
//! sure each review flow renders that copy before its action can proceed.

use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|path| path.parent())
        .expect("crate is two directories below the repo root")
        .to_path_buf()
}

fn read(root: &Path, rel: &str) -> String {
    let path = root.join(rel);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", path.display()))
}

/// Return Rust code in one function, excluding comments and string literals.
fn rust_function(source: &str, marker: &str) -> String {
    let code = code_only(source);
    let start = code
        .find(marker)
        .unwrap_or_else(|| panic!("function `{marker}` is missing"));
    let open = code[start..]
        .find('{')
        .map(|offset| start + offset)
        .unwrap_or_else(|| panic!("function `{marker}` has no body"));
    let mut depth = 0usize;
    for (offset, ch) in code[open..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    return code[open..open + offset + ch.len_utf8()].to_owned();
                }
            }
            _ => {}
        }
    }
    panic!("function `{marker}` has an unclosed body")
}

/// Comments and Rust string literals cannot satisfy a source-wiring check.
fn code_only(source: &str) -> String {
    let chars: Vec<char> = source.chars().collect();
    let mut result = String::with_capacity(source.len());
    let mut index = 0;
    while index < chars.len() {
        let current = chars[index];
        let next = chars.get(index + 1).copied();
        if current == '/' && next == Some('/') {
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
        } else if current == '/' && next == Some('*') {
            index += 2;
            let mut depth = 1usize;
            while index + 1 < chars.len() && depth > 0 {
                if chars[index] == '/' && chars[index + 1] == '*' {
                    depth += 1;
                    index += 2;
                } else if chars[index] == '*' && chars[index + 1] == '/' {
                    depth -= 1;
                    index += 2;
                } else {
                    index += 1;
                }
            }
            result.push(' ');
        } else if current == '"' {
            index += 1;
            while index < chars.len() && chars[index] != '"' {
                if chars[index] == '\\' {
                    index += 1;
                }
                index += 1;
            }
            index = (index + 1).min(chars.len());
            result.push(' ');
        } else {
            result.push(current);
            index += 1;
        }
    }
    result
}

#[test]
fn tauri_commands_project_shared_contributor_copy() {
    let root = repo_root();
    let native_flows = read(
        &root,
        "tauri-desktop/src-tauri/src/commands/native_flows.rs",
    );
    let daemon = read(&root, "tauri-desktop/src-tauri/src/commands/daemon.rs");
    let history = read(&root, "tauri-desktop/src-tauri/src/commands/history.rs");

    let disclosure = rust_function(&native_flows, "fn contributor_disclosure_copy");
    assert!(disclosure.contains("witness_copy::witness_copy"));
    assert!(disclosure.contains("private_inference_copy::private_inference_copy"));
    assert!(disclosure.contains("privacy_scan_copy::privacy_scan_copy"));
    assert!(disclosure.contains("onboarding_copy::onboarding_copy"));

    // The automatic-contribution sentences come from the contributor core,
    // and so does the choice between the model-scrub and patterns-only
    // wording (R1). K3 (#1173) moved the pairing into the core so the C
    // ABI reaches it too: Tauri and the FFI ask
    // `consent_copy::automatic_contribution_copy`, which asks
    // `automatic_gate::disclosure` and passes the answer to
    // `automatic_grant_copy`. None of the three reads the `auto_scrub_*`
    // fields itself, which would put the model-scrub wording on a route
    // where no model ran.
    let automatic = rust_function(&native_flows, "fn automatic_contribution_value");
    assert!(automatic.contains("consent_copy::automatic_contribution_copy"));
    let core_consent = read(
        &root,
        "crates/trace-commons-contributor/src/consent_copy.rs",
    );
    let core_automatic = rust_function(&core_consent, "fn automatic_contribution_copy");
    assert!(core_automatic.contains("automatic_gate::disclosure"));
    assert!(core_automatic.contains("automatic_grant_copy"));
    let ffi = read(&root, "crates/trace-commons-contributor-ffi/src/lib.rs");
    let ffi_automatic = rust_function(&ffi, "fn tc_automatic_contribution_copy_json");
    assert!(ffi_automatic.contains("consent_copy::automatic_contribution_copy"));
    for (who, body) in [
        ("Tauri", &automatic),
        ("the core's pairing", &core_automatic),
        ("the FFI", &ffi_automatic),
    ] {
        for field in ["auto_scrub_scope", "auto_scrub_limit", "AUTO_SCRUB_SCOPE"] {
            assert!(
                !body.contains(field),
                "{who} must not pick the scrub wording itself (`{field}`)"
            );
        }
    }
    let wrapper = rust_function(&native_flows, "fn automatic_contribution_copy");
    assert!(wrapper.contains("automatic_contribution_value"));

    // The Flow 1 grant is reached through named commands, and the grant
    // itself refuses before the daemon is asked without a confirmation and
    // scopes chosen in the picker -- not merely saved, since enrollment
    // saves the floor scope. It passes the daemon the witness the
    // disclosure screen showed, which the daemon checks.
    let consent = read(&root, "tauri-desktop/src-tauri/src/commands/consent.rs");
    let grant = rust_function(&consent, "fn grant_automatic");
    assert!(grant.contains("grant_precondition(confirmed"));
    assert!(grant.contains("witness_signing_address"));
    let precondition = rust_function(&consent, "fn grant_precondition");
    assert!(precondition.contains("consent_scopes_chosen"));

    let witness = rust_function(&native_flows, "fn witness_review_copy");
    assert!(witness.contains("witness_copy::witness_copy"));
    assert!(witness.contains(".review"));

    let eligibility = rust_function(&daemon, "fn eligibility_copy");
    for shared_function in [
        "eligibility_state_line",
        "eligibility_reason_line",
        "eligibility_control",
    ] {
        assert!(
            eligibility.contains(shared_function),
            "Tauri eligibility copy must delegate `{shared_function}` to contributor core"
        );
    }

    let eligibility_group = rust_function(&daemon, "fn eligibility_group_copy");
    // The eligible/withheld arithmetic itself (K6 of #1173) moved into the
    // core's `group_eligibility`, which already calls `group_control`
    // internally; Tauri no longer computes `min`/`saturating_sub` itself.
    assert!(eligibility_group.contains("group_eligibility"));
    assert!(eligibility_group.contains("group_withheld_line"));

    let withdrawal = rust_function(&history, "fn withdrawal_confirmation_prompt");
    assert!(withdrawal.contains("confirmation_prompt_unknown"));

    // A void notice's words, and the project-or-grant choice, come from the
    // contributor core; Tauri passes the wire element through.
    let consent = read(&root, "tauri-desktop/src-tauri/src/commands/consent.rs");
    let void_notice = rust_function(&consent, "fn grant_void_notice");
    // Tauri can give the Flow 1 grant (K10), so it asks for the notice with
    // the re-grant; the core decides which notice carries it.
    assert!(void_notice.contains("consent_copy::void_notice_for_wire_with_regrant"));

    // Connecting inference (K12): the step's words, and the words for each
    // offer's disclosure version, come from the core; select refuses before
    // the daemon is asked without a confirmation or a known disclosure.
    let inference = read(
        &root,
        "tauri-desktop/src-tauri/src/commands/inference_connection.rs",
    );
    let step_copy = rust_function(&inference, "fn inference_connection_copy");
    assert!(step_copy.contains("consent_copy::inference_connection_copy"));
    let disclosures = rust_function(&inference, "fn with_disclosures");
    assert!(disclosures.contains("consent_copy::inference_connection_disclosure"));
    let offers = rust_function(&inference, "fn inference_connection_offers");
    assert!(offers.contains("with_disclosures(response)"));
    let select = rust_function(&inference, "fn inference_connection_select");
    assert!(select.contains("select_params(confirmed"));
    let params = rust_function(&inference, "fn select_params");
    assert!(params.contains("consent_copy::inference_connection_disclosure"));
    let install = rust_function(&inference, "fn inference_connection_install");
    assert!(install.contains("if !confirmed"));
    // So does the notice for sessions held on a busy witness, and its count.
    let capacity_notice = rust_function(&consent, "fn witness_capacity_notice");
    assert!(capacity_notice.contains("consent_copy::witness_capacity_notice_for_wire"));
    // And the two switch-on notices: a folder armed under the old wording,
    // and armed folders the gate holds.
    let reworded = rust_function(&consent, "fn arming_reworded_notice");
    assert!(reworded.contains("consent_copy::arming_reworded_notice_for_wire"));
    let held = rust_function(&consent, "fn gate_held_notice");
    assert!(held.contains("consent_copy::gate_held_notice_for_wire"));

    let preview = rust_function(&daemon, "fn preview_entry");
    assert!(preview.contains("consent_copy::consent_copy"));
    assert!(preview.contains("gate_statement"));

    // Quitting says what keeps running, and that depends on whether this
    // process hosts the watcher or is attached to one. The sentence and the
    // role-to-sentence choice both belong to the shared crate.
    let platform = read(&root, "tauri-desktop/src-tauri/src/commands/platform.rs");
    let quit = rust_function(&platform, "fn quit_confirmation_copy");
    assert!(quit.contains("quit_role()"));
    let quit_value = rust_function(&platform, "fn quit_prompt_value");
    assert!(quit_value.contains("quit_copy::quit_prompt"));
    let state = read(&root, "tauri-desktop/src-tauri/src/state.rs");
    let role = rust_function(&state, "fn quit_role(&self) -> QuitRole");
    for role_name in [
        "QuitRole::Hosting",
        "QuitRole::Attached",
        "QuitRole::Unavailable",
    ] {
        assert!(
            role.contains(role_name),
            "Tauri must map its daemon connection to `{role_name}`"
        );
    }
}

#[test]
fn copy_commands_reach_the_frontend_through_tauri_and_render_at_safety_surfaces() {
    let root = repo_root();
    let build = read(&root, "tauri-desktop/src-tauri/build.rs");
    let handler = read(&root, "tauri-desktop/src-tauri/src/commands/mod.rs");
    for (name, handler_path) in [
        ("eligibility_copy", "daemon::eligibility_copy"),
        ("eligibility_group_copy", "daemon::eligibility_group_copy"),
        (
            "contributor_disclosure_copy",
            "native_flows::contributor_disclosure_copy",
        ),
        ("witness_review_copy", "native_flows::witness_review_copy"),
        (
            "withdrawal_confirmation_prompt",
            "history::withdrawal_confirmation_prompt",
        ),
        ("quit_confirmation_copy", "platform::quit_confirmation_copy"),
        ("grant_void_notice", "consent::grant_void_notice"),
        (
            "witness_capacity_notice",
            "consent::witness_capacity_notice",
        ),
        (
            "acknowledge_grant_voids",
            "consent::acknowledge_grant_voids",
        ),
        ("legacy_migration_copy", "consent::legacy_migration_copy"),
        (
            "legacy_migration_notice",
            "consent::legacy_migration_notice",
        ),
        ("migrate_legacy_invite", "consent::migrate_legacy_invite"),
        (
            "acknowledge_legacy_invite_migration",
            "consent::acknowledge_legacy_invite_migration",
        ),
        ("arming_reworded_notice", "consent::arming_reworded_notice"),
        ("gate_held_notice", "consent::gate_held_notice"),
        (
            "acknowledge_arming_rewordings",
            "consent::acknowledge_arming_rewordings",
        ),
    ] {
        assert!(
            build.contains(&format!("\"{name}\"")),
            "{name} is missing from Tauri's generated command allowlist"
        );
        assert!(
            handler.contains(handler_path),
            "{name} is missing from Tauri's invoke handler"
        );
    }

    let api = read(
        &root,
        "tauri-desktop/frontend/src/lib/tauri/contributor-copy-api.ts",
    );
    for command in [
        "witness_review_copy",
        "contributor_disclosure_copy",
        "eligibility_copy",
        "eligibility_group_copy",
        "withdrawal_confirmation_prompt",
        "quit_confirmation_copy",
    ] {
        assert!(
            api.contains(&format!("invokeTauri(\"{command}\"")),
            "frontend copy adapter no longer invokes `{command}`"
        );
    }

    // The void notice (R6's ship condition) is rendered from the core's
    // copy, in the app shell above every page, and cannot be acknowledged
    // before it is on screen.
    assert!(
        api.contains("invokeTauri(\"grant_void_notice\""),
        "frontend copy adapter no longer invokes `grant_void_notice`"
    );
    let void_notices = read(
        &root,
        "tauri-desktop/frontend/src/app/grant-void-notices.tsx",
    );
    for rendered_copy in [
        "useGrantVoidNotice",
        "copy.data.title",
        "copy.data.body",
        "copy.data.reasons_heading",
        "copy.data.reasons",
        "copy.data.rearm",
        "copy.data.acknowledge",
        "!copy.data",
        "acknowledgeGrantVoids([grantVoid.id])",
        "copy.data.rearm_action",
        "copy.data.rearm_failed",
        "rearmTarget(grantVoid, copy.data)",
        "changeProjectMode(id, \"auto_upload\")",
        // K10: the grant's notice offers the re-grant, which opens the grant
        // screens rather than giving anything in one click.
        "regrantOffered(copy.data)",
        "copy.data.regrant}",
        "copy.data.regrant_action",
        "navigate(flowPaths[\"automatic-contributing\"])",
    ] {
        assert!(
            void_notices.contains(rendered_copy),
            "the void notice must use `{rendered_copy}`"
        );
    }
    assert!(
        !void_notices.contains("grantAutomatic"),
        "the void notice must not give the grant itself"
    );
    let routes = read(&root, "tauri-desktop/frontend/src/app/app-routes.tsx");
    let regrant_flow = routes
        .find("function AutomaticContributingFlow")
        .expect("the re-grant route renders the grant screens");
    assert!(routes[regrant_flow..].contains("<OnboardingPage"));
    assert!(routes.contains("core.data?.daemon.logged_in === true ?"));
    let shell = read(&root, "tauri-desktop/frontend/src/app/app-shell.tsx");
    assert!(shell.contains("<GrantVoidNotices"));

    // Sessions held on a busy witness are told from the core's words, in
    // the queue's health panel, from `status.witness_capacity`.
    assert!(
        api.contains("invokeTauri(\"witness_capacity_notice\""),
        "frontend copy adapter no longer invokes `witness_capacity_notice`"
    );
    let capacity_notice = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/witness-capacity-notice.tsx",
    );
    for rendered_copy in [
        "useWitnessCapacityNotice",
        "copy.data.title",
        "copy.data.body",
        "nextRetryLine(copy.data, capacity)",
    ] {
        assert!(
            capacity_notice.contains(rendered_copy),
            "the witness capacity notice must use `{rendered_copy}`"
        );
    }
    let panel = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/queue-status-panel.tsx",
    );
    assert!(panel.contains("<WitnessCapacityNotice"));
    let waiting = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/waiting-page.tsx",
    );
    assert!(waiting.contains("witnessCapacity={status.daemon.witness_capacity}"));

    // The switch-on notices are rendered from the core's words, above every
    // page. The rewording notice is acknowledged by id and only once on
    // screen; the held notice has no dismiss, and the generic health line
    // steps aside for it.
    for invoked in ["arming_reworded_notice", "gate_held_notice"] {
        assert!(
            api.contains(&format!("invokeTauri(\"{invoked}\"")),
            "frontend copy adapter no longer invokes `{invoked}`"
        );
    }
    let switch_on = read(
        &root,
        "tauri-desktop/frontend/src/app/switch-on-notices.tsx",
    );
    for rendered_copy in [
        "useArmingRewordedNotice",
        "useGateHeldNotice",
        "copy.data.title",
        "copy.data.body",
        "copy.data.now_heading",
        "copy.data.scope",
        "copy.data.limit",
        "copy.data.no_review",
        "copy.data.acknowledge",
        "copy.data.ask_first_action",
        "copy.data.ask_first_failed",
        "acknowledgeArmingRewordings([rewording.id])",
        "askFirstTarget(rewording.wire, copy.data)",
        "changeProjectMode(id, \"notify_only\")",
        "copy.data.reasons",
        "copy.data.release",
        "copy.data.ask_first",
        "project.line",
        "project.ask_first_action",
    ] {
        assert!(
            switch_on.contains(rendered_copy),
            "the switch-on notices must use `{rendered_copy}`"
        );
    }
    assert!(shell.contains("<ArmingRewordingNotices"));
    assert!(shell.contains("<GateHeldNotice"));
    assert!(panel.contains("GATE_HELD_LABEL"));
    assert!(waiting.contains("gateHeld={status.daemon.automatic_contribution_held}"));

    let witness = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/witness-review-overlay.tsx",
    );
    for rendered_copy in [
        "useWitnessReviewCopy",
        "copy.data?.disclosure",
        "copy.data?.confirm",
    ] {
        assert!(
            witness.contains(rendered_copy),
            "witness review must use `{rendered_copy}`"
        );
    }
    assert!(witness.contains("!confirmed"));
    assert!(witness.contains("mutation.mutate(true)"));

    let admission = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/admission-preparation-overlay.tsx",
    );
    for rendered_copy in [
        "useContributorDisclosureCopy",
        "copy.data?.admission",
        "disclosure?.disclosure",
        "disclosure?.confirm",
    ] {
        assert!(
            admission.contains(rendered_copy),
            "admission must use `{rendered_copy}`"
        );
    }
    assert!(admission.contains("!confirmed"));
    assert!(admission.contains("mutation.mutate(true)"));
    assert!(!admission.contains("Admission preparation could not be completed. Nothing was sent."));

    let withdrawal = read(
        &root,
        "tauri-desktop/frontend/src/features/history/components/withdrawal-control.tsx",
    );
    assert!(withdrawal.contains("useWithdrawalConfirmationPrompt(confirming)"));
    assert!(withdrawal.contains("{confirmation.data}"));
    assert!(withdrawal.contains("busy || !confirmation.data"));

    let quit = read(
        &root,
        "tauri-desktop/frontend/src/app/quit-confirmation.tsx",
    );
    for rendered_copy in [
        "useQuitConfirmationCopy",
        "copy.data?.body",
        "copy.data?.confirm",
    ] {
        assert!(
            quit.contains(rendered_copy),
            "quit confirmation must use `{rendered_copy}`"
        );
    }
    assert!(
        quit.contains("!copy.data"),
        "quit must not be confirmable before the true sentence for this process is shown"
    );
    assert!(
        !quit.contains("Quitting stops"),
        "quit confirmation must not hard-code the hosting sentence; it is false when attached"
    );

    let wallet = read(
        &root,
        "tauri-desktop/frontend/src/features/onboarding/components/onboarding-wallet-connect.tsx",
    );
    assert!(wallet.contains("useContributorDisclosureCopy"));
    assert!(wallet.contains("disclosure?.disclosure"));
    assert!(wallet.contains("!disclosure"));

    let near_ai = read(
        &root,
        "tauri-desktop/frontend/src/features/onboarding/components/onboarding-near-ai-join.tsx",
    );
    assert!(near_ai.contains("useContributorDisclosureCopy"));
    assert!(near_ai.contains("disclosure?.what"));
    assert!(near_ai.contains("disclosure?.action"));
    assert!(near_ai.contains("!disclosure"));
    let credential_cost = near_ai
        .find("{disclosures.data.credential_cost}")
        .expect("NEAR AI credential cost is rendered");
    let sign_in = near_ai
        .find("Start sign-in")
        .expect("NEAR AI sign-in action is rendered");
    assert!(
        credential_cost < sign_in,
        "NEAR AI credential cost must appear before the sign-in action"
    );

    let private_ai_panel = read(
        &root,
        "tauri-desktop/frontend/src/features/private-ai/components/private-ai-connection-panel.tsx",
    );
    let panel_cost = private_ai_panel
        .find("{disclosure.data.credential_cost}")
        .expect("Private AI credential cost is rendered");
    let panel_action = private_ai_panel
        .find("<PrivateAiCredentialAction")
        .expect("Private AI credential action is rendered");
    assert!(
        panel_cost < panel_action,
        "Private AI credential cost must appear before the sign-in action"
    );

    // The daemon holds uploads while the NEAR AI notice is unacknowledged.
    // Tauri must offer the shared notice and its acknowledgement outside
    // onboarding, and never offer the confirmation without the notice.
    let queue_status = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/queue-status-panel.tsx",
    );
    assert!(queue_status.contains("<NearAiNoticeRecovery"));
    assert!(queue_status.contains("!== NEAR_AI_NOTICE_LABEL"));
    let recovery = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/near-ai-notice-recovery.tsx",
    );
    for rendered_copy in [
        "useContributorDisclosureCopy",
        "disclosure.data?.privacy_scan",
        "{copy.disclosure}",
        "{copy.offer}",
        "copy.recovery_action",
        "{copy.recovery_failed}",
    ] {
        assert!(
            recovery.contains(rendered_copy),
            "NEAR AI notice recovery must use `{rendered_copy}`"
        );
    }
    let not_ready = recovery
        .find("recovery.kind !== \"ready\"")
        .expect("the recovery refuses to render without shared copy");
    let confirm = recovery
        .find("copy.recovery_confirm")
        .expect("the recovery renders the shared confirmation");
    assert!(
        not_ready < confirm,
        "the confirmation must be unreachable until the shared notice loads"
    );
    let recovery_hook = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/hooks/use-near-ai-notice-recovery.ts",
    );
    assert!(recovery_hook.contains("mutationFn: acknowledgeNearAiNotice"));

    let privacy_step = read(
        &root,
        "tauri-desktop/frontend/src/features/onboarding/components/onboarding-privacy-step.tsx",
    );
    assert!(privacy_step.contains("disclosure.data?.privacy_scan"));
    assert!(privacy_step.contains("{copy.disclosure}"));
    assert!(privacy_step.contains("disabled={busy || !copy}"));

    let roots_step = read(
        &root,
        "tauri-desktop/frontend/src/features/onboarding/components/onboarding-roots-step.tsx",
    );
    for rendered_copy in [
        "shell.roots_required",
        "shell.watcher_starting",
        "shell?.watcher_start_failed",
        "!rootsAnswered",
    ] {
        assert!(
            roots_step.contains(rendered_copy),
            "the roots step must use `{rendered_copy}`"
        );
    }

    // The Flow 1 grant screens (K10, K11) render the core's words, reach the
    // grant only through `requestGrant`, and never choose the scrub wording.
    for (name, handler_path) in [
        (
            "automatic_contribution_copy",
            "native_flows::automatic_contribution_copy",
        ),
        (
            "project_automatic_contribution_copy",
            "native_flows::project_automatic_contribution_copy",
        ),
        ("automatic_grant", "consent::automatic_grant"),
        ("grant_automatic", "consent::grant_automatic"),
        (
            "withdraw_automatic_grant",
            "consent::withdraw_automatic_grant",
        ),
    ] {
        assert!(
            build.contains(&format!("\"{name}\"")),
            "{name} is missing from Tauri's generated command allowlist"
        );
        assert!(
            handler.contains(handler_path),
            "{name} is missing from Tauri's invoke handler"
        );
    }
    assert!(api.contains("invokeTauri(\"automatic_contribution_copy\""));
    // K6: an armed project's disclosure is chosen by the core over that
    // project's own sessions; the settings row renders it and never chooses.
    assert!(api.contains("invokeTauri(\"project_automatic_contribution_copy\""));
    let project_disclosure = read(
        &root,
        "tauri-desktop/frontend/src/features/settings/components/project-automatic-disclosure.tsx",
    );
    assert!(project_disclosure.contains("useProjectAutomaticCopy"));
    assert!(project_disclosure.contains("scrubDisclosureLines(copy)"));
    for forbidden in ["model_scrubbed", "patterns_only", "auto_scrub"] {
        assert!(
            !project_disclosure.contains(forbidden),
            "the project disclosure must not choose its wording (`{forbidden}`)"
        );
    }
    // Its failure line is the core's too, served through the shared copy
    // command; the component holds no sentence of its own.
    assert!(project_disclosure.contains("project_automatic_unavailable"));
    assert!(
        !project_disclosure
            .contains(trace_commons_contributor::consent_copy::AUTO_PROJECT_DISCLOSURE_UNAVAILABLE),
        "the project disclosure's failure line must come from the core"
    );
    assert!(!project_disclosure.contains("could not be loaded"));
    let native_flows_source = read(
        &root,
        "tauri-desktop/src-tauri/src/commands/native_flows.rs",
    );
    assert!(
        native_flows_source.contains("consent_copy::AUTO_PROJECT_DISCLOSURE_UNAVAILABLE"),
        "contributor_disclosure_copy must serve the core's failure line"
    );
    assert!(api.contains("\"project_automatic_unavailable\""));
    let native_flows = read(
        &root,
        "tauri-desktop/src-tauri/src/commands/native_flows.rs",
    );
    let project_wrapper = rust_function(&native_flows, "fn project_automatic_contribution_copy");
    assert!(project_wrapper.contains("call_result_or_view"));
    for field in ["auto_scrub", "AUTO_SCRUB", "Disclosure::"] {
        assert!(
            !project_wrapper.contains(field),
            "the Tauri wrapper forwards the core's choice and never makes one (`{field}`)"
        );
    }
    let grant_copy = read(
        &root,
        "tauri-desktop/frontend/src/lib/tauri/automatic-grant-copy.ts",
    );
    assert!(grant_copy.contains("Patterns-only grant copy carried model-scrub wording"));
    let onboarding_dir = "tauri-desktop/frontend/src/features/onboarding";
    let scrub_step = read(
        &root,
        &format!("{onboarding_dir}/components/onboarding-scrub-disclosure-step.tsx"),
    );
    assert!(scrub_step.contains("useAutomaticGrantCopy"));
    assert!(scrub_step.contains("scrubDisclosureLines(copy)"));
    assert!(scrub_step.contains("disabled={busy || !copy}"));
    for forbidden in ["model_scrubbed", "patterns_only", "auto_scrub"] {
        assert!(
            !scrub_step.contains(forbidden),
            "the scrub disclosure must not choose its wording (`{forbidden}`)"
        );
    }
    let witness_step = read(
        &root,
        &format!("{onboarding_dir}/components/onboarding-witness-disclosure-step.tsx"),
    );
    for rendered_copy in [
        "useWitness",
        "status.state_line",
        // K11: the raw send, both enclaves and where the witness came from
        // are the daemon's facts in the core's words, not shell sentences.
        "useRouteDisclosure",
        "<RouteDisclosureBody disclosure={disclosure.data} />",
        "status?.state === \"pinned\"",
        "disabled={busy || !ready}",
    ] {
        assert!(
            witness_step.contains(rendered_copy),
            "the witness disclosure must use `{rendered_copy}`"
        );
    }
    // K11's disclosure surfaces: the commands exist, the adapter calls them
    // through the parser that refuses words not matching their facts, and
    // every block is the core's sentence.
    for (name, handler_path) in [
        ("route_disclosure", "witness::route_disclosure"),
        ("certificate_detail", "witness::certificate_detail"),
        (
            "route_disclosure_unreadable_copy",
            "witness::route_disclosure_unreadable_copy",
        ),
    ] {
        assert!(
            build.contains(&format!("\"{name}\"")),
            "{name} is missing from Tauri's generated command allowlist"
        );
        assert!(
            handler.contains(handler_path),
            "{name} is missing from Tauri's invoke handler"
        );
        assert!(
            api.contains(&format!("invokeTauri(\"{name}\"")),
            "frontend copy adapter no longer invokes `{name}`"
        );
    }
    let witness_commands = read(&root, "tauri-desktop/src-tauri/src/commands/witness.rs");
    let disclosure_value = rust_function(&witness_commands, "fn route_disclosure_value");
    assert!(disclosure_value.contains("consent_copy::route_disclosure_copy(&parsed)"));
    assert!(api.contains("parseRouteDisclosure(await invokeTauri(\"route_disclosure\"))"));
    let route_body = read(
        &root,
        "tauri-desktop/frontend/src/components/route-disclosure.tsx",
    );
    for rendered_copy in [
        "copy.route",
        "copy.local_filter",
        "copy.witness.check",
        "copy.witness.classifier",
        "copy.witness.origin",
        "copy.receipts",
        "copy.attested_bodies",
        "facts.witness.pinned_measurements",
    ] {
        assert!(
            route_body.contains(rendered_copy),
            "the route disclosure must render `{rendered_copy}`"
        );
    }
    let session_block = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/session-send-disclosure.tsx",
    );
    for rendered_copy in [
        "copy.session.before_line",
        "copy.session.after_line",
        "certificate.data.copy.verified_at_review",
        "holds_certificate === true",
    ] {
        assert!(
            session_block.contains(rendered_copy),
            "the per-session disclosure must render `{rendered_copy}`"
        );
    }
    assert!(
        read(
            &root,
            "tauri-desktop/frontend/src/features/waiting/components/preview-inspector.tsx",
        )
        .contains("<SessionSendDisclosure preview={preview} />")
    );
    // The section's title and its unreadable lines are the core's too
    // (`consent_copy::disclosure_unreadable_copy`), never a sentence the
    // shell writes -- including the eyebrow and the loading line.
    let unreadable_copy = rust_function(&witness_commands, "fn route_disclosure_unreadable_copy");
    assert!(unreadable_copy.contains("consent_copy::disclosure_unreadable_copy"));
    assert!(route_body.contains("unreadable.data?.title"));
    assert!(route_body.contains("unreadable.data?.panel"));
    assert!(session_block.contains("unreadable.data?.session"));
    {
        use trace_commons_contributor::consent_copy as copy;
        for sentence in [
            copy::DISCLOSURE_TITLE,
            copy::DISCLOSURE_UNREADABLE,
            copy::DISCLOSURE_SESSION_UNREADABLE,
            "WHERE SESSIONS GO",
            "could not be read",
            "Reading where",
        ] {
            for (name, source) in [("route", &route_body), ("session", &session_block)] {
                assert!(
                    !source.contains(sentence),
                    "the Tauri {name} disclosure words itself: {sentence}"
                );
            }
        }
    }
    let consent_step = read(
        &root,
        &format!("{onboarding_dir}/components/onboarding-consent-step.tsx"),
    );
    assert!(consent_step.contains("useState<string[]>(initialScopeSelection)"));
    assert!(consent_step.contains("copy.scope_required"));
    // Continue saves only a complete choice; a try without one marks the
    // missing scope and saves nothing.
    assert!(consent_step.contains("if (choice.canContinue && copy && privacyKnown)"));
    assert!(consent_step.contains("aria-describedby"));
    let grant_step = read(
        &root,
        &format!("{onboarding_dir}/components/onboarding-grant-step.tsx"),
    );
    assert!(grant_step.contains("copy.no_review"));
    assert!(grant_step.contains("disabled={busy || !copy || blocked}"));
    assert!(!grant_step.contains("bg-primary"));
    let skip = grant_step
        .find("onboarding.skipGrant")
        .expect("the grant can be declined");
    let give = grant_step
        .find("onboarding.grant()")
        .expect("the grant is offered");
    assert!(
        skip < give,
        "declining the grant must come before giving it"
    );
    assert!(
        witness_step.contains("acknowledgeWitnessDisclosure(")
            && witness_step.contains("status?.signing_address ?? null"),
        "the witness screen records the witness it showed"
    );
    // Connecting inference (K12): named commands, the core's words, no
    // offer pre-selected, select and install as separate confirmed actions,
    // and skipping before either, unaccented.
    for (name, handler_path) in [
        (
            "inference_connection_copy",
            "inference_connection::inference_connection_copy",
        ),
        (
            "inference_connection_offers",
            "inference_connection::inference_connection_offers",
        ),
        (
            "inference_connection_current",
            "inference_connection::inference_connection_current",
        ),
        (
            "inference_connection_select",
            "inference_connection::inference_connection_select",
        ),
        (
            "inference_connection_install",
            "inference_connection::inference_connection_install",
        ),
        (
            "inference_connection_disconnect",
            "inference_connection::inference_connection_disconnect",
        ),
    ] {
        assert!(
            build.contains(&format!("\"{name}\"")),
            "{name} is missing from Tauri's generated command allowlist"
        );
        assert!(
            handler.contains(handler_path),
            "{name} is missing from Tauri's invoke handler"
        );
    }
    assert!(api.contains("invokeTauri(\"inference_connection_copy\""));
    let inference_step = read(
        &root,
        &format!("{onboarding_dir}/components/onboarding-inference-step.tsx"),
    );
    for rendered_copy in [
        "useInferenceConnectionCopy",
        "copy.why",
        "copy.grants_nothing",
        "copy.sign_in",
        "copy.one_device",
        "copy.other_device",
        "copy.reselect",
        "copy.install}",
        "copy.installed",
        "copy.select_failed",
        "copy.install_failed",
        "offer.disclosure",
        "useState<string | null>(null)",
        "chosen === null",
    ] {
        assert!(
            inference_step.contains(rendered_copy),
            "the connect-inference step must use `{rendered_copy}`"
        );
    }
    assert!(!inference_step.contains("bg-primary"));
    let skip = inference_step
        .find("onboarding.finishInference")
        .expect("the step can be skipped");
    for action in ["inference.select(", "inference.install("] {
        let at = inference_step
            .find(action)
            .unwrap_or_else(|| panic!("`{action}` is offered"));
        assert!(skip < at, "skipping must come before `{action}`");
    }

    let hook = read(&root, &format!("{onboarding_dir}/hooks/use-onboarding.ts"));
    assert!(hook.contains("requestGrant(current, grantAutomatic)"));
    // A withdraw is confirmed by re-reading the grant, here and in Settings.
    assert!(hook.contains("withdrawAndConfirm(withdrawAutomaticGrant, getAutomaticGrant)"));
    let settings_grant = read(
        &root,
        "tauri-desktop/frontend/src/features/settings/hooks/use-automatic-grant.ts",
    );
    assert!(settings_grant.contains("withdrawAndConfirm(withdrawAutomaticGrant"));
    assert!(settings_grant.contains("getAutomaticGrant"));
    let grant_calls = hook.matches("grantAutomatic").count();
    assert_eq!(
        grant_calls, 2,
        "grantAutomatic is imported once and called only through requestGrant"
    );

    let private_inference = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/private-inference-offer.tsx",
    );
    assert!(private_inference.contains("useContributorDisclosureCopy"));
    assert!(private_inference.contains("copy.offer_exposure"));
    assert!(private_inference.contains("copy.offer_no_repoint"));
    assert!(private_inference.contains("disabled={busy || !copy}"));
    assert!(private_inference.contains("copy.destination"));
    assert!(private_inference.contains("copy.offer_title"));
    for stale_label in [
        "OPTIONAL PRIVATE INFERENCE",
        "OPTIONAL PRIVATE AI",
        "?? \"Private AI\"",
        "\"Private inference\"",
        "Enable private inference",
    ] {
        assert!(
            !private_inference.contains(stale_label),
            "private inference's internal name must not be contributor-facing: {stale_label}"
        );
    }

    let arming = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/arming-offer.tsx",
    );
    let decline = arming
        .find("copy.data?.decline")
        .expect("arming decline copy is rendered");
    let confirm = arming
        .find("copy.data?.confirm")
        .expect("arming confirmation copy is rendered");
    assert!(
        decline < confirm,
        "the arming offer must put the non-consequential action first"
    );
    assert!(
        !arming.contains("bg-primary"),
        "the arming confirmation must not be visually accented"
    );

    let history_row = read(
        &root,
        "tauri-desktop/frontend/src/features/history/components/history-row.tsx",
    );
    assert!(history_row.contains("historyCreditLine("));
    let history_model = read(
        &root,
        "tauri-desktop/frontend/src/features/history/history-view-model.ts",
    );
    assert!(history_model.contains("still being scored"));

    let eligibility = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/waiting-review.tsx",
    );
    assert!(eligibility.contains("eligibilityCopy.state_line"));
    assert!(eligibility.contains("eligibilityCopy.reason_line"));
    assert!(eligibility.contains("!canApprove ||"));
    assert!(eligibility.contains("!eligibilityReady ||"));

    let row = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/waiting-entry-row.tsx",
    );
    assert!(row.contains("useEligibilityCopy"));
    assert!(row.contains("eligibility.data.state_line"));
    assert!(row.contains("eligibility.data.reason_line"));

    let copy_hook = read(
        &root,
        "tauri-desktop/frontend/src/lib/tauri/use-contributor-copy.ts",
    );
    assert!(copy_hook.contains("getEligibilityGroupCopy"));
    assert!(copy_hook.contains("export function useEligibilityGroupCopy"));
    for path in [
        "tauri-desktop/frontend/src/features/waiting/components/waiting-project-folder.tsx",
        "tauri-desktop/frontend/src/features/waiting/components/waiting-project-group.tsx",
    ] {
        let group = read(&root, path);
        assert!(group.contains("useEligibilityGroupCopy"));
        assert!(group.contains("eligibility.data?.can_contribute === true"));
        assert!(group.contains("eligibility.data.eligible_count"));
    }
}

fn visit_sources(dir: &Path, found: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(dir)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry is readable").path();
        if path.is_dir() {
            visit_sources(&path, found);
        } else if path
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| matches!(extension, "ts" | "tsx" | "mjs" | "rs"))
        {
            found.push(path);
        }
    }
}

/// The destination is called "Private AI" (`private_inference_copy::DESTINATION`).
/// "Private inference" is the setting's internal name and a privacy claim the
/// feature does not make, so no Tauri source may put it in front of a
/// contributor -- not in a label, an error, a tray item, or a story fixture.
#[test]
fn tauri_never_says_private_inference_to_a_contributor() {
    let root = repo_root();
    let mut sources = Vec::new();
    for dir in ["tauri-desktop/frontend/src", "tauri-desktop/src-tauri/src"] {
        visit_sources(&root.join(dir), &mut sources);
    }
    assert!(!sources.is_empty(), "no Tauri sources were found");
    let offenders: Vec<String> = sources
        .iter()
        .filter(|path| {
            std::fs::read_to_string(path)
                .unwrap_or_default()
                .to_lowercase()
                .contains("private inference")
        })
        .map(|path| {
            path.strip_prefix(&root)
                .unwrap_or(path)
                .display()
                .to_string()
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "\"private inference\" is contributor-facing in: {offenders:?}"
    );
}

/// The legacy invite migration's words live in the contributor core and
/// nowhere in the Tauri shell: the panel and the notice render only what the
/// core's copy commands return, including the transport-failure line.
#[test]
fn legacy_migration_copy_is_central_and_the_shell_holds_no_literal() {
    use trace_commons_contributor::consent_copy as copy;
    let root = repo_root();
    let panel = read(
        &root,
        "tauri-desktop/frontend/src/features/settings/components/legacy-migration-panel.tsx",
    );
    let notice = read(
        &root,
        "tauri-desktop/frontend/src/app/legacy-migration-notice.tsx",
    );
    let offer = copy::legacy_migration_offer();
    let mut sentences = vec![
        offer.title,
        offer.body,
        offer.action,
        offer.working,
        offer.invite_prompt,
        offer.start_failed,
        copy::LEGACY_MIGRATION_NOTICE_TITLE,
        copy::LEGACY_MIGRATION_NOTICE_BODY,
        copy::LEGACY_MIGRATION_NOTICE_ARMED_KEPT,
        copy::LEGACY_MIGRATION_NOTICE_NOTHING_ARMED,
        copy::LEGACY_MIGRATION_NOTICE_ACKNOWLEDGE,
    ];
    for label in [
        "legacy_migration_tenant_pooled",
        "legacy_migration_admission_not_ready",
        "legacy_migration_invite_needed",
        "no-such-label",
    ] {
        sentences.push(copy::legacy_migration_refusal_line(label));
    }
    for sentence in sentences {
        for (name, source) in [("panel", &panel), ("notice", &notice)] {
            assert!(
                !source.contains(sentence),
                "the Tauri {name} holds core copy as a literal: {sentence}"
            );
        }
    }
    // Nor a sentence of its own: a literal "Nothing was changed" is the
    // fallback this test exists to keep out.
    for (name, source) in [("panel", &panel), ("notice", &notice)] {
        assert!(
            !source.contains("Nothing was changed"),
            "the Tauri {name} words a failure itself"
        );
    }
    for field in [
        "copy.data.title",
        "copy.data.body",
        "copy.data.action",
        "copy.data.working",
        "copy.data.invite_prompt",
        "copy.data.start_failed",
        "answer.line",
    ] {
        assert!(panel.contains(field), "the panel does not render {field}");
    }
    assert!(panel.contains("getLegacyMigrationOffer"));
    for field in [
        "copy.data.title",
        "copy.data.body",
        "copy.data.folders",
        "copy.data.acknowledge",
    ] {
        assert!(notice.contains(field), "the notice does not render {field}");
    }
    assert!(notice.contains("getLegacyMigrationNotice"));
    let consent = read(&root, "tauri-desktop/src-tauri/src/commands/consent.rs");
    let serve = rust_function(&consent, "fn legacy_migration_copy");
    assert!(serve.contains("legacy_migration_offer"));
}
