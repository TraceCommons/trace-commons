using System;
using System.IO;
using System.Linq;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The credit arrival's motion, pinned to the Mac. Every expected number was
/// printed by a throwaway SwiftPM probe that compiled the Mac's own
/// TCDesign and TCShellCore at this branch's base (841d81ba5) under
/// <c>TC_GLASS_THEME=flat</c>; the probe is not committed. File and line
/// are the Swift the number comes from.
/// </summary>
public class CreditMotionTests
{
    private const double Tight = 1e-6;

    private static void Coin(Coin c, double angle, double speed, double diameter, double spin, double phase, double tilt, double delay)
    {
        Assert.Equal(angle, c.Angle, Tight);
        Assert.Equal(speed, c.Speed, Tight);
        Assert.Equal(diameter, c.Diameter, Tight);
        Assert.Equal(spin, c.Spin, Tight);
        Assert.Equal(phase, c.Phase, Tight);
        Assert.Equal(tilt, c.Tilt, Tight);
        Assert.Equal(delay, c.Delay, Tight);
    }

    /// <summary>CoinBurst.swift:30-99: Home's 44 coins, as the probe printed them.</summary>
    [Fact]
    public void HomesCoinsAreTheMacs()
    {
        Assert.Equal(44, CoinBurst.Coins.Count);
        Coin(CoinBurst.Coins[0], -0.045279439, 225.420958759, 4.173587869, 1.865122414, 0.056142745, 0.569936333, 0.009774930);
        Coin(CoinBurst.Coins[1], 0.177083699, 234.780820724, 4.549750023, 3.215898850, 0.155711821, 0.378986489, 0.008614963);
        Coin(CoinBurst.Coins[17], 2.390153041, 311.170974690, 4.601725453, 2.425531956, 0.466468971, 0.463764974, 0.066267293);
        Coin(CoinBurst.Coins[43], 6.080404390, 320.084667093, 3.635890589, 2.070014675, 0.092186304, -0.309069997, 0.007778390);
        Assert.Equal(121.016367039, CoinBurst.Farthest, Tight);
        Assert.Equal(5.981687024, CoinBurst.Widest, Tight);
        Assert.Equal(0.335201601, CoinBurst.Noise(3, 2), Tight);
    }

    /// <summary>CoinBurst.swift:105-111: drag, gravity and squash, and the sign: y runs down, so a coin falls to +y.</summary>
    [Fact]
    public void HomesCoinsFlyAndFallAsTheMacs()
    {
        Offset a = CoinBurst.OffsetOf(CoinBurst.Coins[0], 0.3);
        Assert.Equal(41.542201232, a.Dx, Tight);
        Assert.Equal(0.750513012, a.Dy, Tight);
        Offset b = CoinBurst.OffsetOf(CoinBurst.Coins[17], 1.0);
        Assert.Equal(-64.079086168, b.Dx, Tight);
        Assert.Equal(64.901660546, b.Dy, Tight);
        Offset c = CoinBurst.OffsetOf(CoinBurst.Coins[43], 1.5);
        Assert.Equal(91.636327520, c.Dx, Tight);
        Assert.Equal(42.102571579, c.Dy, Tight);
        Assert.Equal(1.0, CoinBurst.Sideways(150), Tight);
        Assert.Equal(0.0, CoinBurst.Sideways(0), Tight);
        Offset d = CoinBurst.OffsetOf(CoinBurst.Coins[0], 0.8, CoinBurst.Sideways(150));
        Assert.Equal(61.721848849, d.Dx, Tight);
        Assert.Equal(13.597809140, d.Dy, Tight);
    }

    /// <summary>CoinBurst.swift:131-135.</summary>
    [Theory]
    [InlineData(0.0, 1.0)]
    [InlineData(0.935, 1.0)]
    [InlineData(1.0, 0.915032680)]
    [InlineData(1.5, 0.261437908)]
    [InlineData(1.7, 0.0)]
    [InlineData(2.0, 0.0)]
    public void TheCoinsFadeAsTheMacs(double t, double opacity) =>
        Assert.Equal(opacity, CoinBurst.Opacity(t), Tight);

