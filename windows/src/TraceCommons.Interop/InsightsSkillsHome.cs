using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace TraceCommons.Interop;

/// <summary>
/// The Usage section's figures (<c>InsightsSkillsUsage</c>,
/// macos/Sources/TraceCommonsApp/InsightsSkillsUsage.swift). Invocation
/// counts add across the disjoint direct and delegated populations; session
/// counts and load-only observations never stand in for uses.
/// </summary>
public static class InsightsSkillsUsage
{
    public static readonly int[] Periods = { 7, 30, 90 };

    public sealed record Day(string Id, int Uses);

    public sealed record Skill(string Id, string Name, string Agent, int Uses, IReadOnlyList<Day> Daily,
        DateTimeOffset Start, DateTimeOffset End, IReadOnlyList<SkillsUsageRow> Sources);

    public static string EmptyState(SkillsUsage usage, int days)
    {
        if (!usage.Enabled) return "skills_analysis_off";
        if (!usage.Coverage.Any(c => c.Status == "explicit_invocations")) return "skills_usage_unavailable";
        var agents = usage.Coverage.Where(c => c.Status == "explicit_invocations").Select(c => c.Agent).ToHashSet();
        var relevant = usage.Skills.Where(r => agents.Contains(r.Agent));
        if (usage.Partial || usage.LastAnalysisAt is null || !Periods.Contains(days)
            || !relevant.All(r => r.Population is "direct" or "delegated" && r.UndatedEvents == 0 && r.Windows.Any(w => w.Days == days)))
        {
            return "skills_usage_pending";
        }

        return "skills_usage_empty";
    }

    public static IReadOnlyList<Skill> Ranked(SkillsUsage usage, int days, DateTimeOffset now, TimeZoneInfo zone, string? includingId = null)
    {
        if (!usage.Enabled || !Periods.Contains(days)) return Array.Empty<Skill>();
        var agents = usage.Coverage.Where(c => c.Status == "explicit_invocations").Select(c => c.Agent).ToHashSet();
        var rows = usage.Skills.Where(r => agents.Contains(r.Agent) && r.Population is "direct" or "delegated"
            && r.Windows.Any(w => w.Days == days));
        var result = new List<Skill>();
        foreach (IGrouping<string, SkillsUsageRow> group in rows.GroupBy(r => r.Agent + "\0" + r.Name))
        {
            SkillsUsageRow first = group.First();
            int total = group.Sum(r => Math.Max(0, r.Windows.FirstOrDefault(w => w.Days == days)?.Invocations ?? 0));
            if (total <= 0 && group.Key != includingId) continue;
            // Daemon windows roll from request time; buckets are local days.
            // The first bucket may hold events before the rolling cutoff, so
            // it is left out rather than shown as if it fit the window.
            DateTimeOffset cutoff = now.AddDays(-days);
            DateTimeOffset firstFullDay = SkillsDates.AddDays(SkillsDates.StartOfDay(cutoff, zone), 1, zone);
            string firstDay = SkillsDates.Format(firstFullDay, zone), lastDay = SkillsDates.Format(now, zone);
            var daily = group.SelectMany(r => r.Daily)
                .Where(d => string.CompareOrdinal(d.Day, firstDay) >= 0 && string.CompareOrdinal(d.Day, lastDay) <= 0)
                .GroupBy(d => d.Day)
                .Select(g => new Day(g.Key, g.Sum(d => Math.Max(0, d.Invocations))))
                .OrderBy(d => d.Id, StringComparer.Ordinal).ToArray();
            result.Add(new Skill(group.Key, first.Name, first.Agent, total, daily, cutoff, now,
                group.OrderBy(r => r.Identity, StringComparer.Ordinal).ToArray()));
        }

        return result.OrderByDescending(s => s.Uses).ThenBy(s => s.Id, StringComparer.Ordinal).ToArray();
    }

    public static IReadOnlyList<InstalledSkill> RemovalCandidates(SkillsSnapshot snapshot, int days, InsightsSkillsModel model,
        DateTimeOffset now, TimeZoneInfo zone)
    {
        SkillsUsage usage = snapshot.Usage;
        if (!usage.Enabled || usage.Partial || snapshot.Inventory.Partial || !Periods.Contains(days)
            || usage.LastAnalysisAt is null || SkillsDates.Parse(usage.LastAnalysisAt) is not { } checkedAt
            || usage.EarliestEvidence is null || SkillsDates.Parse(usage.EarliestEvidence) is not { } earliest
            || earliest > now.AddDays(-days) || checkedAt > now)
        {
            return Array.Empty<InstalledSkill>();
        }

        DateTimeOffset cutoff = now.AddDays(-days);
        return snapshot.Inventory.Skills.Where(skill =>
            {
                if (!snapshot.Inventory.Details.TryGetValue(skill.Id, out SkillInstallationDetails? details)
                    || details.Availability != "available" || details.DefinitionStatus != "valid") return false;
                string[] required = skill.Agent == "shared" ? new[] { "codex", "claude_code", "opencode" } : new[] { skill.Agent };
                bool established = required.All(agent =>
                    usage.Coverage.Any(c => c.Agent == agent && c.Status == "explicit_invocations")
                    && usage.Skills.Where(r => r.Agent == agent && r.Population is "direct" or "delegated")
                        .SelectMany(r => r.Daily).Any(d => d.Invocations > 0 && SkillsDates.Day(d.Day, zone) is { } start
                            && SkillsDates.AddDays(start, 1, zone) <= cutoff));
                if (!established || model.UsageState(skill, snapshot) != "skills_no_usage") return false;
                return model.UsageRows(skill, snapshot).All(row =>
                    row.Population is "direct" or "delegated" && row.UndatedEvents == 0
                    && row.Windows.FirstOrDefault(w => w.Days == days) is { } window
                    && window.Invocations == 0 && window.Loads == 0);
            })
            .OrderBy(s => s, Comparer<InstalledSkill>.Create((a, b) =>
                a.Name == b.Name ? string.CompareOrdinal(a.Id, b.Id) : SkillsText.StandardCompare(a.Name, b.Name)))
            .ToArray();
    }
}

