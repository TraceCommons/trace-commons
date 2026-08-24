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

    [Fact]
    public void TitleNamesTheProject()
    {
        Assert.Equal("Ignore api?", ProjectIgnoreCopy.ConfirmationTitle("api"));
    }
}
