using System;
using System.IO;
using System.Linq;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Where a step's column sits (FirstRunFrame.swift:186-226): below the
/// thread's 44, the free height is split three ways -- above the graphic,
/// between the graphic and the title, below the card -- none under 24. On
/// Uses and Join the card is a ScrollView (UsesScreen.swift:647,
/// JoinScreen.swift:494), which takes all the free height, so every gap
/// rests at 24 and the column starts under the thread.
/// </summary>
public class FirstRunFrameLayoutTests
{
    [Theory]
    [InlineData(FirstRunStep.Tools)]
    [InlineData(FirstRunStep.PrivateAI)]
    [InlineData(FirstRunStep.Traces)]
    [InlineData(FirstRunStep.Terms)]
    public void AFixedCardSplitsTheFreeHeightThreeWays(FirstRunStep step)
    {
        FirstRunFrameSplit split = FirstRunFrameLayout.Split(step, 300);
        Assert.Equal(100, split.AboveGraphic);
        Assert.Equal(100, split.BelowGraphic);
        Assert.Equal(100, split.BelowCard);
        Assert.Equal(0, split.CardExtra);
    }

    [Theory]
    [InlineData(FirstRunStep.Uses)]
    [InlineData(FirstRunStep.Join)]
    public void AScrollingCardTakesTheFreeHeightAndTheColumnStartsUnderTheThread(FirstRunStep step)
    {
        FirstRunFrameSplit split = FirstRunFrameLayout.Split(step, 300);
        Assert.Equal(24, split.AboveGraphic);
        Assert.Equal(24, split.BelowGraphic);
        Assert.Equal(24, split.BelowCard);
        Assert.Equal(300 - 72, split.CardExtra);
    }

    [Theory]
    [InlineData(FirstRunStep.Tools)]
    [InlineData(FirstRunStep.Uses)]
    public void NoGapIsEverUnderTheMinimum(FirstRunStep step)
    {
        FirstRunFrameSplit split = FirstRunFrameLayout.Split(step, 30);
        Assert.Equal(24, split.AboveGraphic);
        Assert.Equal(24, split.BelowGraphic);
        Assert.Equal(24, split.BelowCard);
        Assert.Equal(0, split.CardExtra);
    }

    [Fact]
    public void TheRowsAreTheMacsOrder()
    {
        Assert.Equal(
            new[] { FirstRunFrameSlot.Gap, FirstRunFrameSlot.Graphic, FirstRunFrameSlot.Gap, FirstRunFrameSlot.Title, FirstRunFrameSlot.Card, FirstRunFrameSlot.Notice, FirstRunFrameSlot.Gap },
            FirstRunFrameLayout.Rows(FirstRunStep.Tools).Select(row => row.Slot));
        Assert.All(FirstRunFrameLayout.Rows(FirstRunStep.Tools), row => Assert.Equal(row.Slot == FirstRunFrameSlot.Gap, row.Fills));
        Assert.All(FirstRunFrameLayout.Rows(FirstRunStep.Uses), row => Assert.Equal(row.Slot == FirstRunFrameSlot.Card, row.Fills));
        Assert.All(
            FirstRunFrameLayout.Rows(FirstRunStep.Join).Where(row => row.Slot == FirstRunFrameSlot.Gap),
            row => Assert.Equal(FirstRunFrameLayout.MinimumGap, row.MinHeight));
    }

    /// <summary>The window lays a step out from these rows; it no longer centres the column.</summary>
    [Fact]
    public void TheWindowLaysTheStepOutFromTheRows()
    {
        string source = ShellSource("TraceCommons.App/FirstRunWindow.xaml.cs");
        int start = source.IndexOf("private UIElement Framed(", StringComparison.Ordinal);
        Assert.True(start >= 0, "Framed is gone");
        int end = source.IndexOf("\n    }\n", start, StringComparison.Ordinal);
        string framed = source[start..end];
        Assert.DoesNotContain("VerticalAlignment.Center", framed, StringComparison.Ordinal);
        Assert.Contains("FirstRunFrameLayout.Rows(step)", framed, StringComparison.Ordinal);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
