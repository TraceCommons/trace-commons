using System;
using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The queue frame's two remaining pieces: the health banner's words and the
/// week band's labels. Both are copy, and copy is the half of this shell a
/// machine without WinUI can still hold to account.
/// </summary>
public class HealthCopyTests
{
    /// <summary>
    /// Every label in the design's failure-state table, and the fallback.
    /// </summary>
    public static TheoryData<string> EveryLabel() => new()
    {
        "not-logged-in",
        "pii-filter-unavailable",
        "privacy-filter-canary-failed",
        "near-ai-notice-not-acknowledged",
        "claim-mint-failed",
        "ingest-unreachable",
        "daily-cap-reached",
        "queue-full",
        "a-label-nobody-has-written-yet",
    };

    [Fact]
    public void NoLabelIsHealthRatherThanAnUnknownCondition()
    {
        Assert.Null(HealthCopy.ForLabel(null));
        Assert.Null(HealthCopy.ForLabel(string.Empty));
    }

    [Theory]
    [MemberData(nameof(EveryLabel))]
    public void EveryLabelRendersSomething(string label)
    {
        HealthCopy? copy = HealthCopy.ForLabel(label);

        Assert.NotNull(copy);
        Assert.NotEmpty(copy!.Title);
        Assert.NotEmpty(copy.Detail);
    }

    /// <summary>
    /// The design's first copy rule: never name the mechanism. The labels
    /// themselves use these words; the sentences must not.
    /// </summary>
    [Theory]
    [MemberData(nameof(EveryLabel))]
    public void NoSentenceNamesAnInternalMechanism(string label)
    {
        HealthCopy copy = HealthCopy.ForLabel(label)!;
        string text = (copy.Title + " " + copy.Detail).ToLowerInvariant();

        foreach (string forbidden in new[] { "privacy filter", "canary", "claim", "ingest", "pii" })
        {
            Assert.False(
                text.Contains(forbidden, StringComparison.Ordinal),
                $"{label} names the mechanism: {text}");
        }
    }

    /// <summary>
    /// An unknown label must not be echoed. A label is an internal name, and
    /// printing it is the most direct possible breach of the rule above.
    /// </summary>
    [Fact]
    public void AnUnknownLabelIsNeverEchoedBackAsTheExplanation()
    {
        HealthCopy copy = HealthCopy.ForLabel("some-future-label-with-a-path-in-it")!;

        Assert.DoesNotContain("some-future-label", copy.Title, StringComparison.Ordinal);
        Assert.DoesNotContain("some-future-label", copy.Detail, StringComparison.Ordinal);
    }

    /// <summary>
    /// The design's second copy rule: always state the data consequence.
    /// </summary>
    [Theory]
    [MemberData(nameof(EveryLabel))]
    public void EverySentenceStatesWhatHappenedToTheData(string label)
    {
        HealthCopy copy = HealthCopy.ForLabel(label)!;
        string text = (copy.Title + " " + copy.Detail).ToLowerInvariant();

        string[] consequences =
        {
            "nothing has been lost",
            "queue is safe",
            "rather than going out unscanned",
            "nothing is being sent",
            "nothing has gone out",
            "contributions resume",
            "goes out tomorrow",
            "stopped queuing",
        };

        Assert.Contains(consequences, c => text.Contains(c, StringComparison.Ordinal));
    }

    /// <summary>
    /// Only the two labels the design gives an action get one. Every other
    /// condition clears on its own, and a button that cannot change what it
    /// sits beside is a button that teaches people not to trust buttons.
    /// </summary>
    [Theory]
    [MemberData(nameof(EveryLabel))]
    public void OnlyTheTwoActionableLabelsCarryAButton(string label)
    {
        string? action = HealthCopy.ForLabel(label)!.ActionLabel;

        switch (label)
        {
            case "not-logged-in":
                Assert.Equal("Reconnect", action);
                break;
            case "near-ai-notice-not-acknowledged":
                Assert.Equal("Review and confirm", action);
                break;
            default:
                Assert.Null(action);
                break;
        }
    }

    /// <summary>
    /// The two sentences the visual design quotes verbatim, compared whole.
    /// A paraphrase here is a contributor being told a different thing on
    /// Windows than on Linux about the same daemon state.
    /// </summary>
    [Fact]
    public void TheQuotedBannersAreTranscribedNotParaphrased()
    {
        HealthCopy nearAi = HealthCopy.ForLabel("near-ai-notice-not-acknowledged")!;
        Assert.Equal("One thing to confirm.", nearAi.Title);
        Assert.Equal(
            "You chose the extra privacy scan, which sends message text to NEAR AI. "
            + "Confirm you're OK with that and contributions resume.",
            nearAi.Detail);

        HealthCopy notConnected = HealthCopy.ForLabel("not-logged-in")!;
        Assert.Equal("Not connected.", notConnected.Title);
        Assert.Equal(
            "Sessions are being queued, but nothing can be sent until you reconnect. "
            + "Nothing has been lost.",
            notConnected.Detail);
    }

    /// <summary>
    /// The two labels that share one banner in the design share one here.
    /// </summary>
    [Fact]
    public void TheTwoUnreachableLabelsAreMergedAsTheDesignMergesThem()
    {
        Assert.Equal(
            HealthCopy.ForLabel("claim-mint-failed"),
            HealthCopy.ForLabel("ingest-unreachable"));
    }
}

public class WeekBandCopyTests
{
    [Fact]
    public void TheBandSaysWhatTheDesignSaysItSays()
    {
        Assert.Equal("This week", WeekBandCopy.ThisWeek);
        Assert.Equal("Contributed", WeekBandCopy.Contributed);
    }

    /// <summary>
    /// The two labels the band shares with History are the same strings, not
    /// two spellings of one state.
    /// </summary>
    [Fact]
    public void TheSharedLabelsAreSharedRatherThanRestated()
    {
        Assert.Equal(HistoryCopy.QuarantineHeading, WeekBandCopy.Held);
        Assert.Equal(HistoryCopy.InTheCommons, WeekBandCopy.InTheCommons);
    }
}
