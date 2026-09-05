using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class ProjectManualModeTests
{
    [Theory]
    [InlineData("auto_upload", "ask")]
    [InlineData("ignore", "ask")]
    [InlineData("ask", "ignore")]
    public void ManualActionRestoresReviewOrIgnoresWithoutArming(string current, string expected)
    {
        Assert.Equal(expected, ProjectManualMode.Next(current));
        Assert.NotEqual("auto_upload", ProjectManualMode.Next(current));
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("future-mode")]
    public void UnknownModeHasNoImplicitTransition(string? mode)
    {
        Assert.Null(ProjectManualMode.Next(mode));
    }
}
