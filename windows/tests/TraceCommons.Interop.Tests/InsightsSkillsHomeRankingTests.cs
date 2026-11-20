using System;
using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop;
using Xunit;
using Home = TraceCommons.Interop.InsightsSkillsHome;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// macos/Tests/TraceCommonsAppTests/InsightsSkillsHomeRankingTests.swift,
/// against the Windows ranker, and the ranker's numbers at their edges
/// (InsightsSkillsHomeRanking.swift:33, :50, :175).
/// </summary>
public class InsightsSkillsHomeRankingTests
{
    private static readonly DateTimeOffset Now = DateTimeOffset.Parse("2026-11-17T12:00:00Z");
    private static readonly TimeZoneInfo Utc = TimeZoneInfo.Utc;

    private static DateTimeOffset Date(string value) => DateTimeOffset.Parse(value);

    private static CatalogSkill Skill(string id, string publisher = "maker", string name = "review") =>
        new(id, publisher, name, "", publisher + "/skills", "skills/" + id + "/SKILL.md", "main", new string('b', 64),
            "https://example.com/source", "https://example.com");

    private static Home.CreatorUpdate Change(string id, string? skillId = null, string publisher = "maker", string observed = "2026-11-17T11:30:00Z")
    {
        var maker = new SkillsPublisher(publisher, publisher, publisher + "/skills", "skills", "https://example.com", null, null);
        return new Home.CreatorUpdate(id, new SkillsFeedEvent(id, maker.Id, skillId ?? id, "updated", observed),
            Skill(skillId ?? id, maker.Id), maker, Date(observed));
    }

    private static Home.Update Update(string id, string? skillId = null, string checkedAt = "2026-11-17T10:00:00Z") =>
        new("update:" + id, id, id, id, new[] { "claude_code" }, Skill(skillId ?? id), Date(checkedAt));

    private static Home.Trend Trend(string id, int previous = 2, int current = 5, string agent = "claude_code") =>
        new(agent + "\0" + id, id, agent, previous, current,
            current > previous ? (previous == 0 ? Home.Direction.Started : Home.Direction.More) : Home.Direction.Less,
            Date("2026-11-17T00:00:00Z"), Array.Empty<int>());

    private static Home.Snapshot Feed(IEnumerable<Home.Update>? updates = null, IEnumerable<Home.Trend>? trends = null,
        IEnumerable<Home.CreatorUpdate>? creators = null, Home.RankingContext? context = null) =>
        Home.Snapshot.None() with
        {
            Updates = (updates ?? Array.Empty<Home.Update>()).ToArray(),
            Trends = (trends ?? Array.Empty<Home.Trend>()).ToArray(),
            Creators = (creators ?? Array.Empty<Home.CreatorUpdate>()).ToArray(),
            Context = context ?? new Home.RankingContext(),
        };

    private static IReadOnlyList<Home.Story> Ranked(Home.Snapshot snapshot) => snapshot.RankedStories(Now, Utc);

    private static string[] Ids(Home.Snapshot snapshot) => Ranked(snapshot).Select(s => s.Id).ToArray();

    [Fact]
    public void VerifiedUsedUpdateLeadsNewerDiscoveryAndKeepsObservedTime()
    {
        // A newer unrelated catalog observation must not hide an actionable update.
        var stories = Ranked(Feed(new[] { Update("mine") }, new[] { Trend("mine") }, new[] { Change("discovery", publisher: "other") },
            new Home.RankingContext { UsedSkillIds = { "mine" } }));
        Assert.Equal("update:mine", stories[0].Id);
        Assert.Equal("update:mine", Home.Snapshot.RankedHeroId(stories));
        var retained = Assert.IsType<Home.Story.CreatorStory>(stories.Single(s => s.Id == "creator:discovery"));
        Assert.Equal(Date("2026-11-17T11:30:00Z"), retained.Item.ObservedAt);
    }

    [Fact]
    public void OneEventAndNegligibleRelativeChangesAreNotStories()
    {
        var feed = Feed(trends: new[] { Trend("first", 0, 1), Trend("tiny", 10, 11), Trend("flat", 1_000, 1_002), Trend("meaningful") });
        Assert.Equal(new[] { "trend:claude_code\0meaningful" }, Ids(feed));
    }

    [Fact]
    public void AChangeIsRankedFromATwoUseQuarterOfItsPastWeekUp()
    {
        // delta >= 2 and delta / max(1, previous) >= 0.25 (InsightsSkillsHomeRanking.swift:50),
        // each on its edge: 8 -> 10 is exactly a quarter, 10 -> 12 a fifth,
        // 12 -> 9 a quarter down, and 1 -> 2 a single use.
        var feed = Feed(trends: new[] { Trend("quarter", 8, 10), Trend("fifth", 10, 12), Trend("quarter-down", 12, 9), Trend("single", 1, 2) });
        Assert.Equal(new[] { "trend:claude_code\0quarter", "trend:claude_code\0quarter-down" }, Ids(feed).OrderBy(i => i, StringComparer.Ordinal));
    }