    /// <summary>DockCoinBurst.swift:30-77: the taskbar's 24 coins and the icon's pulse.</summary>
    [Fact]
    public void TheTaskbarBurstIsTheDocks()
    {
        Assert.Equal(24, DockCoinBurst.Coins.Count);
        Coin(DockCoinBurst.Coins[0], 0.073791407, 162.976601636, 7.247294792, 2.495624311, 0.618693736, 0.573845156, 0.055086434);
        Coin(DockCoinBurst.Coins[23], 5.902248296, 123.579582760, 8.808729575, 1.796087411, 0.398195948, 0.354888739, 0.008213352);
        Offset o = DockCoinBurst.OffsetOf(DockCoinBurst.Coins[5], 0.6);
        Assert.Equal(4.395285231, o.Dx, Tight);
        Assert.Equal(36.987140775, o.Dy, Tight);
        Offset small = DockCoinBurst.OffsetOf(DockCoinBurst.Coins[5], 0.6, 64);
        Assert.Equal(2.197642616, small.Dx, Tight);
        Assert.Equal(18.493570388, small.Dy, Tight);
        Assert.Equal(1.031819805, DockCoinBurst.IconScale(0.09, flash: true), Tight);
        Assert.Equal(1.045, DockCoinBurst.IconScale(0.18, flash: true), Tight);
        Assert.Equal(1.0225, DockCoinBurst.IconScale(0.3, flash: true), Tight);
        Assert.Equal(1.0, DockCoinBurst.IconScale(0.36, flash: true), Tight);
        Assert.Equal(1.0, DockCoinBurst.IconScale(0.18, flash: false), Tight);
    }

    /// <summary>MenuBarPanel.swift:521-570: the tray's ten mini coins, the gleam and the hop.</summary>
    [Fact]
    public void TheTrayBurstIsTheMenuBars()
    {
        Assert.Equal(10, StripBurst.Coins.Count);
        Coin(StripBurst.Coins[0], -1.352744386, 61.958669024, 2.956287735, 2.642961844, 0.301787142, -0.143937902, 0.074621611);
        Coin(StripBurst.Coins[4], -0.158726996, 46.371888845, 2.715672040, 2.705053835, 0.249321381, 0.410465293, 0.049102714);
        Coin(StripBurst.Coins[9], 1.413435964, 69.020084422, 2.928921055, 2.007901682, 0.742162587, 0.272076944, 0.038632573);
        Offset o = StripBurst.OffsetOf(StripBurst.Coins[4], 0.3);
        Assert.Equal(7.405465587, o.Dx, Tight);
        Assert.Equal(-0.218691453, o.Dy, Tight);
    }

    [Theory]
    [InlineData(-0.1, 0.0)]
    [InlineData(0.0, 0.0)]
    [InlineData(0.05, 0.333333333)]
    [InlineData(0.15, 1.0)]
    [InlineData(0.45, 1.0)]
    [InlineData(0.7, 1.0)]
    [InlineData(1.0, 0.7)]
    [InlineData(1.69, 0.01)]
    [InlineData(1.7, 0.0)]
    public void TheGleamRisesHoldsAndFadesAsTheMacs(double t, double gleam) =>
        Assert.Equal(gleam, StripBurst.Gleam(t), Tight);

    /// <summary>The hop is upward: negative y, as the Mac's (MenuBarPanel.swift:558-561).</summary>
    [Theory]
    [InlineData(0.0, 0.0)]
    [InlineData(0.1, -1.563662965)]
    [InlineData(0.175, -2.0)]
    [InlineData(0.3, -0.867767478)]
    [InlineData(0.35, 0.0)]
    [InlineData(0.45, 0.0)]
    public void TheMarkHopsUpAsTheMacs(double t, double hop) =>
        Assert.Equal(hop, StripBurst.Hop(t), Tight);

    /// <summary>BalanceArrival.swift:149-164: 41 frames at 24 fps, or the one still frame at 0.45 s held 1.6 s.</summary>
    [Fact]
    public void TheIconFramesAreTheMacs()
    {
        var frames = BalanceArrival.IconAnimation.FrameTimes(reduceMotion: false);
        Assert.Equal(41, frames.Count);
        Assert.Equal(0.0, frames[0], Tight);
        Assert.Equal(0.041666667, frames[1], Tight);
        Assert.Equal(1.666666667, frames[^1], Tight);
        Assert.Equal(new[] { 0.45 }, BalanceArrival.IconAnimation.FrameTimes(reduceMotion: true));
        Assert.Equal(TimeSpan.FromMilliseconds(42), BalanceArrival.IconAnimation.FrameInterval);
        Assert.Equal(TimeSpan.FromMilliseconds(1600), BalanceArrival.IconAnimation.ReduceMotionHold);
    }

