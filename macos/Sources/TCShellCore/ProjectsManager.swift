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
/// suffix because their folder names collide (`policy.rs`
/// `label_with_suffix_if`), usually clones or copies of one repository.
public struct ProjectGroup: Identifiable, Equatable, Sendable {
    /// The case-folded base name, which is what the daemon compares.
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

    /// Traces waiting across the members, which a Never would clear.
    public var pending: Int { members.reduce(0) { $0 + ($1.pendingCount ?? 0) } }
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

/// What a bulk or group mode change does, by project id.
public enum ProjectsBulkPlan: Equatable, Sendable {
    /// Every target is already there, or none can take the mode.
    case nothing
    /// Set each to the mode, one existing `set_project_mode` call each.
    case apply([String])
    /// Never would clear `pending` waiting traces: ask first, as one row's
    /// Never does.
    case confirmNever([String], pending: Int)
    /// Automatic is a consent grant, made one project at a time through the
    /// arming confirmation; it is never applied in bulk.
    case refused
}

/// Settings > Projects' summary and Manage sheet, as pure rules (owner
/// design, 2026-11-04).
public enum ProjectsManager {
    /// The modes a bulk or group change may set. Never Automatic: arming is
    /// a grant (owner rulings 2026-10-02), so it stays per project, behind
    /// the arming confirmation.
    public static let bulkModes: [ProjectMode] = [.ask, .ignore]

    /// A label's base name and the daemon's four-hex-digit suffix, or nil
    /// when it has none. Exactly the daemon's shape: `"{label} ({hex4})"`
    /// with lowercase hex.
    public static func split(_ label: String) -> (base: String, suffix: String)? {
        let chars = Array(label)
        // "x (abcd)" is at least 8 characters.
        guard chars.count >= 8, chars[chars.count - 1] == ")", chars[chars.count - 6] == "(",
              chars[chars.count - 7] == " "
        else { return nil }
        let hex = chars[(chars.count - 5)..<(chars.count - 1)]
        guard hex.allSatisfy({ ("0"..."9").contains($0) || ("a"..."f").contains($0) }) else { return nil }
        let base = String(chars[0..<(chars.count - 7)])
        guard !base.trimmingCharacters(in: .whitespaces).isEmpty else { return nil }
        return (base, String(hex))
    }

    /// What a row in a group shows: its suffix, the only part that differs.
    /// A row alone shows its whole label.
    public static func memberLabel(_ row: ProjectRow) -> String {
        guard !row.isUnresolvedBucket, let split = split(row.displayLabel) else { return row.displayLabel }
        return split.suffix
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

    /// The rows as entries: rows sharing a base name (two or more) become a
    /// group where the first of them stood; every other row stands alone.
    public static func entries(_ rows: [ProjectRow]) -> [ProjectEntry] {
        func key(_ row: ProjectRow) -> String? {
            guard !row.isUnresolvedBucket, let split = split(row.displayLabel) else { return nil }
            return split.base.lowercased()
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
            let name = split(group[0].displayLabel)?.base ?? group[0].displayLabel
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
    /// hidden by the search or filter is not acted on.
    public static func selected(_ ids: Set<String>, in shown: [ProjectRow]) -> [ProjectRow] {
        shown.filter { ids.contains($0.id) }
    }

    /// What setting `rows` to `mode` at once does. Rows already in the mode
    /// and rows that cannot take it (`offerableModes`) are skipped.
    public static func plan(_ rows: [ProjectRow], to mode: ProjectMode) -> ProjectsBulkPlan {
        guard bulkModes.contains(mode) else { return .refused }
        let targets = rows.filter { $0.mode != mode && $0.offerableModes.contains(mode) }
        guard !targets.isEmpty else { return .nothing }
        let ids = targets.map(\.id)
        if mode == .ignore {
            let pending = targets.reduce(0) { $0 + ($1.pendingCount ?? 0) }
            if pending > 0 { return .confirmNever(ids, pending: pending) }
        }
        return .apply(ids)
    }
}
