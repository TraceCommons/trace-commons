//! Swift mirrors of `GET /_ironwire/settings`.
//
// The settings screen is where a GUI is most tempted to grow its own opinion:
// four privacy modes look like four buttons, and "log in" looks like something
// an app should just do. Both would be wrong.
//
// So this type carries the *decisions* as well as the values. Whether `full` is
// selectable is a rule that lives in `full_is_blocked` (`control.rs`) and is
// more than it looks — a named destination is not a usable one — so the app
// reads `selectable` and `unavailableBecause` and never the inputs behind them.
// The consent question arrives as text from the daemon,
// because a second copy of a consent prompt is a second prompt while the
// recorded version claims otherwise (`docs/TRUST.md` §2).

import Foundation

/// What can be changed, and what it would take.
public struct SettingsView: Decodable, Sendable, Equatable {
    /// The privacy filter, and the modes it could be switched to.
    public let privacy: PrivacySettingsView
    /// Everything a user can log into.
    public let services: [ServiceView]
    /// Every coding agent the daemon knows about, and whether it routes here.
    public let tools: [ToolView]

    private enum CodingKeys: String, CodingKey {
        case privacy, services, tools
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        privacy = try c.decodeIfPresent(PrivacySettingsView.self, forKey: .privacy)
            ?? PrivacySettingsView()
        services = try c.decodeIfPresent([ServiceView].self, forKey: .services) ?? []
        tools = try c.decodeIfPresent([ToolView].self, forKey: .tools) ?? []
    }

    public init(
        privacy: PrivacySettingsView = PrivacySettingsView(), services: [ServiceView] = [],
        tools: [ToolView] = []
    ) {
        self.privacy = privacy
        self.services = services
        self.tools = tools
    }
}

/// One coding agent on this machine.
///
/// The distinction this type carries is the one the menu exists to make: a
/// backend being healthy says IronWire *could* serve a request, and a tool
/// being wired says anything is actually *sending* one. A consented
/// subscription beside an unwired agent is a machine where nothing routes, and
/// that used to be invisible.
public struct ToolView: Decodable, Sendable, Equatable, Identifiable {
    /// Stable id — what `ironwire disconnect <id>` accepts.
    public let id: String
    /// What the user calls it.
    public let name: String
    /// The file IronWire would edit, when one can be located.
    public let configPath: String?
    /// Whether the tool looks present on this machine.
    public let installed: Bool
    /// Whether its config currently routes through IronWire.
    public let wired: Bool
    /// What to run to point it here. Named rather than offered as a button:
    /// editing somebody's agent config needs the same "name the file first"
    /// ceremony the CLI does, which a dropdown cannot give it.
    public let connectCommand: String

    private enum CodingKeys: String, CodingKey {
        case id, name, configPath, installed, wired, connectCommand
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(String.self, forKey: .id) ?? ""
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? id
        configPath = try c.decodeIfPresent(String.self, forKey: .configPath)
        installed = try c.decodeIfPresent(Bool.self, forKey: .installed) ?? false
        wired = try c.decodeIfPresent(Bool.self, forKey: .wired) ?? false
        connectCommand = try c.decodeIfPresent(String.self, forKey: .connectCommand) ?? ""
    }

    public init(
        id: String, name: String, configPath: String? = nil, installed: Bool = true,
        wired: Bool = false, connectCommand: String = ""
    ) {
        self.id = id
        self.name = name
        self.configPath = configPath
        self.installed = installed
        self.wired = wired
        self.connectCommand = connectCommand
    }
}

/// The privacy filter as a settings screen sees it.
public struct PrivacySettingsView: Decodable, Sendable, Equatable {
    /// The mode in force: `off` / `credentials` / `pii` / `full`.
    public let mode: String
    /// What the filter is *doing*, in the daemon's words. Rendered verbatim or
    /// not at all (`docs/TRUST.md` I7).
    public let summary: String
    /// Every rung of the ladder, in order.
    ///
    /// The daemon also publishes `trusted_backends`, and this type does not
    /// carry it. It is the input to a rule — whether `full` has anywhere usable
    /// to route — and the answer is already here as `selectable`. Decoding the
    /// input would be leaving the ingredients for a second implementation of
    /// that rule lying around next to the answer.
    public let options: [PrivacyOptionView]

    private enum CodingKeys: String, CodingKey {
        case mode, summary, options
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        mode = try c.decodeIfPresent(String.self, forKey: .mode) ?? "off"
        summary = try c.decodeIfPresent(String.self, forKey: .summary) ?? "off"
        options = try c.decodeIfPresent([PrivacyOptionView].self, forKey: .options) ?? []
    }

    public init(
        mode: String = "off", summary: String = "off", options: [PrivacyOptionView] = []
    ) {
        self.mode = mode
        self.summary = summary
        self.options = options
    }
}

/// One rung of the privacy ladder.
public struct PrivacyOptionView: Decodable, Sendable, Equatable, Identifiable {
    /// The value to send back. Also the identity, so a mode this build has never
    /// heard of still renders as itself.
    public let id: String
    /// What this level substitutes, in one clause.
    public let describes: String
    /// Whether switching to it right now would work.
    public let selectable: Bool
    /// Why it would not. Shown beside the disabled option, because an option
    /// that is greyed out for unstated reasons is worse than one that is absent.
    public let unavailableBecause: String?

