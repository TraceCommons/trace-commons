using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;
using TraceCommons.Interop;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The whole <c>tc_first_run_copy_json</c> table, read from the core over the
/// C ABI (crates/trace-commons-contributor/src/first_run_copy.rs:546), as the
/// Mac's <c>FirstRunCopy.decode</c> reads it (FirstRunCopy.swift:325-327).
/// </summary>
public sealed class FirstRunCopyTests
{
    private static string Native() => NativeMethods.TakeOwnedString(NativeMethods.tc_first_run_copy_json())!;

    [Fact]
    public void TheCoresTableDecodesEveryField()
    {
        FirstRunCopy copy = Assert.IsType<FirstRunCopy>(FirstRunCopy.Decode(Native()));
        // Words the first run shows, read through the decoder, each the
        // core's own (first_run_copy.rs).
        JsonNode table = JsonNode.Parse(Native())!;
        Assert.Equal((string)table["welcome"]!["title"]!, copy.Welcome.Title);
        Assert.Equal((string)table["frame"]!["continue_button"]!, copy.Frame.ContinueButton);
        Assert.Equal((string)table["uses"]!["this_computer"]!, copy.Uses.ThisComputer);
        Assert.Equal((string)table["traces"]!["on_this_computer"]!, copy.Traces.OnThisComputer);
        Assert.Equal((string)table["folders"]!["loading_this_computer"]!, copy.Folders.LoadingThisComputer);
        Assert.Equal(7, copy.Rules.Weekdays.Count);
        Assert.Equal(12, copy.Rules.Months.Count);
        Assert.Equal((string)table["welcome"]!["legal"]!["accept_button"]!, copy.Welcome.Legal.AcceptButton);
        // W7-D2 (a): the "this computer" words never say Mac.
        Assert.DoesNotContain("Mac", copy.Uses.ThisComputer);
        Assert.DoesNotContain("Mac", copy.Traces.OnThisComputer);
        Assert.DoesNotContain("Mac", copy.Folders.LoadingThisComputer);
        // The thread names the six steps after Welcome, in the Mac's order.
        Assert.Equal(new[] { copy.Frame.StepTools, copy.Frame.StepPrivateAi, copy.Frame.StepTraces, copy.Frame.StepUses, copy.Frame.StepTerms, copy.Frame.StepJoin }, copy.ThreadLabels);
    }

    [Fact]
    public void ABlankWordOrAnUnknownFieldRefusesTheTable()
    {
        JsonNode blank = JsonNode.Parse(Native())!;
        blank["join"]!["skip"] = "";
        Assert.Null(FirstRunCopy.Decode(blank.ToJsonString()));
        JsonNode month = JsonNode.Parse(Native())!;
        month["rules"]!["months"]![3] = "";
        Assert.Null(FirstRunCopy.Decode(month.ToJsonString()));
        JsonNode extra = JsonNode.Parse(Native())!;
        extra["frame"]!["a_word_this_build_does_not_know"] = "x";
        Assert.Null(FirstRunCopy.Decode(extra.ToJsonString()));
        JsonNode missing = JsonNode.Parse(Native())!;
        missing["uses"]!.AsObject().Remove("this_computer");
        Assert.Null(FirstRunCopy.Decode(missing.ToJsonString()));
        Assert.Null(FirstRunCopy.Decode(null));
        Assert.Null(FirstRunCopy.Decode("not json"));
    }

    [Fact]
    public void FillReplacesEachKey()
    {
        Assert.Equal("of 3 tools", FirstRunCopy.Fill("of {total} tools", new System.Collections.Generic.Dictionary<string, string> { ["total"] = "3" }));
    }
}
