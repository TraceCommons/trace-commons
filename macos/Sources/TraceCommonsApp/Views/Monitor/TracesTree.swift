import Foundation
import TCShellCore

/// The Traces tree (R6 of #1173): tool › folder › session, built only from
/// what the core reports through `DaemonDataClient`. Nothing is estimated.
///
/// The same rules as #1146's `traces-model.ts`:
/// - A tool's switch is its source declaration. `unset` is never drawn as
///   off: an unset tool has no switch at all.
/// - A folder sits under the tool most of its sessions came from. The core
///   does not yet say which tool a folder belongs to (K11 of #1173); until it
///   does, a folder with no waiting session cannot be placed and is listed
///   on its own after the tools.
/// - Ignored folders are left out.
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
    }

    /// A tool's source declaration in `get_settings`.
    enum SourceMode: Equatable {
        case watch, off, unset

        init(_ raw: String?) {
            switch raw {
            case "watch": self = .watch
            case "off": self = .off
            default: self = .unset
            }
        }
    }

    static func build(
        entries: [DaemonData.QueueEntry],
        projects: [ProjectRow],
        settings: DaemonData.Settings?
    ) -> TracesTree {
        var folders: [String: FolderNode] = [:]
        var order: [String] = []
        func folder(_ id: String, _ label: String, _ mode: ProjectMode?, _ modes: [ProjectMode]) {
            guard folders[id] == nil else { return }
            folders[id] = FolderNode(id: id, label: label, mode: mode, offerableModes: modes, sessions: [])
            order.append(id)
        }
        for project in projects where !project.isUnresolvedBucket {
            folder(project.projectId, project.displayLabel, project.mode, project.offerableModes)
        }
        for entry in entries {
            folder(entry.projectId, entry.projectLabel, nil, [])
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
            let mode = SourceMode(sourceMode(kind, in: settings))
            let nodes = (byTool[kind] ?? []).sorted { $0.sessions.count > $1.sessions.count }
            // A tool with nothing declared and nothing waiting is not drawn.
            guard mode != .unset || !nodes.isEmpty else { return nil }
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