/// <summary>
/// Skills Home: bounded stories derived only from the current local
/// snapshot (<c>InsightsSkillsHome</c>, InsightsSkillsHome.swift), ranked
/// locally (InsightsSkillsHomeRanking.swift). Nothing here is fetched.
/// </summary>
public static class InsightsSkillsHome
{
    public enum Period { Today, ThisWeek, Earlier }

    public static string CopyKey(Period period) => period switch
    {
        Period.Today => "skills_home_today",
        Period.ThisWeek => "skills_home_this_week",
        _ => "skills_home_earlier",
    };

    public enum Direction { More, Less, Started }

    public sealed record Update(string Id, string GroupId, string InstalledId, string Name, IReadOnlyList<string> Agents,
        CatalogSkill CatalogSkill, DateTimeOffset CheckedAt, string? PublisherName = null);

    public sealed record TrendBar(int Id, int Uses, bool Recent);

    public sealed record Trend(string Id, string Name, string Agent, int PreviousUses, int CurrentUses, Direction Direction,
        DateTimeOffset PeriodEnd, IReadOnlyList<int> Daily)
    {
        /// <summary>The chart's bars, or none unless there are exactly fourteen days.</summary>
        public IReadOnlyList<TrendBar> Bars => Daily.Count != 14
            ? Array.Empty<TrendBar>()
            : Daily.Select((uses, i) => new TrendBar(i, Math.Max(0, uses), i >= 7)).ToArray();
    }

    public sealed record WeekSkill(string Id, string Name, string Agent, int Uses);

    public sealed record Active(SkillsPublisher Publisher, int Stars, int Commits, IReadOnlyList<CatalogSkill> Skills)
    {
        public string Id => Publisher.Id;

        /// <summary>A count in at most four characters, cut and never rounded up: 1,949 is "1.9K".</summary>
        public static string Short(int count)
        {
            foreach ((int unit, string letter) in new[] { (1_000_000, "M"), (1_000, "K") })
            {
                if (count < unit) continue;
                int tenths = count / (unit / 10);
                return tenths < 100 && tenths % 10 != 0
                    ? $"{tenths / 10}.{tenths % 10}{letter}"
                    : $"{tenths / 10}{letter}";
            }

            return Math.Max(0, count).ToString(CultureInfo.InvariantCulture);
        }
    }

    public sealed record DefaultCreator(SkillsPublisher Publisher, IReadOnlyList<CatalogSkill> Skills)
    {
        public string Id => Publisher.Id;
    }

    public sealed record CreatorUpdate(string Id, SkillsFeedEvent Event, CatalogSkill Skill, SkillsPublisher Publisher, DateTimeOffset ObservedAt);

    public sealed record Rollup(string Id, SkillsPublisher Publisher, IReadOnlyList<CreatorUpdate> Changes);

    /// <summary>One item of the feed.</summary>
    public abstract record Story
    {
        public abstract string Id { get; }

        public sealed record UpdateStory(Update Item) : Story { public override string Id => Item.Id; }
        public sealed record TrendStory(Trend Item) : Story { public override string Id => "trend:" + Item.Id; }
        public sealed record CreatorStory(CreatorUpdate Item) : Story { public override string Id => "creator:" + Item.Id; }
        public sealed record RollupStory(Rollup Item) : Story { public override string Id => "rollup:" + Item.Id; }
        public sealed record WeekStory(IReadOnlyList<WeekSkill> Items) : Story { public override string Id => "week"; }
        public sealed record ActiveStory(IReadOnlyList<Active> Items) : Story { public override string Id => "active"; }
    }

    public sealed record Group(Period Period, IReadOnlyList<Story> Stories);

    /// <summary>Catalog ids are exact identities; usage ids name an agent and reach the catalog only through a verified installed source.</summary>
    public sealed class RankingContext
    {
        public HashSet<string> UsedSkillIds { get; } = new(StringComparer.Ordinal);
        public HashSet<string> InstalledSkillIds { get; } = new(StringComparer.Ordinal);
        public HashSet<string> FollowedPublisherIds { get; } = new(StringComparer.Ordinal);
        public Dictionary<string, HashSet<string>> CatalogIdsByUsageId { get; } = new(StringComparer.Ordinal);
    }

