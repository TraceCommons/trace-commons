import Foundation
import TCShellCore

/// Bounded Home items derived only from the current local Skills snapshot.
enum InsightsSkillsHome {
    struct Snapshot {
        let updates: [Update]
        let trends: [Trend]
        let creators: [CreatorUpdate]
        /// The most-used skills of the past seven days, most used first.
        var week: [WeekSkill] = []
        /// Publishers with recent public activity, most active first. Only
        /// ever filled from a verified server catalog.
        var active: [Active] = []
        var isEmpty: Bool { updates.isEmpty && trends.isEmpty && creators.isEmpty && week.isEmpty && active.isEmpty }

        /// The feed: every item in one column, newest first, under the
        /// period it falls in. Empty periods are left out.
        ///
        /// Only a catalog change has a time of its own (when the catalog
        /// feed observed it). An available update and a usage change say
        /// what is true now, so both sit under Today whenever they were
        /// last checked: an update at its check time, never before the
        /// day began, and usage changes after everything else today, in
        /// the order given (largest change first). A catalog change for a
        /// skill whose update is already in the feed is the same news and
        /// is left out. Two or more changes from one publisher in one
        /// period are one roll-up, at the newest change's time. The week's
        /// most-used skills open This week. This week is the local ISO
        /// week, as elsewhere in Insights.
        func timeline(now: Date = Date(), calendar: Calendar = .current) -> [Group] {
            let today = calendar.startOfDay(for: now)
            var iso = Calendar(identifier: .iso8601)
            iso.timeZone = calendar.timeZone
            let week = min(today, iso.dateInterval(of: .weekOfYear, for: now)?.start ?? today)
            func period(_ date: Date) -> Period { date >= today ? .today : date >= week ? .thisWeek : .earlier }

            var entries: [(date: Date, rank: Int, story: Story)] = []
            entries += updates.map { (min(max($0.checkedAt, today), now), 0, .update($0)) }

            let updating = Set(updates.map(\.catalogSkill.id))
            let changes = creators.filter { !updating.contains($0.skill.id) }
            var catalog: [(date: Date, id: String, story: Story)] = []
            for (key, found) in Dictionary(grouping: changes, by: { RollupKey(period: period($0.observedAt), publisher: $0.publisher.id) }) {
                let newest = found.sorted { $0.observedAt == $1.observedAt ? $0.id < $1.id : $0.observedAt > $1.observedAt }
                if newest.count > 1 {
                    let rollup = Rollup(id: "\(key.period):\(key.publisher)", publisher: newest[0].publisher, changes: newest)
                    catalog.append((newest[0].observedAt, rollup.id, .rollup(rollup)))
                } else {
                    catalog += newest.map { ($0.observedAt, $0.id, .creator($0)) }
                }
            }
            // A dictionary has no order of its own: date, then id.
            entries += catalog.sorted { $0.date == $1.date ? $0.id < $1.id : $0.date > $1.date }.map { ($0.date, 1, $0.story) }
            entries += trends.map { (today, 2, .trend($0)) }

            // Each source is already in its own order; the index keeps it.
            let ordered = entries.enumerated().sorted { a, b in
                if a.element.date != b.element.date { return a.element.date > b.element.date }
                if a.element.rank != b.element.rank { return a.element.rank < b.element.rank }
                return a.offset < b.offset
            }.map(\.element)
            return Period.allCases.compactMap { which in
                var stories = ordered.filter { period($0.date) == which }.map(\.story)
                // The recap, then the active publishers, lead This week. The
                // publishers' signals have no day of their own to sort by.
                if which == .thisWeek, !active.isEmpty { stories.insert(.active(active), at: 0) }
                if which == .thisWeek, !self.week.isEmpty { stories.insert(.week(self.week), at: 0) }
                return stories.isEmpty ? nil : Group(period: which, stories: stories)
            }
        }

        /// The one story drawn tall: the first under Today, when it is
        /// about a catalog skill and so has a cover to show. A usage
        /// change, a roll-up, or a feed with nothing today has none.
        static func heroID(in groups: [Group]) -> String? {
            guard let first = groups.first, first.period == .today, let story = first.stories.first else { return nil }
            switch story {
            case .update, .creator: return story.id
            case .trend, .rollup, .week, .active: return nil
            }
        }
    }
    private struct RollupKey: Hashable {
        let period: Period
        let publisher: String
    }
    struct Update: Identifiable {
        let id: String
        let groupID: String
        let installedID: String
        let name: String
        let agents: [String]
        let catalogSkill: DaemonData.CatalogSkill
        let checkedAt: Date
        /// The catalog's name for the publisher, for the cover's mark.
        var publisherName: String? = nil
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
        /// Uses on each of the fourteen completed local days the story
        /// compares, oldest first: the previous seven, then the last
        /// seven. Empty when the story was not built from daily history.
        var daily: [Int] = []