    private enum CodingKeys: String, CodingKey {
        case id, describes, selectable, unavailableBecause
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(String.self, forKey: .id) ?? ""
        describes = try c.decodeIfPresent(String.self, forKey: .describes) ?? ""
        // Absent means selectable: an older daemon that does not send the field
        // has no restrictions to report.
        selectable = try c.decodeIfPresent(Bool.self, forKey: .selectable) ?? true
        unavailableBecause = try c.decodeIfPresent(String.self, forKey: .unavailableBecause)
    }

    public init(id: String, describes: String, selectable: Bool = true, unavailableBecause: String? = nil) {
        self.id = id
        self.describes = describes
        self.selectable = selectable
        self.unavailableBecause = unavailableBecause
    }
}

/// One thing a user can log into.
public struct ServiceView: Decodable, Sendable, Equatable, Identifiable {
    /// Backend id.
    public let id: String
    /// Display name.
    public let name: String
    /// `subscription` / `api_key` / `credits` / `local`.
    public let kind: String
    /// Whether a credential was found.
    public let authenticated: Bool
    /// Why not, when it was not.
    public let detail: String?
    /// Whether this backend is gated behind recorded consent.
    public let requiresConsent: Bool
    /// Whether that consent is recorded, at the current prompt version.
    public let consented: Bool
    /// The exact question that has to be answered to enable it.
    public let consentPrompt: ConsentPromptView?
    /// What to run for the part a GUI has no business doing.
    public let connectCommand: String?

    private enum CodingKeys: String, CodingKey {
        case id, name, kind, authenticated, detail
        case requiresConsent, consented, consentPrompt, connectCommand
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decodeIfPresent(String.self, forKey: .id) ?? ""
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? id
        kind = try c.decodeIfPresent(String.self, forKey: .kind) ?? ""
        authenticated = try c.decodeIfPresent(Bool.self, forKey: .authenticated) ?? false
        detail = try c.decodeIfPresent(String.self, forKey: .detail)
        requiresConsent = try c.decodeIfPresent(Bool.self, forKey: .requiresConsent) ?? false
        consented = try c.decodeIfPresent(Bool.self, forKey: .consented) ?? false
        consentPrompt = try c.decodeIfPresent(ConsentPromptView.self, forKey: .consentPrompt)
        connectCommand = try c.decodeIfPresent(String.self, forKey: .connectCommand)
    }

    public init(
        id: String, name: String, kind: String = "subscription", authenticated: Bool = true,
        detail: String? = nil, requiresConsent: Bool = false, consented: Bool = true,
        consentPrompt: ConsentPromptView? = nil, connectCommand: String? = nil
    ) {
        self.id = id
        self.name = name
        self.kind = kind
        self.authenticated = authenticated
        self.detail = detail
        self.requiresConsent = requiresConsent
        self.consented = consented
        self.consentPrompt = consentPrompt
        self.connectCommand = connectCommand
    }

    /// Whether this is a switch the app can actually throw.
    ///
    /// Consent is the only thing here the daemon can act on directly. A
    /// credential that has not been found is not something a menu can conjure —
    /// it comes from the user logging into Claude Code or Codex, or exporting an
    /// API key, and the honest thing is to say so and name the command.
    public var canToggle: Bool { requiresConsent && authenticated }
}

/// The consent question, exactly as the daemon words it.
///
/// Never composed here, never abridged, never reordered so the cost reads last.
/// This is the wording `CONSENT_PROMPT_VERSION` is recorded against, and the
/// version travels with it so the answer can be checked against the question.
public struct ConsentPromptView: Decodable, Sendable, Equatable {
    /// The version this wording is. Sent back with the answer.
    public let version: Int
    /// Backend this grants.
    public let backendId: String
    /// Opaque login identity from the daemon, echoed unchanged with a grant.
    public let binding: String?
    /// What the user calls it.
    public let product: String
    /// What IronWire will do, in one sentence.
    public let summary: String
    /// What the user is taking on, one point at a time.
    public let points: [String]
    /// The question itself.
    public let question: String

    private enum CodingKeys: String, CodingKey {
        case version, backendId, binding, product, summary, points, question
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        version = try c.decodeIfPresent(Int.self, forKey: .version) ?? 0
        backendId = try c.decodeIfPresent(String.self, forKey: .backendId) ?? ""
        binding = try c.decodeIfPresent(String.self, forKey: .binding)
        product = try c.decodeIfPresent(String.self, forKey: .product) ?? ""
        summary = try c.decodeIfPresent(String.self, forKey: .summary) ?? ""
        points = try c.decodeIfPresent([String].self, forKey: .points) ?? []
        question = try c.decodeIfPresent(String.self, forKey: .question) ?? ""
    }

    public init(
        version: Int, backendId: String, product: String,
        summary: String, points: [String], question: String, binding: String? = nil
    ) {
        self.version = version
        self.backendId = backendId
        self.binding = binding
        self.product = product
        self.summary = summary
        self.points = points
        self.question = question
    }

    /// Whether this is safe to present at all.
    ///
    /// A prompt that arrived without its summary or its points is not a consent
    /// screen — it is a button with a title. Better to send the user to the CLI
    /// than to collect an answer to a question we failed to ask.
    public var isComplete: Bool {
        version > 0 && !summary.isEmpty && !points.isEmpty && !question.isEmpty
    }
}
