using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The switch-on notices: <c>status.arming_rewordings</c> (K5) and
/// <c>status.automatic_contribution_held</c>, handed back to the ABI verbatim,
/// and the notices it returns.
/// </summary>
public sealed class SwitchOnNoticesTests
{
    private const string Rewording =
        "{\"id\":2,\"project_id\":\"p\",\"project_label\":\"api\","
        + "\"was\":\"model_scrubbed\",\"now\":\"patterns_only\"}";

    private const string Held =
        "{\"held_sessions\":2,\"reasons\":[\"admission-evidence-is-per-session\"],"
        + "\"projects\":[{\"project_id\":\"p\",\"project_label\":\"api\",\"held_sessions\":2}]}";

    private static readonly IReadOnlyDictionary<string, object?> RewordedComplete = new Dictionary<string, object?>
    {
        ["title"] = "T",
        ["body"] = "B",
        ["now_heading"] = "H",
        ["scope"] = "S",
        ["limit"] = "L",
        ["no_review"] = "N",
        ["acknowledge"] = "K",
        ["ask_first_action"] = "Ask me",
        ["ask_first_failed"] = "F",
    };

    private static List<JsonElement> Elements(params string[] json) =>
        json.Select(j => JsonDocument.Parse(j).RootElement.Clone()).ToList();

    [Fact]
    public void EachRewordingGoesToTheAbiVerbatimAndKeepsItsIdAndProject()
    {
        var seen = new List<string>();
        ArmingRewordingCard card = Assert.Single(SwitchOnNotices.RewordingCards(
            Elements(Rewording),
            wire =>
            {
                seen.Add(wire);
                return JsonSerializer.Serialize(RewordedComplete);
            }));
        Assert.Equal(2UL, card.Id);
        Assert.True(card.CanAcknowledge);
        Assert.True(card.CanAskFirst);
        Assert.Equal("p", card.AskFirstProjectId);
        Assert.Equal(
            JsonDocument.Parse(Rewording).RootElement.ToString(),
            JsonDocument.Parse(Assert.Single(seen)).RootElement.ToString());

        using JsonDocument request = JsonDocument.Parse(SwitchOnNotices.AskFirstParams("p"));
        Assert.Equal("p", request.RootElement.GetProperty("project_id").GetString());
        Assert.Equal("notify_only", request.RootElement.GetProperty("mode").GetString());
    }

    [Fact]
    public void ARewordingNoticeMissingASentenceOrOutOfPairIsRefused()
    {
        Assert.NotNull(SwitchOnNotices.ParseRewording(JsonSerializer.Serialize(RewordedComplete)));
        foreach (string field in ArmingRewordedNotice.ConsumedFields
                     .Where(f => f != "ask_first_action" && f != "ask_first_failed"))
        {
            var partial = new Dictionary<string, object?>(RewordedComplete) { [field] = string.Empty };
            Assert.Null(SwitchOnNotices.ParseRewording(JsonSerializer.Serialize(partial)));
        }

        var unpaired = new Dictionary<string, object?>(RewordedComplete) { ["ask_first_failed"] = null };
        Assert.Null(SwitchOnNotices.ParseRewording(JsonSerializer.Serialize(unpaired)));
        Assert.Null(SwitchOnNotices.ParseRewording("not json"));
    }

    [Fact]
    public void ADaemonOlderThanTheFieldsHasNothingToShow()
    {
        Assert.Empty(SwitchOnNotices.RewordingCards(null, _ => throw new InvalidOperationException()));
        Assert.Null(SwitchOnNotices.Held(null, _ => throw new InvalidOperationException()));

        DaemonStatus? without = JsonSerializer.Deserialize<DaemonStatus>(
            "{\"logged_in\":true}", DaemonProtocol.SerializerOptions);
        Assert.Null(Assert.IsType<DaemonStatus>(without).ArmingRewordings);
        Assert.Null(without!.AutomaticContributionHeld);

        DaemonStatus? with = JsonSerializer.Deserialize<DaemonStatus>(
            "{\"logged_in\":true,\"arming_rewordings\":[" + Rewording + "],"
            + "\"automatic_contribution_held\":" + Held + "}",
            DaemonProtocol.SerializerOptions);
        Assert.Single(Assert.IsType<DaemonStatus>(with).ArmingRewordings!);
        Assert.NotNull(with!.AutomaticContributionHeld);
    }

    [Fact]
    public void AHeldNoticeMissingASentenceOrOutOfPairIsRefused()
    {
        var complete = new Dictionary<string, object?>
        {
            ["title"] = "T",
            ["body"] = "B",
            ["reasons"] = new[] { "R" },
            ["release"] = "R2",
            ["ask_first"] = "A",
            ["projects"] = new[]
            {
                new Dictionary<string, object?>
                {
                    ["project_id"] = "p", ["line"] = "api: 2 sessions waiting",
                    ["ask_first_action"] = "Ask me", ["ask_first_failed"] = "F",
                },
            },
        };
        GateHeldNotice notice = Assert.IsType<GateHeldNotice>(
            SwitchOnNotices.ParseHeld(JsonSerializer.Serialize(complete)));
        Assert.True(Assert.Single(notice.Projects).CanAskFirst);

        foreach (string field in new[] { "title", "body", "release", "ask_first" })
        {
            var partial = new Dictionary<string, object?>(complete) { [field] = string.Empty };
            Assert.Null(SwitchOnNotices.ParseHeld(JsonSerializer.Serialize(partial)));
        }

        var noReasons = new Dictionary<string, object?>(complete) { ["reasons"] = Array.Empty<string>() };
        Assert.Null(SwitchOnNotices.ParseHeld(JsonSerializer.Serialize(noReasons)));
    }

    /// <summary>
    /// Through the real ABI: each notice is the Rust's, with exactly the
    /// fields this shell decodes, and nothing held gets no notice.
    /// </summary>
    [Fact]
    public void TheLiveAbiWordsBothNoticesAndExportsExactlyTheConsumedFields()
    {
        ArmingRewordingCard card = Assert.Single(SwitchOnNotices.RewordingCards(Elements(Rewording)));
        Assert.Contains("api", card.Notice.Title, StringComparison.Ordinal);
        Assert.True(card.CanAskFirst);
        string rewordingJson = NativeMethods.TakeOwnedString(NativeMethods.tc_arming_reworded_notice(Rewording))
            ?? throw new InvalidOperationException("tc_arming_reworded_notice returned NULL");
        Assert.Equal(
            ArmingRewordedNotice.ConsumedFields.OrderBy(k => k),
            JsonDocument.Parse(rewordingJson).RootElement.EnumerateObject().Select(p => p.Name).OrderBy(k => k));

        GateHeldNotice held = Assert.IsType<GateHeldNotice>(
            SwitchOnNotices.Held(JsonDocument.Parse(Held).RootElement.Clone()));
        Assert.StartsWith("2 ", held.Body, StringComparison.Ordinal);
        Assert.Equal("p", Assert.Single(held.Projects).ProjectId);
        string heldJson = NativeMethods.TakeOwnedString(NativeMethods.tc_gate_held_notice(Held))
            ?? throw new InvalidOperationException("tc_gate_held_notice returned NULL");
        Assert.Equal(
            GateHeldNotice.ConsumedFields.OrderBy(k => k),
            JsonDocument.Parse(heldJson).RootElement.EnumerateObject().Select(p => p.Name).OrderBy(k => k));

        Assert.Null(SwitchOnNotices.Held(
            JsonDocument.Parse("{\"held_sessions\":0,\"reasons\":[],\"projects\":[]}").RootElement.Clone()));
    }
}