    public sealed record Snapshot(
        IReadOnlyList<Update> Updates, IReadOnlyList<Trend> Trends, IReadOnlyList<CreatorUpdate> Creators,
        IReadOnlyList<WeekSkill> Week, IReadOnlyList<Active> ActivePublishers, IReadOnlyList<DefaultCreator> DefaultCreators,
        RankingContext Context)
    {
        public static Snapshot None() => new(Array.Empty<Update>(), Array.Empty<Trend>(), Array.Empty<CreatorUpdate>(),
            Array.Empty<WeekSkill>(), Array.Empty<Active>(), Array.Empty<DefaultCreator>(), new RankingContext());

        public bool IsEmpty => Updates.Count == 0 && Trends.Count == 0 && Creators.Count == 0 && Week.Count == 0
            && ActivePublishers.Count == 0 && DefaultCreators.Count == 0;

        /// <summary>
        /// Every item in one column, newest first, under the period it falls
        /// in; empty periods are left out (<c>timeline(now:calendar:)</c>).
        /// </summary>
        public IReadOnlyList<Group> Timeline(DateTimeOffset now, TimeZoneInfo zone)
        {
            DateTimeOffset today = SkillsDates.StartOfDay(now, zone);
            DateTimeOffset isoWeek = SkillsDates.IsoWeekStart(now, zone);
            DateTimeOffset week = isoWeek < today ? isoWeek : today;
            Period PeriodOf(DateTimeOffset date) => date >= today ? Period.Today : date >= week ? Period.ThisWeek : Period.Earlier;

            var entries = new List<(DateTimeOffset Date, int Rank, Story Story)>();
            entries.AddRange(Updates.Select(u =>
                (Min(Max(u.CheckedAt, today), now), 0, (Story)new Story.UpdateStory(u))));

            var updating = Updates.Select(u => u.CatalogSkill.Id).ToHashSet(StringComparer.Ordinal);
            var changes = Creators.Where(c => !updating.Contains(c.Skill.Id));
            var catalog = new List<(DateTimeOffset Date, string Id, Story Story)>();
            foreach (var found in changes.GroupBy(c => (Period: PeriodOf(c.ObservedAt), Publisher: c.Publisher.Id)))
            {
                var newest = found.OrderByDescending(c => c.ObservedAt).ThenBy(c => c.Id, StringComparer.Ordinal).ToArray();
                if (newest.Length > 1)
                {
                    var rollup = new Rollup(PeriodName(found.Key.Period) + ":" + found.Key.Publisher, newest[0].Publisher, newest);
                    catalog.Add((newest[0].ObservedAt, rollup.Id, new Story.RollupStory(rollup)));
                }
                else
                {
                    catalog.AddRange(newest.Select(c => (c.ObservedAt, c.Id, (Story)new Story.CreatorStory(c))));
                }
            }

            entries.AddRange(catalog.OrderByDescending(c => c.Date).ThenBy(c => c.Id, StringComparer.Ordinal)
                .Select(c => (c.Date, 1, c.Story)));
            entries.AddRange(Trends.Select(t => (today, 2, (Story)new Story.TrendStory(t))));

            var ordered = entries.Select((e, i) => (e, i))
                .OrderByDescending(x => x.e.Date).ThenBy(x => x.e.Rank).ThenBy(x => x.i)
                .Select(x => x.e).ToArray();
            var groups = new List<Group>();
            foreach (Period which in new[] { Period.Today, Period.ThisWeek, Period.Earlier })
            {
                var stories = ordered.Where(e => PeriodOf(e.Date) == which).Select(e => e.Story).ToList();
                // The recap, then the active publishers, lead This week.
                if (which == Period.ThisWeek && ActivePublishers.Count > 0) stories.Insert(0, new Story.ActiveStory(ActivePublishers));
                if (which == Period.ThisWeek && Week.Count > 0) stories.Insert(0, new Story.WeekStory(Week));
                if (stories.Count > 0) groups.Add(new Group(which, stories));
            }

            return groups;
        }

        /// <summary>The first ranked card may use its real catalog cover; later cards are never promoted to fill the slot.</summary>
        public static string? RankedHeroId(IReadOnlyList<Story> stories) =>
            stories.FirstOrDefault() is Story.UpdateStory or Story.CreatorStory ? stories[0].Id : null;

        private sealed record Candidate(Story Story, string Source, HashSet<string> Publishers, HashSet<string> Topics,
            double Score, DateTimeOffset Observed);

        /// <summary>
        /// Selects locally from supported stories without changing their facts
        /// or times (<c>rankedStories(now:calendar:)</c>): at most twelve.
        /// </summary>
        public IReadOnlyList<Story> RankedStories(DateTimeOffset now, TimeZoneInfo zone)
        {
            var updateIds = new HashSet<string>(StringComparer.Ordinal);
            var uniqueUpdates = Updates.OrderByDescending(u => u.CheckedAt).ThenBy(u => u.Id, StringComparer.Ordinal)
                .Where(u => u.CheckedAt <= now && (now - u.CheckedAt).TotalSeconds < 6 * 3_600 && updateIds.Add(u.CatalogSkill.Id))
                .ToArray();
            var creatorIds = new HashSet<string>(StringComparer.Ordinal);
            var uniqueCreators = Creators.OrderByDescending(c => c.ObservedAt).ThenBy(c => c.Id, StringComparer.Ordinal)
                .Where(c => c.ObservedAt <= now && (now - c.ObservedAt).TotalSeconds <= 30 * 86_400
                    && !updateIds.Contains(c.Skill.Id) && creatorIds.Add(c.Skill.Id))
                .ToArray();
            var trendIds = new HashSet<string>(StringComparer.Ordinal);
            DateTimeOffset todayStart = SkillsDates.StartOfDay(now, zone);
            var meaningfulTrends = Trends.OrderBy(t => t.Id, StringComparer.Ordinal).Where(t =>
            {
                if (!trendIds.Add(t.Id) || t.PreviousUses < 0 || t.CurrentUses < 0 || t.PeriodEnd > now
                    || SkillsDates.StartOfDay(t.PeriodEnd, zone) != todayStart) return false;
                double delta = Math.Abs((double)t.CurrentUses - t.PreviousUses);
                return delta >= 2 && delta / Math.Max(1, (double)t.PreviousUses) >= 0.25;
            }).ToArray();
            var normalized = this with { Updates = uniqueUpdates, Trends = meaningfulTrends, Creators = uniqueCreators };
            var candidates = normalized.Timeline(now, zone).SelectMany(g => g.Stories).Select(s => RankedCandidate(s, now)).ToList();
            var remaining = BoundedCandidates(candidates);
            var selected = new List<Candidate>();
            while (remaining.Count > 0 && selected.Count < 12)
            {
                var recent = selected.Skip(Math.Max(0, selected.Count - 3)).ToArray();
                int best = 0;
                for (int i = 1; i < remaining.Count; i++)
                {
                    if (Better(remaining[i], remaining[best], recent)) best = i;
                }

                selected.Add(remaining[best]);
                remaining.RemoveAt(best);
            }

            return selected.Select(c => c.Story).ToArray();
        }

        private static bool Better(Candidate a, Candidate b, IReadOnlyList<Candidate> recent)
        {
            double aScore = a.Score - RepetitionPenalty(a, recent), bScore = b.Score - RepetitionPenalty(b, recent);
            if (aScore != bScore) return aScore > bScore;
            if (a.Observed != b.Observed) return a.Observed > b.Observed;
            return string.CompareOrdinal(a.Story.Id, b.Story.Id) < 0;
        }

        private Candidate RankedCandidate(Story story, DateTimeOffset now)
        {
            double Freshness(DateTimeOffset date, double scale, double days) =>
                scale * Math.Max(0, 1 - Math.Max(0, (now - date).TotalSeconds) / (days * 86_400));
            double Relevance(HashSet<string> ids) =>
                (ids.Overlaps(Context.UsedSkillIds) ? 18 : 0) + (ids.Overlaps(Context.InstalledSkillIds) ? 12 : 0);
            double Followed(HashSet<string> publishers) => publishers.Overlaps(Context.FollowedPublisherIds) ? 14 : 0;
            HashSet<string> UsageTopics(IEnumerable<string> ids)
            {
                var topics = new HashSet<string>(StringComparer.Ordinal);
                foreach (string id in ids)
                {
                    if (Context.CatalogIdsByUsageId.TryGetValue(id, out HashSet<string>? catalogIds) && catalogIds.Count > 0)
                        topics.UnionWith(catalogIds);
                    else
                        topics.Add("usage:" + id); // Unresolved names stay agent-scoped, never fuzzy-matched.
                }

                return topics;
            }

            static HashSet<string> Set(params string[] ids) => new(ids, StringComparer.Ordinal);
            switch (story)
            {
                case Story.UpdateStory { Item: var item }:
                {
                    var topics = Set(item.CatalogSkill.Id);
                    return new Candidate(story, "update", Set(item.CatalogSkill.PublisherId), topics,
                        125 + Relevance(topics) + Freshness(item.CheckedAt, 12, 1), item.CheckedAt);
                }
                case Story.TrendStory { Item: var item }:
                {
                    var topics = UsageTopics(new[] { item.Id });
                    double delta = Math.Abs((double)item.CurrentUses - item.PreviousUses);
                    double magnitude = Math.Min(4, Math.Log2(1 + delta)) * 2;
                    double relative = Math.Min(1, delta / Math.Max(1, (double)item.PreviousUses)) * 4;
                    return new Candidate(story, "usage", Set(), topics,
                        72 + Math.Min(18, Relevance(topics)) + magnitude + relative + Freshness(item.PeriodEnd, 10, 2), item.PeriodEnd);
                }
                case Story.CreatorStory { Item: var item }:
                {
                    var topics = Set(item.Skill.Id);
                    var publishers = Set(item.Publisher.Id);
                    return new Candidate(story, "creator", publishers, topics,
                        46 + Relevance(topics) + Followed(publishers) + Freshness(item.ObservedAt, 26, 14), item.ObservedAt);
                }
                case Story.RollupStory { Item: var item }:
                {
                    var topics = item.Changes.Select(c => c.Skill.Id).ToHashSet(StringComparer.Ordinal);
                    var publishers = Set(item.Publisher.Id);
                    DateTimeOffset observed = item.Changes.Count == 0 ? DateTimeOffset.MinValue : item.Changes.Max(c => c.ObservedAt);
                    // A batch earns no volume bonus.
                    return new Candidate(story, "creator", publishers, topics,
                        46 + Relevance(topics) + Followed(publishers) + Freshness(observed, 26, 14), observed);
                }
                case Story.WeekStory { Items: var items }:
                {
                    var topics = UsageTopics(items.Select(i => i.Id));
                    int count = items.Count == 0 ? 0 : items.Max(i => Math.Min(64, Math.Max(0, i.Uses)));
                    return new Candidate(story, "recap", Set(), topics,
                        52 + Math.Min(14, Relevance(topics)) + Math.Min(6, Math.Log2(1 + count)), DateTimeOffset.MinValue);
                }
                case Story.ActiveStory { Items: var items }:
                {
                    var topics = items.SelectMany(i => i.Skills.Select(s => s.Id)).ToHashSet(StringComparer.Ordinal);
                    var publishers = items.Select(i => i.Id).ToHashSet(StringComparer.Ordinal);
                    int count = items.Count == 0 ? 0 : items.Max(i => Math.Min(16, Math.Max(0, i.Commits)));
                    return new Candidate(story, "activity", publishers, topics,
                        38 + Math.Min(16, Relevance(topics) + Followed(publishers)) + Math.Min(4, Math.Log2(1 + count)),
                        DateTimeOffset.MinValue);
                }
                default:
                    throw new InvalidOperationException();
            }
        }

        /// <summary>Every source and publisher gets a chance before the work is bounded at 96 cards.</summary>
        private static List<Candidate> BoundedCandidates(List<Candidate> candidates)
        {
            var ordered = candidates.OrderByDescending(c => c.Score).ThenByDescending(c => c.Observed)
                .ThenBy(c => c.Story.Id, StringComparer.Ordinal).ToArray();
            var seen = new HashSet<string>(StringComparer.Ordinal);
            var sources = new HashSet<string>(StringComparer.Ordinal);
            var publishers = new HashSet<string>(StringComparer.Ordinal);
            var bounded = new List<Candidate>();
            foreach (Candidate c in ordered)
            {
                bool newSource = !sources.Contains(c.Source), newPublisher = !c.Publishers.IsSubsetOf(publishers);
                if (!newSource && !newPublisher) continue;
                if (bounded.Count >= 96) break;
                if (seen.Add(c.Story.Id))
                {
                    bounded.Add(c);
                    sources.Add(c.Source);
                    publishers.UnionWith(c.Publishers);
                }
            }

            foreach (Candidate c in ordered)
            {
                if (bounded.Count >= 96) break;
                if (seen.Add(c.Story.Id)) bounded.Add(c);
            }

            return bounded;
        }

        private static double RepetitionPenalty(Candidate candidate, IReadOnlyList<Candidate> recent)
        {
            double[] weights = { 1.0, 0.6, 0.3 };
            double total = 0;
            for (int offset = 0; offset < recent.Count; offset++)
            {
                Candidate previous = recent[recent.Count - 1 - offset];
                total += weights[offset] * ((candidate.Source == previous.Source ? 24 : 0)
                    + (candidate.Publishers.Overlaps(previous.Publishers) ? 20 : 0)
                    + (candidate.Topics.Overlaps(previous.Topics) ? 36 : 0));
            }

            return total;
        }
    }