    [Fact]
    public void ExactUsedIdentityOutweighsVolumeWithoutMatchingAnIdenticalNameFromAnotherAgent()
    {
        // Unbounded use volume would bury an exact local skill match.
        Home.Trend local = Trend("review");
        Home.Trend unrelated = Trend("review", 0, int.MaxValue, agent: "opencode");
        var context = new Home.RankingContext { UsedSkillIds = { "catalog-review" } };
        context.CatalogIdsByUsageId[local.Id] = new HashSet<string>(StringComparer.Ordinal) { "catalog-review" };
        Assert.Equal("trend:claude_code\0review", Ids(Feed(trends: new[] { unrelated, local }, context: context))[0]);
    }

    [Fact]
    public void FollowedCreatorOutranksEqualDiscoveryButBothRemain()
    {
        var feed = Feed(creators: new[] { Change("a", publisher: "discovery"), Change("z", publisher: "followed") },
            context: new Home.RankingContext { FollowedPublisherIds = { "followed" } });
        Assert.Equal(new[] { "creator:z", "creator:a" }, Ids(feed));
    }

    [Fact]
    public void InstalledIdentityBoostsOnlyTheExactCatalogSkill()
    {
        // Similar display names must never grant installed-skill relevance.
        var feed = Feed(creators: new[] { Change("a", "lookalike", "other"), Change("z", "installed") },
            context: new Home.RankingContext { InstalledSkillIds = { "installed" } });
        Assert.Equal("creator:z", Ids(feed)[0]);
    }

    [Fact]
    public void PublisherBatchVolumeDoesNotOutrankPersonalUsageOrCrowdOutOtherCreators()
    {
        var burst = Enumerable.Range(0, 30).Select(i => Change("burst-" + i, publisher: "prolific"));
        var stories = Ranked(Feed(trends: new[] { Trend("local") }, creators: burst.Append(Change("other", publisher: "other")),
            context: new Home.RankingContext { FollowedPublisherIds = { "prolific" } }));
        Assert.Equal("trend:claude_code\0local", stories[0].Id);
        Assert.Contains(stories.Take(3), s => s.Id == "creator:other");
        Assert.Single(stories.OfType<Home.Story.RollupStory>());
    }

    [Fact]
    public void TopicAndSourceDiversityBreaksRepeatedCreatorAndUsageRuns()
    {
        // The same topic should not hold consecutive cards while another topic is available.
        Home.Trend local = Trend("review");
        var context = new Home.RankingContext { UsedSkillIds = { "review" }, FollowedPublisherIds = { "followed" } };
        context.CatalogIdsByUsageId[local.Id] = new HashSet<string>(StringComparer.Ordinal) { "review" };
        string[] ids = Ids(Feed(new[] { Update("mine", "review") }, new[] { local, Trend("other") },
            new[] { Change("review", publisher: "followed") }, context));
        Assert.Equal("update:mine", ids[0]);
        Assert.Equal("trend:claude_code\0other", ids[1]);
    }

    [Fact]
    public void RepeatedRelevantUsageAllowsAnotherSourceWithoutStrictAlternation()
    {
        // A local-usage burst must leave room for real discovery near the top.
        var changes = Enumerable.Range(0, 6).Select(i => Trend("local-" + i)).ToArray();
        var context = new Home.RankingContext();
        for (int i = 0; i < changes.Length; i++)
        {
            context.UsedSkillIds.Add("catalog-" + i);
            context.CatalogIdsByUsageId[changes[i].Id] = new HashSet<string>(StringComparer.Ordinal) { "catalog-" + i };
        }

        string[] ids = Ids(Feed(trends: changes, creators: new[] { Change("discovery", publisher: "other") }, context: context));
        Assert.Equal("trend:claude_code\0local-0", ids[0]);
        Assert.Contains("creator:discovery", ids.Take(3));
        Assert.Equal(7, ids.Length); // Diversity reorders supported stories rather than dropping a source.
    }

