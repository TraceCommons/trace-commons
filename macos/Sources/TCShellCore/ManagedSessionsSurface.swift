import Foundation

public struct ManagedAccount: Decodable, Equatable, Identifiable, Sendable {
    public let id: String
    public let tool: String
    public let connection: String
    public let label: String
    public let authState: String
    enum CodingKeys: String, CodingKey { case id, tool, connection, label; case authState = "auth_state" }
}

public struct ManagedSelection: Decodable, Equatable, Sendable {
    public let tool: String
    public let accountID: String
    public let generation: UInt64
    enum CodingKeys: String, CodingKey { case tool, generation; case accountID = "account_id" }
}

public struct ManagedSession: Decodable, Equatable, Identifiable, Sendable {
    public let id: String
    public let tool: String
    public let connection: String
    public let accountID: String
    public let accountLabel: String
    public let projectLabel: String
    public let cwd: String
    public let state: String
    public let purpose: String
    public let exitCode: Int?
    public let model: String?
    public let collection: String?
    /// The daemon can bring this session's window to the front (#1407).
    /// False from an older daemon.
    public let canFocus: Bool
    /// The daemon can ask this session's launcher to stop it. False from an
    /// older daemon and for a launch from the command line.
    public let canStop: Bool
    /// The ended session can be launched again as it was.
    public let canStartAgain: Bool
    /// When the session started; nil when absent or unreadable.
    public let startedAt: Date?
    public var holdsAccount: Bool { state != "exited" && state != "failed" }
    enum CodingKeys: String, CodingKey {
        case id, tool, connection, cwd, state, purpose, model, collection
        case accountID = "account_id", accountLabel = "account_label", projectLabel = "project_label", exitCode = "exit_code"
        case canFocus = "can_focus", canStop = "can_stop", canStartAgain = "can_start_again", startedAt = "started_at"
    }

    /// Decodes its own timestamp, as `PastSession` does: chrono writes
    /// nanoseconds, which `ISO8601DateFormatter` refuses.
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        tool = try c.decode(String.self, forKey: .tool)
        connection = try c.decode(String.self, forKey: .connection)
        accountID = try c.decode(String.self, forKey: .accountID)
        accountLabel = try c.decode(String.self, forKey: .accountLabel)
        projectLabel = try c.decode(String.self, forKey: .projectLabel)
        cwd = try c.decode(String.self, forKey: .cwd)
        state = try c.decode(String.self, forKey: .state)
        purpose = try c.decode(String.self, forKey: .purpose)
        exitCode = try c.decodeIfPresent(Int.self, forKey: .exitCode)
        model = try c.decodeIfPresent(String.self, forKey: .model)
        collection = try c.decodeIfPresent(String.self, forKey: .collection)
        canFocus = try c.decodeIfPresent(Bool.self, forKey: .canFocus) ?? false
        canStop = try c.decodeIfPresent(Bool.self, forKey: .canStop) ?? false
        canStartAgain = try c.decodeIfPresent(Bool.self, forKey: .canStartAgain) ?? false
        startedAt = try c.decodeIfPresent(String.self, forKey: .startedAt).flatMap(SourceCandidate.Wire.parseTimestamp)
    }
}

/// What a session row offers (#1407), in the order drawn.
public enum ManagedSessionAction: Equatable, Sendable {
    /// Bring the session's window to the front.
    case show
    /// Stop the session, after a confirmation.
    case stop
    /// Launch the ended session again, same tool, account and folder.
    case startAgain
    /// Take the ended row off the list.
    case dismiss
}

/// A terminal app a managed session can open in, as the daemon names it:
/// its wire id and the app's own name.
public struct ManagedTerminalChoice: Decodable, Equatable, Identifiable, Sendable {
    public let id: String
    public let name: String
    public init(id: String, name: String) {
        self.id = id
        self.name = name
    }
}

