import Foundation

// Changing the NEAR AI login an account uses (F10). Spec:
// `docs/superpowers/specs/2026-11-03-f10-change-account-design.md`,
// section 2. The daemon runs the ceremony (`account_login_change`); this
// file is the shell's pure half: the words, the sheet's page for each
// state, and the IPC calls, all testable without a daemon.

/// The change words, decoded from what the Rust exported
/// (`tc_account_change_copy_json`).
///
/// Every string is owner-approved (2026-11-03; the mismatch body
/// 2026-11-02). None is written in Swift: a payload with a missing or empty
/// field is refused whole. Templates carry `{provider}`, `{old_provider}` or
/// `{new_provider}`, a login's title, filled by `render(_:provider:old:new:)`.
public struct AccountChangeCopy: Decodable, Equatable, Sendable {
    public let mismatchBody: String
    public let mismatchSignIn: String
    public let mismatchChange: String
    public let change: String
    public let payoutsNeedsLoginBody: String
    public let payoutsNeedsLoginAdd: String
    public let title: String
    public let body: String
    public let bodyOldUnknown: String
    public let bodyWallet: String
    public let privateAI: String
    public let confirm: String
    public let cancel: String
    public let stepCurrent: String
    public let stepCurrentWallet: String
    public let stepNew: String
    public let errorCurrentLogin: String
    public let errorWallet: String
    public let errorInUse: String
    public let errorTooSoon: String
    public let errorUnavailable: String
    public let errorRetired: String
    public let stepCurrentAccessibilityName: String
    public let stepNewAccessibilityName: String
    public let confirmAccessibilityName: String
    public let providerNear: String
    public let providerGithub: String
    public let providerGoogle: String

    enum CodingKeys: String, CodingKey {
        case change, title, body, confirm, cancel
        case mismatchBody = "mismatch_body"
        case mismatchSignIn = "mismatch_sign_in"
        case mismatchChange = "mismatch_change"
        case payoutsNeedsLoginBody = "payouts_needs_login_body"
        case payoutsNeedsLoginAdd = "payouts_needs_login_add"
        case bodyOldUnknown = "body_old_unknown"
        case bodyWallet = "body_wallet"
        case privateAI = "private_ai"
        case stepCurrent = "step_current"
        case stepCurrentWallet = "step_current_wallet"
        case stepNew = "step_new"
        case errorCurrentLogin = "error_current_login"
        case errorWallet = "error_wallet"
        case errorInUse = "error_in_use"
        case errorTooSoon = "error_too_soon"
        case errorUnavailable = "error_unavailable"
        case errorRetired = "error_retired"
        case stepCurrentAccessibilityName = "step_current_accessibility_name"
        case stepNewAccessibilityName = "step_new_accessibility_name"
        case confirmAccessibilityName = "confirm_accessibility_name"
        case providerNear = "provider_near"
        case providerGithub = "provider_github"
        case providerGoogle = "provider_google"
    }

    /// The payload fields this shell decodes, by wire name; compared with the
    /// live export by `TCBridgeTests`.
    public static let consumedFields = [
        "mismatch_body", "mismatch_sign_in", "mismatch_change", "change",
        "payouts_needs_login_body", "payouts_needs_login_add", "title", "body",
        "body_old_unknown", "body_wallet", "private_ai", "confirm", "cancel", "step_current",
        "step_current_wallet", "step_new", "error_current_login", "error_wallet", "error_in_use",
        "error_too_soon", "error_unavailable", "error_retired", "step_current_accessibility_name",
        "step_new_accessibility_name", "confirm_accessibility_name", "provider_near",
        "provider_github", "provider_google",
    ]

    public var sentences: [String] {
        [
            mismatchBody, mismatchSignIn, mismatchChange, change, payoutsNeedsLoginBody,
            payoutsNeedsLoginAdd, title, body, bodyOldUnknown, bodyWallet, privateAI, confirm,
            cancel, stepCurrent, stepCurrentWallet, stepNew, errorCurrentLogin, errorWallet,
            errorInUse, errorTooSoon, errorUnavailable, errorRetired, stepCurrentAccessibilityName,
            stepNewAccessibilityName, confirmAccessibilityName, providerNear, providerGithub,
            providerGoogle,
        ]
    }