    // Swift's string interpolation of the Period case names the rollup id.
    private static string PeriodName(Period period) => period switch
    {
        Period.Today => "today",
        Period.ThisWeek => "thisWeek",
        _ => "earlier",
    };

    private static DateTimeOffset Min(DateTimeOffset a, DateTimeOffset b) => a < b ? a : b;
    private static DateTimeOffset Max(DateTimeOffset a, DateTimeOffset b) => a > b ? a : b;

    public static Snapshot Make(SkillsSnapshot snapshot, InsightsSkillsModel model, DateTimeOffset now, TimeZoneInfo zone)
    {
        if (!snapshot.Enabled || !snapshot.Readable) return Snapshot.None();
        return new Snapshot(PersonalizedUpdates(snapshot, model, now), UsageChanges(snapshot.Usage, now, zone),
            CreatorChanges(snapshot, now), WeekTop(snapshot.Usage, now, zone), ActivePublishers(snapshot.Catalog),
            DefaultCreators(snapshot), Ranking(snapshot, model));
    }

    /// <summary>Real catalog entries introduced before the first change; following personalises without removing them.</summary>
    private static IReadOnlyList<DefaultCreator> DefaultCreators(SkillsSnapshot snapshot)
    {
        if (!snapshot.Usage.Enabled) return Array.Empty<DefaultCreator>();
        var seen = new HashSet<string>(StringComparer.Ordinal);
        var creators = new List<DefaultCreator>();
        foreach (string id in snapshot.DefaultPublishers)
        {
            if (!seen.Add(id) || snapshot.Catalog.Publishers.FirstOrDefault(p => p.Id == id) is not { } publisher) continue;
            var named = new HashSet<string>(StringComparer.Ordinal);
            var skills = snapshot.Catalog.Skills
                .Where(s => s.PublisherId == id && s.Repository == publisher.Repository && named.Add(s.Id))
                .OrderBy(s => s.Name, StringComparer.Ordinal).ThenBy(s => s.Id, StringComparer.Ordinal).ToArray();
            if (skills.Length == 0) continue;
            creators.Add(new DefaultCreator(publisher, skills.Take(3).ToArray()));
        }

        return creators.Take(3).ToArray();
    }

