using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.RegularExpressions;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The consent surface's sentences, across the C ABI.
///
/// <para>
/// These replaced the three constants this shell used to hold in
/// <c>ReadGate.cs</c>, and the parity test in the GTK crate that opened this
/// file and grepped it for the exact text. Nothing here spells a sentence
/// out: the words are asserted against the payload, and what this shell is
/// held to is that it authors none of them.
/// </para>
/// </summary>
public sealed class ConsentCopyTests
{
    /// <summary>
    /// A payload with every field present decodes, and the sentences arrive
    /// intact.
    /// </summary>
    [Fact]
    public void TheContractShapeParses()
    {
        ConsentCopy copy = Assert.IsType<ConsentCopy>(ConsentSurface.Parse(Json(Complete)));
        Assert.Equal("The statement.", copy.GateStatement);
        Assert.Equal("The armed tooltip.", copy.ReadyHelp);
        Assert.Equal("The disarmed tooltip.", copy.NotPinnedHelp);
        Assert.Equal("The scope.", copy.AutoScrubScope);
        Assert.Equal("The limit.", copy.AutoScrubLimit);
        Assert.Equal("Nobody looks.", copy.AutoNoReview);
    }

    /// <summary>
    /// A complete payload, one value per field. The refusal cases below each
    /// differ from it in exactly one field, so a case that means to test one
    /// empty field is not also refused for a missing one.
    /// </summary>
    private static readonly IReadOnlyDictionary<string, string> Complete = new Dictionary<string, string>
    {
        ["gate_statement"] = "The statement.",
        ["ready_help"] = "The armed tooltip.",
        ["not_pinned_help"] = "The disarmed tooltip.",
        ["auto_scrub_scope"] = "The scope.",
        ["auto_scrub_limit"] = "The limit.",
        ["auto_no_review"] = "Nobody looks.",
    };

    private static string Json(IReadOnlyDictionary<string, string> fields) => JsonSerializer.Serialize(fields);

    /// <summary>One missing and one empty variant of every field.</summary>
    public static IEnumerable<object?[]> OneFieldShort()
    {
        foreach (string field in Complete.Keys)
        {
            yield return new object?[] { Json(Complete.Where(p => p.Key != field).ToDictionary(p => p.Key, p => p.Value)) };
            yield return new object?[] { Json(Complete.ToDictionary(p => p.Key, p => p.Key == field ? string.Empty : p.Value)) };
        }
    }

    /// <summary>
    /// A field the Rust stopped exporting refuses the WHOLE payload.
    ///
    /// Null, never a partly-filled record: a missing sentence above
    /// Contribute is a missing claim, and rendering a blank where a safety
    /// claim goes is worse than rendering nothing.
    /// </summary>
    [Theory]
    [MemberData(nameof(OneFieldShort))]
    [InlineData("not json at all")]
    [InlineData("")]
    [InlineData(null)]
    public void AnIncompletePayloadIsRefusedWhole(string? json)
    {
        Assert.Null(ConsentSurface.Parse(json));
    }

    /// <summary>
    /// The live payload's field set is exactly what this shell decodes.
    ///
    /// <para>
    /// The round-trip test below proves no required field is missing. It
    /// cannot prove the reverse -- a field ADDED in Rust that this shell
    /// silently ignores would be a sentence the other two shells show and
    /// this one does not. This compares the exported inventory against the
    /// declared consumed set, so adding a field in Rust fails here until
    /// somebody decides what this shell does with it.
    /// </para>
    /// </summary>
    [Fact]
    public void TheExportedFieldsAreExactlyTheOnesThisShellConsumes()
    {
        string json = NativeMethods.TakeOwnedString(NativeMethods.tc_consent_copy())
            ?? throw new InvalidOperationException("tc_consent_copy returned NULL");
        using JsonDocument document = JsonDocument.Parse(json);
        var exported = document.RootElement.EnumerateObject()
            .Select(property => property.Name)
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToList();

        Assert.Equal(
            ConsentCopy.ConsumedFields.OrderBy(name => name, StringComparer.Ordinal).ToList(),
            exported);
    }

    /// <summary>The real cdylib hands over a payload this shell can use.</summary>
    [Fact]
    public void TheLivePayloadDecodes()
    {
        ConsentCopy copy = Assert.IsType<ConsentCopy>(ConsentSurface.Copy());
        Assert.All(copy.Sentences, sentence => Assert.False(string.IsNullOrEmpty(sentence)));
    }

