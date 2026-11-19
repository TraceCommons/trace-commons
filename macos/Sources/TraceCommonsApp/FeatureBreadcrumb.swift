import Foundation
import os

/// Fixed-label lines in the unified log about feature flags and the Claude
/// Desktop status, so a missing flag-gated surface can be diagnosed on a
/// signed build without a debugger: whether the app's flag read succeeded
/// (and its refusal label if not), the Claude Desktop decision, and each
/// Claude Desktop status read's state or refusal label. Labels only: no
/// content, no model ids, no paths. Read with
/// `log show --predicate 'subsystem == "<the app's bundle id>" AND category == "features"'`.
enum FeatureBreadcrumb {
    static let logger = Logger(
        subsystem: Bundle.main.bundleIdentifier ?? ProcessInfo.processInfo.processName,
        category: "features")

    static func flagsRead(claudeDesktop: Bool, source: String) {
        logger.notice("flags read: claude_desktop=\(claudeDesktop, privacy: .public) source=\(source, privacy: .public)")
    }

    static func flagsFailed(_ error: Error?) {
        logger.notice("flags read failed: \(label(error), privacy: .public)")
    }

    static func claudeDesktopStatus(_ state: String) {
        logger.notice("claude desktop status: \(state, privacy: .public)")
    }

    static func claudeDesktopFailed(_ error: Error?) {
        logger.notice("claude desktop status failed: \(label(error), privacy: .public)")
    }

    /// The daemon's fixed code and label, or the error's type name.
    static func label(_ error: Error?) -> String {
        guard let error else { return "none" }
        if let failure = error as? DaemonClient.Failure { return failure.code + ": " + failure.message }
        return String(describing: type(of: error))
    }
}
