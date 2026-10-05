// The first run's daemon calls, kept out of the general client. Each block
// below is one self-contained extension over the shared `call(_:params:as:)`.

import Foundation
import TCShellCore

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
