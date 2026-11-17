import Foundation

extension InsightsSkillsHome {
    /// Catalog IDs are exact identities. Usage IDs include the agent and
    /// resolve to catalog IDs only through a verified installed source.
    struct RankingContext {
        var usedSkillIDs: Set<String> = []
        var installedSkillIDs: Set<String> = []
        var followedPublisherIDs: Set<String> = []
        var catalogIDsByUsageID: [String: Set<String>] = [:]
    }
}

extension InsightsSkillsHome.Snapshot {
    private typealias Story = InsightsSkillsHome.Story
    private struct RankedCandidate {
        let story: Story
        let source: String
        let publishers: Set<String>
        let topics: Set<String>
        let score: Double
        let observed: Date
    }

    /// Select locally from supported stories, without changing their facts
    /// or timestamps. Counts can add at most a small, bounded score; exact
    /// local identity and an explicit follow matter more than volume.
    func rankedStories(now: Date = Date(), calendar: Calendar = .current) -> [InsightsSkillsHome.Story] {
        var updateIDs = Set<String>()
        let uniqueUpdates = updates.sorted {
            $0.checkedAt == $1.checkedAt ? $0.id < $1.id : $0.checkedAt > $1.checkedAt
        }.filter {
            $0.checkedAt <= now && now.timeIntervalSince($0.checkedAt) < 6 * 3_600
                && updateIDs.insert($0.catalogSkill.id).inserted
        }
        var creatorIDs = Set<String>()
        let uniqueCreators = creators.sorted {
            $0.observedAt == $1.observedAt ? $0.id < $1.id : $0.observedAt > $1.observedAt
        }.filter {
            $0.observedAt <= now && now.timeIntervalSince($0.observedAt) <= 30 * 86_400
                && !updateIDs.contains($0.skill.id) && creatorIDs.insert($0.skill.id).inserted
        }
        var trendIDs = Set<String>()
        let meaningfulTrends = trends.sorted { $0.id < $1.id }.filter {
            guard trendIDs.insert($0.id).inserted,
                  $0.previousUses >= 0, $0.currentUses >= 0,
                  $0.periodEnd <= now, calendar.isDate($0.periodEnd, inSameDayAs: now) else { return false }
            // Convert before subtracting: counts may approach Int.max.
            let delta = abs(Double($0.currentUses) - Double($0.previousUses))
            return delta >= 2 && delta / max(1, Double($0.previousUses)) >= 0.25
        }
        let normalized = Self(updates: uniqueUpdates, trends: meaningfulTrends, creators: uniqueCreators,
                              week: week, active: active, context: context)
        let candidates = normalized.timeline(now: now, calendar: calendar).flatMap(\.stories).map {
            rankedCandidate($0, now: now)
        }
        var remaining = boundedCandidates(candidates)
        var selected: [RankedCandidate] = []
        while !remaining.isEmpty && selected.count < 12 {
            let recent = Array(selected.suffix(3))
            let best = remaining.indices.min { lhs, rhs in
                let a = remaining[lhs], b = remaining[rhs]
                let aScore = a.score - repetitionPenalty(a, recent: recent)
                let bScore = b.score - repetitionPenalty(b, recent: recent)
                if aScore != bScore { return aScore > bScore }
                if a.observed != b.observed { return a.observed > b.observed }
                return a.story.id < b.story.id
            }!
            selected.append(remaining.remove(at: best))
        }
        return selected.map(\.story)
    }

    /// The first ranked card may use its real catalog cover. Later cards
    /// are never promoted merely to fill an illustration slot.
    static func rankedHeroID(in stories: [InsightsSkillsHome.Story]) -> String? {
        guard let first = stories.first else { return nil }
        switch first {
        case .update, .creator: return first.id
        case .trend, .rollup, .week, .active: return nil
        }
    }

