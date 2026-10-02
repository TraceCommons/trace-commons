import Foundation
import TCShellCore

/// The Traces tree (R6 of #1173): tool › folder › session, built only from
/// what the core reports through `DaemonDataClient`. Nothing is estimated.
///
/// The same rules as #1146's `traces-model.ts`:
/// - A tool's switch is its source declaration. `unset` is never drawn as
///   off: an unset tool has no switch at all. A tool the core reads from its
///   usual folder while unset (`unset_scans_conventional`: Claude Code and
///   Codex) is always drawn, because it is being read; one that opens
///   nothing while unset is drawn only with something waiting.
/// - Unreadable settings are `unknown`, never `unset`: no switch, and the
///   tab says the declaration could not be confirmed.
/// - A folder sits under the tool most of its sessions came from. The core
///   does not yet say which tool a folder belongs to (K11 of #1173); until it
///   does, a folder with no waiting session cannot be placed and is listed
///   on its own after the tools.
/// - Ignored folders are left out.
/// - The unresolvable bucket (sessions whose folder the core cannot name) is
///   drawn under its shared name, never its `unknown-project` slug, and is
///   never offered automatic.
struct TracesTree: Equatable {
    var tools: [ToolNode]
    /// Folders no session places under a tool yet (K11).
    var unplaced: [FolderNode]

    struct ToolNode: Equatable, Identifiable {
        let kind: SourceKind
        let mode: SourceMode
        var folders: [FolderNode]

        var id: String { kind.rawValue }
        var waiting: Int { folders.reduce(0) { $0 + $1.sessions.count } }
    }

    struct FolderNode: Equatable, Identifiable {
        let id: String
        let label: String
        /// `nil` for a folder the queue names but `list_projects` does not.
        let mode: ProjectMode?
        /// The modes this folder can be set to (`ProjectRow.offerableModes`):
        /// a folder that cannot be armed is never offered automatic.
        var offerableModes: [ProjectMode] = []
        var sessions: [DaemonData.QueueEntry]
        /// The disclosure the daemon chose for an armed folder
        /// (`automatic_disclosure`), worded by the core; nil when not armed.
        var disclosure: String? = nil
        /// The unresolvable bucket, which says why it can never be armed.
        var isBucket = false
    }

    /// A tool's source declaration in `get_settings`. `unknown` is settings
    /// that could not be read: not an answer, and never drawn as one.
    enum SourceMode: Equatable {
        case watch, off, unset, unknown

        init(_ raw: String?) {
            switch raw {
            case "watch": self = .watch
            case "off": self = .off
            default: self = .unset
            }
        }

        /// The `*_source_mode` value the core's check line takes, or nil
        /// for `unknown`, which has no declaration to describe.
        var wire: String? {
            switch self {
            case .watch: "watch"
            case .off: "off"
            case .unset: "unset"
            case .unknown: nil
            }
        }
    }

    /// Whether a tool is drawn. One that is declared, or has folders, is.
    /// Unset or unknown, it is drawn when the core reads it from its usual
    /// folder anyway, since leaving it out would say it is not watched.
    static func drawsTool(_ kind: SourceKind, mode: SourceMode, hasFolders: Bool, scansWhenUnset: Set<SourceKind>) -> Bool {
        switch mode {
        case .watch, .off: return true
        case .unset, .unknown: return hasFolders || scansWhenUnset.contains(kind)
        }
    }

    /// `scansWhenUnset` is the tools the core reads from their usual folder
    /// while unset, from its source copy (`unset_scans_conventional`).
    static func build(
        entries: [DaemonData.QueueEntry],
        projects: [ProjectRow],
        settings: DaemonData.Settings?,
        scansWhenUnset: Set<SourceKind>
    ) -> TracesTree {
        var folders: [String: FolderNode] = [:]
        var order: [String] = []
        func add(_ node: FolderNode) {
            guard folders[node.id] == nil else { return }
            folders[node.id] = node
            order.append(node.id)
        }
        // The core's rows first, so a folder takes its listed name and mode;
        // the bucket's `displayLabel` is the shared name, not the slug.
        for project in projects {
            add(FolderNode(
                id: project.projectId, label: project.displayLabel, mode: project.mode,
                offerableModes: project.offerableModes, sessions: [],
                disclosure: project.mode == .autoUpload ? project.automaticDisclosure : nil,
                isBucket: project.isUnresolvedBucket))
        }
        for entry in entries {
            add(FolderNode(id: entry.projectId, label: entry.projectLabel, mode: nil, sessions: []))
            folders[entry.projectId]?.sessions.append(entry)
        }

        var byTool: [SourceKind: [FolderNode]] = [:]
        var unplaced: [FolderNode] = []
        for id in order {
            guard var node = folders[id], node.mode != .ignore else { continue }
            node.sessions.sort { ($0.startedAt ?? .distantPast) > ($1.startedAt ?? .distantPast) }
            if let kind = majorityTool(node.sessions) {
                byTool[kind, default: []].append(node)
            } else {
                unplaced.append(node)
            }
        }

        let tools = SourceKind.allCases.compactMap { kind -> ToolNode? in
            let mode = settings == nil ? SourceMode.unknown : SourceMode(sourceMode(kind, in: settings))
            let nodes = (byTool[kind] ?? []).sorted { $0.sessions.count > $1.sessions.count }
            guard drawsTool(kind, mode: mode, hasFolders: !nodes.isEmpty, scansWhenUnset: scansWhenUnset) else { return nil }
            return ToolNode(kind: kind, mode: mode, folders: nodes)
        }
        .sorted { $0.waiting > $1.waiting }

        return TracesTree(tools: tools, unplaced: unplaced)
    }

    /// The tool most of a folder's sessions came from; `nil` with none, or
    /// when the sessions name no tool this build knows.
    static func majorityTool(_ sessions: [DaemonData.QueueEntry]) -> SourceKind? {
        var counts: [SourceKind: Int] = [:]
        for session in sessions {
            if let kind = SourceKind(rawValue: session.declaredSource ?? session.source)
                ?? SourceKind(rawValue: session.source)
            {
                counts[kind, default: 0] += 1
            }
        }
        return counts.max { $0.value < $1.value || ($0.value == $1.value && $0.key.rawValue > $1.key.rawValue) }?.key
    }

    /// `<tool>_source_mode` in `get_settings`. The settings keys name the
    /// tool, not the adapter: `claude`, `codex`, `gemini`, `cline`,
    /// `opencode`.
    static func sourceMode(_ kind: SourceKind, in settings: DaemonData.Settings?) -> String? {
        guard let settings else { return nil }
        switch kind {
        case .claudeCode: return settings.claudeSourceMode
        case .codex: return settings.codexSourceMode
        case .geminiCli: return settings.geminiSourceMode
        case .cline: return settings.clineSourceMode
        case .opencode: return settings.opencodeSourceMode
        }
    }

    /// Every session in the tree, in drawing order.
    var allSessions: [DaemonData.QueueEntry] {
        tools.flatMap { $0.folders.flatMap(\.sessions) } + unplaced.flatMap(\.sessions)
    }
}
