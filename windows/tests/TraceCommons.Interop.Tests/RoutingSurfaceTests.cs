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
/// The routing surface this shell renders: one word per tool, the
/// declaration it writes, what the probe answered, and what the daemon is
/// seeing.
///
/// Every word compared against here comes from <see cref="RoutingSurface.Copy"/>
/// -- that is, across the real ABI from the Rust -- and never from a literal
/// in this file. <see cref="RoutingCopyTests"/> owns the one deliberate
/// literal pin on the vocabulary; if a word is renamed in the Rust, that test
/// goes red and every assertion here follows the new word.
/// </summary>
public class RoutingSurfaceTests
{
    private static RoutingCopy Copy()
    {
        RoutingCopy? copy = RoutingSurface.Copy();
        Assert.NotNull(copy);
        return copy!;
    }

    private static RoutingEvidence Reachable(string json) =>
        RoutingEvidence.Parse(json);

    /// <summary>Every tool in use, nothing declared about any of them.</summary>
    private static RoutingModes AllWatched() =>
        new() { Claude = "watch", Codex = "watch", Gemini = "watch" };

    // --- One tool, one word ---------------------------------------------

    /// <summary>
    /// The three states IronWire's answer can put a tool in, each mapping to
    /// exactly one shared word.
    /// </summary>
    [Fact]
    public void EachToolReadsExactlyOneOfTheFourSharedWords()
    {
        RoutingCopy copy = Copy();
        Assert.Equal(copy.WordPrivate, RoutingTools.ToolWord(copy, "watch", ToolWiring.Wired));
        Assert.Equal(copy.WordDirect, RoutingTools.ToolWord(copy, "watch", ToolWiring.NotWired));
        Assert.Equal(copy.WordUnknown, RoutingTools.ToolWord(copy, "watch", ToolWiring.Unknown));

        // "unset" watches the conventional location, which is a tool in use.
        Assert.Equal(copy.WordPrivate, RoutingTools.ToolWord(copy, "unset", ToolWiring.Wired));
        // A mode this build has never heard of is still a tool in use, and
        // still gets no verdict without evidence.
        Assert.Equal(copy.WordUnknown, RoutingTools.ToolWord(copy, "", ToolWiring.Unknown));
    }

    /// <summary>
    /// A tool the contributor said they do not use reads "Not used" whatever
    /// IronWire says: nothing of theirs is being read either way.
    /// </summary>
    [Fact]
    public void ANotUsedToolReadsNotUsedWhateverIronWireSaid()
    {
        RoutingCopy copy = Copy();
        foreach (ToolWiring wiring in new[] { ToolWiring.Wired, ToolWiring.NotWired, ToolWiring.Unknown })
        {
            Assert.Equal(copy.WordNotUsed, RoutingTools.ToolWord(copy, "off", wiring));
        }
    }

    /// <summary>
    /// The case the old single-switch word got confidently wrong, and the one
    /// this surface exists for.
    ///
    /// IronWire has no <c>gemini</c> row upstream at all -- neither built in
    /// nor in its catalogue -- so Gemini CLI reads "Not known" on a machine
    /// where it is installed and in daily use, while the two tools IronWire
    /// does list get real verdicts from the same answer.
    /// </summary>
    [Fact]
    public void GeminiIsUnknownEvenOnAMachineWhereItIsInstalledAndInUse()
    {
        RoutingCopy copy = Copy();
        RoutingEvidence evidence = Reachable(
            """
            {"outcome":"reachable","tools":[
              {"id":"claude","installed":true,"wired":true},
              {"id":"codex","installed":true,"wired":false}
            ]}
            """);

        IReadOnlyList<RoutingToolRow> rows = RoutingTools.Rows(copy, AllWatched(), evidence);

        Assert.Equal(3, rows.Count);
        Assert.Equal(copy.ToolClaude, rows[0].Name);
        Assert.Equal(copy.WordPrivate, rows[0].Word);
        Assert.Equal(copy.ToolCodex, rows[1].Name);
        Assert.Equal(copy.WordDirect, rows[1].Word);
        Assert.Equal(copy.ToolGemini, rows[2].Name);
        Assert.Equal(copy.WordUnknown, rows[2].Word);
    }

    /// <summary>
    /// A row is read as one statement, not as a name and a stray word beside
    /// it.
    /// </summary>
    [Fact]
    public void ARowIsAnnouncedAsOneStatement()
    {
        RoutingCopy copy = Copy();
        RoutingToolRow row = RoutingTools.Rows(copy, AllWatched(), null)[0];
        Assert.Equal($"{copy.ToolClaude}: {copy.WordUnknown}", row.AccessibleLabel);
    }

    /// <summary>
    /// The word never claims privacy from a stale answer. On anything but
    /// <c>reachable</c> every tool reads "Not known", even when the payload
    /// carried rows saying otherwise.
    /// </summary>
    [Fact]
    public void NothingUsableAnsweredMeansNoVerdictForAnyTool()
    {
        RoutingCopy copy = Copy();
        foreach (string payload in new[]
        {
            """{"outcome":"unreachable","port":8463,"tools":[{"id":"claude","installed":true,"wired":true}]}""",
            """{"outcome":"token_unreadable","token_path":"C:\\Users\\x\\.ironwire\\control.token","tools":[{"id":"claude","installed":true,"wired":true}]}""",
            """{"outcome":"something-this-build-has-never-heard-of","tools":[{"id":"claude","installed":true,"wired":true}]}""",
        })
        {
            RoutingEvidence evidence = Reachable(payload);
            Assert.Equal(ToolWiring.Unknown, evidence.WiringFor(RoutingTools.ClaudeId));
            Assert.Equal(
                copy.WordUnknown,
                RoutingTools.Rows(copy, AllWatched(), evidence)[0].Word);
        }
    }

    /// <summary>
    /// The last line, reached directly.
    ///
    /// <see cref="RoutingEvidence.Parse"/> keeps no rows unless the outcome is
    /// reachable, so this shape cannot arrive off the wire today -- which is
    /// exactly why the guard in <c>WiringFor</c> needs a test that can build
    /// it. Without this, deleting that guard changes no test result, and the
    /// surface's protection against a stale verdict rests on one function
    /// instead of two.
    /// </summary>
    [Fact]
    public void RowsHeldBesideAnOutcomeThatIsNotReachableStillGetNoVerdict()
    {
        var rows = new Dictionary<string, RoutedTool>(StringComparer.Ordinal)
        {
            [RoutingTools.ClaudeId] = new RoutedTool(Installed: true, Wired: true),
        };

        foreach (RoutingProbeKind kind in new[]
                 { RoutingProbeKind.Unreachable, RoutingProbeKind.TokenUnreadable, RoutingProbeKind.Unknown })
        {
            var evidence = new RoutingEvidence(new RoutingProbe(kind, null, null), rows);
            Assert.Equal(ToolWiring.Unknown, evidence.WiringFor(RoutingTools.ClaudeId));
            Assert.Equal(
                Copy().WordUnknown,
                RoutingTools.Rows(Copy(), AllWatched(), evidence)[0].Word);
        }
    }

