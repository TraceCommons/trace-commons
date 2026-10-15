import Foundation
import TCShellCore
#if canImport(NearSignerClient)
import NearSignerClient
import NearSignerKit
#endif

/// The daemon half of the signer route, over the app's own client. A daemon
/// refusal is passed on by its control label only.
final class DaemonSignerBridge: NearSignerDaemon, @unchecked Sendable {
    private let client: DaemonClient
    init(client: DaemonClient) { self.client = client }

    func nativePayload(flowID: String) async throws -> NearSignerPayload {
        try await Task.detached { [client] in
            do { return try client.nativeWalletPayload(flowID: flowID) }
            catch { throw NearSignerRefusal(label: DaemonSignerBridge.label(error)) }
        }.value
    }

    func nativeComplete(flowID: String, proof: NearSignerProof) async throws {
        try await Task.detached { [client] in
            do { _ = try client.nativeWalletComplete(flowID: flowID, proof: proof) }
            catch { throw NearSignerRefusal(label: DaemonSignerBridge.label(error)) }
        }.value
    }

    private static func label(_ error: Error) -> String {
        (error as? DaemonClient.Failure)?.message ?? "native_wallet_state"
    }
}

#if canImport(NearSignerClient)
/// The signer half: the installed app, over XPC.
struct InstalledSigner: NearSignerSigning {
    func signIn(_ payload: NearSignerPayload) async throws -> NearSignerProof {
        let wire = NEP413Payload(
            message: payload.message, recipient: payload.recipient,
            nonce: payload.nonce, callbackURL: nil)
        do {
            let signed = try await NearSignerClient().signIn(wire)
            return NearSignerProof(
                accountID: signed.accountID, publicKey: signed.publicKey,
                signature: signed.signatureBase64)
        } catch let refusal as Refusal {
            throw NearSignerRefusal(label: refusal.rawValue)
        }
    }
}
#endif

/// Whether this build can talk to the signer at all.
let nearSignerCompiledIn: Bool = {
    #if canImport(NearSignerClient)
    return true
    #else
    return false
    #endif
}()
