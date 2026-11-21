using System;
using System.IO;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The first-run stage's lighter backdrop on a machine that cannot keep up
/// (owner, 2026-11-20, from the profiling in #2278; Windows only, the Mac
/// has no such mode): software rendering starts without the blurred breath;
/// a stage under 20 frames a second in each of 3 seconds running drops the
/// breath, then the ground gradients; nothing else is ever dropped, and it never steps back.
/// </summary>
public class StageBackdropBudgetTests
{
    /// <summary>Draws at <paramref name="rate"/> a second from <paramref name="from"/> for <paramref name="seconds"/>; the last draw's time.</summary>
    private static double Run(StageBackdropBudget budget, double from, double seconds, double rate, double asked = FirstRunMotion.Sixty)
    {
        double t = from;
        for (int i = 0; i <= (int)Math.Round(seconds * rate); i++)
        {
            t = from + i / rate;
            budget.Drew(t, asked);
        }

        return t;
    }

    [Fact]
    public void AMachineThatKeepsUpKeepsTheFullLook()
    {
        var budget = new StageBackdropBudget(software: false);
        Run(budget, 0, 60, 30);
        Assert.Equal(StageBackdrop.Full, budget.Level);
        Run(budget, 61, 60, 21);
        Assert.Equal(StageBackdrop.Full, budget.Level);
    }

    [Fact]
    public void SoftwareRenderingStartsWithoutTheBreath()
    {
        Assert.Equal(StageBackdrop.NoBreath, new StageBackdropBudget(software: true).Level);
        Assert.Equal(StageBackdrop.Full, new StageBackdropBudget(software: false).Level);
    }

    [Fact]
    public void ASustainedSlowStageDropsTheBreathThenTheGround()
    {
        var budget = new StageBackdropBudget(software: false);
        double t = Run(budget, 0, 2.5, 11);
        Assert.Equal(StageBackdrop.Full, budget.Level);
        t = Run(budget, t + 1.0 / 11, 1, 11);
        Assert.Equal(StageBackdrop.NoBreath, budget.Level);

        // One step at a time: the next is judged on a fresh 3 s at the new look.
        t = Run(budget, t + 1.0 / 18, 2, 18);
        Assert.Equal(StageBackdrop.NoBreath, budget.Level);
        t = Run(budget, t + 1.0 / 18, 1.5, 18);
        Assert.Equal(StageBackdrop.NoGround, budget.Level);

        // The last step: nothing more is dropped, however slow.
        Run(budget, t + 1, 30, 5);
        Assert.Equal(StageBackdrop.NoGround, budget.Level);
    }

    [Fact]
    public void ABriefStallIsNotSustained()
    {
        // 2 s at 5 a second, then 30 a second: the third second keeps up.
        var budget = new StageBackdropBudget(software: false);
        double t = Run(budget, 0, 2, 5);
        Run(budget, t + 1.0 / 30, 30, 30);
        Assert.Equal(StageBackdrop.Full, budget.Level);
    }

    [Fact]
    public void ItNeverStepsBackSoItCannotFlap()
    {
        var budget = new StageBackdropBudget(software: false);
        double t = Run(budget, 0, 3.2, 11);
        Assert.Equal(StageBackdrop.NoBreath, budget.Level);
        Run(budget, t + 1.0 / 60, 120, 60);
        Assert.Equal(StageBackdrop.NoBreath, budget.Level);
    }

    [Fact]
    public void OnlyAStageAskingForThirtyOrMoreIsJudged()
    {
        // A stage asking for 15 a second that draws 15 is keeping up.
        var budget = new StageBackdropBudget(software: false);
        Run(budget, 0, 20, 15, asked: 1.0 / 15);
        Assert.Equal(StageBackdrop.Full, budget.Level);

        // The display's rate (0) and 30 are both judged.
        Run(budget, 21, 3.5, 10, asked: FirstRunMotion.DisplayRate);
        Assert.Equal(StageBackdrop.NoBreath, budget.Level);
        var thirty = new StageBackdropBudget(software: false);
        Run(thirty, 0, 3.5, 10, asked: FirstRunMotion.Thirty);
        Assert.Equal(StageBackdrop.NoBreath, thirty.Level);
    }