    /// <summary>The daemon's recently active publishers, in its order, from a verified server catalog only.</summary>
    private static IReadOnlyList<Active> ActivePublishers(SkillsCatalog catalog)
    {
        if (catalog.Source != "server") return Array.Empty<Active>();
        var seen = new HashSet<string>(StringComparer.Ordinal);
        var found = new List<Active>();
        foreach (SkillsActivePublisher entry in catalog.ActivePublishers)
        {
            if (seen.Contains(entry.PublisherId)
                || catalog.Publishers.FirstOrDefault(p => p.Id == entry.PublisherId) is not { } publisher
                || catalog.PublisherStates.FirstOrDefault(s => s.PublisherId == entry.PublisherId)?.Signals is not { } signals
                || signals.Stars is not { } stars || stars < 0 || signals.Commits7d is not { } commits || commits < 1)
            {
                continue;
            }

            var named = new HashSet<string>(StringComparer.Ordinal);
            var skills = entry.SkillIds.Where(named.Add)
                .Select(id => catalog.Skills.FirstOrDefault(s => s.Id == id && s.PublisherId == publisher.Id))
                .Where(s => s is not null).Select(s => s!).ToArray();
            if (skills.Length == 0) continue;
            seen.Add(publisher.Id);
            found.Add(new Active(publisher, stars, commits, skills.Take(3).ToArray()));
        }

        return found.Take(3).ToArray();
    }