public struct ManagedToolAvailability: Decodable, Equatable, Identifiable, Sendable {
    public let tool: String
    public let installed: Bool
    public let installURL: String
    public var id: String { tool }
    public var url: URL? {
        guard let url = URL(string: installURL), url.scheme == "https" else { return nil }
        return url
    }
    enum CodingKeys: String, CodingKey { case tool, installed; case installURL = "install_url" }
}

public struct ManagedSnapshot: Decodable, Equatable, Sendable {
    public struct Capabilities: Decodable, Equatable, Sendable {
        public let folderSettings: Bool?
        public let managedLaunch: Bool
        public let terminalLaunch: Bool
        /// Terminal installer support; absent from older daemons.
        public let toolSetup: Bool?
        public let terminalDestination: String?
        /// Whether the daemon holds a NEAR AI sign-in. A bool, never the
        /// credential; absent from an older daemon, which reads as no.
        public let nearAiSignin: Bool?
        /// The chosen terminal's id; nil from an older daemon, or for a
        /// stored choice the daemon does not know.
        public let terminalCurrent: String?
        /// The terminal apps installed on this Mac; absent from an older
        /// daemon.
        public let terminalChoices: [ManagedTerminalChoice]?
        enum CodingKeys: String, CodingKey {
            case managedLaunch = "managed_launch", terminalLaunch = "terminal_launch", terminalDestination = "terminal_destination"
            case toolSetup = "tool_setup"
            case folderSettings = "folder_settings"
            case nearAiSignin = "near_ai_signin"
            case terminalCurrent = "terminal_current", terminalChoices = "terminal_choices"
        }
    }
    public var cliOptions: [String: String]?
    public let tools: [ManagedToolAvailability]?
    public let copy: [String: String]?
    public var revision: UInt64
    public let accounts: [ManagedAccount]
    /// One entry per tool, derived from the NEAR AI sign-in and never
    /// saved. Empty, or absent from an older daemon, without one.
    public let signInAccounts: [ManagedAccount]?
    public let defaults: [ManagedSelection]
    public let generations: [String: UInt64]
    public let sessions: [ManagedSession]
    public let capabilities: Capabilities
    /// Whether a coding session has started on this computer, from the app
    /// or the command line (#1408). Absent from an older daemon.
    public let hasLaunched: Bool?
    enum CodingKeys: String, CodingKey {
        case cliOptions = "cli_options"
        case tools, copy, revision, accounts, defaults, generations, sessions, capabilities
        case signInAccounts = "sign_in_accounts", hasLaunched = "has_launched"
    }
}

/// Copy keys for the managed-session values the daemon sends, so no shell
/// draws a raw wire value or a daemon error code. Every word is in the
/// shared table (`managed/copy.rs`); this only picks the key.
public enum ManagedSurface {
    public static func visibleHarnesses(_ rows: [HarnessRow], tools: [ManagedToolAvailability]?) -> [HarnessRow] {
        rows.filter { row in
            // Persistent routing is independent of managed launch availability.
            // Keep stale-port connections removable even after uninstall or proxy Off.
            if row.installed || row.connected || row.canDisconnect || row.lastCallAt != nil { return true }
            return tools?.first(where: { $0.tool == row.id })?.installed == true
        }
    }

    /// The managed tools, in the order the Add and Launch sheets offer them.
    /// OpenCode is one since #1417.
    public static let supportedTools = ["claude", "codex", "opencode"]
    public static func setupTools(_ tools: [ManagedToolAvailability]?) -> [String] {
        guard let tools else { return supportedTools }
        return supportedTools.filter { tool in tools.first(where: { $0.tool == tool })?.installed == true }
    }

    /// The lifecycle values the daemon sends today.
    public static let sessionStates: Set<String> = ["starting", "running", "exited", "failed", "unknown"]
    /// The sign-in values the daemon sends today.
    public static let authStates: Set<String> = ["sign_in_required", "checking", "ready", "unavailable"]