    /// <summary>
    /// The branch crosses. This shell asks which sentence, it does not
    /// choose.
    /// </summary>
    [Fact]
    public void TheHelpSentenceComesFromTheAbiForBothAnswers()
    {
        ConsentCopy copy = Assert.IsType<ConsentCopy>(ConsentSurface.Copy());
        Assert.Equal(copy.ReadyHelp, ConsentSurface.GateHelp(true));
        Assert.Equal(copy.NotPinnedHelp, ConsentSurface.GateHelp(false));
    }

    /// <summary>
    /// A build that cannot read the sentences cannot arm Contribute.
    ///
    /// <para>
    /// The gate statement is the whole of what a contributor is told before
    /// pressing an irreversible button. When the payload will not decode
    /// there is no statement to print, and a sheet that printed a blank
    /// where the claim goes and left the button pressable would be
    /// approving a send against a claim nobody made. Fail closed: no copy,
    /// no arming.
    /// </para>
    /// </summary>
    [Fact]
    public void ASurfaceWithNoSentencesCannotBeContributed()
    {
        ConsentCopy copy = Assert.IsType<ConsentCopy>(ConsentSurface.Copy());

        Assert.False(ReadGate.CanArm(null, pinned: true));
        Assert.False(ReadGate.CanArm(null, pinned: false));
        Assert.False(ReadGate.CanArm(copy, pinned: false));
        Assert.True(ReadGate.CanArm(copy, pinned: true));
    }

    /// <summary>
    /// The sheet asks that rule rather than arming on the pin alone.
    ///
    /// <para>
    /// Asserted about the WinUI source because <c>TraceCommons.App</c>
    /// cannot be built on the machines this suite runs on. The shape this
    /// exists to fail on is the one it shipped with: a
    /// <c>?? string.Empty</c> on the statement beside a
    /// <c>CanContribute</c> that never looked at the copy, which renders a
    /// blank safety claim above a pressable button.
    /// </para>
    /// </summary>
    [Fact]
    public void ThePreviewSheetRefusesToArmWithoutTheClaim()
    {
        string path = Path.Combine(
            AppContext.BaseDirectory,
            "shell-source", "TraceCommons.App", "ViewModels", "PreviewSheetViewModel.cs.txt");
        Assert.True(File.Exists(path), $"the view model source was not copied to {path}");

        string uncommented = string.Join(
            "\n",
            File.ReadAllText(path)
                .Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)
                    && !line.TrimStart().StartsWith("///", StringComparison.Ordinal)));

        Assert.Contains(
            "ReadGate.CanArm(_consent, Gate.CanContribute)",
            uncommented,
            StringComparison.Ordinal);

        // The pin alone is not the condition any more. This is the exact
        // expression the blank-claim bug was written as.
        Assert.DoesNotContain(
            "CanContribute => Gate.CanContribute &&",
            uncommented,
            StringComparison.Ordinal);
    }

    /// <summary>
    /// No wording is authored in the consent surface's own sources.
    ///
    /// <para>
    /// The strict rule, the same one <c>RoutingTools.cs</c> is held to:
    /// every string literal in these three files must be a wire value.
    /// Asserted about the source rather than about behaviour because a
    /// hand-written sentence that happened to match the Rust would pass
    /// every behavioural test and then survive a rename in exactly one of
    /// the three shells.
    /// </para>
    /// </summary>
    [Theory]
    [InlineData("ConsentCopy.cs.txt")]
    [InlineData("ConsentSurface.cs.txt")]
    [InlineData("ReadGate.cs.txt")]
    public void NoWordingIsAuthoredInTheConsentSurface(string copied)
    {
        string path = Path.Combine(AppContext.BaseDirectory, copied);
        Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");

        // Strip doc comments and line comments: prose about the claim quotes
        // the claim, and nothing in a comment is ever rendered.
        string uncommented = string.Join(
            "\n",
            File.ReadAllText(path)
                .Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));

        var allowed = new HashSet<string>(StringComparer.Ordinal)
        {
            // The payload's wire keys, and nothing else.
            "gate_statement", "ready_help", "not_pinned_help",
            "auto_scrub_scope", "auto_scrub_limit", "auto_no_review",
        };

        foreach (Match match in Regex.Matches(uncommented, "\"([^\"\\\\]|\\\\.)*\""))
        {
            string literal = match.Value[1..^1];
            Assert.True(
                allowed.Contains(literal),
                $"\"{literal}\" is a string literal in {copied} that is not a wire value. "
                + "Wording on this surface comes from consent_copy.rs across the ABI.");
        }
    }
}
