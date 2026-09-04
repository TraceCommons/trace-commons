using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A recent-search list is the contributor's list of the things they were
/// afraid of leaking. It stays in memory for that reason, and it must hold
/// what they actually asked -- not every prefix they typed on the way there.
/// </summary>
/// <remarks>
/// The list is process-wide, so <see cref="RecentSearches.Reset"/> in the
/// constructor is what makes each case start from a known list. xUnit runs
/// the cases of one class serially, which is what makes that enough.
/// </remarks>
public class RecentSearchesTests
{
    public RecentSearchesTests() => RecentSearches.Reset();

    [Fact]
    public void AnEmptyListStartsEmpty() => Assert.Empty(RecentSearches.Current);

    [Fact]
    public void ACommittedTermIsRemembered()
        => Assert.Equal(new[] { "acme-corp" }, RecentSearches.Remember("acme-corp"));

    [Fact]
    public void TheMostRecentTermLeads()
    {
        RecentSearches.Remember("first");

        Assert.Equal(new[] { "second", "first" }, RecentSearches.Remember("second"));
    }

    [Fact]
    public void RepeatingATermMovesItToTheFrontWithoutDuplicating()
    {
        RecentSearches.Remember("a");
        RecentSearches.Remember("b");

        Assert.Equal(new[] { "a", "b" }, RecentSearches.Remember("a"));
    }

    [Fact]
    public void TheListIsCappedAtSix()
    {
        foreach (string term in new[] { "1", "2", "3", "4", "5", "6", "7" })
        {
            RecentSearches.Remember(term);
        }

        Assert.Equal(6, RecentSearches.Current.Count);
        Assert.Equal("7", RecentSearches.Current[0]);
        Assert.DoesNotContain("1", RecentSearches.Current);
    }

    [Fact]
    public void AnEmptyOrBlankTermIsNotRemembered()
    {
        RecentSearches.Remember("");
        RecentSearches.Remember("   ");

        Assert.Empty(RecentSearches.Current);
    }

    /// <summary>
    /// Trimmed, so the same question typed with a stray space is the same
    /// entry rather than a second one costing a slot.
    /// </summary>
    [Fact]
    public void ATermIsTrimmedBeforeItIsRemembered()
    {
        RecentSearches.Remember("  acme-corp ");

        Assert.Equal(new[] { "acme-corp" }, RecentSearches.Remember("acme-corp"));
    }
}
