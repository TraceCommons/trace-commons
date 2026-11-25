using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

// MainViewModel depends on WinUI. These checks cover its wiring alongside
// ApprovalHoldTests, which exercise the daemon reply shapes.
public sealed class SubmitUndoWiringTests
{
    [Theory]
    [InlineData("SubmitEntryAsync")]
    [InlineData("SubmitProjectAsync")]
    public void OneClickSubmitLeavesUndoInPlaceUntilTheReplyIsApplied(string method)
    {
        string body = MethodBody(method);

        Assert.DoesNotContain("ClearUndo()", body, StringComparison.Ordinal);
        Assert.Matches(@"(?s)CallAsync\(.*?ApplySubmitOutcome\(response,", body);
    }

    [Fact]
    public void ARefusedUnreadableOrZeroApprovalCannotReplaceUndo()
    {
        string body = MethodBody("ApplySubmitOutcome");

        Assert.Matches(
            @"(?s)if \(hold is null\)\s*\{[^{}]*?return;\s*\}"
            + @".*?if \(hold.Approved == 0\)\s*\{\s*return;\s*\}"
            + @".*?if \(hold.IsLive\([^)]*\)\)\s*\{[^{}]*?_undoHold = hold;[^{}]*?\}"
            + @"\s*else\s*\{\s*ClearUndo\(\);\s*\}",
            body);
        Assert.Single(Regex.Matches(body, @"_undoHold\s*="));
        Assert.Single(Regex.Matches(body, @"ClearUndo\(\)"));
    }

    [Fact]
    public void PreviewReplacesUndoOnlyWhenAnApprovalWasConfirmed()
    {
        string body = MethodBody("OnDecidedAsync");

        Assert.Matches(
            @"(?s)case PreviewOutcome.Approved when .*?_undoHold = hold;.*?break;"
            + @"\s*case PreviewOutcome.Approved:.*?ClearUndo\(\);.*?break;",
            body);
        Assert.Single(Regex.Matches(body, @"_undoHold\s*="));
        Assert.Single(Regex.Matches(body, @"ClearUndo\(\)"));
    }

    [Fact]
    public void ClosingUndoLeavesAnExistingActionNoticeAvailable()
    {
        string body = MethodBody("DismissUndo");

        Assert.Matches(@"ClearUndo\(\);\s*if \(!HasNotice\)\s*\{\s*Notice = ApprovedNoUndo;\s*\}", body);
    }

    private static string MethodBody(string method)
    {
        string source = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "MainViewModel.cs.txt"));
        Match match = Regex.Match(
            source,
            $@"^    (?:public|private) [^\r\n]*\b{method}\([^\r\n]*\)\r?\n    \{{(?<body>.*?)^    \}}",
            RegexOptions.Multiline | RegexOptions.Singleline);
        Assert.True(match.Success, $"Could not find {method} in MainViewModel.");
        return match.Groups["body"].Value;
    }
}
