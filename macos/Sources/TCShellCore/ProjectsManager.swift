import Foundation

/// How many projects are in each mode, as `list_projects` reports them.
public struct ProjectModeCounts: Equatable, Sendable {
    public var automatic: Int
    public var ask: Int
    public var never: Int

    public init(automatic: Int = 0, ask: Int = 0, never: Int = 0) {
        self.automatic = automatic
        self.ask = ask
        self.never = never
    }

    public var total: Int { automatic + ask + never }

    public func count(for mode: ProjectMode) -> Int {
        switch mode {
        case .autoUpload: automatic
        case .ask: ask
        case .ignore: never
        }
    }
}

/// Projects whose labels share a base name: the daemon gave each a short
/// suffix because their folder names collide, and says so on the row
/// (`ProjectRow.collision`). Usually clones or copies of one repository.
public struct ProjectGroup: Identifiable, Equatable, Sendable {
    /// The daemon's collision group.
    public let id: String
    /// The base name as the first member spells it.
    public let name: String
    public let members: [ProjectRow]

    public init(id: String, name: String, members: [ProjectRow]) {
        self.id = id
        self.name = name
        self.members = members
    }

    /// The members' mode when every one agrees; nil when they differ, which
    /// the sheet shows as Mixed.
    public var mode: ProjectMode? {
        guard let first = members.first?.mode else { return nil }
        return members.allSatisfy { $0.mode == first } ? first : nil
    }

    public var memberIDs: [String] { members.map(\.id) }
}

/// One entry of the Manage sheet: a project alone, or a group of them.
public enum ProjectEntry: Equatable, Sendable {
    case single(ProjectRow)
    case group(ProjectGroup)
}

/// One drawn line of the Manage sheet's list, flat so the list can be lazy.
public enum ProjectLine: Identifiable, Equatable, Sendable {
    case single(ProjectRow)
    case group(ProjectGroup, expanded: Bool)
    case member(ProjectRow, group: String)

    public var id: String {
        switch self {
        case .single(let row), .member(let row, _): "row:" + row.id
        case .group(let group, _): "group:" + group.id
        }
    }
}

/// A checkbox over several rows: none, some or all of them selected.
public enum ProjectsSelectionState: Equatable, Sendable {
    case none, some, all
}

/// What a bulk or group mode change will do, as the daemon plans it
/// (`plan_project_modes`, `daemon::project_bulk`). The rule lives there,
/// read from the live policy and queue, so no shell decides when a bulk
/// change asks first: a change that turns off Automatic, or a Never that
/// removes waiting traces, asks; Automatic is never set in bulk.
public struct ProjectsBulkPlan: Decodable, Equatable, Sendable {
    public enum Outcome: String, Decodable, Equatable, Sendable {
        case nothing, apply, confirm, refused
    }

    /// The confirmation's words, composed by the core.
    public struct Confirmation: Decodable, Equatable, Sendable {
        public let title: String
        public let body: String
        public let button: String
        public let keep: String
        public let help: String?
        public let destructive: Bool

        public init(title: String, body: String, button: String, keep: String, help: String?, destructive: Bool) {
            self.title = title
            self.body = body
            self.button = button
            self.keep = keep
            self.help = help
            self.destructive = destructive
        }
    }

    public let outcome: Outcome
    /// The projects the change reaches.
    public let projectIds: [String]
    public let turnsOffAutomatic: Int
    public let pending: Int
    /// Present when `outcome` is `confirm`.
    public let confirmation: Confirmation?
    /// The modes a bulk change may set: the menus offer these and nothing
    /// else.
    public let bulkModes: [ProjectMode]

    public init(
        outcome: Outcome, projectIds: [String], turnsOffAutomatic: Int = 0, pending: Int = 0,
        confirmation: Confirmation? = nil, bulkModes: [ProjectMode] = [.ask, .ignore]
    ) {
        self.outcome = outcome
        self.projectIds = projectIds
        self.turnsOffAutomatic = turnsOffAutomatic
        self.pending = pending
        self.confirmation = confirmation
        self.bulkModes = bulkModes
    }

    enum CodingKeys: String, CodingKey {
        case outcome
        case projectIds = "project_ids"
        case turnsOffAutomatic = "turns_off_automatic"
        case pending, confirmation
        case bulkModes = "bulk_modes"
    }
}

/// What a bulk change did (`set_project_modes`): the projects that changed,
/// of how many it set out to change, and the refusal that stopped it.
public struct ProjectsBulkResult: Decodable, Equatable, Sendable {
    public let changed: [String]
    public let total: Int
    public let refused: String?