    [Fact]
    public void AStoppedClockIsNotASlowOne()
    {
        // The clock stops when nothing moves, the window hides or motion is
        // off; slow seconds on either side of a stop are not run together.
        var budget = new StageBackdropBudget(software: false);
        double t = Run(budget, 0, 2.2, 11);
        budget.Pause();
        t = Run(budget, t + 10, 2.2, 11);
        Assert.Equal(StageBackdrop.Full, budget.Level);

        // A gap of more than a second with no pause is a stop too.
        budget.Pause();
        t = Run(budget, t + 1, 2.2, 11);
        t = Run(budget, t + 5, 2.2, 11);
        Assert.Equal(StageBackdrop.Full, budget.Level);

        // Three slow seconds with no stop between them still count.
        Run(budget, t + 1.0 / 11, 1.2, 11);
        Assert.Equal(StageBackdrop.NoBreath, budget.Level);
    }

    [Fact]
    public void ALevelChangeIsReported()
    {
        var budget = new StageBackdropBudget(software: false);
        int changes = 0;
        double t = 0;
        for (int i = 0; i < 70; i++)
        {
            t = i / 10.0;
            if (budget.Drew(t, FirstRunMotion.Sixty))
            {
                changes++;
            }
        }

        Assert.Equal(2, changes);
        Assert.Equal(StageBackdrop.NoGround, budget.Level);
    }

    [Theory]
    [InlineData(0x1414u, 0x8Cu, 0u, true)]   // Microsoft Basic Render Driver
    [InlineData(0x1414u, 0x8Cu, 2u, true)]   // WARP, flagged software
    [InlineData(0x10DEu, 0x1EB8u, 0u, false)] // a real GPU
    [InlineData(0x8086u, 0x9A49u, 0u, false)]
    [InlineData(0x8086u, 0x9A49u, 2u, true)] // any adapter flagged software
    public void SoftwareRenderingIsTheBasicRenderDriverOrAFlaggedAdapter(uint vendor, uint device, uint flags, bool software) =>
        Assert.Equal(software, StageBackdropBudget.IsSoftwareAdapter(vendor, device, flags));

    [Fact]
    public void TheStageIsDrawnAtTheBudgetsLevel()
    {
        string window = ShellSource("TraceCommons.App/FirstRunWindow.xaml.cs");
        // Made from the canvas's own device, counted only from the draw handler.
        Assert.Contains("RenderAdapter.Read(sender.Device)", window, StringComparison.Ordinal);
        Assert.Contains("new StageBackdropBudget(RenderAdapter.IsSoftware(adapter))", window, StringComparison.Ordinal);
        int draw = window.IndexOf("private void OnDraw(", StringComparison.Ordinal);
        Assert.True(draw >= 0);
        Assert.Contains("Budget(sender, now);", window[draw..window.IndexOf("\n    }\n", draw, StringComparison.Ordinal)], StringComparison.Ordinal);
        Assert.Contains("_budget.Drew(now, _drawInterval)", window, StringComparison.Ordinal);
        Assert.Contains("backdrop: _budget.Level", window, StringComparison.Ordinal);

        // Not judged while the clock is stopped, unfocused or still; the count starts again on each.
        string stop = window[window.IndexOf("private void Stop()", StringComparison.Ordinal)..];
        Assert.Contains("_budget?.Pause();", stop[..stop.IndexOf("\n    }\n", StringComparison.Ordinal)], StringComparison.Ordinal);
        Assert.Contains("if (!_ticking || !_foreground || _reduceMotion)", window, StringComparison.Ordinal);
        Assert.Contains("_foreground = e.WindowActivationState != WindowActivationState.Deactivated;", window, StringComparison.Ordinal);
        Assert.Contains("_drawInterval = interval;", window, StringComparison.Ordinal);

        string painter = ShellSource("TraceCommons.App/FirstRun/FirstRunStagePainter.cs");
        Assert.Contains("StageBackdrop backdrop = StageBackdrop.Full", painter, StringComparison.Ordinal);
        // Both backdrop calls take the level (the window-sized one behind a scaled stage too).
        Assert.Equal(2, Count(painter, "now, reduce, backdrop);"));

        string adapter = ShellSource("TraceCommons.App/FirstRun/RenderAdapter.cs");
        Assert.Contains("StageBackdropBudget.IsSoftwareAdapter(", adapter, StringComparison.Ordinal);
    }

    private static int Count(string text, string needle)
    {
        int n = 0;
        for (int i = text.IndexOf(needle, StringComparison.Ordinal); i >= 0; i = text.IndexOf(needle, i + 1, StringComparison.Ordinal))
        {
            n++;
        }

        return n;
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