    /// A lifecycle a later daemon grows reads as unknown, never as its raw value.
    public static func stateKey(_ state: String) -> String {
        "state_" + (sessionStates.contains(state) ? state : "unknown")
    }

    /// Nil for a sign-in value this shell does not know: no label rather than the raw value.
    public static func authKey(_ authState: String) -> String? {
        authStates.contains(authState) ? "auth_" + authState : nil
    }

    /// The tools a NEAR AI sign-in session can start, in the order shown.
    /// OpenCode's button also needs the daemon to report it installed
    /// (`ManagedSnapshot.signInStartTools`).
    public static let signInTools = ["claude", "codex", "opencode"]

    /// The Start button's key for a tool, or nil for a tool this shell
    /// does not know.
    public static func startKey(_ tool: String) -> String? {
        signInTools.contains(tool) ? "start_" + tool : nil
    }

    /// The Add sheet's sign-in types. Signed in, NEAR AI is not offered: the
    /// sign-in entries already run on it, with no key. Signed out, a pasted
    /// key stays the fallback.
    public static func connectionOptions(signedIn: Bool, tool: String = "claude") -> [String] {
        // OpenCode has no subscription flow. A saved NEAR AI connection for
        // it uses a pasted key and names its own model, even while signed
        // in; the sign-in entry starts on the default its proxy names.
        if tool == "opencode" { return ["api_key", "near_ai"] }
        return signedIn ? ["subscription", "api_key"] : ["subscription", "api_key", "near_ai"]
    }

    /// The sign-in card's line with `{destination}` filled from the
    /// terminal the daemon reports, as the launch line does; `fallback`
    /// (the core's "your terminal") when it reports none.
    public static func signInScope(_ template: String, destination: String?, fallback: String) -> String {
        template.replacingOccurrences(of: "{destination}", with: destination ?? fallback)
    }

    /// `tool_unavailable` ("{tool} wasn't found on this computer.") with the
    /// tool's display name filled in.
    public static func toolUnavailable(_ tool: String, copy: (String) -> String) -> String {
        copy("tool_unavailable").replacingOccurrences(of: "{tool}", with: copy(toolKey(tool) ?? tool))
    }

    /// The tool's name key, or nil for a tool this shell does not know.
    public static func toolKey(_ tool: String) -> String? {
        ["claude", "codex", "opencode"].contains(tool) ? tool : nil
    }

    /// A failed action is said in the shared copy, never as the daemon's
    /// code: a launch whose outcome is unknown asks for a refresh, anything
    /// else is the general failure. A missing tool is said on its account
    /// row after the refresh that follows, where the tool can be named.
    public static func errorKey(code: String?, message: String?) -> String {
        if code == "managed-collection-account-busy" || message == "managed-collection-account-busy" { return "collection_account_busy" }
        if code == "managed-cli-options-invalid" || message == "managed-cli-options-invalid" { return "cli_options_invalid" }
        if code == "managed-unsupported-version" || message == "managed-unsupported-version" { return "unsupported_version" }
        let launchUnknown = ["launch-unknown", "managed-launch-unknown"]
        if launchUnknown.contains(code ?? "") || launchUnknown.contains(message ?? "") { return "launch_unknown" }
        // No NEAR AI sign-in: the line that asks for one.
        let signInNeeded = "managed-near-ai-sign-in-required"
        if code == signInNeeded || message == signInNeeded { return "sign_in_needed" }
        // The chosen terminal app was not found: the line that says so
        // and points at Settings.
        let notFound = "managed-terminal-not-found"
        if code == notFound || message == notFound { return "terminal_not_found" }
        // A named model NEAR AI does not offer: the line that says so.
        let modelUnavailable = "managed-model-unavailable"
        if code == modelUnavailable || message == modelUnavailable { return "model_unavailable" }
        return "action_failed"
    }