        struct Bar: Identifiable, Equatable {
            let id: Int
            let uses: Int
            /// In the last seven days, the ones the headline is about.
            let recent: Bool
        }
        /// The chart's bars, or none unless there are exactly fourteen days.
        var bars: [Bar] {
            guard daily.count == 14 else { return [] }
            return daily.enumerated().map { Bar(id: $0.offset, uses: max(0, $0.element), recent: $0.offset >= 7) }
        }
    }
    struct WeekSkill: Identifiable, Equatable {
        let id: String
        let name: String
        let agent: String
        let uses: Int
    }
    /// A publisher the server catalog measured as recently active, with
    /// the public repository signals it is shown by. The signals are for
    /// the whole repository, not for its skills alone.
    struct Active: Identifiable {
        let publisher: DaemonData.SkillsPublisher
        let stars: Int
        /// Default-branch commits in the week before the server measured.
        let commits: Int
        let skills: [DaemonData.CatalogSkill]
        var id: String { publisher.id }

        /// A count in at most four characters, cut and never rounded up:
        /// 1,949 is "1.9K", not "2K".
        static func short(_ count: Int) -> String {
            for (unit, letter) in [(1_000_000, "M"), (1_000, "K")] where count >= unit {
                let tenths = count / (unit / 10)
                return tenths < 100 && tenths % 10 != 0 ? "\(tenths / 10).\(tenths % 10)\(letter)" : "\(tenths / 10)\(letter)"
            }
            return String(max(0, count))
        }
    }
    /// Several catalog changes from one publisher in one period.
    struct Rollup: Identifiable {
        let id: String
        let publisher: DaemonData.SkillsPublisher
        /// Newest first; at least two.
        let changes: [CreatorUpdate]
    }
    struct CreatorUpdate: Identifiable {
        let id: String
        let event: DaemonData.SkillsFeedEvent
        let skill: DaemonData.CatalogSkill
        let publisher: DaemonData.SkillsPublisher
        let observedAt: Date
    }

    /// How recent a story is. Headed in the feed in this order.
    enum Period: CaseIterable {
        case today, thisWeek, earlier

        var copyKey: String {
            switch self {
            case .today: "skills_home_today"
            case .thisWeek: "skills_home_this_week"
            case .earlier: "skills_home_earlier"
            }
        }
    }

    /// One item of the feed.
    enum Story: Identifiable {
        case update(Update), trend(Trend), creator(CreatorUpdate), rollup(Rollup), week([WeekSkill]), active([Active])

        var id: String {
            switch self {
            case .update(let item): item.id
            case .trend(let item): "trend:" + item.id
            case .creator(let item): "creator:" + item.id
            case .rollup(let item): "rollup:" + item.id
            case .week: "week"
            case .active: "active"
            }
        }
    }

    struct Group: Identifiable {
        let period: Period
        let stories: [Story]
        var id: Period { period }
    }

    @MainActor
    static func make(_ snapshot: DaemonData.SkillsSnapshot, model: InsightsSkillsModel,
                     now: Date = Date(), calendar: Calendar = .current) -> Snapshot {
        guard snapshot.enabled, snapshot.readable else { return .init(updates: [], trends: [], creators: []) }
        return .init(updates: personalizedUpdates(snapshot, model: model, now: now),
                     trends: usageChanges(snapshot.usage, now: now, calendar: calendar),
                     creators: creatorChanges(snapshot, now: now),
                     week: weekTop(snapshot.usage, now: now, calendar: calendar),
                     active: activePublishers(snapshot.catalog))
    }

