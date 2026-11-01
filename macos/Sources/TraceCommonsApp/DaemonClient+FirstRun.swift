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
    /// The include-existing grant, after the contributor pressed Start.
    /// Its distinct method name makes older daemons refuse before mutation.
    /// `witnessSigningAddress` is the witness the disclosure screen showed,
    /// from `Flow1GrantRequest`; nil is sent as JSON null, never omitted,
    /// because the daemon refuses a request that does not say. Finishing
    /// onboarding explicitly includes eligible history from watched folders;
    /// other grant callers keep their existing from-now semantics.
    ///
    /// `keepFolderAsks`: a folder already set to Ask keeps asking (owner,
    /// 2026-10-30, and #1753's ruling that a folder's own rule always wins);
    /// false makes every folder but a Never one Automatic.
    func grantAutomatic(witnessSigningAddress: String?, keepFolderAsks: Bool = true) throws -> AutomaticGrant {
        let witness: Any = witnessSigningAddress ?? NSNull()
        let grant = try call(
            "grant_automatic_including_existing",
            params: ["confirmed": true, "witness_signing_address": witness, "include_existing": true,
                     "keep_folder_asks": keepFolderAsks],
            as: AutomaticGrant.self
        )
        guard !grant.granted || grant.includeExisting == true else {
            throw Failure(code: "unavailable", message: "automatic-existing-traces-unsupported")
        }
        return grant
    }

    /// `automatic_grant`: whether the standing Automatic grant is in force,
    /// for Settings' sharing choice.
    func automaticGrant() throws -> AutomaticGrant {
        try call("automatic_grant", as: AutomaticGrant.self)
    }

    /// `first_run_estimate`: the commons' dollar estimate per accepted
    /// trace, or no range. Always answers once the daemon runs.
    func firstRunEstimate() throws -> DaemonData.FirstRunEstimate {
        try call("first_run_estimate", as: DaemonData.FirstRunEstimate.self)
    }

    /// `invite_lookup`: the issuer and pay range an invite names. The whole
    /// invite goes as `code`; the daemon parses it.
    func inviteLookup(_ invite: String) throws -> DaemonData.InviteLookup {
        try call("invite_lookup", params: ["code": invite], as: DaemonData.InviteLookup.self)
    }
}
