import Foundation

/// `tc_preview_turns_json`: the turn index over an open preview's redacted
/// body, anchored to that body's digest. Decoded by `TCShellCore.PreviewTurns`.
///
/// The raw call is `TCDaemon.previewTurns(entryID:bodyDigest:)`, because
/// `TCDaemon` is the only place the handle pointer appears; this is the
/// shell's entry to it.
public enum TCPreviewTurns {
    /// The export's fixed, content-free refusal label (for example
    /// `preview-body-changed`, `entry-id-invalid`, or
    /// `invalid-handle-pointer`). Matched or logged, never shown.
    public struct Refusal: Error, Equatable, Sendable {
        public let label: String

        public init(label: String) {
            self.label = label
        }
    }

    /// The index as JSON, or nil when the export refused or the daemon is
    /// gone. Nil is never an empty index.
    public static func turnsJSON(daemon: TCDaemon, entryID: String, bodyDigest: String) -> String? {
        try? daemon.previewTurns(entryID: entryID, bodyDigest: bodyDigest).get()
    }
}