    /// <summary>
    /// No answer held at all is the same amount of evidence about every tool:
    /// none.
    /// </summary>
    [Fact]
    public void NoEvidenceHeldMeansNoVerdict()
    {
        RoutingCopy copy = Copy();
        foreach (RoutingToolRow row in RoutingTools.Rows(copy, AllWatched(), null))
        {
            Assert.Equal(copy.WordUnknown, row.Word);
        }
    }

    /// <summary>
    /// IronWire saying a tool is not present, while this app is watching that
    /// tool's sessions, is two detectors disagreeing about one machine. That
    /// is not evidence for a verdict.
    /// </summary>
    [Fact]
    public void AToolIronWireSaysIsAbsentGetsNoVerdict()
    {
        RoutingEvidence evidence = Reachable(
            """{"outcome":"reachable","tools":[{"id":"claude","installed":false,"wired":false}]}""");
        Assert.Equal(ToolWiring.Unknown, evidence.WiringFor(RoutingTools.ClaudeId));
    }

    /// <summary>
    /// An answer that arrives but lists nothing -- a body over the daemon's
    /// size bound, or one it cannot parse -- is <c>reachable</c> with no
    /// rows. The proxy answered, and that is no evidence about any tool.
    /// </summary>
    [Fact]
    public void AReachableAnswerWithNoRowsIsStillNoVerdict()
    {
        RoutingEvidence evidence = Reachable("""{"outcome":"reachable","tools":[]}""");
        Assert.Equal(RoutingProbeKind.Reachable, evidence.Outcome.Kind);
        foreach (string id in new[] { RoutingTools.ClaudeId, RoutingTools.CodexId, RoutingTools.GeminiId })
        {
            Assert.Equal(ToolWiring.Unknown, evidence.WiringFor(id));
        }
    }

    /// <summary>
    /// The declaration is not an input to any word. Declaring IronWire in
    /// this app has no causal relation to whether a tool is configured to
    /// send through it, and reading the switch is exactly what let a
    /// contributor see the wired word on the same card as "Nothing answered
    /// on port 8463".
    ///
    /// Structural, because the failure was a *reachable* input rather than a
    /// wrong branch: <see cref="RoutingTools.Rows"/> takes the modes and the
    /// evidence, and there is no overload that takes a declaration.
    /// </summary>
    [Fact]
    public void TheDeclarationSwitchIsNotAnInputToAnyWord()
    {
        var parameters = typeof(RoutingTools)
            .GetMethod(nameof(RoutingTools.Rows))!
            .GetParameters()
            .Select(p => p.ParameterType)
            .ToArray();
        Assert.Equal(
            new[] { typeof(RoutingCopy), typeof(RoutingModes), typeof(RoutingEvidence) },
            parameters);

        var wordParameters = typeof(RoutingTools)
            .GetMethod(nameof(RoutingTools.ToolWord))!
            .GetParameters()
            .Select(p => p.ParameterType)
            .ToArray();
        Assert.Equal(
            new[] { typeof(RoutingCopy), typeof(string), typeof(ToolWiring) },
            wordParameters);
    }

    // --- What the probe answered -----------------------------------------

    [Fact]
    public void TheThreeProbeOutcomesAreReadWithTheDaemonsOwnVocabulary()
    {
        Assert.Equal(
            RoutingProbeKind.Reachable,
            RoutingProbe.Parse("""{"outcome":"reachable","tools":[]}""").Kind);

        RoutingProbe unreachable = RoutingProbe.Parse("""{"outcome":"unreachable","port":8463}""");
        Assert.Equal(RoutingProbeKind.Unreachable, unreachable.Kind);
        Assert.Equal((ushort)8463, unreachable.Port);

        RoutingProbe token = RoutingProbe.Parse(
            """{"outcome":"token_unreadable","token_path":"C:\\Users\\x\\.ironwire\\control.token"}""");
        Assert.Equal(RoutingProbeKind.TokenUnreadable, token.Kind);
        Assert.Equal(@"C:\Users\x\.ironwire\control.token", token.TokenPath);
    }

    /// <summary>
    /// An answer this build cannot read claims nothing about the proxy in
    /// either direction, and neither does no answer at all.
    /// </summary>
    [Fact]
    public void AnUnreadableAnswerClaimsNothing()
    {
        foreach (string payload in new[] { null!, "", "   ", "{ not json", "{}", """{"outcome":"vintage"}""" })
        {
            RoutingProbe probe = RoutingProbe.Parse(payload);
            Assert.Equal(RoutingProbeKind.Unknown, probe.Kind);
            Assert.Null(probe.Port);
            Assert.Null(probe.TokenPath);
        }
    }

    /// <summary>
    /// A port the daemon did not report must not become port 0. The Rust
    /// sentence for "no port was tried" names none, and a 0 here would send
    /// somebody to check a port that does not exist.
    /// </summary>
    [Fact]
    public void AnUnreachableOutcomeWithNoPortDoesNotInventOne()
    {
        RoutingProbe probe = RoutingProbe.Parse("""{"outcome":"unreachable"}""");
        Assert.Equal(RoutingProbeKind.Unreachable, probe.Kind);
        Assert.Null(probe.Port);

        string line = RoutingTools.ProbeLine(Copy(), probe);
        Assert.DoesNotContain("0", line, StringComparison.Ordinal);
    }

    /// <summary>
    /// The token-unreadable outcome names the absolute path the daemon
    /// reported. That path is the one fact that makes this fixable: a GUI
    /// never sees IRONWIRE_HOME, so it reads the conventional folder whatever
    /// a shell profile says.
    /// </summary>
    [Fact]
    public void TheTokenOutcomeNamesTheAbsolutePathTheDaemonReported()
    {
        RoutingCopy copy = Copy();
        const string Path = @"C:\Users\x\.ironwire\control.token";
        string line = RoutingTools.ProbeLine(
            copy,
            RoutingProbe.Parse($$"""{"outcome":"token_unreadable","token_path":"C:\\Users\\x\\.ironwire\\control.token"}"""));
        Assert.Contains(Path, line, StringComparison.Ordinal);

        // Absent, not empty, when nothing resolved at all: a different
        // sentence, and it names no path.
        string unnamed = RoutingTools.ProbeLine(
            copy,
            RoutingProbe.Parse("""{"outcome":"token_unreadable"}"""));
        Assert.DoesNotContain(@"C:\Users", unnamed, StringComparison.Ordinal);
        Assert.NotEqual(line, unnamed);
    }

