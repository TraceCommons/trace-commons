import CTraceCommons
import Foundation
import TCLocalization

/// Shared copy and error decisions for owned session detail and reviewed
/// publication. Nothing here authors contributor-facing wording.
public enum TCPublicRun {
    public static func copyJSON() -> String? {
        guard let raw = tc_public_run_copy() else { return nil }
        defer { tc_string_free(raw) }
        return Localization.current.copyJSON(String(cString: raw), namespace: "public_run_copy")
    }

    public static func sessionDetailErrorLine(label: String) -> String? {
        // Legacy equivalent: tc_session_detail_error_line.
        LocalizedPresentation.line("session_detail_error_line", parameters: ["label": label])
    }

    public static func publicationErrorLine(label: String) -> String? {
        // Legacy equivalent: tc_public_run_error_line.
        LocalizedPresentation.line("public_run_error_line", parameters: ["label": label])
    }

    public static func validateEditorJSON(_ json: String) -> String? {
        guard let raw = json.withCString({ tc_public_run_validate_editor($0) }) else {
            return nil
        }
        defer { tc_string_free(raw) }
        return String(cString: raw)
    }
}