    private func rankedCandidate(_ story: Story, now: Date) -> RankedCandidate {
        func freshness(_ date: Date, scale: Double, days: Double) -> Double {
            scale * max(0, 1 - max(0, now.timeIntervalSince(date)) / (days * 86_400))
        }
        func relevance(_ ids: Set<String>) -> Double {
            (ids.isDisjoint(with: context.usedSkillIDs) ? 0 : 18)
                + (ids.isDisjoint(with: context.installedSkillIDs) ? 0 : 12)
        }
        func followed(_ publishers: Set<String>) -> Double {
            publishers.isDisjoint(with: context.followedPublisherIDs) ? 0 : 14
        }
        func usageTopics(_ ids: [String]) -> Set<String> {
            ids.reduce(into: Set<String>()) { topics, id in
                if let catalogIDs = context.catalogIDsByUsageID[id], !catalogIDs.isEmpty {
                    topics.formUnion(catalogIDs)
                } else {
                    // Unresolved names stay agent-scoped, never fuzzy-matched.
                    topics.insert("usage:" + id)
                }
            }
        }
        switch story {
        case .update(let item):
            let topics: Set<String> = [item.catalogSkill.id]
            return .init(story: story, source: "update", publishers: [item.catalogSkill.publisherID], topics: topics,
                         score: 125 + relevance(topics) + freshness(item.checkedAt, scale: 12, days: 1), observed: item.checkedAt)
        case .trend(let item):
            let topics = usageTopics([item.id])
            let delta = abs(Double(item.currentUses) - Double(item.previousUses))
            let magnitude = min(4, log2(1 + delta)) * 2
            let relative = min(1, delta / max(1, Double(item.previousUses))) * 4
            return .init(story: story, source: "usage", publishers: [], topics: topics,
                         score: 72 + min(18, relevance(topics)) + magnitude + relative
                            + freshness(item.periodEnd, scale: 10, days: 2), observed: item.periodEnd)
        case .creator(let item):
            let topics: Set<String> = [item.skill.id], publishers: Set<String> = [item.publisher.id]
            return .init(story: story, source: "creator", publishers: publishers, topics: topics,
                         score: 46 + relevance(topics) + followed(publishers)
                            + freshness(item.observedAt, scale: 26, days: 14), observed: item.observedAt)
        case .rollup(let item):
            let topics = Set(item.changes.map { $0.skill.id }), publishers: Set<String> = [item.publisher.id]
            let observed = item.changes.map(\.observedAt).max() ?? .distantPast
            // A batch earns no volume bonus: thirty changes are not thirty
            // reasons to place this creator ahead of a personal insight.
            return .init(story: story, source: "creator", publishers: publishers, topics: topics,
                         score: 46 + relevance(topics) + followed(publishers)
                            + freshness(observed, scale: 26, days: 14), observed: observed)
        case .week(let items):
            let topics = usageTopics(items.map(\.id))
            let count = items.map { min(64, max(0, $0.uses)) }.max() ?? 0
            return .init(story: story, source: "recap", publishers: [], topics: topics,
                         score: 52 + min(14, relevance(topics)) + min(6, log2(1 + Double(count))), observed: .distantPast)
        case .active(let items):
            let topics = Set(items.flatMap { $0.skills.map(\.id) }), publishers = Set(items.map(\.id))
            let count = items.map { min(16, max(0, $0.commits)) }.max() ?? 0
            return .init(story: story, source: "activity", publishers: publishers, topics: topics,
                         score: 38 + min(16, relevance(topics) + followed(publishers))
                            + min(4, log2(1 + Double(count))), observed: .distantPast)
        }
    }

    /// Give every source and publisher an opportunity before bounding the
    /// work. The input snapshot is already bounded; scoring sees at most 96
    /// cards, including roll-ups made by the existing timeline projection.
    private func boundedCandidates(_ candidates: [RankedCandidate]) -> [RankedCandidate] {
        let ordered = candidates.sorted {
            if $0.score != $1.score { return $0.score > $1.score }
            if $0.observed != $1.observed { return $0.observed > $1.observed }
            return $0.story.id < $1.story.id
        }
        var seenIDs = Set<String>(), sources = Set<String>(), publishers = Set<String>()
        var bounded: [RankedCandidate] = []
        for candidate in ordered {
            let newSource = !sources.contains(candidate.source)
            let newPublisher = !candidate.publishers.isSubset(of: publishers)
            guard newSource || newPublisher else { continue }
            guard bounded.count < 96 else { break }
            if seenIDs.insert(candidate.story.id).inserted {
                bounded.append(candidate)
                sources.insert(candidate.source)
                publishers.formUnion(candidate.publishers)
            }
        }
        for candidate in ordered where bounded.count < 96 {
            if seenIDs.insert(candidate.story.id).inserted { bounded.append(candidate) }
        }
        return bounded
    }

    private func repetitionPenalty(_ candidate: RankedCandidate, recent: [RankedCandidate]) -> Double {
        recent.reversed().enumerated().reduce(0) { total, entry in
            let weight = [1.0, 0.6, 0.3][entry.offset]
            let previous = entry.element
            return total + weight * ((candidate.source == previous.source ? 24 : 0)
                + (candidate.publishers.isDisjoint(with: previous.publishers) ? 0 : 20)
                + (candidate.topics.isDisjoint(with: previous.topics) ? 0 : 36))
        }
    }
}