    /// Nil, never a partly-filled value, for an unparseable payload or an
    /// empty field.
    public static func decode(fromJSON json: String) -> AccountChangeCopy? {
        guard let data = json.data(using: .utf8),
            let copy = try? JSONDecoder().decode(AccountChangeCopy.self, from: data)
        else { return nil }
        return copy.sentences.contains(where: { $0.trimmingCharacters(in: .whitespaces).isEmpty })
            ? nil : copy
    }

    /// A login's title by its tag (`near`, `github`, `google`), or nil for a
    /// tag this build does not know.
    public func providerTitle(_ tag: String?) -> String? {
        switch tag {
        case "near": return providerNear
        case "github": return providerGithub
        case "google": return providerGoogle
        default: return nil
        }
    }

    /// `template` with its login placeholders filled by title. Nil when the
    /// template names a login that is missing or unknown, so a raw tag or a
    /// placeholder is never shown.
    public func render(
        _ template: String, provider: String? = nil, old: String? = nil, new: String? = nil
    ) -> String? {
        var text = template
        for (placeholder, tag) in [("{provider}", provider), ("{old_provider}", old), ("{new_provider}", new)]
        where text.contains(placeholder) {
            guard let title = providerTitle(tag) else { return nil }
            text = text.replacingOccurrences(of: placeholder, with: title)
        }
        return text
    }
}

/// The change fields of `status` (spec 2.4). Absent on an older daemon,
/// which then offers nothing.
public struct AccountLoginChangeStatus: Equatable, Sendable {
    /// This device may offer "Use a different account".
    public let available: Bool
    /// A wallet-origin account: its payouts need a NEAR AI login.
    public let payoutsNeedLogin: Bool
    /// A login the commons refused as another account is held aside: the
    /// account section shows the mismatch block with the change button.
    public let held: Bool
    /// The last NEAR AI sign-in was refused as another account: the account
    /// section shows the approved mismatch body, whether or not the change
    /// is offered (it is offered only with `held`).
    public let mismatch: Bool

    public static let none = AccountLoginChangeStatus(
        available: false, payoutsNeedLogin: false, held: false, mismatch: false)

    public init(available: Bool, payoutsNeedLogin: Bool, held: Bool, mismatch: Bool = false) {
        self.available = available
        self.payoutsNeedLogin = payoutsNeedLogin
        self.held = held
        self.mismatch = mismatch || held
    }

    /// Whether the mismatch block offers "Use a different account": only
    /// with a refused login held aside for it.
    public var offersChange: Bool { available && held }

    /// Whether the payouts section shows "Add a NEAR AI login".
    public var showsPayoutsLine: Bool { available && payoutsNeedLogin }
}

/// Which "Use a different account" button a surface draws (F10, spec 2.1).
public enum AccountLoginChangeEntry: Equatable, Sendable {
    /// Entry point 2, deliberately: no refused login is held aside.
    case change
    /// Entry point 1's button: a login the commons refused as another
    /// account is held aside for the change.
    case mismatchChange
}

extension AccountLoginChangeStatus {
    /// The entry point a surface draws, by one rule wherever it is drawn
    /// (the account section, Private AI): the mismatch's button while a
    /// refused login is held for the change; otherwise the deliberate one
    /// on an enrolled app where the change is offered, unless a wallet-origin
    /// account's payouts line already offers it. Nil draws nothing.
    public func entry(loggedIn: Bool) -> AccountLoginChangeEntry? {
        if offersChange { return .mismatchChange }
        if loggedIn, available, !held, !payoutsNeedLogin { return .change }
        return nil
    }
}

extension AccountChangeCopy {
    /// The approved label for an entry point's button.
    public func label(for entry: AccountLoginChangeEntry) -> String {
        switch entry {
        case .change: return change
        case .mismatchChange: return mismatchChange
        }
    }
}

/// One answer of `account_login_change`: the ceremony as the daemon holds it.
public struct AccountLoginChangeState: Decodable, Equatable, Sendable {
    public enum Step: String, Decodable, Sendable {
        case current, new, confirm, done
    }

