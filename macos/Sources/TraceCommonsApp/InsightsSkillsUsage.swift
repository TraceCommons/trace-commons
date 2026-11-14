import Foundation
import TCShellCore

/// Invocation counts can be added across disjoint direct/delegated populations.
/// Session counts and load-only observations never stand in for uses.
enum InsightsSkillsUsage {
    struct Day: Identifiable, Equatable {
        let id: String
        let uses: Int
    }
    struct Skill: Identifiable {
        let id: String
        let name: String
        let agent: String
        let uses: Int
        let daily: [Day]
        let start: Date
        let end: Date
        let sources: [DaemonData.SkillsUsageRow]
    }

    static func emptyState(_ usage: DaemonData.SkillsUsage, days: Int) -> String {
        guard usage.enabled else { return "skills_analysis_off" }
        guard usage.coverage.contains(where: { $0.status == "explicit_invocations" }) else {
            return "skills_usage_unavailable"
        }
        let agents = Set(usage.coverage.filter { $0.status == "explicit_invocations" }.map(\.agent))
        let relevant = usage.skills.filter { agents.contains($0.agent) }
        guard !usage.partial, usage.lastAnalysisAt != nil, [7, 30, 90].contains(days),
              relevant.allSatisfy({ row in
                  ["direct", "delegated"].contains(row.population) && row.undatedEvents == 0
                      && row.windows.contains { $0.days == days }
              }) else { return "skills_usage_pending" }
        return "skills_usage_empty"
    }

    static func ranked(_ usage: DaemonData.SkillsUsage, days: Int, now: Date = Date()) -> [Skill] {
        guard usage.enabled, [7, 30, 90].contains(days) else { return [] }
        let agents = Set(usage.coverage.filter { $0.status == "explicit_invocations" }.map(\.agent))
        let rows = usage.skills.filter {
            agents.contains($0.agent) && ["direct", "delegated"].contains($0.population)
                && $0.windows.contains { $0.days == days }
        }
        return Dictionary(grouping: rows, by: { $0.agent + "\u{0}" + $0.name }).compactMap { id, rows in
            guard let first = rows.first else { return nil }
            let total = rows.reduce(0) { $0 + max(0, $1.windows.first { $0.days == days }?.invocations ?? 0) }
            guard total > 0 else { return nil }
            // Daemon windows roll from request time; lastAnalysisAt describes
            // the counter pass, not the window endpoint. Buckets are local days.
            let anchor = now
            let cutoff = anchor.addingTimeInterval(-Double(days) * 86_400)
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.dateFormat = "yyyy-MM-dd"
            // The first bucket may include events before the rolling cutoff;
            // omit it rather than pretend its whole-day count fits the window.
            let firstFullDay = Calendar.current.date(byAdding: .day, value: 1,
                to: Calendar.current.startOfDay(for: cutoff)) ?? cutoff
            let firstDay = formatter.string(from: firstFullDay)
            let lastDay = formatter.string(from: anchor)
            let daily = Dictionary(grouping: rows.flatMap(\.daily).filter {
                $0.day >= firstDay && $0.day <= lastDay
            }, by: \.day).map { day, values in
                Day(id: day, uses: values.reduce(0) { $0 + max(0, $1.invocations) })
            }.sorted { $0.id < $1.id }
            return Skill(id: id, name: first.name, agent: first.agent, uses: total,
                         daily: daily, start: cutoff, end: anchor, sources: rows.sorted { $0.identity < $1.identity })
        }.sorted { $0.uses == $1.uses ? $0.id < $1.id : $0.uses > $1.uses }
    }

    @MainActor
    static func removalCandidates(_ snapshot: DaemonData.SkillsSnapshot, days: Int,
                                  model: InsightsSkillsModel, now: Date = Date()) -> [DaemonData.InstalledSkill] {
        let usage = snapshot.usage
        guard usage.enabled, !usage.partial, !snapshot.inventory.partial, [7, 30, 90].contains(days),
              let checked = usage.lastAnalysisAt.flatMap(parse),
              let earliest = usage.earliestEvidence.flatMap(parse),
              earliest <= now.addingTimeInterval(-Double(days) * 86_400), checked <= now else { return [] }
        return snapshot.inventory.skills.filter { skill in
            guard let details = snapshot.inventory.details[skill.id],
                  details.availability == "available", details.definitionStatus == "valid" else { return false }
            let requiredAgents = skill.agent == "shared" ? ["codex", "claude_code", "opencode"] : [skill.agent]
            let cutoff = now.addingTimeInterval(-Double(days) * 86_400)
            let formatter = DateFormatter()
            formatter.locale = Locale(identifier: "en_US_POSIX")
            formatter.dateFormat = "yyyy-MM-dd"
            guard requiredAgents.allSatisfy({ agent in
                usage.coverage.contains { $0.agent == agent && $0.status == "explicit_invocations" }
                    && usage.skills.filter { $0.agent == agent && ["direct", "delegated"].contains($0.population) }
                        .flatMap(\.daily).contains { day in
                            guard day.invocations > 0, let date = formatter.date(from: day.day),
                                  let end = Calendar.current.date(byAdding: .day, value: 1, to: date) else { return false }
                            return end <= cutoff
                        }
            }), model.usageState(for: skill, in: snapshot) == "skills_no_usage" else { return false }
            let rows = model.usageRows(for: skill, in: snapshot)
            return rows.allSatisfy { row in
                guard ["direct", "delegated"].contains(row.population), row.undatedEvents == 0,
                      let window = row.windows.first(where: { $0.days == days }) else { return false }
                return window.invocations == 0 && window.loads == 0
            }
        }.sorted { $0.name == $1.name ? $0.id < $1.id : $0.name.localizedStandardCompare($1.name) == .orderedAscending }
    }

    private static func parse(_ value: String) -> Date? {
        let formatter = ISO8601DateFormatter()
        if let date = formatter.date(from: value) { return date }
        formatter.formatOptions.insert(.withFractionalSeconds)
        return formatter.date(from: value)
    }
}
