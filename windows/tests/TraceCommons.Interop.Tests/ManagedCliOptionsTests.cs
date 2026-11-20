using System.Collections.Generic;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings > Tools' CLI options against the Mac: the drafts
/// (<c>ManagedCLIOptionsDrafts</c>, ManagedSessionsSurface.swift:1117-1137),
/// the rows (<c>ManagedCLIOptionsSection</c>, ManagedCLIOptionsSection.swift:15-19),
/// the save and its confirmation (AppModel.swift:155-161, 193-195, 216-229)
/// and the refusal keys (ManagedSessionsSurface.swift:371-408).
/// </summary>
public class ManagedCliOptionsTests
{
    [Fact]
    public void ADraftKeepsWhatWasTypedUntilThatTextIsSaved()
    {
        var saved = new Dictionary<string, string> { ["claude"] = "--a" };
        var drafts = new ManagedCliOptionsDrafts();
        Assert.Equal("--a", drafts.Value("claude", saved));
        Assert.False(drafts.IsChanged("claude", saved));
        drafts.Edit("claude", "--b");
        Assert.True(drafts.IsChanged("claude", saved));
        drafts.Saved("claude", "--other");
        Assert.Equal("--b", drafts.Value("claude", saved));
        drafts.Saved("claude", "--b");
        Assert.Equal("--a", drafts.Value("claude", saved));
    }

    [Fact]
    public void AToolWithNoSavedValueIsNeverChanged()
    {
        var drafts = new ManagedCliOptionsDrafts();
        drafts.Edit("codex", "--x");
        Assert.False(drafts.IsChanged("codex", new Dictionary<string, string>()));
        Assert.False(drafts.IsChanged("codex", null));
    }

    private sealed class Daemon
    {
        public readonly List<(string Method, string Params)> Sent = new();
        public string Snapshot = """{"result":{"revision":3,"accounts":[],"sessions":[],"generations":{},"capabilities":{},"copy":{},"cli_options":{"opencode":"","claude":"--a","other":"x"}}}""";
        public string Set = """{"result":{"tool":"claude","options":"--b","revision":4}}""";

        public Task<DaemonResponse> Call(string method, string parameters)
        {
            Sent.Add((method, parameters));
            return Task.FromResult(DaemonResponse.Parse(method == "managed_cli_options_set" ? Set : Snapshot));
        }
    }

    [Fact]
    public async Task RowsAreTheSupportedToolsTheDaemonHasOptionsForInTheMacsOrder()
    {
        var flow = new ManagedCliOptionsFlow(new Daemon().Call);
        Assert.Empty(flow.Rows());
        await flow.ReadAsync();
        Assert.Equal(new[] { "claude", "opencode" }, flow.Rows());
    }

    [Fact]
    public async Task AConfirmedSaveSendsToolAndOptionsAndClearsTheDraft()
    {
        var daemon = new Daemon();
        var flow = new ManagedCliOptionsFlow(daemon.Call);
        await flow.ReadAsync();
        Assert.False(flow.CanSave("claude"));
        flow.Drafts.Edit("claude", "--b");
        Assert.True(flow.CanSave("claude"));
        daemon.Sent.Clear();
        await flow.SaveAsync("claude");
        Assert.Equal(("managed_cli_options_set", """{"tool":"claude","options":"--b"}"""), daemon.Sent[0]);
        Assert.Equal("managed_snapshot", daemon.Sent[1].Method);
        Assert.Null(flow.Refusal);
    }

    [Theory]
    [InlineData("""{"result":{"tool":"claude","options":"--b"}}""", "action_failed")]
    [InlineData("""{"result":{"tool":"codex","options":"--b","revision":4}}""", "action_failed")]
    [InlineData("""{"error":{"code":"invalid_params","message":"managed-cli-options-invalid"}}""", "cli_options_invalid")]
    [InlineData("""{"error":{"code":"managed-unsupported-version","message":"x"}}""", "unsupported_version")]
    public async Task AnUnconfirmedOrRefusedSaveKeepsTheDraftAndSaysWhy(string reply, string key)
    {
        var daemon = new Daemon { Set = reply };
        var flow = new ManagedCliOptionsFlow(daemon.Call);
        await flow.ReadAsync();
        flow.Drafts.Edit("claude", "--b");
        await flow.SaveAsync("claude");
        Assert.Equal(("claude", key), flow.Refusal);
        Assert.Equal("--b", flow.Drafts.Value("claude", flow.Snapshot!.CliOptions));
    }

    [Fact]
    public void TheWordsAreTheCores()
    {
        Assert.Equal("CLI options", ManagedWords.Text("cli_options_title"));
        Assert.Equal("Used by new sessions you launch from this app.", ManagedWords.Text("cli_options_scope"));
        Assert.Equal("Save", ManagedWords.Text("cli_options_save"));
    }
}
