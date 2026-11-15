import Foundation
import TCShellCore

/// Bounded Home items derived only from the current local Skills snapshot.
enum InsightsSkillsHome {
    struct Snapshot {
        let updates: [Update]
        let trends: [Trend]
        let creators: [CreatorUpdate]
        var isEmpty: Bool { updates.isEmpty && trends.isEmpty && creators.isEmpty }
    }
    struct Update: Identifiable {
        let id: String
        let groupID: String
        let installedID: String
        let name: String
        let agents: [String]
        let catalogSkill: DaemonData.CatalogSkill
        let checkedAt: Date
    }
    enum Direction { case more, less, started }
    struct Trend: Identifiable {
        let id: String
        let name: String
        let agent: String
        let previousUses: Int
        let currentUses: Int
        let direction: Direction
        let periodEnd: Date
    }
    struct CreatorUpdate: Identifiable {
        let id: String
        let event: DaemonData.SkillsFeedEvent
        let skill: DaemonData.CatalogSkill
        let publisher: DaemonData.SkillsPublisher
        let observedAt: Date
    }

    @MainActor
    static func make(_ snapshot: DaemonData.SkillsSnapshot, model: InsightsSkillsModel,
                     now: Date = Date(), calendar: Calendar = .current) -> Snapshot {
        guard snapshot.enabled, snapshot.readable else { return .init(updates: [], trends: [], creators: []) }
        return .init(updates: personalizedUpdates(snapshot, model: model, now: now),
                     trends: usageChanges(snapshot.usage, now: now, calendar: calendar),
                     creators: creatorChanges(snapshot, now: now))
    }

    @MainActor
    private static func personalizedUpdates(_ snapshot: DaemonData.SkillsSnapshot,
                                            model: InsightsSkillsModel, now: Date) -> [Update] {
        guard snapshot.usage.enabled else { return [] }
        let explicit = Set(snapshot.usage.coverage.filter { $0.status == "explicit_invocations" }.map(\.agent))
        let groups = InsightsSkillsIndex.groups(snapshot.inventory.skills)
        var items: [Update] = []
        for group in groups {
            let used = InsightsSkillsIndex.usageRows(group, snapshot: snapshot, model: model).contains { row in
                explicit.contains(row.agent) && ["direct", "delegated"].contains(row.population)
                    && row.windows.contains { $0.days == 30 && $0.invocations > 0 }
            }
            guard used else { continue }
            let candidates = group.installations.compactMap { installed -> Update? in
                guard validHash(installed.sha256), let source = installed.source,
                      let update = snapshot.updates.first(where: { $0.installedID == installed.id && $0.status == "update_available" }),
                      let catalog = snapshot.catalog.skills.first(where: { $0.id == update.catalogSkillID }),
                      source.repository == catalog.repository, source.path == catalog.path,
                      validHash(catalog.sha256), installed.sha256.lowercased() != catalog.sha256.lowercased(),
                      let state = snapshot.catalog.publisherStates.first(where: { $0.publisherID == catalog.publisherID && $0.status == "current" }),
                      let checked = state.checkedAt.flatMap(parse), checked <= now,
                      now.timeIntervalSince(checked) < 6 * 60 * 60 else { return nil }
                return Update(id: "update:\(group.id)", groupID: group.id, installedID: installed.id,
                              name: installed.name, agents: group.agents, catalogSkill: catalog, checkedAt: checked)
            }
            if let candidate = candidates.sorted(by: { $0.installedID < $1.installedID }).first { items.append(candidate) }
        }
        return Array(items.sorted { $0.checkedAt == $1.checkedAt ? $0.id < $1.id : $0.checkedAt > $1.checkedAt }.prefix(3))
    }

