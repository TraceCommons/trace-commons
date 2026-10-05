// The first run's daemon calls, kept out of the general client. Each block
// below is one self-contained extension over the shared `call(_:params:as:)`.

import Foundation
import TCShellCore

// MARK: - Past-session picker

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

// MARK: - Flow 1 grant and invite lookup

extension DaemonClient {
    /// `grant_automatic`, after the contributor pressed the grant button.
    /// `witnessSigningAddress` is the witness the disclosure screen showed,
    /// from `Flow1GrantRequest`; nil is sent as JSON null, never omitted,
    /// because the daemon refuses a request that does not say.
    func grantAutomatic(witnessSigningAddress: String?) throws -> AutomaticGrant {
        let witness: Any = witnessSigningAddress ?? NSNull()
        return try call(
            "grant_automatic",
            params: ["confirmed": true, "witness_signing_address": witness],
            as: AutomaticGrant.self
        )
    }

    /// `invite_lookup`: the issuer and pay range an invite names. The whole
    /// invite goes as `code`; the daemon parses it.
    func inviteLookup(_ invite: String) throws -> DaemonData.InviteLookup {
        try call("invite_lookup", params: ["code": invite], as: DaemonData.InviteLookup.self)
    }
}
