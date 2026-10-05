import Foundation
import TCShellCore

/// The first run's daemon calls. The picker is backed by
/// `list_past_sessions` and `include_past_sessions`
/// (`crates/trace-commons-contributor/src/daemon/past_sessions.rs`).
extension DaemonClient {
    /// One folder's past sessions. An unknown project is the daemon's
    /// refusal (`project-id-unrecognized`), thrown, never an empty list.
    func listPastSessions(projectID: String) throws -> PastSessionList {
        try call(
            "list_past_sessions",
            params: ["project_id": projectID],
            as: PastSessionList.self
        )
    }

    /// Approve the chosen sessions as the person's own approval. The ids
    /// are sent as chosen: the daemon dedups, bounds and validates them.
    func includePastSessions(projectID: String, sessionIDs: [String]) throws -> IncludeOutcome {
        try call(
            "include_past_sessions",
            params: ["project_id": projectID, "session_ids": sessionIDs],
            as: IncludeOutcome.self
        )
    }
}