    /// <summary>
    /// One outcome, one sentence, and the two failures that are not facts
    /// about IronWire say so rather than sending anyone to look at a port or
    /// a file that is fine.
    /// </summary>
    [Fact]
    public void EachOutcomeGetsItsOwnSentence()
    {
        RoutingCopy copy = Copy();
        Assert.Equal(
            copy.ProbeReachable,
            RoutingTools.ProbeLine(copy, RoutingProbe.Parse("""{"outcome":"reachable","tools":[]}""")));
        Assert.Equal(
            copy.CheckUnavailable,
            RoutingTools.ProbeLine(copy, RoutingProbe.Parse("{ not json")));

        string unreachable = RoutingTools.ProbeLine(
            copy,
            RoutingProbe.Parse("""{"outcome":"unreachable","port":8463}"""));
        Assert.Contains("8463", unreachable, StringComparison.Ordinal);

        foreach (string sentence in new[] { copy.ProbeReachable, copy.CheckUnavailable, unreachable })
        {
            Assert.NotEmpty(sentence);
        }
    }

    // --- What the daemon is seeing ---------------------------------------

    [Fact]
    public void TheDaemonsThreeStatesEachGetTheirOwnSentence()
    {
        RoutingCopy copy = Copy();
        Assert.Equal(copy.StateWaiting, RoutingTools.StateLine(copy, RoutingTools.AwaitingRows));
        Assert.Equal(copy.StateReading, RoutingTools.StateLine(copy, RoutingTools.RowsSeen));
        Assert.Equal(copy.StateOff, RoutingTools.StateLine(copy, RoutingTools.NotDeclared));
        // A state this build has never heard of says what the off state says:
        // it claims nothing.
        Assert.Equal(copy.StateOff, RoutingTools.StateLine(copy, "brand-new-state"));
        Assert.Equal(copy.StateOff, RoutingTools.StateLine(copy, ""));
    }

    /// <summary>
    /// <c>awaiting_rows</c> is not a fault. A reader built a moment ago
    /// starts cold by construction, so this is the state a contributor sees
    /// immediately after turning the switch on or changing the port. Painting
    /// it as an error would accuse a working proxy of being broken at exactly
    /// that moment.
    /// </summary>
    [Fact]
    public void AwaitingRowsIsHeldAndNeverAFault()
    {
        RoutingCopy copy = Copy();
        Assert.Equal(RoutingTone.Held, RoutingTools.StateTone(RoutingTools.AwaitingRows));
        Assert.Equal(copy.StateWaiting, RoutingTools.StateLine(copy, RoutingTools.AwaitingRows));
        Assert.NotEqual(copy.CheckUnavailable, RoutingTools.StateLine(copy, RoutingTools.AwaitingRows));

        Assert.Equal(RoutingTone.Clear, RoutingTools.StateTone(RoutingTools.RowsSeen));
        Assert.Equal(RoutingTone.Neutral, RoutingTools.StateTone(RoutingTools.NotDeclared));
    }

    /// <summary>
    /// "Last checked" is a per-process stamp on the running daemon: it starts
    /// empty again every time that process comes back up. Never an install
    /// date and never a "connected since".
    /// </summary>
    [Fact]
    public void LastCheckedReadsTheTimestampGivenAndNothingElse()
    {
        var now = new DateTimeOffset(2026, 9, 2, 12, 0, 0, TimeSpan.Zero);
        Assert.Equal(
            "Last checked an hour ago",
            RoutingTools.LastCheckedLine(now.AddHours(-1), now));
        Assert.Equal(
            "Last checked yesterday",
            RoutingTools.LastCheckedLine(now.AddDays(-1), now));
    }

    /// <summary>
    /// The stamp is only shown where it says something: never on a state that
    /// has had no answer at all, and never rendered as a half-sentence.
    /// </summary>
    [Fact]
    public void LastCheckedIsWithheldWhereThereHasBeenNoAnswer()
    {
        var now = new DateTimeOffset(2026, 9, 2, 12, 0, 0, TimeSpan.Zero);
        Assert.Null(RoutingTools.LastCheckedLine(null, now));

        RoutingStatusLine undeclared =
            RoutingTools.StatusLine(Copy(), RoutingTools.NotDeclared, now.AddHours(-1), now);
        Assert.Null(undeclared.LastChecked);

        RoutingStatusLine waiting =
            RoutingTools.StatusLine(Copy(), RoutingTools.AwaitingRows, now.AddHours(-1), now);
        Assert.Equal("Last checked an hour ago", waiting.LastChecked);
    }

    // --- What this shell writes ------------------------------------------

    /// <summary>
    /// One <c>set_settings</c> key per edit, and the key is the daemon's.
    /// <c>set_settings</c> refuses an object holding a key it does not
    /// recognise, so a drift here is a silent no-write.
    /// </summary>
    [Fact]
    public void TheDeclarationIsWrittenAsExactlyOneKey()
    {
        using JsonDocument doc = JsonDocument.Parse(
            RoutingTools.SerializeDeclaration(true, 8463, tokenDir: null));
        Assert.Single(doc.RootElement.EnumerateObject());
        JsonElement declaration = doc.RootElement.GetProperty("ironwire");
        Assert.Equal("watch", declaration.GetProperty("mode").GetString());
        Assert.Equal(8463, declaration.GetProperty("port").GetInt32());
        Assert.False(declaration.TryGetProperty("token_dir", out _));
    }

    /// <summary>
    /// Off is <c>null</c>, and the key is still there.
    /// </summary>
    /// <remarks>
    /// Two distinct assertions. Off is not an object with a mode: there is no
    /// conventional fallback for a local service, so absence of a declaration
    /// is the off state. But the KEY must still be written, because
    /// <c>set_settings</c> changes only the keys it is given -- an object with
    /// no <c>ironwire</c> key reads as "never asked" and leaves whatever was
    /// declared before in place, so a contributor turning the switch off would
    /// have nothing happen. The macOS shell pins the same pair in
    /// <c>testTurningItOffWritesNullAndNotAnAbsentKey</c>.
    /// </remarks>
    [Fact]
    public void TurningItOffWritesNullAndNotAnAbsentKey()
    {
        using JsonDocument doc = JsonDocument.Parse(
            RoutingTools.SerializeDeclaration(false, 8463, @"C:\ironwire"));
        Assert.True(
            doc.RootElement.TryGetProperty("ironwire", out JsonElement declaration),
            "off must write the key, not omit it: an omitted key leaves the old declaration standing");
        Assert.Equal(JsonValueKind.Null, declaration.ValueKind);
    }

