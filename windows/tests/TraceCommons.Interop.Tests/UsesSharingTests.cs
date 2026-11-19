using System.Linq;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Uses' Sharing choices carry the Mac's titles and lines
/// (UsesScreen.swift:193-210). Red before: the Windows control titled the
/// Automatic choice with the copy's scope sentence ("Sends past and new
/// traces once ...") and dropped the sign-in line (box tc-win-az3, 2026-11-19).
/// </summary>
public sealed class UsesSharingTests
{
    private static readonly UsesWords Uses = new()
    {
        AskEachTime = "Ask each time",
        AutomaticScope = "Sends past and new traces once they've been unchanged for 24 hours.",
        AutomaticAfterSignIn = "Starts when setup finishes after sign-in.",
    };

    [Fact]
    public void AskThenAutomaticNamedByTheModeTableWithTheScopeAndSignInLines()
    {
        var options = UsesSharing.Options(new FirstRunState(), Uses, "Automatic");
        Assert.Equal(new[] { SharingPath.AskMe, SharingPath.Automatic }, options.Select(o => o.Path));
        Assert.Equal(new[] { "Ask each time", "Automatic" }, options.Select(o => o.Title));
        Assert.Empty(options[0].Lines);
        Assert.Equal(new[] { Uses.AutomaticScope, Uses.AutomaticAfterSignIn }, options[1].Lines);
    }

    [Fact]
    public void TheSignInLineGoesOnceAnEnrollmentIsHeld()
    {
        var state = new FirstRunState { Account = AccountAnswer.Enrolled };
        var options = UsesSharing.Options(state, Uses, "Automatic");
        Assert.Equal(new[] { Uses.AutomaticScope }, options[1].Lines);
    }

    [Fact]
    public void NoModeNameNoAutomatic()
    {
        var options = UsesSharing.Options(new FirstRunState(), Uses, "");
        Assert.Equal(new[] { SharingPath.AskMe }, options.Select(o => o.Path));
    }
}