    /// The daemon's recently active publishers, in its order, resolved
    /// against the rows of the same snapshot. Nothing is ranked here, and
    /// nothing local takes part: the daemon reads the order from the
    /// server catalog's public signals.
    ///
    /// Drawn only when the snapshot says its catalog is the verified
    /// server one. A publisher needs known stars and a known commit count
    /// (unknown is not zero) and at least one skill this snapshot carries
    /// under its own id; anything else is left out, never guessed at.
    private static func activePublishers(_ catalog: DaemonData.SkillsCatalog) -> [Active] {
        guard catalog.source == "server" else { return [] }
        var seen = Set<String>()
        let found = catalog.trending.compactMap { entry -> Active? in
            guard !seen.contains(entry.publisherID),
                  let publisher = catalog.publishers.first(where: { $0.id == entry.publisherID }),
                  let signals = catalog.publisherStates.first(where: { $0.publisherID == entry.publisherID })?.signals,
                  let stars = signals.stars, stars >= 0, let commits = signals.commits7d, commits >= 1 else { return nil }
            var named = Set<String>()
            let skills = entry.skillIDs.filter { named.insert($0).inserted }.compactMap { id in
                catalog.skills.first { $0.id == id && $0.publisherID == publisher.id }
            }
            guard !skills.isEmpty else { return nil }
            seen.insert(publisher.id)
            return Active(publisher: publisher, stars: stars, commits: commits, skills: Array(skills.prefix(3)))
        }
        return Array(found.prefix(3))
    }

    /// The three most-used skills of the last seven completed local days:
    /// the same days a usage story's last week is, so one skill never
    /// shows two totals in one feed. The daemon's rolling seven-day window
    /// is not used: it includes today. Left out while history is partial
    /// or was not analysed today, when a short count would read as the
    /// week; a skill whose daily history cannot be read is left out alone.
    private static func weekTop(_ usage: DaemonData.SkillsUsage, now: Date, calendar: Calendar) -> [WeekSkill] {
        let today = calendar.startOfDay(for: now)
        guard usage.enabled, !usage.partial,
              let checked = usage.lastAnalysisAt.flatMap(parse), checked >= today, checked <= now,
              let start = calendar.date(byAdding: .day, value: -7, to: today) else { return [] }
        let explicit = Set(usage.coverage.filter { $0.status == "explicit_invocations" }.map(\.agent))
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.calendar = calendar
        formatter.timeZone = calendar.timeZone
        formatter.dateFormat = "yyyy-MM-dd"
        let rows = usage.skills.filter { explicit.contains($0.agent) && ["direct", "delegated"].contains($0.population) }
        let skills = Dictionary(grouping: rows, by: { $0.agent + "\u{0}" + $0.name }).compactMap { id, rows -> WeekSkill? in
            guard let first = rows.first, Set(rows.map(\.population)).count == rows.count else { return nil }
            var uses: [Int] = []
            for day in rows.flatMap(\.daily) {
                guard day.invocations >= 0, let date = formatter.date(from: day.day), formatter.string(from: date) == day.day else { return nil }
                if date >= start && date < today { uses.append(day.invocations) }
            }
            guard let total = sum(uses), total > 0 else { return nil }
            return WeekSkill(id: id, name: first.name, agent: first.agent, uses: total)
        }
        return Array(skills.sorted { $0.uses == $1.uses ? $0.id < $1.id : $0.uses > $1.uses }.prefix(3))
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
                              name: installed.name, agents: group.agents, catalogSkill: catalog, checkedAt: checked,
                              publisherName: snapshot.catalog.publishers.first { $0.id == catalog.publisherID }?.name)
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
            var daily = [Int](repeating: 0, count: 14)
            for row in rows {
                guard Set(row.daily.map(\.day)).count == row.daily.count,
                      row.invocations == 0 || !row.daily.isEmpty else { return nil }
                for day in row.daily {
                    guard day.invocations >= 0, let start = date(day.day), start <= today else { return nil }
                    if start >= currentStart && start < today { after.append(day.invocations) }
                    else if start >= previousStart && start < currentStart { before.append(day.invocations) }
                    else { continue }
                    guard let index = calendar.dateComponents([.day], from: previousStart, to: start).day,
                          daily.indices.contains(index) else { return nil }
                    let next = daily[index].addingReportingOverflow(day.invocations)
                    guard !next.overflow else { return nil }
                    daily[index] = next.partialValue
                }
            }
            guard let previous = sum(before), let current = sum(after), previous != current else { return nil }
            return Trend(id: id, name: first.name, agent: first.agent, previousUses: previous, currentUses: current,
                         direction: current > previous ? (previous == 0 ? .started : .more) : .less, periodEnd: today,
                         daily: daily)
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