    /// A line with `{terminal}` filled from the terminal the daemon
    /// reports; `fallback` (the core's "your terminal") when it reports none.
    public static func fillTerminal(_ template: String, terminal: String?, fallback: String) -> String {
        template.replacingOccurrences(of: "{terminal}", with: terminal ?? fallback)
    }
}

/// Session row actions (Ron, #1407, 2026-10-14). An action the daemon cannot
/// do is left out, never drawn disabled.
public extension ManagedSurface {
    /// The row's actions. A running row: Show and Stop where the daemon can
    /// do them. An ended row: Start again where it can be, with a terminal
    /// to start in, then Dismiss. Anything else: none.
    static func sessionActions(_ session: ManagedSession, terminalLaunch: Bool) -> [ManagedSessionAction] {
        if session.state == "running" { return menuActions(session) }
        guard !session.holdsAccount else { return [] }
        return (session.canStartAgain && terminalLaunch ? [.startAgain] : []) + [.dismiss]
    }

    /// The row menu's items: Show and Stop, for a running row only.
    static func menuActions(_ session: ManagedSession) -> [ManagedSessionAction] {
        guard session.state == "running" else { return [] }
        return (session.canFocus ? [.show] : []) + (session.canStop ? [.stop] : [])
    }

    /// Whether clicking the row brings its window to the front.
    static func clickShows(_ session: ManagedSession) -> Bool {
        session.state == "running" && session.canFocus
    }

    /// The stop confirmation's line with `{tool}` and `{folder}` filled.
    static func stopDescription(_ template: String, tool: String, folder: String) -> String {
        template.replacingOccurrences(of: "{tool}", with: tool).replacingOccurrences(of: "{folder}", with: folder)
    }

    /// The `managed_session_start_again` request: the row and a fresh
    /// request id, nothing the shell could change.
    static func startAgainParams(sessionID: String, requestID: String) -> [String: Any] {
        ["session_id": sessionID, "request_id": requestID]
    }

    /// The row's caption: its parts, then the start time in the app's
    /// relative format when the daemon sent one.
    static func caption(_ parts: [String], startedAt: Date?, relative: (Date) -> String = Format.when) -> String {
        (parts + [startedAt.map(relative)].compactMap { $0 }).filter { !$0.isEmpty }.joined(separator: " · ")
    }
}

/// A NEAR AI chat model a launch can name, from `managed_model_catalogue`.
public struct ManagedModel: Decodable, Equatable, Identifiable, Sendable {
    public let id: String
    /// NEAR AI's display name; absent when it gives none.
    public let name: String?
    /// NEAR AI's privacy tier (`incognito`, `near_ai`, `external`) that the
    /// picker groups by; absent from an older daemon or an unlabelled entry.
    public let tier: String?
}

/// `managed_model_catalogue`'s answer: NEAR AI's chat models in its order,
/// the one the tool gets when a launch names none, and the starred ones.
public struct ManagedModelCatalogue: Decodable, Equatable, Sendable {
    public let models: [ManagedModel]
    public let `default`: String?
    /// The person's favorite model ids. An older daemon sends none.
    public let favorites: [String]
    /// Favorites NEAR AI no longer lists, kept in case it offers them again.
    /// An older daemon sends none.
    public let unavailableFavorites: [String]

    private enum CodingKeys: String, CodingKey {
        case models, `default`, favorites
        case unavailableFavorites = "unavailable_favorites"
    }

    public init(from decoder: Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        models = try container.decode([ManagedModel].self, forKey: .models)
        `default` = try container.decodeIfPresent(String.self, forKey: .default)
        favorites = try container.decodeIfPresent([String].self, forKey: .favorites) ?? []
        unavailableFavorites = try container.decodeIfPresent([String].self, forKey: .unavailableFavorites) ?? []
    }
}

/// `ManagedSurface.reconciledModel`'s answer: the picker's value, and
/// whether it fell back to the default because NEAR AI dropped the model.
public struct ManagedModelReconcile: Equatable, Sendable {
    public let model: String?
    public let fellBack: Bool