    [Fact]
    public void TheRepetitionPenaltyFadesOneSixtyThirtyOverTheLastThreeCards()
    {
        // [1.0, 0.6, 0.3] of (same source 24 + shared publisher 20 + shared
        // topic 36) (InsightsSkillsHomeRanking.swift:175). Scores at noon:
        //   usage a, 2 -> 26, topics {T, used}, used: 72 + 18 + 8 + 4 + 7.5 = 109.5
        //   usage b, 2 -> 26, topic {T}:              72 + 8 + 4 + 7.5      = 91.5
        //   usage x, 8 -> 10, its own topic:          72 + 3.17 + 1 + 7.5   = 83.67
        //   creator first, installed and followed, 11:30:   46 + 12 + 14 + 25.96 = 97.96
        //   creator second, installed and followed, 45 h old: 46 + 12 + 14 + 22.52 = 94.52
        // Card 3, after [a, first]: second pays 24 (70.52); x pays 0.6 x 24
        //   (69.27); at 0.5, x would pay 12 (71.67) and come first.
        // Card 4, after [a, first, second]: x pays 0.3 x 24 (76.47); b pays
        //   0.3 x 60 (73.5); at 0.2, x would pay 4.8 (78.87) and b 12 (79.5),
        //   so b would come first.
        Home.Trend a = Trend("a", 2, 26), b = Trend("b", 2, 26), x = Trend("x", 8, 10);
        var context = new Home.RankingContext { UsedSkillIds = { "used" }, InstalledSkillIds = { "first", "second" }, FollowedPublisherIds = { "first", "second" } };
        context.CatalogIdsByUsageId[a.Id] = new HashSet<string>(StringComparer.Ordinal) { "T", "used" };
        context.CatalogIdsByUsageId[b.Id] = new HashSet<string>(StringComparer.Ordinal) { "T" };
        string[] ids = Ids(Feed(trends: new[] { a, b, x }, creators: new[]
        {
            Change("first", publisher: "first"), Change("second", publisher: "second", observed: "2026-11-15T15:00:00Z"),
        }, context: context));
        Assert.Equal(new[] { "trend:claude_code\0a", "creator:first", "creator:second", "trend:claude_code\0x", "trend:claude_code\0b" }, ids);
    }

    [Fact]
    public void FreshDiscoveryCanBeatOldFollowedNewsAndInvalidAgesAreExcluded()
    {
        var feed = Feed(creators: new[]
        {
            Change("fresh", publisher: "new"), Change("old-followed", publisher: "followed", observed: "2026-10-20T11:30:00Z"),
            Change("expired", publisher: "expired", observed: "2026-09-01T11:30:00Z"),
            Change("future", publisher: "future", observed: "2026-11-18T11:30:00Z"),
        }, context: new Home.RankingContext { FollowedPublisherIds = { "followed" } });
        Assert.Equal(new[] { "creator:fresh", "creator:old-followed" }, Ids(feed));
    }

    [Fact]
    public void AnUpdateIsRankedForSixHoursAfterItsCheck()
    {
        // now - checkedAt < 6 * 3600 (InsightsSkillsHomeRanking.swift:33).
        var feed = Feed(new[]
        {
            Update("inside", checkedAt: "2026-11-17T06:00:01Z"), Update("edge", checkedAt: "2026-11-17T06:00:00Z"),
            Update("future", checkedAt: "2026-11-17T12:00:01Z"),
        });
        Assert.Equal(new[] { "update:inside" }, Ids(feed));
    }

    [Fact]
    public void DuplicateSkillNewsIsSuppressedBeforeRollup()
    {
        var stories = Ranked(Feed(new[] { Update("mine", "installed"), Update("copy", "installed") }, creators: new[]
        {
            Change("same-as-update", "installed"), Change("new", "one", observed: "2026-11-17T11:00:00Z"),
            Change("old", "one", observed: "2026-11-09T11:00:00Z"),
        }));
        Assert.Equal(2, stories.Count);
        Assert.Contains(stories, s => s.Id == "creator:new");
        Assert.DoesNotContain(stories, s => s.Id == "creator:old");
    }

    [Fact]
    public void StableTiesAndTwelveStoryLimit()
    {
        var changes = Enumerable.Range(0, 25).Select(i => Change(i.ToString("00", System.Globalization.CultureInfo.InvariantCulture), publisher: "publisher-" + i)).ToArray();
        string[] first = Ids(Feed(creators: changes));
        string[] reversed = Ids(Feed(creators: changes.Reverse()));
        Assert.Equal(first, reversed);
        Assert.Equal(12, first.Length);
        Assert.Equal("creator:00", first[0]);
        Assert.Equal(first.Length, first.Distinct().Count());
    }

    [Fact]
    public void EmptyAndUsageOnlyFeedsDoNotForceUnsupportedCardsOrCover()
    {
        Assert.Empty(Ranked(Feed()));
        var stories = Ranked(Feed(trends: new[] { Trend("only") }));
        Assert.Equal(new[] { "trend:claude_code\0only" }, stories.Select(s => s.Id));
        Assert.Null(Home.Snapshot.RankedHeroId(stories));
    }
}
