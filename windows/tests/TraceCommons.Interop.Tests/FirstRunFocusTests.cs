using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class FirstRunFocusTests
{
    [Fact]
    public void ARebuildReturnsFocusToTheControlOrTheHeading()
    {
        var focus = new FirstRunFocus();
        Assert.Equal(FirstRunFocus.Heading, focus.Target(_ => true));

        focus.Focused("sign-in");
        Assert.Equal("sign-in", focus.Target(k => k == "sign-in"));

        // The commit disables Sign in: the heading holds focus meanwhile,
        // and Sign in gets it back once enabled.
        Assert.Equal(FirstRunFocus.Heading, focus.Target(_ => false));
        focus.Focused(FirstRunFocus.Heading);
        Assert.Equal("sign-in", focus.Target(k => k == "sign-in"));

        focus.Focused("sharing:Automatic");
        Assert.Equal("sharing:Automatic", focus.Target(_ => true));
        focus.StepChanged();
        Assert.Equal(FirstRunFocus.Heading, focus.Target(_ => true));
    }
}