    /// <summary>
    /// A displayed default must never become a declaration. The port field
    /// shows IronWire's conventional number so nobody has to know it, and
    /// nothing is written until the contributor turns the switch on: a
    /// default that wrote itself would have this window announce a local
    /// service nobody mentioned.
    /// </summary>
    [Fact]
    public void TheDisplayedDefaultPortIsNotADeclaration()
    {
        Assert.Equal((ushort)8463, RoutingTools.DefaultPort);
        using JsonDocument doc = JsonDocument.Parse(
            RoutingTools.SerializeDeclaration(false, RoutingTools.DefaultPort, null));
        Assert.Equal(JsonValueKind.Null, doc.RootElement.GetProperty("ironwire").ValueKind);
    }

    /// <summary>
    /// An empty folder box is left out rather than sent as an empty string:
    /// the daemon refuses an empty string outright, and absence is what falls
    /// back to the conventional location.
    /// </summary>
    [Fact]
    public void AnEmptyFolderBoxIsOmittedRatherThanSentEmpty()
    {
        foreach (string? empty in new[] { null, "", "   " })
        {
            using JsonDocument declaration = JsonDocument.Parse(
                RoutingTools.SerializeDeclaration(true, 8463, empty));
            Assert.False(
                declaration.RootElement.GetProperty("ironwire").TryGetProperty("token_dir", out _));

            using JsonDocument probe = JsonDocument.Parse(
                RoutingTools.SerializeProbeParams(8463, empty));
            Assert.False(probe.RootElement.TryGetProperty("token_dir", out _));
        }

        using JsonDocument named = JsonDocument.Parse(
            RoutingTools.SerializeDeclaration(true, 8463, @"  C:\ironwire  "));
        Assert.Equal(
            @"C:\ironwire",
            named.RootElement.GetProperty("ironwire").GetProperty("token_dir").GetString());
    }

    [Fact]
    public void TheProbeIsAskedAboutTheDeclaredPort()
    {
        using JsonDocument doc = JsonDocument.Parse(
            RoutingTools.SerializeProbeParams(9001, @"C:\ironwire"));
        Assert.Equal(9001, doc.RootElement.GetProperty("port").GetInt32());
        Assert.Equal(@"C:\ironwire", doc.RootElement.GetProperty("token_dir").GetString());
    }

    /// <summary>
    /// The method name is the daemon's. A name that is not in its pinned
    /// METHODS array is a call that can only ever be refused.
    /// </summary>
    [Fact]
    public void TheProbeMethodIsTheDaemonsOwn()
    {
        Assert.Equal("probe_routed_tools", DaemonProtocol.Methods.ProbeRoutedTools);
    }

    // --- What the daemon told us -----------------------------------------

    /// <summary>
    /// The <c>get_settings</c> fields this surface reads. <c>*_root_configured</c>
    /// cannot carry the distinction: it is false both for a source pointed at
    /// the conventional location and for one the contributor said they do not
    /// use, and only the second reads "Not used".
    /// </summary>
    [Fact]
    public void TheSettingsSnapshotCarriesTheSourceModesAndTheDeclaration()
    {
        DaemonSettingsSnapshot? settings = JsonSerializer.Deserialize<DaemonSettingsSnapshot>(
            """
            {"claude_source_mode":"watch","codex_source_mode":"off","gemini_source_mode":"unset",
             "ironwire":{"mode":"watch","port":9001,"token_dir":"C:\\ironwire"}}
            """);
        Assert.NotNull(settings);
        Assert.Equal("watch", settings!.ClaudeSourceMode);
        Assert.Equal("off", settings.CodexSourceMode);
        Assert.Equal("unset", settings.GeminiSourceMode);
        Assert.True(settings.RoutingDeclared);
        Assert.Equal((ushort)9001, settings.Routing!.Port);
        Assert.Equal(@"C:\ironwire", settings.Routing.TokenDir);
    }

    /// <summary>
    /// No block at all, and a block that says off, are both off. Absent means
    /// off: there is no conventional fallback for a local service.
    /// </summary>
    [Fact]
    public void AnAbsentOrOffDeclarationIsOff()
    {
        foreach (string payload in new[] { "{}", """{"ironwire":null}""", """{"ironwire":{"mode":"off"}}""" })
        {
            DaemonSettingsSnapshot? settings =
                JsonSerializer.Deserialize<DaemonSettingsSnapshot>(payload);
            Assert.NotNull(settings);
            Assert.False(settings!.RoutingDeclared);
        }
    }

    /// <summary>
    /// <c>status.routing</c>. A daemon that predates the block reports
    /// nothing, and that reads as the state that claims nothing.
    /// </summary>
    [Fact]
    public void TheStatusCarriesTheRoutingStateAndTheStamp()
    {
        DaemonStatus? status = JsonSerializer.Deserialize<DaemonStatus>(
            """{"routing":{"state":"rows_seen","last_refresh_at":"2026-09-02T11:00:00Z"}}""");
        Assert.NotNull(status);
        Assert.Equal(RoutingTools.RowsSeen, status!.Routing!.State);
        Assert.Equal(
            new DateTimeOffset(2026, 9, 2, 11, 0, 0, TimeSpan.Zero),
            status.Routing.LastRefreshAt);

        DaemonStatus? older = JsonSerializer.Deserialize<DaemonStatus>("{}");
        Assert.NotNull(older);
        Assert.Equal(string.Empty, older!.RoutingState);
        Assert.Equal(Copy().StateOff, RoutingTools.StateLine(Copy(), older.RoutingState));
    }

    // --- Sweeps -----------------------------------------------------------