    public init(model: String?, fellBack: Bool) {
        self.model = model
        self.fellBack = fellBack
    }
}

/// One favorite in Settings' list: its id, and whether NEAR AI stopped
/// listing it.
public struct ManagedFavoriteRow: Equatable, Sendable, Identifiable {
    public let model: String
    public let unavailable: Bool
    public var id: String { model }

    public init(model: String, unavailable: Bool) {
        self.model = model
        self.unavailable = unavailable
    }
}

/// One entry in the Launch sheet's model picker.
public struct ManagedModelOption: Equatable, Sendable {
    public let title: String
    public let value: String
    /// The heading of the group it is listed under; none for the default
    /// and for a model with no known tier.
    public var section: String? = nil
}

/// Choosing the model of a NEAR AI session in the Launch sheet.
public extension ManagedSurface {
    /// The picker value that names no model: the session takes the tool's
    /// default. Never a model id, which is never empty.
    static let defaultModel = ""

    /// Whether the Launch sheet offers a model picker for this account:
    /// Claude Code and Codex on a saved NEAR AI key, whose proxy pins the
    /// model. OpenCode keeps its own model field.
    static func offersModelChoice(tool: String, connection: String) -> Bool {
        ["claude", "codex"].contains(tool) && connection == "near_ai"
    }

    /// NEAR AI's privacy tiers in the picker's order (owner, 2026-10-30),
    /// each with the copy key of its heading.
    static let modelTiers = [
        ("incognito", "model_group_incognito"),
        ("near_ai", "model_group_near_ai"),
        ("external", "model_group_external"),
    ]

    /// The picker's options: the default first, which names the model it
    /// will be when the catalogue says, then the favorites, then the other
    /// chat models grouped by privacy tier in `modelTiers` order, each in
    /// NEAR AI's order. Favorites head no group; a model with no known tier
    /// comes last, under no heading. A favorite NEAR AI does not list is not
    /// shown. With no catalogue, only the default.
    static func modelOptions(_ catalogue: ManagedModelCatalogue?, copy: (String) -> String) -> [ManagedModelOption] {
        let first = catalogue?.default.map { copy("model_default").replacingOccurrences(of: "{model}", with: $0) }
            ?? copy("model_default_unknown")
        let models = catalogue?.models ?? []
        let favorites = Set(catalogue?.favorites ?? [])
        let favored = models.filter { favorites.contains($0.id) }
            .map { ManagedModelOption(title: $0.id, value: $0.id) }
        let others = models.filter { !favorites.contains($0.id) }
        let known = Set(modelTiers.map(\.0))
        let grouped = modelTiers.flatMap { tier, key in
            others.filter { $0.tier == tier }
                .map { ManagedModelOption(title: $0.id, value: $0.id, section: copy(key)) }
        }
        let rest = others.filter { !known.contains($0.tier ?? "") }
            .map { ManagedModelOption(title: $0.id, value: $0.id) }
        return [ManagedModelOption(title: first, value: defaultModel)] + favored + grouped + rest
    }

    /// The favorites NEAR AI no longer lists, each shown with a way to
    /// remove it. None without a catalogue: unknown is not removed.
    static func unavailableFavorites(_ catalogue: ManagedModelCatalogue?) -> [String] {
        catalogue?.unavailableFavorites ?? []
    }

    /// The picker's value once the catalogue is known. A model it does not
    /// list falls back to the default (`fellBack`), which the sheet says;
    /// with no catalogue the model is kept, unknown rather than removed.
    static func reconciledModel(_ model: String?, in catalogue: ManagedModelCatalogue?) -> ManagedModelReconcile {
        guard let model, let catalogue else { return ManagedModelReconcile(model: model, fellBack: false) }
        if catalogue.models.contains(where: { $0.id == model }) { return ManagedModelReconcile(model: model, fellBack: false) }
        return ManagedModelReconcile(model: nil, fellBack: true)
    }

