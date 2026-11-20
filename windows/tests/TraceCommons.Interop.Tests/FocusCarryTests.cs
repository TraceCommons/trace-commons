using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Where keyboard focus lands when a Settings card is drawn again (W17
/// review F2; checklist O2, E2): on the same control while it can take
/// focus, otherwise on the nearest one that can in the same group (the
/// control's key up to its last '/'), after it first, then before it,
/// otherwise the nearest anywhere in the card; never nowhere while the
/// card has a control that can take focus.
/// </summary>
public class FocusCarryTests
{
    private static FocusCandidate C(string key, bool enabled = true) => new(key, enabled);

    [Fact]
    public void TheSameControlKeepsFocusWhileItCanTakeIt()
    {
        Assert.Equal("watcher/pause", FocusCarry.Target("watcher/pause",
            new[] { C("watcher/pause"), C("watcher/resume", enabled: false) }));
    }

    [Fact]
    public void PauseThatTurnsItselfOffHandsFocusToResume()
    {
        Assert.Equal("watcher/resume", FocusCarry.Target("watcher/pause",
            new[] { C("watcher/pause", enabled: false), C("watcher/resume") }));
    }

    [Fact]
    public void ResumeThatTurnsItselfOffHandsFocusBackToPause()
    {
        Assert.Equal("watcher/pause", FocusCarry.Target("watcher/resume",
            new[] { C("watcher/pause"), C("watcher/resume", enabled: false) }));
    }

    [Fact]
    public void ASaveThatDisarmsReturnsToItsOwnFieldNotTheNextTools()
    {
        Assert.Equal("cli/claude/field", FocusCarry.Target("cli/claude/save", new[]
        {
            C("cli/claude/field"), C("cli/claude/save", enabled: false),
            C("cli/codex/field"), C("cli/codex/save", enabled: false),
        }));
    }

    [Fact]
    public void AGroupWithNothingFocusableFallsBackToTheNearestInTheCard()
    {
        Assert.Equal("cli/codex/field", FocusCarry.Target("cli/claude/save", new[]
        {
            C("cli/claude/field", enabled: false), C("cli/claude/save", enabled: false),
            C("cli/codex/field"),
        }));
    }

    [Fact]
    public void AControlThatIsGoneHandsFocusToTheFirstThatCanTakeIt()
    {
        Assert.Equal("insights/recap", FocusCarry.Target("read/retry",
            new[] { C("insights/recap") }));
    }

    [Fact]
    public void NothingWasFocusedOrNothingCanTakeItMeansNoMove()
    {
        Assert.Null(FocusCarry.Target(null, new[] { C("watcher/pause") }));
        Assert.Null(FocusCarry.Target("watcher/pause",
            new[] { C("watcher/pause", enabled: false), C("watcher/resume", enabled: false) }));
    }
}