    public let changeID: String
    public let step: Step
    /// `wallet` or `near_ai_login`.
    public let currentKind: String
    public let oldProvider: String?
    public let newProvider: String?
    /// A browser leg or a commons call is running.
    public let busy: Bool
    /// `current_login`, `wallet`, `in_use`, `too_soon` or `unavailable`.
    public let error: String?
    /// Only on the answer that began a browser leg.
    public let browserURL: String?

    enum CodingKeys: String, CodingKey {
        case step, busy, error
        case changeID = "change_id"
        case currentKind = "current_kind"
        case oldProvider = "old_provider"
        case newProvider = "new_provider"
        case browserURL = "browser_url"
    }

    public init(
        changeID: String, step: Step, currentKind: String, oldProvider: String? = nil,
        newProvider: String? = nil, busy: Bool = false, error: String? = nil,
        browserURL: String? = nil
    ) {
        self.changeID = changeID
        self.step = step
        self.currentKind = currentKind
        self.oldProvider = oldProvider
        self.newProvider = newProvider
        self.busy = busy
        self.error = error
        self.browserURL = browserURL
    }

    public var isWallet: Bool { currentKind == "wallet" }
}

/// What the sheet shows for one state. Pure: every sentence comes from the
/// core's copy, every fact from the daemon.
public struct AccountLoginChangePage: Equatable, Sendable {
    public enum Action: Equatable, Sendable {
        /// Step 1: prove the current identity (a browser leg).
        case proveCurrent
        /// Step 2: sign in with the new login (a browser leg).
        case signInNew
        /// Step 3: the person confirms.
        case confirm
        /// Done: close the sheet.
        case close
    }

    public let title: String
    /// The step's heading, when it has one.
    public let heading: String?
    /// Lines under the heading, in order.
    public let lines: [String]
    /// The step's primary button, with its own accessibility name.
    public let primary: String
    public let primaryAccessibilityName: String
    public let action: Action
    /// Step 1 of a wallet-origin account: the NEAR account name field.
    public let asksWalletAccount: Bool
    /// The step's refusal, when the last try failed.
    public let error: String?
    public let busy: Bool
    public let cancel: String
}

public enum AccountLoginChangeSurface {
    /// The sheet's page for `state`. Nil only when the words for this state
    /// cannot be drawn (a template names a login nobody knows), so the
    /// shell closes rather than show a placeholder.
    public static func page(_ state: AccountLoginChangeState, copy: AccountChangeCopy)
        -> AccountLoginChangePage?
    {
        let error = errorLine(state, copy: copy)
        switch state.step {
        case .current:
            let lines = state.isWallet ? [copy.stepCurrentWallet] : []
            let name =
                state.isWallet
                ? copy.stepCurrent
                : (copy.render(copy.stepCurrentAccessibilityName, old: state.oldProvider)
                    ?? copy.stepCurrent)
            return AccountLoginChangePage(
                title: copy.title, heading: copy.stepCurrent, lines: lines,
                primary: copy.stepCurrent, primaryAccessibilityName: name, action: .proveCurrent,
                asksWalletAccount: state.isWallet, error: error, busy: state.busy,
                cancel: copy.cancel)
        case .new:
            return AccountLoginChangePage(
                title: copy.title, heading: copy.stepNew, lines: [], primary: copy.stepNew,
                primaryAccessibilityName: copy.stepNewAccessibilityName, action: .signInNew,
                asksWalletAccount: false, error: error, busy: state.busy, cancel: copy.cancel)
        case .confirm:
            guard let body = confirmBody(state, copy: copy),
                let privateAI = copy.render(copy.privateAI, new: state.newProvider),
                let name = copy.render(copy.confirmAccessibilityName, new: state.newProvider)
            else { return nil }
            return AccountLoginChangePage(
                title: copy.title, heading: nil, lines: [body, privateAI], primary: copy.confirm,
                primaryAccessibilityName: name, action: .confirm, asksWalletAccount: false,
                error: error, busy: state.busy, cancel: copy.cancel)
        case .done:
            // No separate confirmation (spec 2.2): the account section says
            // which login is connected. The sheet closes.
            return AccountLoginChangePage(
                title: copy.title, heading: nil, lines: [], primary: copy.cancel,
                primaryAccessibilityName: copy.cancel, action: .close, asksWalletAccount: false,
                error: nil, busy: false, cancel: copy.cancel)
        }
    }