    /// The copy key for a refused `managed_model_favorite_set`, from the
    /// daemon's label: only NEAR AI not listing the model says so. A full
    /// list, a failed write or no daemon is the generic failure, never a
    /// claim that the model is unavailable.
    static func favoriteErrorKey(_ label: String?) -> String {
        label == "managed-model-unavailable" ? "model_unavailable" : "action_failed"
    }

    /// Settings' favorites list: the listed ones in NEAR AI's order, then
    /// the ones it stopped listing, marked. Without a catalogue, the stored
    /// order and nothing marked: unknown is not removed.
    static func favoriteRows(stored: [String], catalogue: ManagedModelCatalogue?) -> [ManagedFavoriteRow] {
        guard let catalogue else { return stored.map { ManagedFavoriteRow(model: $0, unavailable: false) } }
        let unavailable = Set(catalogue.unavailableFavorites)
        let rank = Dictionary(catalogue.models.enumerated().map { ($1.id, $0) }, uniquingKeysWith: { first, _ in first })
        let listed = stored.filter { !unavailable.contains($0) }
            .sorted { (rank[$0] ?? Int.max, $0) < (rank[$1] ?? Int.max, $1) }
        return listed.map { ManagedFavoriteRow(model: $0, unavailable: false) }
            + stored.filter(unavailable.contains).map { ManagedFavoriteRow(model: $0, unavailable: true) }
    }

    /// Whether `model` is starred.
    static func isFavoriteModel(_ model: String, in catalogue: ManagedModelCatalogue?) -> Bool {
        catalogue?.favorites.contains(model) == true
    }

    /// Whether the star is shown for the picker's value: only a model NEAR
    /// AI lists, never the default option or a saved model it dropped.
    static func offersFavorite(_ model: String?, in catalogue: ManagedModelCatalogue?) -> Bool {
        guard let model, model != defaultModel else { return false }
        return catalogue?.models.contains { $0.id == model } == true
    }

    /// The `managed_model_favorite_set` request.
    static func favoriteParams(model: String, favorite: Bool) -> [String: Any] {
        ["model": model, "favorite": favorite]
    }

    /// The Launch sheet's `managed_launch_prepare` request. A model is sent
    /// only where one was chosen: OpenCode's typed model with its
    /// collection opt-in, or a picked model where `offersModelChoice`.
    static func launchParams(
        account: ManagedAccount, cwd: String?, requestID: String, generation: UInt64,
        model: String?, collectTraces: Bool
    ) -> [String: Any] {
        var params: [String: Any] = [
            "request_id": requestID, "purpose": "coding", "tool": account.tool,
            "connection": account.connection, "account_id": account.id,
            "expected_generation": generation, "save_default": false,
        ]
        // No folder: the daemon starts the session in the home folder.
        if let cwd { params["cwd"] = cwd }
        if account.tool == "opencode" {
            params["model"] = (model ?? "").trimmingCharacters(in: .whitespacesAndNewlines)
            params["collect_traces"] = collectTraces
        } else if offersModelChoice(tool: account.tool, connection: account.connection),
                  let model, model != defaultModel {
            params["model"] = model
        }
        return params
    }
}

public extension ManagedSnapshot {
    /// A failed operation identifies its tool; old error payloads fall back
    /// to installed tools, never an unrelated hardcoded provider.
    func updateRecoveryTools(attemptedTool: String?) -> [ManagedToolAvailability] {
        (tools ?? []).filter { availability in
            guard ManagedSurface.toolKey(availability.tool) != nil else { return false }
            if let attemptedTool { return availability.tool == attemptedTool }
            return availability.installed
        }
    }

    /// Explicitly reported missing tools, in the same order as launch buttons.
    /// An older daemon without availability rows cannot establish absence.
    var missingInstallTools: [ManagedToolAvailability] {
        ManagedSurface.supportedTools.compactMap { tool in
            tools?.first { $0.tool == tool && !$0.installed }
        }
    }