    /// <summary>
    /// The three most-used skills of the last seven completed local days,
    /// the same days a usage story's last week is. Left out while history is
    /// partial or was not analysed today.
    /// </summary>
    private static IReadOnlyList<WeekSkill> WeekTop(SkillsUsage usage, DateTimeOffset now, TimeZoneInfo zone)
    {
        DateTimeOffset today = SkillsDates.StartOfDay(now, zone);
        if (!usage.Enabled || usage.LastAnalysisAt is null || SkillsDates.Parse(usage.LastAnalysisAt) is not { } checkedAt
            || checkedAt < today || checkedAt > now) return Array.Empty<WeekSkill>();
        DateTimeOffset start = SkillsDates.AddDays(today, -7, zone);
        var explicitAgents = Complete(usage.Coverage.Where(c => c.Status == "explicit_invocations").Select(c => c.Agent).ToHashSet(),
            usage, start, today, zone);
        var rows = usage.Skills.Where(r => explicitAgents.Contains(r.Agent) && r.Population is "direct" or "delegated");
        var skills = new List<WeekSkill>();
        foreach (IGrouping<string, SkillsUsageRow> group in rows.GroupBy(r => r.Agent + "\0" + r.Name))
        {
            var list = group.ToArray();
            if (list.Select(r => r.Population).Distinct().Count() != list.Length) continue;
            var uses = new List<int>();
            bool readable = true;
            foreach (SkillsUsageDay day in list.SelectMany(r => r.Daily))
            {
                if (day.Invocations < 0 || SkillsDates.Day(day.Day, zone) is not { } date) { readable = false; break; }
                if (date >= start && date < today) uses.Add(day.Invocations);
            }

            if (!readable || Sum(uses) is not { } total || total <= 0) continue;
            skills.Add(new WeekSkill(group.Key, list[0].Name, list[0].Agent, total));
        }

        return skills.OrderByDescending(s => s.Uses).ThenBy(s => s.Id, StringComparer.Ordinal).Take(3).ToArray();
    }

    private static IReadOnlyList<Update> PersonalizedUpdates(SkillsSnapshot snapshot, InsightsSkillsModel model, DateTimeOffset now)
    {
        if (!snapshot.Usage.Enabled) return Array.Empty<Update>();
        var explicitAgents = snapshot.Usage.Coverage.Where(c => c.Status == "explicit_invocations").Select(c => c.Agent).ToHashSet();
        var items = new List<Update>();
        foreach (InsightsSkillsIndex.Group group in InsightsSkillsIndex.Groups(snapshot.Inventory.Skills))
        {
            bool used = InsightsSkillsIndex.UsageRows(group, snapshot, model).Any(r => explicitAgents.Contains(r.Agent)
                && r.Population is "direct" or "delegated" && r.Windows.Any(w => w.Days == 30 && w.Invocations > 0));
            if (!used) continue;
            var candidates = new List<Update>();
            foreach (InstalledSkill installed in group.Installations)
            {
                if (!ValidHash(installed.Sha256) || installed.Source is not { } source
                    || snapshot.Updates.FirstOrDefault(u => u.InstalledId == installed.Id && u.Status == "update_available") is not { } update
                    || snapshot.Catalog.Skills.FirstOrDefault(s => s.Id == update.CatalogSkillId) is not { } catalog
                    || source.Repository != catalog.Repository || source.Path != catalog.Path
                    || !ValidHash(catalog.Sha256)
                    || string.Equals(installed.Sha256, catalog.Sha256, StringComparison.OrdinalIgnoreCase)
                    || snapshot.Catalog.PublisherStates.FirstOrDefault(s => s.PublisherId == catalog.PublisherId && s.Status == "current") is not { } state
                    || state.CheckedAt is null || SkillsDates.Parse(state.CheckedAt) is not { } checkedAt
                    || checkedAt > now || (now - checkedAt).TotalSeconds >= 6 * 60 * 60)
                {
                    continue;
                }

                candidates.Add(new Update("update:" + group.Id, group.Id, installed.Id, installed.Name, group.Agents, catalog, checkedAt,
                    snapshot.Catalog.Publishers.FirstOrDefault(p => p.Id == catalog.PublisherId)?.Name));
            }

            if (candidates.OrderBy(c => c.InstalledId, StringComparer.Ordinal).FirstOrDefault() is { } chosen) items.Add(chosen);
        }

        return items.OrderByDescending(u => u.CheckedAt).ThenBy(u => u.Id, StringComparer.Ordinal).Take(12).ToArray();
    }