    /// Step 3's body: the wallet body for a wallet-origin account, the body
    /// naming both logins for a NEAR AI account, or the body for an old
    /// login nobody knows.
    static func confirmBody(_ state: AccountLoginChangeState, copy: AccountChangeCopy) -> String? {
        if state.isWallet {
            return copy.render(copy.bodyWallet, new: state.newProvider)
        }
        return copy.render(copy.body, old: state.oldProvider, new: state.newProvider)
            ?? copy.render(copy.bodyOldUnknown, new: state.newProvider)
    }

    /// The daemon's step error as the approved sentence. A current-login
    /// error whose old login is not known has no approved words naming it,
    /// and says the generic line instead.
    static func errorLine(_ state: AccountLoginChangeState, copy: AccountChangeCopy) -> String? {
        switch state.error {
        case nil: return nil
        case "current_login":
            return copy.render(copy.errorCurrentLogin, old: state.oldProvider) ?? copy.errorUnavailable
        case "wallet": return copy.errorWallet
        case "in_use": return copy.errorInUse
        case "too_soon": return copy.errorTooSoon
        default: return copy.errorUnavailable
        }
    }
}

/// The two IPC methods over any `DaemonTransport`, so the method names and
/// parameters are asserted without a daemon. Calls block: run them off the
/// main actor.
public struct AccountLoginChangeClient {
    /// A daemon refusal: its fixed label, safe to route on and never shown.
    public struct Failure: Error, Equatable {
        public let label: String
    }

    private let transport: any DaemonTransport

    public init(transport: any DaemonTransport) {
        self.transport = transport
    }

    public func start() throws -> AccountLoginChangeState {
        try call(["step": "start"])
    }

    public func status(changeID: String) throws -> AccountLoginChangeState {
        try call(["step": "status", "change_id": changeID])
    }

    /// Step 1. A wallet-origin account sends the NEAR account name typed in
    /// the sheet.
    public func proveCurrent(changeID: String, walletAccountID: String?) throws
        -> AccountLoginChangeState
    {
        var params: [String: Any] = ["step": "prove_current", "change_id": changeID]
        if let walletAccountID { params["wallet_account_id"] = walletAccountID }
        return try call(params)
    }

    public func signInNew(changeID: String) throws -> AccountLoginChangeState {
        try call(["step": "sign_in_new", "change_id": changeID])
    }

    /// Step 3: only from the person's Switch.
    public func confirm(changeID: String) throws -> AccountLoginChangeState {
        try call(["step": "confirm", "change_id": changeID, "confirmed": true])
    }

    public func cancel(changeID: String?) throws {
        var params: [String: Any] = [:]
        if let changeID { params["change_id"] = changeID }
        _ = try raw("account_login_change_cancel", params)
    }

    private func call(_ params: [String: Any]) throws -> AccountLoginChangeState {
        let result = try raw("account_login_change", params)
        guard let bytes = try? JSONSerialization.data(withJSONObject: result, options: [.fragmentsAllowed]),
            let state = try? JSONDecoder().decode(AccountLoginChangeState.self, from: bytes)
        else { throw Failure(label: "unparseable-response") }
        return state
    }

    private func raw(_ method: String, _ params: [String: Any]) throws -> Any {
        let data = try JSONSerialization.data(withJSONObject: params)
        let frame = transport.call(method, params: String(decoding: data, as: UTF8.self))
        guard let bytes = frame.data(using: .utf8),
            let object = try? JSONSerialization.jsonObject(with: bytes) as? [String: Any]
        else { throw Failure(label: "unparseable-response") }
        if let error = object["error"] as? [String: Any] {
            throw Failure(label: error["message"] as? String ?? "unavailable")
        }
        guard let result = object["result"] else { throw Failure(label: "unparseable-response") }
        return result
    }
}
