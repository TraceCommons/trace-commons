using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings > Updates over the Windows updater against the Mac's
/// <c>UpdatesSection</c> (StartupSection.swift:184-224, 244-252) and the
/// core's words (shell_words_copy.rs:507-527).
/// </summary>
public class UpdatesCardTests
{
    [Fact]
    public void AnUnpackagedCopyIsTheMacsDisabledBranch()
    {
        UpdatesCard card = UpdatesCard.Live(packaged: false, version: "0.40.1");
        Assert.Equal("Version", card.VersionLabel);
        Assert.Equal("0.40.1", card.Version);
        Assert.Equal("Updates unavailable", card.Tag);
        Assert.False(card.TagOn);
        Assert.Equal("Updates are turned off for this build.", card.Caption);
        Assert.Null(card.CheckNow);
    }

    [Fact]
    public void APackagedCopyOffersCheckNowAndClaimsNoScheduleOrPrompt()
    {
        UpdatesCard card = UpdatesCard.Live(packaged: true, version: "0.40.1");
        Assert.Equal("Check Now", card.CheckNow);
        // "Checks daily" and "Asks before installing an update." are untrue
        // under App Installer's 8-hour background task.
        Assert.Null(card.Tag);
        Assert.Null(card.Caption);
    }

    [Fact]
    public void AMissingWordDrawsNothingInItsPlace()
    {
        UpdatesCard card = UpdatesCard.Of(false, "1", _ => string.Empty);
        Assert.Null(card.Tag);
        Assert.Null(card.Caption);
        Assert.Null(UpdatesCard.Of(true, "1", _ => string.Empty).CheckNow);
    }
}
