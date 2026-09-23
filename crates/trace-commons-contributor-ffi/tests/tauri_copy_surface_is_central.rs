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
    assert!(eligibility_group.contains("group_control"));
    assert!(eligibility_group.contains("group_withheld_line"));

    let withdrawal = rust_function(&history, "fn withdrawal_confirmation_prompt");
    assert!(withdrawal.contains("confirmation_prompt_unknown"));

    let preview = rust_function(&daemon, "fn preview_entry");
    assert!(preview.contains("consent_copy::consent_copy"));
    assert!(preview.contains("gate_statement"));
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
    ] {
        assert!(
            api.contains(&format!("invokeTauri(\"{command}\"")),
            "frontend copy adapter no longer invokes `{command}`"
        );
    }

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

    let private_inference = read(
        &root,
        "tauri-desktop/frontend/src/features/waiting/components/private-inference-offer.tsx",
    );
    assert!(private_inference.contains("useContributorDisclosureCopy"));
    assert!(private_inference.contains("copy.offer_exposure"));
    assert!(private_inference.contains("copy.offer_no_repoint"));
    assert!(private_inference.contains("disabled={busy || !copy}"));
    for stale_label in [
        "OPTIONAL PRIVATE INFERENCE",
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
    assert!(history_row.contains("still being scored"));

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
