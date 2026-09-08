import Foundation

/// The one action a shell may offer for a credential state, decoded from
/// `TC_CREDENTIAL_ACTION_*`.
///
/// A range of its own, disjoint from every tone range, and the arms are
/// spelled out rather than derived from declaration order. Anything unknown
/// is `.none`, and that is not the usual "safe default" hand-wave: `.obtain`
/// opens a browser and mints a key at a third party, so drawing it for a
/// state nobody could read is how a contributor ends up holding a second key
/// in an account nothing on this screen will ever mention again.
public enum CredentialAction: Equatable, Sendable {
    case none
    case obtain
    case cancel
    case forget

    public static func fromABI(_ value: Int32) -> CredentialAction {
        switch value {
        case 31: return .obtain
        case 32: return .cancel
        case 33: return .forget
        default: return .none
        }
    }
}

/// `near_ai_credential_status` as the daemon answers it.
///
/// The state is carried as the daemon's own string and never parsed into a
/// Swift enum, for the reason `PrivateInferenceState` carries its label that
/// way: a state a later daemon grows would otherwise have to be spelled here
/// before it could be shown, and the shared tables already answer an
/// unfamiliar label safely.
///
/// `attemptID` and `attemptStatus` arrive only when the caller named the
/// CURRENT attempt, so both are optional and neither is required to render
/// anything. The field is `attempt_status` and not `status`: the daemon
/// deliberately did not put a `status` beside a `state`, because a shell
/// reading the wrong one of those two is the failure the naming prevents.
public struct CredentialStatus: Equatable, Sendable {
    public let state: String
    public let attemptID: String?
    public let attemptStatus: String?

    public init(state: String, attemptID: String? = nil, attemptStatus: String? = nil) {
        self.state = state
        self.attemptID = attemptID
        self.attemptStatus = attemptStatus
    }

    /// What a payload this build cannot read says: that the question was not
    /// answered. NOT that no key is kept here -- see `credentialUnreported`.
    public static let unreported = CredentialStatus(state: "")

    /// From the method's own result object.
    ///
    /// A missing or unreadable `state` reads as the empty label, which the
    /// shared table answers as unreported and the shared action table answers
    /// as no action at all.
    public static func parse(fromJSON json: String) -> CredentialStatus {
        guard let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
        else { return .unreported }
        return CredentialStatus(
            state: object["state"] as? String ?? "",
            attemptID: object["attempt_id"] as? String,
            attemptStatus: object["attempt_status"] as? String)
    }
}

/// What `near_ai_credential_start` handed back.
///
/// `browserURL` is served ONCE, by start, and no poll re-serves it -- so a
/// shell that means to open it later has to keep this, and a shell that lost
/// it has to begin again rather than expect a status call to hand it over.
public struct CredentialAttempt: Equatable, Sendable {
    public let attemptID: String
    public let browserURL: String
    /// The ceremony's own lifecycle word, spelled `status` on THIS method and
    /// `attempt_status` on the status method. Both are the same word; only
    /// the status method has a `state` beside it to be confused with.
    public let status: String

    public init(attemptID: String, browserURL: String, status: String) {
        self.attemptID = attemptID
        self.browserURL = browserURL
        self.status = status
    }

    /// All or nothing. An attempt with no id cannot be cancelled and an
    /// attempt with no URL cannot be finished, and either half missing means
    /// the ceremony did not begin in a way this shell can carry through.
    public static func parse(fromJSON json: String) -> CredentialAttempt? {
        guard let data = json.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
            let attemptID = object["attempt_id"] as? String, !attemptID.isEmpty,
            let browserURL = object["browser_url"] as? String, !browserURL.isEmpty
        else { return nil }
        return CredentialAttempt(
            attemptID: attemptID, browserURL: browserURL,
            status: object["status"] as? String ?? "")
    }
}

/// The three shared tables this surface reads across the C ABI, injected so
/// `TCShellCore` can be tested without linking the dylib.
///
/// All three take the daemon's LABEL, not a decoded state, so a state a later
/// daemon grows never has to be spelled in Swift before it can be shown.
/// Production wiring is `TCNearAiCredential`; see `AppModel`.
public struct CredentialCalls: Sendable {
    /// The sentence for one state label.
    public let stateLine: @Sendable (String) -> String?
    /// How firmly that sentence reads, as a raw `TC_PRIVATE_INFERENCE_TONE_*`
    /// value. Never recovered by reading the sentence.
    public let stateTone: @Sendable (String) -> Int32
    /// The one action offered for that state, as a raw
    /// `TC_CREDENTIAL_ACTION_*` value.
    public let action: @Sendable (String) -> Int32
    /// Why a connect control is not on offer, for `harness_list`'s
    /// `destination_credentialed` as a tri-state.
    public let harnessNotice: @Sendable (Int32) -> String

    public init(
        stateLine: @escaping @Sendable (String) -> String?,
        stateTone: @escaping @Sendable (String) -> Int32,
        action: @escaping @Sendable (String) -> Int32,
        harnessNotice: @escaping @Sendable (Int32) -> String
    ) {
        self.stateLine = stateLine
        self.stateTone = stateTone
        self.action = action
        self.harnessNotice = harnessNotice
    }
}

/// What this shell renders about the key that lets this destination answer.
///
/// Holds no words and takes no branch of its own. Every sentence is a field
/// of `PrivateInferenceCopy` or comes back from `calls`, and the three
/// decisions that matter -- which sentence, which tone, which button -- are
/// all the shared tables'. A `switch` here would be the third copy of a
/// decision that draws a button which mints a key at a third party.
public enum CredentialSurface {
    /// The IPC methods, named once.
    public static let statusMethod = "near_ai_credential_status"
    public static let startMethod = "near_ai_credential_start"
    public static let cancelMethod = "near_ai_credential_cancel"
    public static let forgetMethod = "near_ai_credential_forget"