    public init(changed: [String], total: Int, refused: String? = nil) {
        self.changed = changed
        self.total = total
        self.refused = refused
    }
}

/// Settings > Projects' summary and Manage sheet, as pure rules (owner
/// design, 2026-11-04). Grouping is the daemon's (`ProjectRow.collision`)
/// and the bulk rule is the daemon's (`ProjectsBulkPlan`); what is left
/// here is layout and selection.
public enum ProjectsManager {
    /// What a row in a group shows: its suffix, the only part that differs.
    /// A row alone shows its whole label.
    public static func memberLabel(_ row: ProjectRow) -> String {
        guard !row.isUnresolvedBucket, let collision = row.collision else { return row.displayLabel }
        return collision.suffix
    }

    public static func counts(_ rows: [ProjectRow]) -> ProjectModeCounts {
        var counts = ProjectModeCounts()
        for row in rows {
            switch row.mode {
            case .autoUpload: counts.automatic += 1
            case .ask: counts.ask += 1
            case .ignore: counts.never += 1
            }
        }
        return counts
    }

    /// The rows matching `query` (in the shown name, ignoring case and the
    /// spaces around it) and `mode` (nil for all), in their order.
    public static func filter(_ rows: [ProjectRow], query: String, mode: ProjectMode?) -> [ProjectRow] {
        let needle = query.trimmingCharacters(in: .whitespacesAndNewlines)
        return rows.filter { row in
            (mode == nil || row.mode == mode)
                && (needle.isEmpty || row.displayLabel.localizedCaseInsensitiveContains(needle))
        }
    }

    /// The rows as entries: rows in one of the daemon's collision groups
    /// (two or more of them shown) become a group where the first of them
    /// stood; every other row stands alone. A row with no `collision` is
    /// never grouped, so an older daemon draws a plain list.
    public static func entries(_ rows: [ProjectRow]) -> [ProjectEntry] {
        func key(_ row: ProjectRow) -> String? {
            row.isUnresolvedBucket ? nil : row.collision?.group
        }
        var members: [String: [ProjectRow]] = [:]
        for row in rows {
            if let key = key(row) { members[key, default: []].append(row) }
        }
        var entries: [ProjectEntry] = []
        var placed: Set<String> = []
        for row in rows {
            guard let key = key(row), let group = members[key], group.count > 1 else {
                entries.append(.single(row))
                continue
            }
            guard placed.insert(key).inserted else { continue }
            let name = group[0].collision?.baseLabel ?? group[0].displayLabel
            entries.append(.group(ProjectGroup(id: key, name: name, members: group)))
        }
        return entries
    }

    /// The entries as drawn lines. A group shows its members when open, and
    /// every group is open while searching, so a match is never hidden.
    public static func lines(_ entries: [ProjectEntry], expanded: Set<String>, expandAll: Bool) -> [ProjectLine] {
        entries.flatMap { entry -> [ProjectLine] in
            switch entry {
            case .single(let row): return [.single(row)]
            case .group(let group):
                let open = expandAll || expanded.contains(group.id)
                return [.group(group, expanded: open)] + (open ? group.members.map { .member($0, group: group.id) } : [])
            }
        }
    }

    /// The selected rows among those shown, in their order. A selection
    /// hidden by the search or the filter is not acted on. One inside a
    /// collapsed group is: the group's header checkbox shows it (checked or
    /// mixed), and that checkbox is how a whole group is selected without
    /// opening it.
    public static func selected(_ ids: Set<String>, in shown: [ProjectRow]) -> [ProjectRow] {
        shown.filter { ids.contains($0.id) }
    }

    /// How many of `ids` are selected, for a checkbox over them.
    public static func selectionState(_ selection: Set<String>, of ids: [String]) -> ProjectsSelectionState {
        let picked = ids.reduce(0) { $0 + (selection.contains($1) ? 1 : 0) }
        if picked == 0 { return .none }
        return picked == ids.count ? .all : .some
    }

    /// What a checkbox over `ids` does when set `on` or off: all of them
    /// selected, or none of them. Other selections stay. The same answer
    /// however many times it is applied, because a mixed checkbox writes
    /// each of its sources.
    public static func selecting(_ selection: Set<String>, ids: [String], on: Bool) -> Set<String> {
        on ? selection.union(ids) : selection.subtracting(ids)
    }

    /// The selection after a bulk change: only the projects that changed
    /// leave it, so a change stopped part way keeps the rest selected.
    public static func afterChange(_ selection: Set<String>, changed: [String]) -> Set<String> {
        selection.subtracting(changed)
    }
}