    /// <summary>
    /// Usage changes over fourteen completed local days. An agent with a gap
    /// in those days tells no change: a gap could read as a decrease or a first use.
    /// </summary>
    private static IReadOnlyList<Trend> UsageChanges(SkillsUsage usage, DateTimeOffset now, TimeZoneInfo zone)
    {
        DateTimeOffset today = SkillsDates.StartOfDay(now, zone);
        if (!usage.Enabled || usage.LastAnalysisAt is null || SkillsDates.Parse(usage.LastAnalysisAt) is not { } checkedAt
            || checkedAt < today || checkedAt > now) return Array.Empty<Trend>();
        DateTimeOffset previousStart = SkillsDates.AddDays(today, -14, zone), currentStart = SkillsDates.AddDays(today, -7, zone);
        if (usage.EarliestEvidence is null || SkillsDates.Parse(usage.EarliestEvidence) is not { } earliest || earliest > previousStart)
            return Array.Empty<Trend>();
        var explicitAgents = Complete(usage.Coverage.Where(c => c.Status == "explicit_invocations").Select(c => c.Agent).ToHashSet(),
            usage, previousStart, today, zone);
        // Another agent's older records cannot establish this agent's span.
        var established = explicitAgents.Where(agent => usage.Skills
            .Where(r => r.Agent == agent && r.Population is "direct" or "delegated").SelectMany(r => r.Daily)
            .Any(d => d.Invocations > 0 && SkillsDates.Day(d.Day, zone) is { } start && SkillsDates.AddDays(start, 1, zone) <= previousStart))
            .ToHashSet();
        var items = new List<Trend>();
        foreach (IGrouping<string, SkillsUsageRow> group in usage.Skills.Where(r => established.Contains(r.Agent)).GroupBy(r => r.Agent + "\0" + r.Name))
        {
            var rows = group.ToArray();
            if (rows.Select(r => r.Population).Distinct().Count() != rows.Length
                || !rows.All(r => r.Population is "direct" or "delegated" && r.UndatedEvents == 0)) continue;
            var before = new List<int>();
            var after = new List<int>();
            var daily = new int[14];
            bool readable = true;
            foreach (SkillsUsageRow row in rows)
            {
                if (row.Daily.Select(d => d.Day).Distinct().Count() != row.Daily.Count || (row.Invocations != 0 && row.Daily.Count == 0))
                {
                    readable = false;
                    break;
                }

                foreach (SkillsUsageDay day in row.Daily)
                {
                    if (day.Invocations < 0 || SkillsDates.Day(day.Day, zone) is not { } start || start > today) { readable = false; break; }
                    if (start >= currentStart && start < today) after.Add(day.Invocations);
                    else if (start >= previousStart && start < currentStart) before.Add(day.Invocations);
                    else continue;
                    int index = (int)Math.Round((TimeZoneInfo.ConvertTime(start, zone).Date - TimeZoneInfo.ConvertTime(previousStart, zone).Date).TotalDays);
                    if (index < 0 || index >= 14) { readable = false; break; }
                    long next = (long)daily[index] + day.Invocations;
                    if (next > int.MaxValue) { readable = false; break; }
                    daily[index] = (int)next;
                }

                if (!readable) break;
            }

            if (!readable || Sum(before) is not { } previous || Sum(after) is not { } current || previous == current) continue;
            items.Add(new Trend(group.Key, rows[0].Name, rows[0].Agent, previous, current,
                current > previous ? (previous == 0 ? Direction.Started : Direction.More) : Direction.Less, today, daily));
        }

        return items.OrderByDescending(t => Math.Abs((long)t.CurrentUses - t.PreviousUses)).ThenBy(t => t.Id, StringComparer.Ordinal).ToArray();
    }

    private static IReadOnlyList<CreatorUpdate> CreatorChanges(SkillsSnapshot snapshot, DateTimeOffset now)
    {
        var followed = snapshot.FollowedPublishers.ToHashSet(StringComparer.Ordinal);
        var defaults = (snapshot.Usage.Enabled ? snapshot.DefaultPublishers : Array.Empty<string>()).ToHashSet(StringComparer.Ordinal);
        // Following stays explicit; an explicit event wins when an older daemon sends it in both.
        var events = snapshot.Feed.Where(e => followed.Contains(e.PublisherId))
            .Concat(snapshot.HomeFeed.Where(e => defaults.Contains(e.PublisherId) && !followed.Contains(e.PublisherId)));
        DateTimeOffset oldest = now.AddDays(-30);
        var valid = new List<CreatorUpdate>();
        foreach (SkillsFeedEvent e in events)
        {
            if (e.Kind is not ("added" or "updated") || SkillsDates.Parse(e.ObservedAt) is not { } observed || observed > now || observed < oldest
                || snapshot.Catalog.Publishers.FirstOrDefault(p => p.Id == e.PublisherId) is not { } publisher
                || snapshot.Catalog.Skills.FirstOrDefault(s => s.Id == e.SkillId && s.PublisherId == e.PublisherId) is not { } skill)
            {
                continue;
            }

            valid.Add(new CreatorUpdate(e.Id, e, skill, publisher, observed));
        }

        var seen = new HashSet<string>(StringComparer.Ordinal);
        var perPublisher = new Dictionary<string, int>(StringComparer.Ordinal);
        return valid.OrderByDescending(c => c.ObservedAt).ThenBy(c => c.Id, StringComparer.Ordinal)
            .Where(c =>
            {
                if (!seen.Add(c.Publisher.Id + "\0" + c.Skill.Id)) return false;
                int count = perPublisher.TryGetValue(c.Publisher.Id, out int n) ? n : 0;
                if (count >= 8) return false;
                perPublisher[c.Publisher.Id] = count + 1;
                return true;
            })
            .Take(96).ToArray();
    }