    private static func usageChanges(_ usage: DaemonData.SkillsUsage, now: Date, calendar: Calendar) -> [Trend] {
        let today = calendar.startOfDay(for: now)
        guard usage.enabled, !usage.partial,
              let checked = usage.lastAnalysisAt.flatMap(parse), checked >= today, checked <= now,
              let previousStart = calendar.date(byAdding: .day, value: -14, to: today),
              let currentStart = calendar.date(byAdding: .day, value: -7, to: today),
              let earliest = usage.earliestEvidence.flatMap(parse), earliest <= previousStart else { return [] }
        let explicit = Set(usage.coverage.filter { $0.status == "explicit_invocations" }.map(\.agent))
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.dateFormat = "yyyy-MM-dd"
        func date(_ day: String) -> Date? {
            guard let date = formatter.date(from: day), formatter.string(from: date) == day else { return nil }
            return date
        }
        // A different agent's older records cannot establish this agent's span.
        let establishedAgents = Set(explicit.filter { agent in
            usage.skills.filter { $0.agent == agent && ["direct", "delegated"].contains($0.population) }
                .flatMap(\.daily).contains { day in
                    guard day.invocations > 0, let start = date(day.day),
                          let end = calendar.date(byAdding: .day, value: 1, to: start) else { return false }
                    return end <= previousStart
                }
        })
        let grouped = Dictionary(grouping: usage.skills.filter { establishedAgents.contains($0.agent) }, by: { $0.agent + "\u{0}" + $0.name })
        let items = grouped.compactMap { id, rows -> Trend? in
            guard let first = rows.first, Set(rows.map(\.population)).count == rows.count,
                  rows.allSatisfy({ ["direct", "delegated"].contains($0.population) && $0.undatedEvents == 0 }) else { return nil }
            var before: [Int] = [], after: [Int] = []
            for row in rows {
                guard Set(row.daily.map(\.day)).count == row.daily.count,
                      row.invocations == 0 || !row.daily.isEmpty else { return nil }
                for day in row.daily {
                    guard day.invocations >= 0, let start = date(day.day), start <= today else { return nil }
                    if start >= currentStart && start < today { after.append(day.invocations) }
                    else if start >= previousStart && start < currentStart { before.append(day.invocations) }
                }
            }
            guard let previous = sum(before), let current = sum(after), previous != current else { return nil }
            return Trend(id: id, name: first.name, agent: first.agent, previousUses: previous, currentUses: current,
                         direction: current > previous ? (previous == 0 ? .started : .more) : .less, periodEnd: today)
        }
        return Array(items.sorted {
            let a = abs($0.currentUses - $0.previousUses), b = abs($1.currentUses - $1.previousUses)
            return a == b ? $0.id < $1.id : a > b
        }.prefix(3))
    }

    private static func creatorChanges(_ snapshot: DaemonData.SkillsSnapshot, now: Date) -> [CreatorUpdate] {
        let followed = Set(snapshot.followedPublishers)
        let oldest = now.addingTimeInterval(-30 * 86_400)
        let valid = snapshot.feed.compactMap { event -> CreatorUpdate? in
            guard followed.contains(event.publisherID),
                  ["added", "updated"].contains(event.kind), let observed = parse(event.observedAt), observed <= now, observed >= oldest,
                  let publisher = snapshot.catalog.publishers.first(where: { $0.id == event.publisherID }),
                  let skill = snapshot.catalog.skills.first(where: { $0.id == event.skillID && $0.publisherID == event.publisherID }) else { return nil }
            return CreatorUpdate(id: event.id, event: event, skill: skill, publisher: publisher, observedAt: observed)
        }.sorted { $0.observedAt == $1.observedAt ? $0.id < $1.id : $0.observedAt > $1.observedAt }
        // Repeated updates to one skill do not crowd out other creators/skills.
        var seen = Set<String>()
        return Array(valid.filter { seen.insert($0.publisher.id + "\u{0}" + $0.skill.id).inserted }.prefix(5))
    }

    private static func validHash(_ value: String) -> Bool { value.count == 64 && value.allSatisfy(\.isHexDigit) }
    private static func sum(_ values: [Int]) -> Int? {
        var result = 0
        for value in values {
            let next = result.addingReportingOverflow(value)
            guard !next.overflow else { return nil }
            result = next.partialValue
        }
        return result
    }
    private static func parse(_ value: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        if let value = formatter.date(from: value) { return value }
        formatter.formatOptions.insert(.withFractionalSeconds)
        return formatter.date(from: value)
    }
}
