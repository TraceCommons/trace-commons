import Foundation

/// Launch preferences for one folder and agent. Accounts are never included.
public struct ManagedFolderSettings: Codable, Equatable, Sendable {
    public let tool: String
    public let model: String?
    /// The folder's background model; absent (none) from an older daemon.
    public let backgroundModel: String?
    public let collectTraces: Bool
    public let cliOptions: String
    public let remembered: Bool
    public let revision: UInt64

    enum CodingKeys: String, CodingKey {
        case tool, model, remembered, revision
        case backgroundModel = "background_model"
        case collectTraces = "collect_traces", cliOptions = "cli_options"
    }
}

/// The canonical configuration projection, also used by config show/preview.
public struct ManagedFolderProfile: Codable, Equatable, Sendable {
    public let tool: String
    public let cwd: String
    public let model: String?
    public let collectTraces: Bool
    public let cliOptions: String
    enum CodingKeys: String, CodingKey {
        case tool, cwd, model
        case collectTraces = "collect_traces", cliOptions = "cli_options"
    }
}

/// A load token identifies the entire form context, including an account change
/// within the same agent. Late responses can never replace newer typing.
public struct ManagedFolderDraft: Equatable, Sendable {
    public enum Phase: Equatable, Sendable { case idle, loading, ready, failed }
    public private(set) var phase: Phase = .idle
    public private(set) var tool: String?
    public private(set) var cwd: String?
    public private(set) var remembered = false
    private var token: UUID?
    public var model: String?
    /// The picker fell back to the default because NEAR AI no longer lists
    /// the folder's saved model. The launch runs the default and sends
    /// `keep_saved_model`, so the saved model is not replaced by it.
    public private(set) var keepsSavedModel = false
    /// The model for the tool's background work; none for its default.
    public private(set) var backgroundModel: String?
    public var collectTraces = false
    public var cliOptions = ""

    public init() {}

    @discardableResult
    /// `cwd` nil is the home folder the daemon resolves (owner, 2026-10-29).
    public mutating func begin(tool: String, cwd: String?) -> UUID {
        let next = UUID()
        self = Self()
        self.tool = tool
        self.cwd = cwd
        token = next
        phase = .loading
        return next
    }

    @discardableResult
    public mutating func complete(_ settings: ManagedFolderSettings, token: UUID) -> Bool {
        guard self.token == token else { return false }
        guard settings.tool == tool else { phase = .failed; return false }
        model = settings.model
        backgroundModel = settings.backgroundModel
        collectTraces = settings.collectTraces
        cliOptions = settings.cliOptions
        remembered = settings.remembered
        phase = .ready
        return true
    }

    public mutating func fail(token: UUID) {
        if self.token == token { phase = .failed }
    }

    public func matches(tool: String, cwd: String?) -> Bool {
        self.tool == tool && self.cwd == cwd && phase == .ready
    }

    /// The saved model is not listed: run the default, keep the saved one.
    public mutating func fallBackToDefault() {
        model = nil
        keepsSavedModel = true
    }

    /// A model the person picked (nil for the default), which the launch
    /// remembers.
    public mutating func choose(_ model: String?) {
        self.model = model
        keepsSavedModel = false
    }

    /// `choose`, then clear a background choice the new main model does
    /// not allow (`ManagedSurface.reconciledBackground`): the background
    /// model is never less private than the main one.
    public mutating func choose(_ model: String?, in catalogue: ManagedModelCatalogue?) {
        choose(model)
        reconcileBackground(in: catalogue)
    }

    /// A background model the person picked (nil for the default).
    public mutating func chooseBackground(_ model: String?) {
        backgroundModel = model
    }

    /// Clear a background choice the current main model does not allow.
    public mutating func reconcileBackground(in catalogue: ManagedModelCatalogue?) {
        backgroundModel = ManagedSurface.reconciledBackground(backgroundModel, main: model, in: catalogue)
    }

    public mutating func useDefaults(tool: String, cwd: String?, cliOptions: String) {
        self = Self()
        self.tool = tool
        self.cwd = cwd
        self.cliOptions = cliOptions
        phase = .ready
    }
}

public enum ManagedFolderModelMode: Equatable, Sendable { case none, catalogue, text }

public extension ManagedSurface {
    static func folderModelMode(tool: String, connection: String) -> ManagedFolderModelMode {
        if tool == "opencode" { return connection == "near_ai_signin" ? .catalogue : .text }
        return ["near_ai", "near_ai_signin"].contains(connection) ? .catalogue : .none
    }
}

public extension ManagedFolderDraft {
    /// Only launch choices change; the account remains the caller's selection.
    /// `keep_saved_model` goes only to a daemon that reads it
    /// (`capabilities.keep_saved_model`): an older one refuses the field.
    /// `background_model` likewise goes only to a daemon that reads it
    /// (`capabilities.background_model`), and only for a tool with a
    /// background picker.
    func applying(to params: [String: Any], daemonKeepsSavedModel: Bool,
                  daemonReadsBackground: Bool = false) -> [String: Any] {
        var result = params
        result["cli_options"] = cliOptions
        result["collect_traces"] = collectTraces
        let tool = params["tool"] as? String ?? "", connection = params["connection"] as? String ?? ""
        let mode = ManagedSurface.folderModelMode(tool: tool, connection: connection)
        result["model"] = mode == .none ? nil : model
        if keepsSavedModel && mode != .none && daemonKeepsSavedModel { result["keep_saved_model"] = true }
        if daemonReadsBackground, ManagedSurface.offersBackgroundModel(tool: tool, connection: connection),
           let backgroundModel {
            result["background_model"] = backgroundModel
        }
        return result
    }
}
