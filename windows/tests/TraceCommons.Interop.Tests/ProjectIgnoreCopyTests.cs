using TraceCommons.Interop;
using Xunit;

public class ProjectIgnoreCopyTests
{
    [Theory]
    [InlineData(1, "1 waiting trace")]
    [InlineData(12, "12 waiting traces")]
    public void CountsInWordsAPersonCanRead(int n, string expected)
    {
        Assert.Contains(expected, ProjectIgnoreCopy.ConfirmationBody("api", n));
    }

    [Fact]
    public void SingularIsNotPluralised()
    {
        Assert.DoesNotContain("traces", ProjectIgnoreCopy.ConfirmationBody("api", 1));
    }

    /// <summary>
    /// No group renders with nothing waiting today -- every shell groups the
    /// pending list alone. The branch is defensive: this method must be right
    /// about whatever count it is handed.
    /// </summary>
    [Fact]
    public void NothingWaitingDropsTheRemovalClause()
    {
        var body = ProjectIgnoreCopy.ConfirmationBody("api", 0);
        Assert.DoesNotContain("0", body);
        Assert.DoesNotContain("removes", body.ToLowerInvariant());
        Assert.Contains("Stops this project being offered.", body);
    }

    [Theory]
    [InlineData(0)]
    [InlineData(1)]
    [InlineData(7)]
    public void AlwaysNamesTheWayBack(int n)
    {
        var body = ProjectIgnoreCopy.ConfirmationBody("api", n);
        Assert.Contains("undo this in Settings", body);
        Assert.Contains("Nothing already submitted is affected.", body);
    }

    [Theory]
    [InlineData(3, 3)]
    [InlineData(0, 0)]
    public void ReconciliationSaysNothingWhenTheCountHeld(int promised, int purged)
    {
        Assert.Null(ProjectIgnoreCopy.Reconciliation("api", promised, purged));
    }

    [Fact]
    public void ReconciliationNamesBothCountsWhenTheyMoved()
    {
        Assert.Equal(
            "Ignored api. The queue changed while you were deciding: "
                + "5 waiting traces were removed, not 3.",
            ProjectIgnoreCopy.Reconciliation("api", 3, 5));
        Assert.Equal(
            "Ignored api. The queue changed while you were deciding: "
                + "1 waiting trace was removed, not 3.",
            ProjectIgnoreCopy.Reconciliation("api", 3, 1));
    }

    [Fact]
    public void TitleNamesTheProject()
    {
        Assert.Equal("Ignore api?", ProjectIgnoreCopy.ConfirmationTitle("api"));
    }
}
