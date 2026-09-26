using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// R6's void notice: the element off <c>status.grant_voids</c>, handed back
/// to the ABI verbatim, and the notice it returns.
/// </summary>
public sealed class GrantVoidNoticeTests
{
    private const string ProjectVoid =
        "{\"id\":4,\"kind\":\"project\",\"project_id\":\"p\",\"project_label\":\"api\","
        + "\"reasons\":[\"witness-changed\"],\"voided_at\":\"2026-09-26T12:00:00Z\"}";

    private static readonly IReadOnlyDictionary<string, object?> Complete = new Dictionary<string, object?>
    {
        ["title"] = "T",
        ["body"] = "B",
        ["reasons_heading"] = "H",
        ["reasons"] = new[] { "R" },
        ["rearm"] = "A",
        ["acknowledge"] = "K",
        ["rearm_action"] = "Turn back on",
        ["rearm_failed"] = "F",
    };

    private static Dictionary<string, object?> NoButton() => new(Complete)
    {
        ["rearm_action"] = null,
        ["rearm_failed"] = null,
    };

    private static List<JsonElement> Elements(params string[] json) =>
        json.Select(j => JsonDocument.Parse(j).RootElement.Clone()).ToList();

    /// <summary>
    /// The element goes to the ABI exactly as the daemon sent it, and the
    /// card carries its id for the acknowledgement.
    /// </summary>
    [Fact]
    public void EachElementGoesToTheAbiVerbatimAndKeepsItsId()
    {
        var seen = new List<string>();
        IReadOnlyList<GrantVoidCard> cards = GrantVoidNotices.Cards(
            Elements(ProjectVoid),
            wire =>
            {
                seen.Add(wire);
                return JsonSerializer.Serialize(Complete);
            });

        GrantVoidCard card = Assert.Single(cards);
        Assert.Equal(4UL, card.Id);
        Assert.True(card.CanAcknowledge);
        Assert.Equal("T", card.Notice.Title);
        Assert.Equal(new[] { "R" }, card.Notice.Reasons);
        Assert.Equal(
            JsonDocument.Parse(ProjectVoid).RootElement.ToString(),
            JsonDocument.Parse(Assert.Single(seen)).RootElement.ToString());
    }

    /// <summary>
    /// "Turn back on" is offered only when the core offered it, and it arms
    /// the element's own project through the Settings call, unchanged.
    /// </summary>
    [Fact]
    public void TheRearmButtonArmsTheElementsProjectOnlyWhenOffered()
    {
        GrantVoidCard offered = Assert.Single(GrantVoidNotices.Cards(
            Elements(ProjectVoid), _ => JsonSerializer.Serialize(Complete)));
        Assert.True(offered.CanRearm);
        Assert.Equal("p", offered.RearmProjectId);
        Assert.Equal("Turn back on", offered.Notice.RearmAction);
        using JsonDocument request = JsonDocument.Parse(
            GrantVoidNotices.RearmParams(offered) ?? throw new InvalidOperationException());
        Assert.Equal("p", request.RootElement.GetProperty("project_id").GetString());
        Assert.Equal("auto_upload", request.RootElement.GetProperty("mode").GetString());

        GrantVoidCard withheld = Assert.Single(GrantVoidNotices.Cards(
            Elements(ProjectVoid), _ => JsonSerializer.Serialize(NoButton())));
        Assert.False(withheld.CanRearm);
        Assert.Null(GrantVoidNotices.RearmParams(withheld));
    }

    /// <summary>The button and its refusal line come as a pair.</summary>
    [Fact]
    public void TheRearmButtonAndItsRefusalLineComeAsAPair()
    {
        Assert.NotNull(GrantVoidNotices.Parse(JsonSerializer.Serialize(NoButton())));
        foreach ((object? action, object? failed) in new (object?, object?)[]
                 {
                     ("a", null), (null, "f"), ("", "f"),
                 })
        {
            var odd = new Dictionary<string, object?>(Complete)
            {
                ["rearm_action"] = action,
                ["rearm_failed"] = failed,
            };
            Assert.Null(GrantVoidNotices.Parse(JsonSerializer.Serialize(odd)));
        }
    }

