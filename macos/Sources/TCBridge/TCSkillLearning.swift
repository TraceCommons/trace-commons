// INTEGRATION: exposes skill-learning copy and safe daemon error labels to the
// native shell through the contributor FFI boundary.

import CTraceCommons
import Foundation
import TCLocalization

/// Shared tested-skill wording and fixed error decisions. Nothing here
/// authors contributor-facing wording.
public enum TCSkillLearning {
    public static func copyJSON() -> String? {
        guard let raw = tc_skill_learning_copy() else { return nil }
        defer { tc_string_free(raw) }
        return Localization.current.copyJSON(String(cString: raw), namespace: "skill_learning_copy")
    }

    public static func errorLine(label: String) -> String? {
        // Legacy equivalent: tc_skill_learning_error_line.
        LocalizedPresentation.line("skill_learning_error_line", parameters: ["label": label])
    }

    public static func contractSummaryLine(
        tasks: Int, cases: Int, requests: Int, outputTokens: Int, seconds: Int
    ) -> String? {
        LocalizedPresentation.line("skill_learning_contract_summary_line", parameters: [
            "tasks": tasks, "cases": cases, "requests": requests,
            "output_tokens": outputTokens, "seconds": seconds,
        ])
    }

    public static func modelBudgetLine(owner: String, tasks: Int, outputTokens: Int) -> String? {
        LocalizedPresentation.line("skill_learning_model_budget_line", parameters: [
            "owner": owner, "tasks": tasks, "output_tokens": outputTokens,
        ])
    }

    public static func reviewBudgetLine(requests: Int, outputTokens: Int, owner: String) -> String? {
        LocalizedPresentation.line("skill_learning_review_budget_line", parameters: [
            "requests": requests, "output_tokens": outputTokens, "owner": owner,
        ])
    }

    public static func validateDraftJSON(_ json: String) -> String? {
        guard let raw = json.withCString({ tc_skill_draft_validate($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
