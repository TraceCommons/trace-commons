using System;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The owner's rule (2026-11-20): animations run at full rate only while
/// their window is focused; while it is visible but another window is
/// active they draw at most 15 frames a second; hidden or minimised they
/// stop. One rule, in the clock every animated surface already asks.
/// </summary>
public class FocusFrameRuleTests
{
    private sealed class Win
    {
    }

    [Fact]
    public void FocusedRunsAtFullRateUnfocusedAtFifteenHiddenNotAtAll()
    {
        Assert.Equal(0, FocusFrameRule.Interval(0, WindowAttention.Focused));
        Assert.Equal(1.0 / 30, FocusFrameRule.Interval(1.0 / 30, WindowAttention.Focused));
        Assert.Equal(1.0 / 15, FocusFrameRule.Interval(0, WindowAttention.Unfocused));
        Assert.Equal(1.0 / 15, FocusFrameRule.Interval(1.0 / 60, WindowAttention.Unfocused));
        Assert.Equal(0.5, FocusFrameRule.Interval(0.5, WindowAttention.Unfocused));
        Assert.Null(FocusFrameRule.Interval(0, WindowAttention.Hidden));
    }

    [Fact]
    public void TheRegistryKnowsEachWindowsAttention()
    {
        var windows = new WindowClockRegistry<Win>();
        var a = new Win();
        var b = new Win();
        int changes = 0;
        windows.Changed += (_, _) => changes++;
        Assert.Equal(WindowAttention.Focused, windows.Attention(a));
        Assert.Equal(WindowAttention.Hidden, windows.Attention(null));
        windows.SetFocused(a, false);
        Assert.Equal(1, changes);
        Assert.Equal(WindowAttention.Unfocused, windows.Attention(a));
        Assert.Equal(WindowAttention.Focused, windows.Attention(b));
        windows.Set(a, false);
        Assert.Equal(WindowAttention.Hidden, windows.Attention(a));
        Assert.Null(windows.Interval(a, 0));
        windows.Set(a, true);
        windows.SetFocused(a, true);
        Assert.Equal(0, windows.Interval(a, 0));
        windows.SetFocused(a, true);
        Assert.Equal(4, changes);
    }

    [Fact]
    public void APaceDrawsEveryFrameFocusedFifteenUnfocusedAndFullAgainAtOnce()
    {
        var windows = new WindowClockRegistry<Win>();
        var w = new Win();
        var pace = new FocusFramePace<Win>(windows);
        int Drawn(double from, double seconds)
        {
            int n = 0;
            for (int f = 0; f < seconds * 60; f++)
            {
                if (pace.Due(w, from + f / 60.0))
                {
                    n++;
                }
            }

            return n;
        }

        Assert.Equal(60, Drawn(0, 1));
        windows.SetFocused(w, false);
        Assert.InRange(Drawn(1, 1), 15, 16);
        windows.SetFocused(w, true);
        Assert.True(pace.Due(w, 2 + 1 / 120.0), "refocused: the very next frame draws");
        Assert.Equal(60, Drawn(3, 1));
        windows.Set(w, false);
        Assert.Equal(0, Drawn(4, 1));
        Assert.False(pace.Skips(w, 5), "hidden: the surface's own stop logic runs");
        windows.Set(w, true);
        windows.SetFocused(w, false);
        Assert.False(pace.Skips(w, 6));
        Assert.True(pace.Skips(w, 6 + 1 / 60.0));
    }

    [Fact]
    public void AGlassClockDrawsAtMostFifteenWhileItsWindowIsUnfocused()
    {
        var windows = new WindowClockRegistry<Win>();
        var w = new Win();
        double now = 0;
        int draws = 0;
        var core = new GlassClockCore<Win>(windows, 0, () => { }, () => { }, _ => draws++, () => now) { Window = w, Loaded = true };
        draws = 0;
        for (int f = 0; f < 60; f++, now += 1 / 60.0)
        {
            core.OnFrame();
        }

        Assert.Equal(60, draws);
        windows.SetFocused(w, false);
        draws = 0;
        for (int f = 0; f < 60; f++, now += 1 / 60.0)
        {
            core.OnFrame();
        }

        Assert.InRange(draws, 15, 16);
        windows.SetFocused(w, true);
        draws = 0;
        for (int f = 0; f < 60; f++, now += 1 / 60.0)
        {
            core.OnFrame();
        }

        Assert.InRange(draws, 59, 60);
    }

    /// <summary>
    /// Every surface that draws on the display's frames, and every
    /// animation timer, takes its rate from the window's rule.
    /// </summary>
    [Fact]
    public void EveryAnimatedSurfaceAsksTheSharedRule()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App");
        var files = Directory.GetFiles(root, "*.cs.txt", SearchOption.AllDirectories)
            .Select(f => (Name: Path.GetRelativePath(root, f).Replace('\\', '/'), Text: File.ReadAllText(f)))
            .ToList();
        var rendering = files.Where(f => f.Text.Contains("CompositionTarget.Rendering +=", StringComparison.Ordinal)).ToList();
        Assert.True(rendering.Count >= 9, "found " + rendering.Count);
        foreach (var (name, text) in rendering)
        {
            if (name == "Look/GlassClock.cs.txt")
            {
                Assert.Contains("GlassClockCore<XamlRoot>", text, StringComparison.Ordinal);
                continue;
            }

            Assert.True(Regex.IsMatch(text, @"FocusFramePace<XamlRoot> _\w+ = new\(WindowClocks\.Windows\)") && text.Contains(".Skips(", StringComparison.Ordinal),
                name + " draws on the display's frames without the window's rule");
        }

        // The timers: the map's two clocks (the moving layer's, and the
        // station's own while the moving one is slower, #2268) and Home's
        // glyph and count clocks.
        string map = files.Single(f => f.Name == "Controls/TracesMapView.cs.txt").Text;
        Assert.Equal(2, Regex.Matches(map, @"WindowClocks\.Windows\.Interval\(XamlRoot, ").Count);
        Assert.Contains("WindowClocks.Windows.Interval(XamlRoot, own)", map, StringComparison.Ordinal);
        Assert.Contains("WindowClocks.Windows.Interval(XamlRoot, library)", map, StringComparison.Ordinal);
        string home = files.Single(f => f.Name == "Controls/HomeView.cs.txt").Text;
        Assert.Contains("WindowClocks.Windows.Interval(XamlRoot, ", home, StringComparison.Ordinal);

        // Focus reaches the registry from every watched window and the main one.
        string clocks = files.Single(f => f.Name == "WindowClocks.cs.txt").Text;
        Assert.Contains("WindowActivationState.Deactivated", clocks, StringComparison.Ordinal);
        Assert.Contains("Windows.SetFocused(", clocks, StringComparison.Ordinal);
        Assert.Contains("FollowFocus(window);", clocks, StringComparison.Ordinal);
        Assert.Contains(files, f => f.Name.StartsWith("MainWindow", StringComparison.Ordinal) && f.Text.Contains("WindowClocks.FollowFocus(this);", StringComparison.Ordinal));
    }
}
