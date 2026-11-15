import Foundation
import TCShellCore

/// Content identity groups copies across agents, never just a display name.
enum InsightsSkillsIndex {
    struct Group: Identifiable {
        let installations: [DaemonData.InstalledSkill]
        var id: String { installations[0].id }
        var representative: DaemonData.InstalledSkill { installations[0] }
        var agents: [String] { Array(Set(installations.map(\.agent))).sorted() }

        func definitionInstallation(in snapshot: DaemonData.SkillsSnapshot) -> DaemonData.InstalledSkill {
            installations.first {
                let details = snapshot.inventory.details[$0.id]
                return details?.availability == "available" && details?.definitionStatus == "valid"
            } ?? representative
        }
    }

    static func groups(_ installations: [DaemonData.InstalledSkill],
                       matching orderedIDs: [String]? = nil) -> [Group] {
        let buckets = Dictionary(grouping: installations) { skill in
            let hash = skill.sha256.lowercased()
            // Missing/invalid hashes cannot establish identical definitions.
            guard hash.count == 64, hash.allSatisfy({ $0.isHexDigit }) else { return "id:\(skill.id)" }
            return "content:\(skill.name)\u{0}\(hash)"
        }
        var result: [Group] = []
        for copies in buckets.values {
            let origins = Set(copies.compactMap { $0.source.map { $0.repository + "\u{0}" + $0.path } })
            if origins.count > 1 {
                // Equal files can be published independently. Keep conflicting
                // known origins separate; unknown origins cannot pick a side.
                let partitions = Dictionary(grouping: copies) { skill in
                    skill.source.map { $0.repository + "\u{0}" + $0.path } ?? "unknown:\(skill.id)"
                }
                result += partitions.values.map { Group(installations: $0.sorted { $0.id < $1.id }) }
            } else {
                result.append(Group(installations: copies.sorted { $0.id < $1.id }))
            }
        }
        if let orderedIDs {
            let positions = Dictionary(uniqueKeysWithValues: orderedIDs.enumerated().map { ($1, $0) })
            func position(_ group: Group) -> Int { group.installations.compactMap { positions[$0.id] }.min() ?? .max }
            return result.filter { position($0) != .max }.sorted { position($0) == position($1) ? $0.id < $1.id : position($0) < position($1) }
        }
        return result.sorted { $0.id < $1.id }
    }

    @MainActor
    static func usageRows(_ group: Group, snapshot: DaemonData.SkillsSnapshot,
                          model: InsightsSkillsModel) -> [DaemonData.SkillsUsageRow] {
        let rows = group.installations.flatMap { model.usageRows(for: $0, in: snapshot) }
        return Dictionary(grouping: rows, by: \.identity).values.compactMap(\.first).sorted { $0.identity < $1.identity }
    }
}