    var setupTools: [String] { ManagedSurface.setupTools(tools) }
    var needsToolInstallation: Bool {
        guard let tools else { return false }
        return !tools.contains { ManagedSurface.supportedTools.contains($0.tool) && $0.installed }
    }
    func canLaunch(tool: String) -> Bool {
        guard ManagedSurface.supportedTools.contains(tool) else { return false }
        guard let tools else { return true }
        return tools.first { $0.tool == tool }?.installed == true
    }

    /// A running, starting or unknown session holds its account: no sign-in,
    /// key change or removal while it does.
    func isHeld(_ account: ManagedAccount) -> Bool {
        sessions.contains { $0.accountID == account.id && $0.holdsAccount }
    }

    func isDefault(_ account: ManagedAccount) -> Bool {
        defaults.contains { $0.accountID == account.id }
    }

    /// Whether Start explains the folder choice before the picker: until a
    /// session has started on this computer (#1408). An older daemon that
    /// does not say reads as never launched.
    var explainsFolderChoice: Bool { hasLaunched != true }

    /// Whether the daemon holds a NEAR AI sign-in.
    var nearAiSignedIn: Bool { capabilities.nearAiSignin == true }

    /// The terminal apps a picker offers: the installed ones, none from an
    /// older daemon.
    var terminalChoices: [ManagedTerminalChoice] { capabilities.terminalChoices ?? [] }

    /// The tool's NEAR AI sign-in entry, only while signed in.
    func signInEntry(_ tool: String) -> ManagedAccount? {
        guard nearAiSignedIn else { return nil }
        return signInAccounts?.first { $0.tool == tool && $0.connection == "near_ai_signin" }
    }

    /// The tools the sign-in card offers a Start button for, in order: each
    /// with a sign-in entry that can launch. OpenCode only while the
    /// daemon's `tools` row says it is installed; an older daemon that
    /// sends no `tools` offers no OpenCode button (fail closed), while
    /// Claude Code and Codex keep `canLaunch`'s older-daemon reading.
    var signInStartTools: [String] {
        ManagedSurface.signInTools.filter { tool in
            guard signInEntry(tool) != nil, canLaunch(tool: tool) else { return false }
            guard tool == "opencode" else { return true }
            return tools?.first { $0.tool == tool }?.installed == true
        }
    }

    /// The `managed_launch_prepare` request that starts `tool` on the NEAR
    /// AI sign-in in `cwd`; nil unless the card offers that tool's Start
    /// button. It names no model: the session's proxy pins one. A nil `cwd`
    /// names no folder, and the daemon starts the session in the person's
    /// home folder (owner, 2026-10-29).
    func signInLaunchParams(tool: String, cwd: String?, requestID: String) -> [String: Any]? {
        guard signInStartTools.contains(tool), let entry = signInEntry(tool) else { return nil }
        var params: [String: Any] = [
            "request_id": requestID, "purpose": "coding", "tool": entry.tool,
            "connection": entry.connection, "account_id": entry.id,
            "expected_generation": generations[entry.tool] ?? 0, "save_default": false,
        ]
        if let cwd { params["cwd"] = cwd }
        return params
    }
}

/// Drafts belong to the model so refreshes and navigation cannot erase typing.
public struct ManagedCLIOptionsDrafts: Equatable, Sendable {
    private var values: [String: String] = [:]
    public init() {}

    public func value(tool: String, saved: [String: String]?) -> String {
        values[tool] ?? saved?[tool] ?? ""
    }

    public func isChanged(tool: String, saved: [String: String]?) -> Bool {
        guard let stored = saved?[tool] else { return false }
        return value(tool: tool, saved: saved) != stored
    }

    public mutating func edit(tool: String, options: String) {
        values[tool] = options
    }

    public mutating func saved(tool: String, options: String) {
        if values[tool] == options { values.removeValue(forKey: tool) }
    }
}