    /// <summary>BalanceArrival.swift:49-57 and BalanceArrivalFlourish.swift:36-37: 24 frames of 50 ms, ending exactly on the new figure.</summary>
    [Fact]
    public void TheCountUpIsTheMacs()
    {
        Assert.Equal(
            new long[] { 1041, 1083, 1125, 1166, 1208, 1250, 1291, 1333, 1375, 1416, 1458, 1500, 1541, 1583, 1625, 1666, 1708, 1750, 1791, 1833, 1875, 1916, 1958, 2000 },
            BalanceArrival.CountSteps(1000, 2000, BalanceArrival.CountFrames));
        Assert.Equal(new long[] { 4, 3, 2 }, BalanceArrival.CountSteps(5, 2, 3));
        Assert.Equal(TimeSpan.FromMilliseconds(50), BalanceArrival.CountFrame);
    }

    /// <summary>BalanceArrival.swift:36-45: credit arrived only when received_total_nanos rose; never from the balance.</summary>
    [Fact]
    public void ArrivalIsReadFromReceivedNeverFromTheBalance()
    {
        var shown = new BalanceArrival.Reading(100, 50);
        Assert.Equal(BalanceArrival.EffectKind.None, BalanceArrival.EffectOf(null, new(200, 60), false).Kind);
        Assert.Equal(BalanceArrival.EffectKind.None, BalanceArrival.EffectOf(shown, new(100, 900), false).Kind);
        Assert.Equal(BalanceArrival.EffectKind.Highlight, BalanceArrival.EffectOf(shown, new(200, 60), true).Kind);
        Assert.Equal(BalanceArrival.EffectKind.Highlight, BalanceArrival.EffectOf(shown, new(200, 50), false).Kind);
        Assert.Equal(new BalanceArrival.Effect(BalanceArrival.EffectKind.CountUp, 50, 60), BalanceArrival.EffectOf(shown, new(200, 60), false));
    }

    /// <summary>Credit that lands while away plays once; another account's total is not new credit.</summary>
    [Fact]
    public void ASavedBaselineBelongsToItsAccountAndPlaysOnce()
    {
        string dir = Path.Combine(Path.GetTempPath(), "tc-credit-baseline-" + Guid.NewGuid().ToString("N"));
        try
        {
            string path = Path.Combine(dir, "shell-preferences.json");
            var before = new BalanceArrival.Tracker();
            Assert.False(before.Observe(4_000_000_000));
            Assert.False(before.ObserveCreditFinal(10));
            before.Save("sha256:a", new ShellPreferences(path));

            var relaunched = BalanceArrival.Tracker.Stored("sha256:a", new ShellPreferences(path));
            Assert.True(relaunched.Observe(5_000_000_000));
            Assert.True(relaunched.ObserveCreditFinal(10.1));
            relaunched.Save("sha256:a", new ShellPreferences(path));

            var again = BalanceArrival.Tracker.Stored("sha256:a", new ShellPreferences(path));
            Assert.False(again.Observe(5_000_000_000));
            Assert.False(again.ObserveCreditFinal(10.1));

            var other = BalanceArrival.Tracker.Stored("sha256:b", new ShellPreferences(path));
            Assert.Null(other.LastReceivedNanos);
            Assert.False(other.Observe(9_000_000_000));
            Assert.Null(BalanceArrival.Tracker.Stored(null, new ShellPreferences(path)).LastReceivedNanos);
        }
        finally
        {
            if (Directory.Exists(dir))
            {
                Directory.Delete(dir, recursive: true);
            }
        }
    }

    /// <summary>The Mac's tracker: a baseline first, an unknown keeps it, a fall resets it, any real rise celebrates.</summary>
    [Fact]
    public void TheTrackerCelebratesAsTheMacs()
    {
        var tracker = new BalanceArrival.Tracker();
        Assert.Equal(
            new[] { false, false, true, false, false, false, true },
            new[] { tracker.Observe(5_000_000_000), tracker.Observe(5_000_000_000), tracker.Observe(5_000_000_001), tracker.Observe(null), tracker.Observe(5_000_000_001), tracker.Observe(4_000_000_000), tracker.Observe(5_000_000_000) });
        var credit = new BalanceArrival.Tracker();
        Assert.Equal(
            new[] { false, false, true, false, false, true },
            new[] { credit.ObserveCreditFinal(10), credit.ObserveCreditFinal(10.00001), credit.ObserveCreditFinal(10.1), credit.ObserveCreditFinal(9), credit.ObserveCreditFinal(double.NaN), credit.ObserveCreditFinal(9.1) });
    }
}
