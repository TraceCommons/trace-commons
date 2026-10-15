import AppKit
import Foundation
import Security

// Which way the NEAR wallet step signs in: through the NEAR signer app, or
// through the browser. The signer route is opt-in (a setting, off by default)
// and needs the signer app installed AND signed by the pinned team. A refusal
// from the signer never falls back to the browser on its own; the person
// chooses it. Errors are labels only: no payload, account, key or signature is
// ever put in a Result, a log line or a refusal.

public enum WalletRoute: Equatable, Sendable {
    case nativeSigner
    case browser
}

public let nearSignerBundleID = "com.iqlusion.near-signer"
public let nearSignerTeamID = "KXSWJN7WY8"

/// The designated requirement the installed signer must satisfy.
public let nearSignerCodeRequirement =
    "anchor apple generic and identifier \"\(nearSignerBundleID)\" and certificate leaf[subject.OU] = \"\(nearSignerTeamID)\""

/// UserDefaults key of the "Use the NEAR signer app" setting.
public let nearSignerSettingKey = "trace_commons.near_signer.enabled"

public func chooseRoute(settingOn: Bool, signerInstalled: Bool) -> WalletRoute {
    settingOn && signerInstalled ? .nativeSigner : .browser
}

/// True when an app with the signer's bundle ID is installed and its code
/// signature is valid and satisfies `nearSignerCodeRequirement`. Any failure
/// reads as "not installed".
public func signerInstalled() -> Bool {
    guard let url = NSWorkspace.shared.urlForApplication(withBundleIdentifier: nearSignerBundleID) else {
        return false
    }
    return codeSatisfiesSignerRequirement(at: url)
}

func codeSatisfiesSignerRequirement(at url: URL) -> Bool {
    var code: SecStaticCode?
    guard SecStaticCodeCreateWithPath(url as CFURL, [], &code) == errSecSuccess, let code else { return false }
    var requirement: SecRequirement?
    guard SecRequirementCreateWithString(nearSignerCodeRequirement as CFString, [], &requirement) == errSecSuccess,
          let requirement else { return false }
    let flags = SecCSFlags(rawValue: kSecCSCheckAllArchitectures | kSecCSStrictValidate)
    return SecStaticCodeCheckValidity(code, flags, requirement) == errSecSuccess
}

// MARK: - Running the signer route

/// What the daemon asks to be signed (`native_payload`).
public struct NearSignerPayload: Decodable, Equatable, Sendable {
    public let message: String
    public let recipient: String
    public let nonce: Data

    public init(message: String, recipient: String, nonce: Data) {
        self.message = message
        self.recipient = recipient
        self.nonce = nonce
    }

    enum CodingKeys: String, CodingKey { case message, recipient, nonce }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        message = try c.decode(String.self, forKey: .message)
        recipient = try c.decode(String.self, forKey: .recipient)
        guard let nonce = Data(base64Encoded: try c.decode(String.self, forKey: .nonce)) else {
            throw DecodingError.dataCorruptedError(forKey: .nonce, in: c, debugDescription: "nonce")
        }
        self.nonce = nonce
    }
}

/// What the signer returns, handed to the daemon (`native_complete`).
public struct NearSignerProof: Equatable, Sendable {
    public let accountID: String
    public let publicKey: String
    public let signature: String

    public init(accountID: String, publicKey: String, signature: String) {
        self.accountID = accountID
        self.publicKey = publicKey
        self.signature = signature
    }
}

/// A refusal from the signer or the daemon, by label only.
public struct NearSignerRefusal: Error, Equatable, Sendable {
    public let label: String
    public init(label: String) { self.label = label }
}

public protocol NearSignerSigning: Sendable {
    func signIn(_ payload: NearSignerPayload) async throws -> NearSignerProof
}

public protocol NearSignerDaemon: Sendable {
    func nativePayload(flowID: String) async throws -> NearSignerPayload
    func nativeComplete(flowID: String, proof: NearSignerProof) async throws
}

/// Gets the flow's payload from the daemon, has the signer sign it, and hands
/// the proof back. Any failure is returned as a label; none leads to the
/// browser route.
public func runNativeSigner(
    flowID: String,
    daemon: some NearSignerDaemon,
    signer: some NearSignerSigning
) async -> Result<Void, NearSignerRefusal> {
    do {
        let payload = try await daemon.nativePayload(flowID: flowID)
        let proof = try await signer.signIn(payload)
        try await daemon.nativeComplete(flowID: flowID, proof: proof)
        return .success(())
    } catch let refusal as NearSignerRefusal {
        return .failure(refusal)
    } catch {
        return .failure(NearSignerRefusal(label: "near_signer_unavailable"))
    }
}

// COPY: needs owner approval
public enum NearSignerCopy {
    public static let settingTitle = "Use the NEAR signer app"
    public static let settingCaption =
        "Sign in to NEAR with the NEAR signer app when it is installed. Otherwise the browser is used."
    public static let useBrowserInstead = "Use the browser instead"
    public static let failed = "The NEAR signer app did not sign in."
}
