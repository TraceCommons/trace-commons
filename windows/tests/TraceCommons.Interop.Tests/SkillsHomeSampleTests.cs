using System;
using System.IO;
using System.Linq;
using System.Text.Json;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The <c>skillsHome</c> sample extra (TRACE_COMMONS_SAMPLE_EXTRA=skillsHome):
/// a Debug sample launch shows Insights &gt; Skills with Home's story cards
/// and its usage bars, which had never been drawn on Windows (#2233). A
/// production build can never turn <c>insights_skills</c> on, so the extra's
/// flag snapshot takes this build's own environment and digest at launch;
/// and Skills reads its days and weeks at the extra's own instant.
/// </summary>
public class SkillsHomeSampleTests
{
    private static string Root => Path.Combine(AppContext.BaseDirectory, "RecordedSamples");

    private static readonly BuildFeatureProfile Build = new(1, "production", new string('c', 64));

    private static SampleDaemon Sample() =>
        new("normalDay", Root, SampleDaemon.Extra(SampleDaemon.SkillsHomeExtra, Root)) { BuildProfile = () => Build };

    [Fact]
    public void TheExtraIsFoundByName()
    {
        string? dir = SampleDaemon.Extra("skillsHome", Root);
        Assert.NotNull(dir);
        Assert.True(File.Exists(Path.Combine(dir!, "insights_skills.json")), dir);
    }

    [Fact]
    public void TheFlagSnapshotIsThisBuildsWithSkillsOn()
    {
        string frame = Sample().Call(FeatureFlagGate.SnapshotMethod);
        JsonElement result = JsonDocument.Parse(frame).RootElement.GetProperty("result");
        FeatureFlagSnapshot? snapshot = FeatureFlagSnapshot.Parse(result);
        Assert.NotNull(snapshot);
        Assert.True(Build.Agrees(snapshot!.Environment, snapshot.ProfileDigest));
        Assert.True(snapshot.IsEnabled(FeatureFlagGate.InsightsSkills));
    }

    [Fact]
    public void WithoutTheExtraNoFlagSnapshotIsAnswered()
    {
        var plain = new SampleDaemon("normalDay", Root) { BuildProfile = () => Build };
        Assert.Contains("\"error\"", plain.Call(FeatureFlagGate.SnapshotMethod), StringComparison.Ordinal);
    }

    [Fact]
    public void OnlyASampleWithTheExtraAnswersFlags()
    {
        Assert.True(Sample().AnswersFlags);
        Assert.False(new SampleDaemon("normalDay", Root).AnswersFlags);
    }

    [Fact]
    public void TheWindowReadsTheExtrasFlagsOnceAndNoOtherSamplesFlags()
    {
        string window = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/MainWindow.xaml.cs.txt"))
            .Replace("\r\n", "\n", StringComparison.Ordinal);
        int at = window.IndexOf("private bool ReadsNoFlags()", StringComparison.Ordinal);
        Assert.True(at >= 0, "ReadsNoFlags is gone");
        string body = window[at..window.IndexOf("\n    }\n", at, StringComparison.Ordinal)];
        Assert.Contains("_host.IsSample && SampleDaemon.FromEnvironment()?.AnswersFlags != true", body, StringComparison.Ordinal);
        Assert.Contains("if (_host.IsSample && !ReadsNoFlags())", window, StringComparison.Ordinal);
    }

    [Fact]
    public void TheExtraCarriesItsInstant()
    {
        Assert.Equal(DateTimeOffset.Parse("2026-11-18T16:00:00Z", System.Globalization.CultureInfo.InvariantCulture), Sample().PinnedNow);
        Assert.Null(new SampleDaemon("normalDay", Root).PinnedNow);
    }

    [Fact]
    public void AtItsInstantHomeHasEveryKindOfStoryAndTheBars()
    {
        SampleDaemon sample = Sample();
        JsonElement result = JsonDocument.Parse(sample.Call(InsightsSkillsModel.Method)).RootElement.GetProperty("result");
        SkillsSnapshot snapshot = SkillsSnapshot.Parse(result) ?? throw new InvalidOperationException("undecodable");
        Assert.True(snapshot.Enabled && snapshot.Readable);
        DateTimeOffset now = sample.PinnedNow!.Value;
        TimeZoneInfo zone = TimeZoneInfo.Utc;
        InsightsSkillsHome.Snapshot home = InsightsSkillsHome.Make(snapshot, new InsightsSkillsModel(), now, zone);

        Assert.NotEmpty(home.Trends);
        Assert.All(home.Trends, t => Assert.Equal(14, t.Daily.Count));
        Assert.Contains(home.Trends, t => t.Direction == InsightsSkillsHome.Direction.Started);
        Assert.Contains(home.Trends, t => t.Direction == InsightsSkillsHome.Direction.More);
        Assert.Contains(home.Trends, t => t.Direction == InsightsSkillsHome.Direction.Less);
        Assert.NotEmpty(home.Week);
        Assert.NotEmpty(home.Creators);
        Assert.NotEmpty(home.DefaultCreators);

        var stories = home.Timeline(now, zone).SelectMany(g => g.Stories).ToList();
        Assert.Contains(stories, s => s is InsightsSkillsHome.Story.TrendStory);
        Assert.Contains(stories, s => s is InsightsSkillsHome.Story.WeekStory);
        Assert.Contains(stories, s => s is InsightsSkillsHome.Story.RollupStory or InsightsSkillsHome.Story.CreatorStory);
    }

    [Fact]
    public void TheEarlierWeeksBarsAreTheMacsFaintInk()
    {
        // InsightsSkillsHomeView.swift:464: bar.recent ? tint : GlassColor.ink(0.22).
        Assert.Equal(0.22, InsightsSkillsHome.EarlierBarInk);
        string view = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/Controls/InsightsSkillsView.cs.txt"))
            .Replace("\r\n", "\n", StringComparison.Ordinal);
        int at = view.IndexOf("private FrameworkElement UsageBars(", StringComparison.Ordinal);
        string body = view[at..view.IndexOf("\n    }\n", at, StringComparison.Ordinal)];
        Assert.Contains("InsightsSkillsHome.EarlierBarInk", body, StringComparison.Ordinal);
        Assert.Contains("TcGlassInkBrush", body, StringComparison.Ordinal);
    }

    [Fact]
    public void SkillsReadsTheModelsClockAndTheWindowPinsItInASample()
    {
        string view = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/Controls/InsightsSkillsView.cs.txt"));
        Assert.DoesNotContain("DateTimeOffset.Now", view, StringComparison.Ordinal);
        Assert.Contains("_model.Now()", view, StringComparison.Ordinal);
        string window = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App/MainWindow.xaml.cs.txt"));
        Assert.Contains("SampleDaemon.FromEnvironment()?.PinnedNow", window, StringComparison.Ordinal);
    }
}
