//! INTEGRATION: centralizes reviewed skill-learning copy and maps daemon
//! refusal labels to actionable, credential-free native-app messages.

use serde::Serialize;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SkillLearningCopy {
    pub heading: &'static str,
    pub promise: &'static str,
    pub supported_family: &'static str,
    pub learn_action: &'static str,
    pub learning: &'static str,
    pub candidate_heading: &'static str,
    pub generated_source: &'static str,
    pub name: &'static str,
    pub applicability: &'static str,
    pub procedure: &'static str,
    pub source_evidence: &'static str,
    pub manual_instruction: &'static str,
    pub test_contract: &'static str,
    pub contract_summary_format: &'static str,
    pub review_action: &'static str,
    pub reviewing: &'static str,
    pub exact_package: &'static str,
    pub digest: &'static str,
    pub evaluation_disclosure: &'static str,
    pub review_budget_format: &'static str,
    pub approve_and_test: &'static str,
    pub edit_skill: &'static str,
    pub testing: &'static str,
    pub results: &'static str,
    pub passed_gate: &'static str,
    pub failed_gate: &'static str,
    pub repository_plans: &'static str,
    pub skill_applicability: &'static str,
    pub regressions: &'static str,
    pub no_regressions: &'static str,
    pub inspect_runs: &'static str,
    pub model_and_budget: &'static str,
    pub model_budget_format: &'static str,
    pub open_fixture_source: &'static str,
    pub model_output: &'static str,
    pub edits: &'static str,
    pub commands: &'static str,
    pub checks: &'static str,
    pub baseline: &'static str,
    pub simple_instruction: &'static str,
    pub candidate_skill: &'static str,
    pub passed: &'static str,
    pub failed: &'static str,
    pub preparing: &'static str,
    pub review_install: &'static str,
    pub retry_install: &'static str,
    pub install_preview: &'static str,
    pub install_disclosure: &'static str,
    pub persistent_writes: &'static str,
    pub target_path: &'static str,
    pub skill_file: &'static str,
    pub ownership_marker: &'static str,
    pub marker_digest: &'static str,
    pub exact_marker: &'static str,
    pub coding_tool: &'static str,
    pub install_action: &'static str,
    pub installing: &'static str,
    pub installed: &'static str,
    pub rollback: &'static str,
    pub rolling_back: &'static str,
    pub rollback_disclosure: &'static str,
    pub rollback_incomplete: &'static str,
    pub unavailable: &'static str,
}

#[must_use]
pub fn skill_learning_copy() -> SkillLearningCopy {
    SkillLearningCopy {
        heading: "Learn from session",
        promise: "Turn this correction into an Agent Skill, score its plans and applicability on held-out repository scenarios, then install only a version that improves on both controls.",
        supported_family: "The first supported family repairs generated files through their authoritative source.",
        learn_action: "Learn from session",
        learning: "Learning…",
        candidate_heading: "Candidate skill",
        generated_source: "Generated source",
        name: "Skill name",
        applicability: "When it applies",
        procedure: "Corrective procedure",
        source_evidence: "Source evidence",
        manual_instruction: "Simple instruction control",
        test_contract: "Held-out plan test",
        contract_summary_format: "%1$d tasks across %2$d debugging cases. %3$d requests compare the baseline, a simple instruction, and this candidate. Each request allows %4$d output tokens and %5$d seconds.",
        review_action: "Review skill",
        reviewing: "Reviewing…",
        exact_package: "Exact installable package",
        digest: "SHA-256",
        evaluation_disclosure: "Approving starts the disclosed NEAR AI Cloud requests. Every arm uses the same private model and output limit. Requests contain public repository fixtures and the exact skill package shown above. Source correction and evidence fields stay on this device; remove anything private you copied into the package before approving.",
        review_budget_format: "%1$d model requests; %2$d output tokens per request; model owner must be %3$@.",
        approve_and_test: "Approve and test",
        edit_skill: "Edit skill",
        testing: "Scoring held-out plans…",
        results: "Held-out plan results",
        passed_gate: "Passed plan gate",
        failed_gate: "Needs revision",
        repository_plans: "Repository plans",
        skill_applicability: "Skill applicability",
        regressions: "Regressions",
        no_regressions: "No control-passing repository plan regressed.",
        inspect_runs: "Inspect all runs",
        model_and_budget: "Model and budget",
        model_budget_format: "Owner %1$@ · %2$d tasks · %3$d output tokens per run",
        open_fixture_source: "Open fixture source",
        model_output: "Full model output",
        edits: "Direct source edits",
        commands: "Commands",
        checks: "Checks",
        baseline: "Baseline",
        simple_instruction: "Simple instruction",
        candidate_skill: "Candidate skill",
        passed: "Passed",
        failed: "Failed",
        preparing: "Preparing…",
        review_install: "Review install",
        retry_install: "Retry install",
        install_preview: "Installation preview",
        install_disclosure: "Trace Commons will create the directory and two persistent files shown below. It will refuse an occupied path and can roll back only while the installed files remain unchanged.",
        persistent_writes: "Persistent writes",
        target_path: "Target path",
        skill_file: "Skill file",
        ownership_marker: "Ownership marker",
        marker_digest: "Marker file SHA-256",
        exact_marker: "Exact ownership marker",
        coding_tool: "Coding tool",
        install_action: "Install skill",
        installing: "Installing…",
        installed: "Installed in Codex",
        rollback: "Roll back",
        rolling_back: "Rolling back…",
        rollback_disclosure: "Rollback is available while Trace Commons can verify the installed package has not changed.",
        rollback_incomplete: "The skill could not be removed. Inspect the installed directory before retrying.",
        unavailable: "The skill workflow could not complete. Retry the current step.",
    }
}