    private static RankingContext Ranking(SkillsSnapshot snapshot, InsightsSkillsModel model)
    {
        var result = new RankingContext();
        result.FollowedPublisherIds.UnionWith(snapshot.FollowedPublishers);
        var explicitAgents = snapshot.Usage.Coverage.Where(c => c.Status == "explicit_invocations").Select(c => c.Agent).ToHashSet();
        foreach (InsightsSkillsIndex.Group group in InsightsSkillsIndex.Groups(snapshot.Inventory.Skills))
        {
            var ids = snapshot.Catalog.Skills.Where(catalog => group.Installations.Any(i =>
                    i.Source?.Repository == catalog.Repository && i.Source?.Path == catalog.Path))
                .Select(c => c.Id).ToHashSet(StringComparer.Ordinal);
            result.InstalledSkillIds.UnionWith(ids);
            if (!snapshot.Usage.Enabled || ids.Count == 0) continue;
            foreach (SkillsUsageRow row in InsightsSkillsIndex.UsageRows(group, snapshot, model)
                         .Where(r => explicitAgents.Contains(r.Agent) && r.Population is "direct" or "delegated"))
            {
                string key = row.Agent + "\0" + row.Name;
                if (!result.CatalogIdsByUsageId.TryGetValue(key, out HashSet<string>? set))
                {
                    set = new HashSet<string>(StringComparer.Ordinal);
                    result.CatalogIdsByUsageId[key] = set;
                }

                set.UnionWith(ids);
                if (row.Windows.Any(w => w.Days == 30 && w.Invocations > 0)) result.UsedSkillIds.UnionWith(ids);
            }
        }

        return result;
    }

    /// <summary>
    /// The agents whose history has no gap in the local days start..&lt;end.
    /// Without per-agent gaps, any partial history leaves none.
    /// </summary>
    private static HashSet<string> Complete(HashSet<string> agents, SkillsUsage usage, DateTimeOffset start, DateTimeOffset end, TimeZoneInfo zone)
    {
        if (usage.PartialAgents is not { } gaps) return usage.Partial ? new HashSet<string>() : agents;
        return agents.Where(agent => gaps.Where(g => g.Agent == agent).All(gap =>
                gap.Undated == 0 && gap.Days.Count > 0 && gap.Days.All(day =>
                    SkillsDates.Day(day, zone) is { } date && (date < start || date >= end))))
            .ToHashSet();
    }

    private static bool ValidHash(string value) => value.Length == 64 && value.All(Uri.IsHexDigit);

    private static int? Sum(IEnumerable<int> values)
    {
        long total = 0;
        foreach (int v in values)
        {
            total += v;
            if (total > int.MaxValue || total < int.MinValue) return null;
        }

        return (int)total;
    }
}

/// <summary>Where a Home action leads, resolved against the current snapshot (<c>SkillsHomeRoute</c>).</summary>
public sealed record SkillsHomeRoute(string Section, string? UsageId = null, string? CatalogId = null, string? InstalledId = null)
{
    public static SkillsHomeRoute? Usage(string id, SkillsSnapshot snapshot) =>
        snapshot is { Enabled: true, Readable: true } && snapshot.Usage.Enabled
        && snapshot.Usage.Skills.Any(r => r.Agent + "\0" + r.Name == id && r.Population is "direct" or "delegated"
            && snapshot.Usage.Coverage.Any(c => c.Agent == r.Agent && c.Status == "explicit_invocations"))
            ? new SkillsHomeRoute("usage", UsageId: id)
            : null;

    public static SkillsHomeRoute? Catalog(string id, string? installedId, SkillsSnapshot snapshot)
    {
        if (snapshot is not { Enabled: true, Readable: true } || snapshot.Catalog.Skills.FirstOrDefault(s => s.Id == id) is not { } catalog)
            return null;
        if (installedId is not null && !snapshot.Inventory.Skills.Any(s =>
                s.Id == installedId && s.Source?.Repository == catalog.Repository && s.Source?.Path == catalog.Path))
            return null;
        return new SkillsHomeRoute("catalog", CatalogId: id, InstalledId: installedId);
    }

    public static SkillsHomeRoute? Index(SkillsSnapshot snapshot) =>
        snapshot is { Enabled: true, Readable: true } ? new SkillsHomeRoute("inventory") : null;

    public static SkillsHomeRoute? Browse(SkillsSnapshot snapshot) =>
        snapshot is { Enabled: true, Readable: true } ? new SkillsHomeRoute("catalog") : null;

    public static SkillsUpdate? CatalogUpdate(string catalogId, string? installedId, SkillsSnapshot snapshot)
    {
        if (snapshot.Catalog.Skills.FirstOrDefault(s => s.Id == catalogId) is not { } catalog) return null;
        InstalledSkill? installed = snapshot.Inventory.Skills.FirstOrDefault(s =>
            (installedId is null || s.Id == installedId) && s.Source?.Repository == catalog.Repository && s.Source?.Path == catalog.Path);
        return installed is null ? null : snapshot.Updates.FirstOrDefault(u => u.InstalledId == installed.Id);
    }
}