    /// <summary>
    /// Everything this surface can say, in one list, for the sweeps below.
    /// </summary>
    private static IReadOnlyList<string> EverythingThisSurfaceSays()
    {
        RoutingCopy copy = Copy();
        var now = new DateTimeOffset(2026, 9, 2, 12, 0, 0, TimeSpan.Zero);
        var said = new List<string>();
        foreach (var property in typeof(RoutingCopy).GetProperties()
                     .Where(p => p.PropertyType == typeof(string)))
        {
            said.Add((string)property.GetValue(copy)!);
        }

        foreach (string outcome in new[]
        {
            """{"outcome":"reachable","tools":[]}""",
            """{"outcome":"unreachable","port":8463}""",
            """{"outcome":"unreachable"}""",
            """{"outcome":"token_unreadable","token_path":"C:\\Users\\x\\.ironwire\\control.token"}""",
            """{"outcome":"token_unreadable"}""",
            "{ not json",
        })
        {
            said.Add(RoutingTools.ProbeLine(copy, RoutingProbe.Parse(outcome)));
        }

        foreach (string state in new[]
                 { RoutingTools.NotDeclared, RoutingTools.AwaitingRows, RoutingTools.RowsSeen, "vintage" })
        {
            RoutingStatusLine line = RoutingTools.StatusLine(copy, state, now.AddHours(-1), now);
            said.Add(line.Text);
            if (line.LastChecked is not null)
            {
                said.Add(line.LastChecked);
            }
        }

        RoutingEvidence evidence = Reachable(
            """
            {"outcome":"reachable","tools":[
              {"id":"claude","installed":true,"wired":true},
              {"id":"codex","installed":true,"wired":false}
            ]}
            """);
        foreach (RoutingToolRow row in RoutingTools.Rows(copy, AllWatched(), evidence))
        {
            said.Add(row.Name);
            said.Add(row.Word);
            said.Add(row.AccessibleLabel);
        }

        return said;
    }

    /// <summary>
    /// No restart notice. Declarations apply on the next poll, and nothing
    /// here waits on the app being started again -- a sentence implying
    /// otherwise would be false as well as discouraging.
    /// </summary>
    [Fact]
    public void NothingOnThisSurfaceAsksForARestart()
    {
        foreach (string said in EverythingThisSurfaceSays())
        {
            string lower = said.ToLowerInvariant();
            foreach (string word in new[]
                     { "restart", "relaunch", "reopen", "reboot", "sign out", "log out", "quit and" })
            {
                Assert.DoesNotContain(word, lower, StringComparison.Ordinal);
            }
        }
    }

    /// <summary>
    /// This surface is for somebody with no invite. Nothing about corpora,
    /// credits, ownership, contribution or money appears on it -- not greyed
    /// out, and not as a teaser.
    /// </summary>
    [Fact]
    public void NothingOnThisSurfaceMentionsCorporaCreditsOrMoney()
    {
        foreach (string said in EverythingThisSurfaceSays())
        {
            string lower = said.ToLowerInvariant();
            foreach (string word in new[]
                     {
                         "credit", "corpus", "corpora", "reward", "payment", "paid", "money",
                         "earn", "invite", "ownership", "royalt", "dataset", "$",
                     })
            {
                Assert.DoesNotContain(word, lower, StringComparison.Ordinal);
            }
        }
    }

    /// <summary>
    /// No word on this surface denies privacy. "Private" is a substring of
    /// "Not private", the same shape that let a <c>Contains</c> on
    /// "reachable" match "unreachable" on this surface, so the not-wired word
    /// is "Sends direct" and this is what stops anybody tidying it back.
    ///
    /// Swept over everything the surface can say, not just the four words: a
    /// sentence is as capable of carrying the denial as a chip.
    /// </summary>
    [Fact]
    public void NothingOnThisSurfaceDeniesPrivacy()
    {
        RoutingCopy copy = Copy();
        foreach (string said in EverythingThisSurfaceSays())
        {
            string lower = said.ToLowerInvariant();
            foreach (string denial in new[] { "not private", "isn't private", "is not private", "no privacy" })
            {
                Assert.DoesNotContain(denial, lower, StringComparison.Ordinal);
            }
        }

        Assert.Contains("privat", copy.WordPrivate.ToLowerInvariant(), StringComparison.Ordinal);
        foreach (string word in new[] { copy.WordDirect, copy.WordUnknown, copy.WordNotUsed })
        {
            Assert.DoesNotContain("privat", word.ToLowerInvariant(), StringComparison.Ordinal);
        }
    }

    // --- What the machine already knows --------------------------------

    /// <summary>
    /// The method this shell calls is the one the daemon advertises.
    /// </summary>
    /// <remarks>
    /// <c>discover_routing</c> sat in the daemon unused: its only references
    /// outside the IPC module were two doc comments and a list of names. A
    /// literal misspelled here would put it straight back to unused, and the
    /// failure would look exactly like a machine without IronWire.
    /// </remarks>
    [Fact]
    public void TheDiscoveryMethodIsSpelledTheWayTheDaemonAdvertisesIt()
    {
        Assert.Equal("discover_routing", DaemonProtocol.Methods.DiscoverRouting);
    }

    /// <summary>The shape a running IronWire produces.</summary>
    [Fact]
    public void APublishedPointerYieldsThePortAndThePath()
    {
        RoutingDiscovery found = RoutingDiscovery.Parse(
            """{"found":true,"port":9143,"token_path":"C:\\Users\\x\\.ironwire\\control.token"}""");

        Assert.Equal((ushort)9143, found.Port);
        Assert.Equal("C:\\Users\\x\\.ironwire\\control.token", found.TokenPath);
        Assert.True(found.Found);
    }