#[must_use]
pub fn skill_learning_error_line(label: &str) -> &'static str {
    match label {
        "account-session-required" => "Sign in to your Trace Commons account, then retry.",
        "session-detail-not-found" => "This contribution is no longer available to this account.",
        "skill-correction-required" => {
            "This session has no contributed human correction to learn from."
        }
        "skill-session-not-accepted" => {
            "This session must be accepted before its correction can become a skill."
        }
        "skill-source-lineage-invalid" => {
            "Trace Commons could not verify this skill against the selected session."
        }
        "skill-workflow-capacity" => {
            "Too many reviewed skill workflows are open. Finish or retry an existing workflow."
        }
        "skill-candidate-unknown" | "skill-review-unknown" | "skill-evaluation-unknown" => {
            "The local skill workflow expired. Start again from this session."
        }
        "skill-family-not-supported" => {
            "This correction does not match the first supported generated-file debugging family."
        }
        "skill-name-invalid" => "Use a lowercase hyphenated skill name with at most 64 characters.",
        "skill-description-required" | "skill-procedure-required" => {
            "Add the applicability and corrective procedure."
        }
        "skill-description-too-long" | "skill-procedure-too-long" => {
            "Shorten the highlighted skill field."
        }
        "skill-control-character" | "skill-sensitive-text" => {
            "Remove private data, credentials, wallet recovery text, and control characters from the skill."
        }
        "skill-evaluation-credential-required" => {
            "Add a NEAR AI inference key in Private AI settings, then retry."
        }
        "skill-evaluation-private-model-unavailable" => {
            "No ready NEAR AI private text model supports this evaluation."
        }
        "skill-evaluation-provider-rejected" => {
            "NEAR AI rejected the inference credential. Renew it in Private AI settings."
        }
        "skill-evaluation-funding-required" => {
            "Add NEAR AI Cloud credit to this account, then retry the skill test."
        }
        "skill-evaluation-model-changed" => {
            "The provider served different models across the comparison. Run the test again."
        }
        "skill-evaluation-in-progress" => {
            "This exact skill is already being tested. Wait for its result."
        }
        "skill-evaluation-busy" => "Another skill test is running. Retry after it finishes.",
        "skill-evaluation-credential-changed" => {
            "The inference credential changed during testing. Retry with the current credential."
        }
        "skill-evaluation-did-not-pass" => {
            "The skill must pass every applicability check and its repository plans must beat both controls without regressions before installation."
        }
        "skill-install-occupied" => {
            "A skill already occupies this path. Choose another name or remove it, then retry."
        }
        "skill-install-plan-changed" | "skill-review-changed" => {
            "The reviewed skill changed. Review the exact package again."
        }
        "skill-install-plan-unknown" => "The installation preview expired. Create a new preview.",
        "skill-install-modified" | "skill-install-extra-files" => {
            "The installed directory changed. Trace Commons refused rollback to protect those changes."
        }
        "skill-install-multiple-matches" => {
            "More than one installed skill claims this session. Inspect the Codex skills directory."
        }
        "skill-install-scan-limit" => {
            "The Codex skills directory is too large to verify safely. Inspect it before retrying."
        }
        "skill-install-not-owned" => "Trace Commons cannot verify ownership of this installation.",
        "skill-install-unknown" => {
            "Trace Commons could not find this installed skill. Its status will be checked again."
        }
        "skill-home-unavailable" | "skill-install-root-invalid" => {
            "Codex's local skills directory is unavailable."
        }
        "skill-install-write-failed" => {
            "The skill could not be written. Check the target directory permissions."
        }
        "skill-install-rollback-incomplete" => {
            "The install could not be fully rolled back. Inspect the installed skill before retrying."
        }
        _ => skill_learning_copy().unavailable,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn skill_copy_is_complete_and_unknown_errors_are_actionable() {
        let copy = skill_learning_copy();
        assert!(!copy.heading.is_empty());
        assert!(copy.contract_summary_format.contains("%1$d"));
        assert_eq!(copy.test_contract, "Held-out plan test");
        assert_eq!(copy.results, "Held-out plan results");
        assert_eq!(copy.passed_gate, "Passed plan gate");
        assert_eq!(copy.repository_plans, "Repository plans");
        assert_eq!(copy.skill_applicability, "Skill applicability");
        assert_eq!(copy.marker_digest, "Marker file SHA-256");
        assert!(copy.promise.contains("score its plans and applicability"));
        assert!(copy.testing.contains("plans"));
        assert!(copy.no_regressions.contains("repository plan"));
        assert_eq!(copy.edits, "Direct source edits");
        assert!(
            skill_learning_error_line("skill-evaluation-did-not-pass").contains("repository plans")
        );
        assert_eq!(skill_learning_error_line("unknown"), copy.unavailable);
    }

    #[test]
    fn funding_error_names_the_actionable_cloud_credit() {
        assert_eq!(
            skill_learning_error_line("skill-evaluation-funding-required"),
            "Add NEAR AI Cloud credit to this account, then retry the skill test."
        );
    }
}
