// INTEGRATION: exposes skill-learning copy and safe daemon error labels to the
// native shell through the contributor FFI boundary.

import CTraceCommons
import Foundation

/// Shared tested-skill wording and fixed error decisions. Nothing here
/// authors contributor-facing wording.
public enum TCSkillLearning {
    public static func copyJSON() -> String? {
        guard let raw = tc_skill_learning_copy() else { return nil }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    public static func errorLine(label: String) -> String? {
        guard let raw = label.withCString({ tc_skill_learning_error_line($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }

    public static func validateDraftJSON(_ json: String) -> String? {
        guard let raw = json.withCString({ tc_skill_draft_validate($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