    /// <summary>
    /// Every shape that means there is nothing to offer, and none of them is
    /// an error. A machine without IronWire is the ordinary machine.
    /// </summary>
    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("not json at all")]
    [InlineData("[]")]
    [InlineData("""{}""")]
    [InlineData("""{"found":false}""")]
    [InlineData("""{"found":false,"port":8463}""")]
    [InlineData("""{"found":true}""")]
    [InlineData("""{"found":true,"port":0}""")]
    [InlineData("""{"found":true,"port":70000}""")]
    [InlineData("""{"found":true,"port":"8463"}""")]
    [InlineData("""{"found":"true","port":8463}""")]
    public void NothingToOfferIsOneStateAndNotAnError(string? json)
    {
        RoutingDiscovery discovery = RoutingDiscovery.Parse(json);

        Assert.Equal(RoutingDiscovery.Nothing, discovery);
        Assert.Null(discovery.Port);
        Assert.False(discovery.Found);
    }

    /// <summary>
    /// A pointer that named no credential still names a port. The path is the
    /// extra, not the thing the call is for.
    /// </summary>
    [Theory]
    [InlineData("""{"found":true,"port":9143}""")]
    [InlineData("""{"found":true,"port":9143,"token_path":""}""")]
    [InlineData("""{"found":true,"port":9143,"token_path":null}""")]
    public void APointerWithoutATokenPathStillCarriesThePort(string json)
    {
        RoutingDiscovery discovery = RoutingDiscovery.Parse(json);

        Assert.Equal((ushort)9143, discovery.Port);
        Assert.Null(discovery.TokenPath);
    }

    /// <summary>
    /// The rule the whole feature turns on. A declared port always wins: a
    /// pointer is a file that survives the daemon that wrote it, so a stale
    /// one naming 9000 must not replace a declared 8463.
    /// </summary>
    [Fact]
    public void ADiscoveredPortNeverReplacesADeclaredOne()
    {
        Assert.Equal((ushort)8463, RoutingTools.ShownPort(8463, 9000));
        Assert.Equal((ushort)9000, RoutingTools.ShownPort(9000, null));

        // And it fills in where nothing is declared, ahead of the
        // conventional number rather than behind it.
        Assert.Equal((ushort)9143, RoutingTools.ShownPort(null, 9143));
        Assert.Equal(RoutingTools.DefaultPort, RoutingTools.ShownPort(null, null));
    }

    /// <summary>
    /// A shown port is still not a declaration, discovered or not: off is
    /// spelled null whatever number the box carries.
    /// </summary>
    [Fact]
    public void ADiscoveredPortInTheBoxIsNotADeclaration()
    {
        ushort shown = RoutingTools.ShownPort(null, 9143);

        Assert.Equal((ushort)9143, shown);
        using JsonDocument doc = JsonDocument.Parse(
            RoutingTools.SerializeDeclaration(false, shown, null));
        Assert.Equal(JsonValueKind.Null, doc.RootElement.GetProperty("ironwire").ValueKind);
    }

    /// <summary>
    /// Two states, two sentences, both from the Rust and neither a fault.
    /// </summary>
    [Fact]
    public void TheDiscoverySentenceIsTheSharedOneForBothStates()
    {
        RoutingCopy copy = Copy();

        string found = RoutingTools.DiscoveryLine(copy, new RoutingDiscovery(9143, null));
        Assert.Contains("9143", found, StringComparison.Ordinal);

        string nothing = RoutingTools.DiscoveryLine(copy, RoutingDiscovery.Nothing);
        Assert.NotEqual(found, nothing);
        Assert.NotEmpty(nothing);
        Assert.DoesNotContain("0", nothing, StringComparison.Ordinal);
        foreach (string fault in new[] { "error", "failed", "problem", "wrong" })
        {
            Assert.DoesNotContain(fault, nothing.ToLowerInvariant(), StringComparison.Ordinal);
            Assert.DoesNotContain(fault, found.ToLowerInvariant(), StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// The boxes become a disclosure only once the machine answered. Where it
    /// did not they are the only way to answer, so they stay open.
    /// </summary>
    [Fact]
    public void ThePortAndFolderCollapseOnlyOnceSomethingWasDiscovered()
    {
        Assert.True(RoutingTools.OverrideIsCollapsed(new RoutingDiscovery(9143, null)));
        Assert.False(RoutingTools.OverrideIsCollapsed(RoutingDiscovery.Nothing));
    }

    /// <summary>
    /// This shell no longer owns the branch table: which of the four words a
    /// tool gets is decided in <c>routing_copy.rs</c> and crosses the ABI.
    ///
    /// Asserted on the source, because a C# reimplementation that happened to
    /// agree with the Rust today would pass every behavioural test here and
    /// then drift the first time only one of the two was edited -- which is
    /// the failure this removes, not a hypothetical one.
    /// </summary>
    [Fact]
    public void TheWordAndToneBranchTablesAreNotReimplementedInThisShell()
    {
        string source = ImplementationSource();

        Assert.Contains("RoutingSurface.ToolWord(sourceMode, wiring)", source, StringComparison.Ordinal);
        Assert.Contains("RoutingSurface.ToolTone(sourceMode, wiring)", source, StringComparison.Ordinal);
        Assert.Contains("RoutingSurface.StateLine(state)", source, StringComparison.Ordinal);
        Assert.Contains("RoutingSurface.StateTone(state)", source, StringComparison.Ordinal);

        // The state names are wire values this shell and its tests talk in.
        // They may no longer be the arms of a table.
        Assert.DoesNotContain("AwaitingRows =>", source, StringComparison.Ordinal);
        Assert.DoesNotContain("RowsSeen =>", source, StringComparison.Ordinal);

        // The words may be reached only as the fallback for a call the ABI
        // refused, never as the arms of a table. WordUnknown and StateOff are
        // those fallbacks; the rest would mean a word is still chosen here.
        foreach (string field in new[]
                 {
                     "WordPrivate", "WordDirect", "WordNotUsed", "StateWaiting", "StateReading",
                 })
        {
            Assert.False(
                source.Contains(field, StringComparison.Ordinal),
                $"copy.{field} is reached in RoutingTools.cs, so a word is still chosen here");
        }
    }

    /// <summary>
    /// No styling decision on this surface reads the rendered word.
    ///
    /// The tone comes from <see cref="ToolWiring"/> through the same shared
    /// table that chose the word. A bool meaning "this is the privacy word"
    /// would invite a text comparison later, and "Private" is a substring of
    /// the denial that must never come back -- the same shape that once let
    /// <c>Contains("reachable")</c> match "unreachable" here.
    /// </summary>
    [Fact]
    public void NoStylingDecisionReadsTheRenderedWord()
    {
        string source = ImplementationSource();
        foreach (string comparison in new[]
                 {
                     "Word ==", "Word !=", "Word.Contains", "Word.StartsWith", "Word.Equals",
                     "word ==", "word !=", "word.Contains", "word.Equals",
                 })
        {
            Assert.False(
                source.Contains(comparison, StringComparison.Ordinal),
                $"a styling decision reads the rendered word: {comparison}");
        }
    }

    /// <summary>
    /// The reassuring tone falls on exactly the word that claims privacy, over
    /// every input pair rather than the three a screenshot would show.
    /// </summary>
    [Fact]
    public void TheReassuringToneFallsOnThePrivateWordAlone()
    {
        RoutingCopy copy = Copy();
        foreach (string mode in new[] { "off", "watch", "unset", "", "something_new" })
        {
            foreach (ToolWiring wiring in new[] { ToolWiring.Wired, ToolWiring.NotWired, ToolWiring.Unknown })
            {
                string word = RoutingTools.ToolWord(copy, mode, wiring);
                RoutingTone tone = RoutingTools.ToolTone(mode, wiring);
                Assert.Equal(word == copy.WordPrivate, tone == RoutingTone.Clear);
                Assert.True(
                    tone == RoutingTone.Clear || tone == RoutingTone.Neutral,
                    $"{mode}/{wiring} painted {tone}");
            }
        }

        // The rows carry it, so a view never has to work it out.
        RoutingEvidence evidence = Reachable(
            """
            {"outcome":"reachable","tools":[
              {"id":"claude","installed":true,"wired":true},
              {"id":"codex","installed":true,"wired":false}
            ]}
            """);
        IReadOnlyList<RoutingToolRow> rows = RoutingTools.Rows(copy, AllWatched(), evidence);
        Assert.Equal(RoutingTone.Clear, rows[0].Tone);
        Assert.Equal(RoutingTone.Neutral, rows[1].Tone);
        Assert.Equal(RoutingTone.Neutral, rows[2].Tone);

        // "Not used" is a preference, not an achievement.
        RoutingToolRow unused = RoutingTools.Rows(
            copy,
            new RoutingModes { Claude = "off", Codex = "off", Gemini = "off" },
            evidence)[0];
        Assert.Equal(copy.WordNotUsed, unused.Word);
        Assert.Equal(RoutingTone.Neutral, unused.Tone);
    }

    /// <summary>
    /// <see cref="ToolWiring"/>'s numbering is the ABI's <c>TC_TOOL_WIRING_*</c>.
    ///
    /// The cast in <see cref="RoutingSurface.ToolWord"/> relies on it, and a
    /// reordered enum would send "wired" across as "not wired" -- a wrong
    /// verdict on a privacy claim, not a crash.
    /// </summary>
    [Fact]
    public void TheWiringEnumIsNumberedAsTheAbiSpellsIt()
    {
        Assert.Equal(0, (int)ToolWiring.Wired);
        Assert.Equal(1, (int)ToolWiring.NotWired);
        Assert.Equal(2, (int)ToolWiring.Unknown);

        RoutingCopy copy = Copy();
        Assert.Equal(copy.WordPrivate, RoutingSurface.ToolWord("watch", ToolWiring.Wired));
        Assert.Equal(copy.WordDirect, RoutingSurface.ToolWord("watch", ToolWiring.NotWired));
        Assert.Equal(copy.WordUnknown, RoutingSurface.ToolWord("watch", ToolWiring.Unknown));
    }

    /// <summary>
    /// A state this build has never heard of claims nothing: it reads as the
    /// off line and never falls through to either "on" sentence.
    /// </summary>
    [Fact]
    public void AnUnknownStateReadsAsTheOffLineAcrossTheAbi()
    {
        RoutingCopy copy = Copy();
        Assert.Equal(copy.StateOff, RoutingSurface.StateLine("a_state_from_a_later_daemon"));
        Assert.Equal(copy.StateOff, RoutingSurface.StateLine(""));
        Assert.Equal(copy.StateOff, RoutingSurface.StateLine(null));
        Assert.Equal(copy.StateWaiting, RoutingSurface.StateLine(RoutingTools.AwaitingRows));
        Assert.Equal(copy.StateReading, RoutingSurface.StateLine(RoutingTools.RowsSeen));
    }

    /// <summary>
    /// The daemon's state tone is carried into the view and painted there.
    ///
    /// <see cref="RoutingTools.StateTone"/> was already load-bearing -- it
    /// gates the "last checked" stamp inside
    /// <see cref="RoutingTools.StatusLine"/> -- but this shell threw
    /// <see cref="RoutingStatusLine.Tone"/> away and painted the sentence
    /// flat, while GTK has painted the same three states since it was
    /// written. This is that parity.
    ///
    /// Asserted about the app's source because <c>TraceCommons.App</c> is a
    /// WinUI project and cannot be built on the machines this suite runs on.
    /// That is a real limitation and worth naming: this catches a tone
    /// thrown away or recovered from a string, and it does not catch a
    /// binding that never reaches the screen.
    /// </summary>
    [Fact]
    public void TheStatusSentenceIsPaintedFromTheStateAndNotFromItsOwnText()
    {
        string viewModel = AppSource("ContributorSettingsViewModel.cs.txt");
        string xaml = AppSource("SettingsView.xaml.txt");

        Assert.Contains("RoutingStateTone = line.Tone;", viewModel, StringComparison.Ordinal);
        Assert.Contains(
            "RoutingStateTone == RoutingTone.Clear", viewModel, StringComparison.Ordinal);
        Assert.Contains(
            "RoutingStateTone == RoutingTone.Held", viewModel, StringComparison.Ordinal);
        foreach (string binding in new[]
                 {
                     "Settings.RoutingStateIsNeutral", "Settings.RoutingStateIsHeld",
                     "Settings.RoutingStateIsClear",
                 })
        {
            Assert.Contains(binding, xaml, StringComparison.Ordinal);
        }

        // None of the three states is a fault, so none of them may reach a
        // fault colour. awaiting_rows is what a contributor sees immediately
        // after touching anything on this card; painting it as broken would
        // accuse a working proxy at exactly that moment.
        Assert.NotEqual(RoutingTone.Clear, RoutingTools.StateTone(RoutingTools.AwaitingRows));
        Assert.Equal(RoutingTone.Held, RoutingTools.StateTone(RoutingTools.AwaitingRows));
        foreach (string alarming in new[]
                 {
                     "TcGoldTextBrush", "TcGoldBrandBrush", "TcCoralTextBrush", "TcCoralBrandBrush",
                 })
        {
            Assert.DoesNotContain(alarming, RoutingCardMarkup(xaml), StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// No styling decision in the app layer reads a rendered string.
    ///
    /// The tool row's tone rides on <see cref="RoutingToolRow.Tone"/> and the
    /// status line's on <see cref="RoutingStatusLine.Tone"/>; both were
    /// decided from an enum. A comparison against
    /// <see cref="RoutingToolRowViewModel"/>'s word or against the state text
    /// would be a text match on a privacy claim -- "Private" is a substring
    /// of the denial that must never come back.
    /// </summary>
    [Fact]
    public void NoStylingDecisionInTheAppLayerReadsARenderedString()
    {
        string viewModel = AppSource("ContributorSettingsViewModel.cs.txt");

        Assert.Contains("Tone = row.Tone;", viewModel, StringComparison.Ordinal);
        Assert.Contains("Tone == RoutingTone.Clear", viewModel, StringComparison.Ordinal);
        foreach (string recovered in new[]
                 {
                     "Word ==", "Word.Contains", "Word.Equals", "Word.StartsWith",
                     "RoutingStateText ==", "RoutingStateText.Contains",
                     "\"Private\"", "WordPrivate",
                 })
        {
            Assert.False(
                viewModel.Contains(recovered, StringComparison.Ordinal),
                $"a styling decision reads a rendered string: {recovered}");
        }
    }

    /// <summary>
    /// One of the app-layer sources, copied beside the test assembly, with
    /// its C# comments stripped -- prose about the rule quotes the very
    /// strings the rule forbids.
    /// </summary>
    private static string AppSource(string name)
    {
        string path = Path.Combine(AppContext.BaseDirectory, name);
        Assert.True(File.Exists(path), $"the app source was not copied to {path}");
        return string.Join(
            "\n",
            File.ReadAllText(path)
                .Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal))
                .Where(line => !line.TrimStart().StartsWith("///", StringComparison.Ordinal)));
    }

    /// <summary>
    /// The routing card's slice of the settings markup: from the tool rows
    /// down to the last-checked stamp. Scoped, because the rest of that file
    /// paints things that legitimately are faults.
    /// </summary>
    private static string RoutingCardMarkup(string xaml)
    {
        int start = xaml.IndexOf("Settings.RoutingToolRows", StringComparison.Ordinal);
        int end = xaml.IndexOf("Settings.HasRoutingLastChecked", StringComparison.Ordinal);
        Assert.True(start >= 0 && end > start, "the routing card is no longer in this markup");
        return xaml[start..end];
    }

    /// <summary>
    /// The daemon state's tone is the shared table's answer, and it agrees
    /// with the sentence that state gets.
    ///
    /// This was the last routing branch table still written out natively in
    /// all three shells. Change which tone a state maps to in
    /// <c>routing_copy.rs</c> -- without touching a string -- and this goes
    /// red.
    /// </summary>
    [Fact]
    public void TheToneEachStateGetsIsTheRustsChoiceAndNotThisShells()
    {
        RoutingCopy copy = Copy();
        Assert.Equal(RoutingTone.Held, RoutingTools.StateTone(RoutingTools.AwaitingRows));
        Assert.Equal(RoutingTone.Clear, RoutingTools.StateTone(RoutingTools.RowsSeen));
        Assert.Equal(RoutingTone.Neutral, RoutingTools.StateTone(RoutingTools.NotDeclared));

        foreach (string state in new[]
                 {
                     RoutingTools.NotDeclared, RoutingTools.AwaitingRows, RoutingTools.RowsSeen,
                     "", "ROWS_SEEN", "a_state_from_a_later_daemon",
                 })
        {
            RoutingTone tone = RoutingTools.StateTone(state);
            // The tone and the sentence are one decision.
            Assert.Equal(tone == RoutingTone.Neutral, RoutingTools.StateLine(copy, state) == copy.StateOff);
            // And the stamp is gated on that same reading.
            Assert.Equal(
                tone != RoutingTone.Neutral,
                RoutingTools.StatusLine(copy, state, DateTimeOffset.UtcNow).LastChecked is not null);
        }

        // A state this build has never heard of claims nothing rather than
        // falling through to either "on" tone.
        Assert.Equal(RoutingTone.Neutral, RoutingTools.StateTone("a_state_from_a_later_daemon"));
        Assert.Equal(RoutingTone.Neutral, RoutingTools.StateTone(""));
    }

    /// <summary>
    /// One tone numbering serves both calls, and a tool word can never take
    /// the held tone.
    /// </summary>
    /// <remarks>
    /// Two numberings would mean two <c>1</c>s meaning different things on
    /// one ABI, and a shell that mapped the wrong one would mispaint a
    /// privacy claim rather than fail. <see cref="RoutingTone"/>'s own
    /// ordering happens to agree with the ABI's; that is asserted rather
    /// than cast, so a reordered enum fails here instead of silently.
    /// </remarks>
    [Fact]
    public void OneToneNumberingServesBothCallsAndAToolWordIsNeverHeld()
    {
        RoutingCopy copy = Copy();
        foreach (string mode in new[] { "off", "watch", "unset", "", "something_new" })
        {
            foreach (ToolWiring wiring in new[] { ToolWiring.Wired, ToolWiring.NotWired, ToolWiring.Unknown })
            {
                Assert.NotEqual(RoutingTone.Held, RoutingTools.ToolTone(mode, wiring));
            }
        }

        // The held tone is reachable, from the one thing that may hold.
        Assert.Equal(RoutingTone.Held, RoutingTools.StateTone(RoutingTools.AwaitingRows));
        Assert.Equal(copy.StateWaiting, RoutingTools.StateLine(copy, RoutingTools.AwaitingRows));
    }

    /// <summary>
    /// <c>RoutingTools.cs</c> with its comments stripped: prose about the wire
    /// may quote it, and nothing in a comment is ever rendered or executed.
    /// </summary>
    private static string ImplementationSource()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "RoutingTools.cs.txt");
        Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");
        return string.Join(
            "\n",
            File.ReadAllText(path)
                .Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal))
                .Where(line => !line.TrimStart().StartsWith("///", StringComparison.Ordinal)));
    }

    /// <summary>
    /// This shell authors no wording on this surface.
    ///
    /// Read from the implementation's own source rather than asserted about
    /// its behaviour: a hand-written word that happened to match the Rust
    /// would pass every other test in this file, and would then survive a
    /// rename on the Rust side in exactly one of the three shells. Every
    /// string literal in <c>RoutingTools.cs</c> must be a wire value -- a
    /// JSON key, a daemon state name, an IronWire tool id -- and the
    /// allow-list below is the whole of what that may be.
    /// </summary>
    [Fact]
    public void NoWordingIsAuthoredInThisShell()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "RoutingTools.cs.txt");
        Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");
        string source = File.ReadAllText(path);

        // Strip doc comments and line comments: prose about the wire may
        // quote it, and nothing in a comment is ever rendered.
        var uncommented = string.Join(
            "\n",
            source.Split('\n').Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));

        var allowed = new HashSet<string>(StringComparer.Ordinal)
        {
            // set_settings / probe_routed_tools wire keys and values.
            "ironwire", "mode", "watch", "off", "port", "token_dir",
            // discover_routing's answer. "found" is the daemon's own key and
            // not a word anybody reads: the sentence beside it is assembled
            // in routing_copy.rs and crosses the ABI already finished.
            "found",
            "outcome", "reachable", "unreachable", "token_unreadable",
            "token_path", "tools", "id", "installed", "wired",
            // The daemon's routing states, and the status block they arrive in.
            "not_declared", "awaiting_rows", "rows_seen",
            "state", "last_refresh_at",
            // IronWire's own stable tool ids.
            "claude", "codex", "gemini",
            // Punctuation, not wording.
            ": ",
        };

        foreach (Match match in Regex.Matches(uncommented, "\"([^\"\\\\]|\\\\.)*\""))
        {
            string literal = match.Value[1..^1];
            Assert.True(
                allowed.Contains(literal),
                $"\"{literal}\" is a string literal in RoutingTools.cs that is not a wire value. "
                + "Wording on this surface comes from routing_copy.rs across the ABI.");
        }
    }
}
