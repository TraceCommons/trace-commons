using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json.Nodes;
using System.Threading.Tasks;
using TraceCommons.Interop.FirstRun;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Train coding models on traces already sent, offered at the first run's
/// site (W20 round 2): the first run's Start writes the consent scopes, and a
/// write the daemon confirmed that turned Train coding models on raises the
/// sheet on the main window, which the first run hands off to, not over the
/// Start still running (<c>saveConsentScopes</c>, AppModel.swift:5049-5058).
/// </summary>
public sealed class FirstRunTrainingOfferTests
{
    private static string Recorded(string name) =>
        File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "first-run", name + ".json"));

    /// <summary>The recorded status (normalDay), its consent scopes as given.</summary>
    private static string Status(params string[] scopes)
    {
        JsonNode frame = new JsonObject
        {
            ["id"] = 1,
            ["result"] = JsonNode.Parse(File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "RecordedSamples", "normalDay", "status.json"))),
        };
        var list = new JsonArray();
        foreach (string scope in scopes) list.Add(scope);
        frame["result"]!["consent_scopes"] = list;
        return frame.ToJsonString();
    }

    private sealed class Wire
    {
        public string StatusReply = Status("debugging_evaluation");
        public string ConsentReply = Recorded("set_consent_scopes");
        public readonly List<string> Sent = new();

        public Task<DaemonResponse> Call(string method, string json)
        {
            Sent.Add(method);
            return Task.FromResult(DaemonResponse.Parse(method switch
            {
                "status" => StatusReply,
                "set_consent_scopes" => ConsentReply,
                _ => throw new NotSupportedException("no recorded reply for " + method),
            }));
        }
    }

    private static (FirstRunDaemon Daemon, Wire Wire, List<(IReadOnlyList<string> Before, IReadOnlyList<string> Confirmed)> Reported) Make()
    {
        var wire = new Wire();
        var daemon = new FirstRunDaemon(wire.Call, _ => Task.FromResult(true), () => true,
            () => Task.FromResult(true), () => Task.FromResult(true), _ => { }, _ => Task.FromResult(true));
        var reported = new List<(IReadOnlyList<string>, IReadOnlyList<string>)>();
        daemon.ConsentScopesConfirmed += (before, confirmed) => reported.Add((before, confirmed));
        return (daemon, wire, reported);
    }

    [Fact]
    public async Task AConfirmedWriteReportsTheScopesBeforeAndAfterIt()
    {
        var (daemon, wire, reported) = Make();

        Assert.True(await daemon.SaveConsentScopes(new[] { "debugging_evaluation", "model_training" }));

        var (before, confirmed) = Assert.Single(reported);
        Assert.Equal(new[] { "debugging_evaluation" }, before);
        Assert.Equal(new[] { "debugging_evaluation", "model_training" }, confirmed);
        Assert.True(SentTracesTrainingState.TurnsOn(before, confirmed));
        // The scopes are read before the write, as the Mac's status already holds them.
        Assert.Equal(new[] { "status", "set_consent_scopes" }, wire.Sent);
    }

    [Fact]
    public async Task ARefusedWriteReportsNothing()
    {
        var (daemon, wire, reported) = Make();
        wire.ConsentReply = "{\"id\":1,\"error\":{\"code\":\"refused\",\"message\":\"not-logged-in\"}}";

        Assert.False(await daemon.SaveConsentScopes(new[] { "model_training" }));
        Assert.Empty(reported);
    }

    [Fact]
    public async Task AReplyWithoutScopesOrAnUnreadStatusReportsNothing()
    {
        var (daemon, wire, reported) = Make();
        wire.ConsentReply = "{\"id\":1,\"result\":{}}";
        Assert.True(await daemon.SaveConsentScopes(new[] { "model_training" }));
        Assert.Empty(reported);

        (daemon, wire, reported) = Make();
        wire.StatusReply = "{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"daemon-unavailable\"}}";
        Assert.True(await daemon.SaveConsentScopes(new[] { "model_training" }));
        Assert.Empty(reported);
    }

    [Fact]
    public async Task AWriteThatLeavesTrainingAsItWasOffersNothing()
    {
        var (daemon, wire, reported) = Make();
        wire.StatusReply = Status("debugging_evaluation", "model_training");

        Assert.True(await daemon.SaveConsentScopes(new[] { "debugging_evaluation", "model_training" }));

        var (before, confirmed) = Assert.Single(reported);
        Assert.False(SentTracesTrainingState.TurnsOn(before, confirmed));
    }

    private static string Shell(string relative) => File.ReadAllText(
        Path.Combine(AppContext.BaseDirectory, "shell-source", relative + ".txt")).Replace("\r\n", "\n", StringComparison.Ordinal);

    /// <summary>
    /// The window raises the offer at the Monitor site after a turn-on, and
    /// shows it on the main window once the first run has handed off.
    /// </summary>
    [Fact]
    public void TheMainWindowOffersItAfterTheFirstRunHandsOff()
    {
        string firstRun = Shell("TraceCommons.App/FirstRunWindow.xaml.cs");
        Assert.Contains("daemon.ConsentScopesConfirmed += (before, confirmed) => ConsentScopesConfirmed?.Invoke(before, confirmed);", firstRun);
        string main = Shell("TraceCommons.App/MainWindow.xaml.cs");
        Assert.Contains("onboarding.ConsentScopesConfirmed += OnFirstRunConsentScopesConfirmed;", main);
        Assert.Contains("ShowFirstRunTrainingOffer();", main);
        string training = Shell("TraceCommons.App/MainWindow.SentTracesTraining.cs");
        Assert.Contains("if (!SentTracesTrainingState.TurnsOn(before, confirmed))", training);
        Assert.Contains("await SentTraining.OfferAfterTurnOnAsync(SentTracesTrainingSite.Monitor);", training);
        Assert.Contains("_ = ShowSentTrainingSheetAsync(root, SentTracesTrainingSite.Monitor);", training);
        Assert.Contains("if (_onboarding is null)", training);
        // A window hidden to the tray keeps the offer until it is activated.
        Assert.Contains("if (!AppWindow.IsVisible || OpenRoot is not XamlRoot root)", training);
        Assert.Contains("Activated += OnActivatedWithOffer;", training);
    }
}