    // MARK: - The state

    /// The sentence under the title. Falls back to the payload's unreported
    /// sentence when the Rust caught a panic -- the sentence that says the
    /// question was not answered, never one that says what is kept here.
    public static func stateLine(
        _ status: CredentialStatus, copy: PrivateInferenceCopy, calls: CredentialCalls
    ) -> String {
        calls.stateLine(status.state) ?? copy.credentialUnreported
    }

    /// The tone that sentence is painted in.
    ///
    /// `PrivateInferenceTone` is reused rather than duplicated, which is what
    /// the ABI asks for: a shell maps those five values onto colours once,
    /// and a second enum with the same five meanings is a second mapping to
    /// keep in agreement.
    public static func tone(
        _ status: CredentialStatus, calls: CredentialCalls
    ) -> PrivateInferenceTone {
        PrivateInferenceTone.fromABI(calls.stateTone(status.state))
    }

    // MARK: - The one action

    /// The button this state may offer, or none at all.
    public static func action(
        _ status: CredentialStatus, calls: CredentialCalls
    ) -> CredentialAction {
        CredentialAction.fromABI(calls.action(status.state))
    }

    /// That button's words, from the payload. `nil` for `.none`, which draws
    /// no button rather than a disabled one -- there is nothing to enable.
    public static func actionLabel(
        _ action: CredentialAction, copy: PrivateInferenceCopy
    ) -> String? {
        switch action {
        case .none: return nil
        case .obtain: return copy.credentialObtain
        case .cancel: return copy.credentialCancel
        case .forget: return copy.credentialForget
        }
    }

    /// Whether this shell can actually address the offered action to the
    /// daemon.
    ///
    /// FALSE IS AN ORDINARY CASE, NOT AN EDGE ONE. `near_ai_credential_status`
    /// resolves `obtaining` from the ceremony the daemon is holding, with no
    /// attempt id required -- so a shell that started after the ceremony did,
    /// which is any app restarted while the daemon kept running, reads
    /// `obtaining` and is offered Cancel. But `near_ai_credential_cancel`
    /// REQUIRES the attempt id, and this shell has none to give.
    ///
    /// A control drawn live that silently does nothing is the worst of the
    /// three answers, so the card asks this and refuses to enable what it
    /// cannot send. It lives here rather than in the SwiftUI body so it is
    /// exercised off-platform, and not left resting on a binding only CI can
    /// see.
    ///
    /// This is a consistency fix and NOT a correctness one. The daemon can
    /// cancel the ceremony it is holding; it simply refuses to be asked
    /// without an id. So a disabled Cancel still tells a contributor
    /// something false -- that this sign-in cannot be stopped from here. The
    /// honest fix is daemon-side, and #728 tracks it; if it lands, all three
    /// shells should go back to drawing Cancel live, because it would then
    /// always work.
    ///
    /// Every other action stands on its own: Obtain and Forget name no
    /// attempt, so an absent id says nothing about them, and `.none` has no
    /// button to enable in the first place.
    public static func canAddress(_ action: CredentialAction, attemptID: String?) -> Bool {
        guard action == .cancel else { return true }
        guard let attemptID else { return false }
        return !attemptID.isEmpty
    }

    /// The sentence that must accompany the button, or none.
    ///
    /// Returned FROM THE ACTION rather than left to a view's own `if`, so the
    /// pairing cannot be broken by a layout change. `.obtain` carries the
    /// three consequences of pressing it; `.forget` carries the fact that
    /// forgetting is local and the key stays valid until the contributor
    /// removes it in their own account. `.cancel` stops a ceremony this app
    /// started and costs nothing, and `.none` has no button to qualify.
    public static func actionExplains(
        _ action: CredentialAction, copy: PrivateInferenceCopy
    ) -> String? {
        switch action {
        case .obtain: return copy.credentialCost
        case .forget: return copy.credentialForgetExplains
        case .cancel, .none: return nil
        }
    }

    // MARK: - The tools that need one

    /// Why a connect control is not on offer, or nothing at all.
    ///
    /// `credentialed` is `harness_list`'s `destination_credentialed`, and it
    /// is a tri-state. AN ABSENT FIELD IS NOT A REFUSED CONNECT: a daemon
    /// that predates the credential gate reports nothing, and a destination
    /// the contributor runs themselves reports `true`. Both draw no sentence,
    /// because telling somebody to sign in before connecting a tool they can
    /// connect right now would be false. Collapsing the absence into `false`
    /// is what would make it false, which is why this crosses the ABI as
    /// three values rather than two.
    public static func harnessNotice(
        credentialed: Bool?, calls: CredentialCalls
    ) -> String? {
        let sentence = calls.harnessNotice(HarnessList.credentialedABIValue(credentialed))
        return sentence.isEmpty ? nil : sentence
    }

    // MARK: - The calls

    /// `near_ai_credential_status` names the attempt when this shell has one.
    ///
    /// Naming it is what buys back `attempt_id` and `attempt_status`; a poll
    /// that names none still gets `state`, which is the fact about the
    /// machine and is not a secret.
    public static func statusParams(attemptID: String?) -> [String: Any] {
        guard let attemptID, !attemptID.isEmpty else { return [:] }
        return ["attempt_id": attemptID]
    }

    /// `near_ai_credential_cancel` REQUIRES the attempt id. There is no
    /// cancel-whatever-is-running: an attempt this shell cannot name is one
    /// it did not start.
    public static func cancelParams(attemptID: String) -> [String: Any] {
        ["attempt_id": attemptID]
    }
}