    [Fact]
    public void ADaemonOlderThanTheFieldHasNoCards()
    {
        Assert.Empty(GrantVoidNotices.Cards(null, _ => throw new InvalidOperationException()));
    }

    /// <summary>
    /// Shown whole or not at all: a notice missing any sentence, or with no
    /// reason, is refused rather than drawn in part.
    /// </summary>
    [Fact]
    public void ANoticeMissingASentenceIsRefused()
    {
        Assert.NotNull(GrantVoidNotices.Parse(JsonSerializer.Serialize(Complete)));
        foreach (string field in GrantVoidNotice.ConsumedFields
                     .Where(f => f != "rearm_action" && f != "rearm_failed"))
        {
            var partial = new Dictionary<string, object?>(Complete)
            {
                [field] = field == "reasons" ? Array.Empty<string>() : string.Empty,
            };
            Assert.Null(GrantVoidNotices.Parse(JsonSerializer.Serialize(partial)));
        }

        Assert.Null(GrantVoidNotices.Parse(null));
        Assert.Null(GrantVoidNotices.Parse("not json"));
    }

    /// <summary>
    /// A status with the field decodes each element; one without it reads
    /// as none.
    /// </summary>
    [Fact]
    public void StatusCarriesGrantVoidsWhenTheDaemonSendsThem()
    {
        DaemonStatus? with = JsonSerializer.Deserialize<DaemonStatus>(
            "{\"logged_in\":true,\"grant_voids\":[" + ProjectVoid + "]}",
            DaemonProtocol.SerializerOptions);
        Assert.Single(Assert.IsType<DaemonStatus>(with).GrantVoids!);

        DaemonStatus? without = JsonSerializer.Deserialize<DaemonStatus>(
            "{\"logged_in\":true}", DaemonProtocol.SerializerOptions);
        Assert.Null(Assert.IsType<DaemonStatus>(without).GrantVoids);
    }

    /// <summary>
    /// Through the real ABI: a project void and the grant's void each get
    /// the Rust's words, different from each other, and the exported field
    /// set is exactly what this shell decodes.
    /// </summary>
    [Fact]
    public void TheLiveAbiWordsBothKindsAndExportsExactlyTheConsumedFields()
    {
        IReadOnlyList<GrantVoidCard> cards = GrantVoidNotices.Cards(Elements(
            ProjectVoid,
            "{\"id\":5,\"kind\":\"automatic_grant\",\"reasons\":[\"scopes-widened\"]}"));
        Assert.Equal(2, cards.Count);
        Assert.Contains("api", cards[0].Notice.Title, StringComparison.Ordinal);
        Assert.NotEqual(cards[0].Notice.Title, cards[1].Notice.Title);
        // Only the project's notice offers "Turn back on".
        Assert.True(cards[0].CanRearm);
        Assert.False(cards[1].CanRearm);

        string json = NativeMethods.TakeOwnedString(NativeMethods.tc_grant_void_notice(ProjectVoid))
            ?? throw new InvalidOperationException("tc_grant_void_notice returned NULL");
        var keys = JsonDocument.Parse(json).RootElement.EnumerateObject().Select(p => p.Name).OrderBy(k => k);
        Assert.Equal(GrantVoidNotice.ConsumedFields.OrderBy(k => k), keys);
    }

    /// <summary>
    /// An element the ABI cannot place is still worded by the ABI, so this
    /// shell never needs a fallback sentence of its own.
    /// </summary>
    [Fact]
    public void AnElementTheAbiCannotPlaceIsStillWordedByTheAbi()
    {
        GrantVoidCard card = Assert.Single(GrantVoidNotices.Cards(Elements("{\"id\":9,\"kind\":\"folder\"}")));
        Assert.Equal(9UL, card.Id);
        Assert.False(string.IsNullOrEmpty(card.Notice.Title));
        Assert.False(card.CanRearm, "no project to arm");
    }
}
