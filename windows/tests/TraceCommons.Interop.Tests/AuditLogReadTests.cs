using System;
using System.Collections.Generic;
using System.IO;
using System.Threading.Tasks;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The change log's read (<c>ChangesSection</c>, ChangesSection.swift:11-15;
/// <c>SettingsRead</c>, SettingsStateRow.swift:36-44): until <c>list_audit</c>
/// answers, the read notice, never "No changes yet.", because the empty log
/// would be a count nothing reported. Once answered, a later failure keeps
/// what it said ("answered at least once; what it said is real, even if
/// stale").
/// </summary>
public class AuditLogReadTests
{
    private const string TwoEntries =
        "{\"id\":1,\"result\":{\"entries\":[{\"at\":\"2026-11-19T10:00:00Z\",\"action\":\"armed-auto-upload\",\"project_label\":\"repo-a\"},"
        + "{\"at\":\"2026-11-18T10:00:00Z\",\"action\":\"consent-scopes-changed\"}]}}";

    private const string NoEntries = "{\"id\":1,\"result\":{\"entries\":[]}}";

    private const string Refused = "{\"id\":1,\"error\":{\"code\":-32000,\"message\":\"down\"}}";

    private static (AuditLogRead Read, List<string> Asked) With(params string[] replies)
    {
        var asked = new List<string>();
        int next = 0;
        var read = new AuditLogRead((method, json) =>
        {
            asked.Add(method + " " + json);
            string reply = replies[Math.Min(next++, replies.Length - 1)];
            if (reply == "throw")
            {
                throw new InvalidOperationException("no daemon");
            }

            return Task.FromResult(DaemonResponse.Parse(reply));
        });
        return (read, asked);
    }

    [Fact]
    public void BeforeAnyReadItIsAwaitingAndClaimsNothing()
    {
        (AuditLogRead read, _) = With(NoEntries);
        Assert.Equal(AuditLogState.Awaiting, read.State);
        Assert.True(read.ShowsReadNotice);
        Assert.False(read.ShowsNothingChanged);
    }

    [Theory]
    [InlineData(Refused)]
    [InlineData("throw")]
    public async Task AFailedFirstReadShowsTheNoticeNotNoChanges(string reply)
    {
        (AuditLogRead read, _) = With(reply);
        await read.ReadAsync();
        Assert.Equal(AuditLogState.Failed, read.State);
        Assert.True(read.ShowsReadNotice);
        Assert.False(read.ShowsNothingChanged);
        Assert.Empty(read.Entries);
    }

    [Fact]
    public async Task AnAnsweredEmptyLogSaysNoChanges()
    {
        (AuditLogRead read, List<string> asked) = With(NoEntries);
        await read.ReadAsync();
        Assert.Equal(new[] { "list_audit {\"limit\":20}" }, asked);
        Assert.Equal(AuditLogState.Answered, read.State);
        Assert.False(read.ShowsReadNotice);
        Assert.True(read.ShowsNothingChanged);
    }

    [Fact]
    public async Task ALaterFailureKeepsWhatTheLastAnswerSaid()
    {
        (AuditLogRead read, _) = With(TwoEntries, Refused);
        await read.ReadAsync();
        Assert.Equal(2, read.Entries.Count);
        await read.ReadAsync();
        Assert.Equal(AuditLogState.Answered, read.State);
        Assert.Equal(2, read.Entries.Count);
        Assert.Equal("armed-auto-upload", read.Entries[0].Action);
        Assert.False(read.ShowsNothingChanged);
    }

    [Fact]
    public async Task AReplyOvertakenByANewerReadIsDropped()
    {
        var first = new TaskCompletionSource<DaemonResponse>();
        int calls = 0;
        var read = new AuditLogRead((_, _) =>
            ++calls == 1 ? first.Task : Task.FromResult(DaemonResponse.Parse(NoEntries)));
        Task older = read.ReadAsync();
        await read.ReadAsync();
        first.SetResult(DaemonResponse.Parse(TwoEntries));
        await older;
        Assert.Empty(read.Entries);
        Assert.True(read.ShowsNothingChanged);
    }

    /// <summary>
    /// The window draws from this read: the empty line is bound to the
    /// answered-and-empty condition, and the notice is drawn from the read's
    /// state (SettingsView.xaml.cs), not from the entry count.
    /// </summary>
    [Fact]
    public void TheChangeLogIsDrawnFromTheRead()
    {
        string viewModel = ShellSource("TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs");
        Assert.Contains("public bool HasNoAuditEntries => Audit.ShowsNothingChanged;", viewModel);
        Assert.Contains("await Audit.ReadAsync()", viewModel);
        Assert.DoesNotContain("ListAudit", viewModel);

        string view = ShellSource("TraceCommons.App/Controls/SettingsView.xaml.cs");
        Assert.Contains("Settings.AuditReadChanged += DrawAuditNotice;", view);
        Assert.Contains("AuditNoticeHost.Content = audit.ShowsReadNotice", view);

        string xaml = ShellSource("TraceCommons.App/Controls/SettingsView.xaml");
        Assert.Contains("Visibility=\"{x:Bind Settings.HasNoAuditEntries, Mode=OneWay}\"", xaml);
        Assert.Contains("x:Name=\"AuditNoticeHost\"", xaml);
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
