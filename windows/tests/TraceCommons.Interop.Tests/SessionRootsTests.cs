using System;
using System.Collections.Generic;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The roots declaration: what gets sent, and what must never be.
///
/// These run on any platform, which is the point of keeping the rules in the
/// interop assembly. The WinUI project that renders them cannot be built or
/// tested anywhere but Windows, and "does an unanswered source get watched
/// anyway" is too important to be verifiable only there.
/// </summary>
public sealed class SessionRootsTests
{
    /// <summary>A payload in the shape <c>tc_discover_sources</c> returns.</summary>
    private const string DiscoveryJson = """
        [
          {"source":"claude-code","path":"C:\\Users\\z\\.claude\\projects","exists":true,
           "session_count":946,"most_recent":"2026-08-19T09:00:00Z","relocated_by_env":false},
          {"source":"codex","path":"C:\\Users\\z\\.codex\\sessions","exists":true,
           "session_count":3066,"most_recent":"2026-08-19T10:30:00Z","relocated_by_env":true}
        ]
        """;

    [Fact]
    public void DiscoveryParsesEveryFieldTheConsentPromptNeeds()
    {
        IReadOnlyList<SourceCandidate> found = SourceDiscovery.Parse(DiscoveryJson);

        Assert.Equal(2, found.Count);

        SourceCandidate? claude = SourceDiscovery.For(found, SourceDiscovery.ClaudeCode);
        Assert.NotNull(claude);
        Assert.Equal(@"C:\Users\z\.claude\projects", claude.Path);
        Assert.True(claude.Exists);
        Assert.Equal(946UL, claude.SessionCount);
        Assert.False(claude.RelocatedByEnv);
        Assert.Equal(
            new DateTimeOffset(2026, 8, 19, 9, 0, 0, TimeSpan.Zero),
            claude.MostRecent);

        SourceCandidate? codex = SourceDiscovery.For(found, SourceDiscovery.Codex);
        Assert.NotNull(codex);
        Assert.Equal(3066UL, codex.SessionCount);
        Assert.True(codex.RelocatedByEnv);
    }

    /// <summary>
    /// A discovery failure must not become a refusal to let the contributor
    /// name a folder by hand.
    /// </summary>
    [Theory]
    [InlineData("")]
    [InlineData("   ")]
    [InlineData("not json")]
    [InlineData("{\"not\":\"an array\"}")]
    public void UnusableDiscoveryYieldsNoCandidatesRatherThanThrowing(string json)
    {
        Assert.Empty(SourceDiscovery.Parse(json));
    }

    /// <summary>
    /// The rule the whole screen exists for: nothing is answered until the
    /// contributor answers it.
    /// </summary>
    [Fact]
    public void AFreshDeclarationIsIncompleteAndSendsNothing()
    {
        var declaration = new SessionRootsDeclaration();

        Assert.False(declaration.IsComplete);
        Assert.Null(declaration.SettingsJson());
    }

    [Fact]
    public void OneAnsweredSourceIsStillNotADeclaration()
    {
        var declaration = new SessionRootsDeclaration
        {
            Claude = SourceDecision.Watch(@"C:\Users\z\.claude\projects"),
        };

        Assert.False(declaration.IsComplete);
        Assert.Null(declaration.SettingsJson());
    }

    /// <summary>
    /// "I don't use this agent" must travel as a declaration.
    ///
    /// This is the test that matters most in this file. An unset source is not
    /// "off" to the daemon: it means the conventional per-user location, i.e.
    /// the contributor's real store. So a screen that expressed "no" by simply
    /// not sending the key would watch the very thing the contributor declined
    /// -- on a working machine, thousands of sessions.
    /// </summary>
    [Fact]
    public void DecliningASourceSendsAnExplicitOffRatherThanOmittingIt()
    {
        var declaration = new SessionRootsDeclaration
        {
            Claude = SourceDecision.Watch(@"C:\Users\z\.claude\projects"),
            Codex = SourceDecision.Off,
        };

        using JsonDocument json = JsonDocument.Parse(declaration.SettingsJson()!);
        JsonElement root = json.RootElement;

        JsonElement codex = root.GetProperty("codex_source");
        Assert.Equal("off", codex.GetProperty("mode").GetString());
        Assert.False(codex.TryGetProperty("path", out _));

        JsonElement claude = root.GetProperty("claude_source");
        Assert.Equal("watch", claude.GetProperty("mode").GetString());
        Assert.Equal(@"C:\Users\z\.claude\projects", claude.GetProperty("path").GetString());
    }

    /// <summary>
    /// Exactly the two recognized keys. The settings validator rejects an
    /// unknown top-level key outright rather than ignoring it, so an extra
    /// field here would refuse the whole declaration.
    /// </summary>
    [Fact]
    public void TheDeclarationCarriesOnlyTheTwoRecognizedKeys()
    {
        var declaration = new SessionRootsDeclaration
        {
            Claude = SourceDecision.Off,
            Codex = SourceDecision.Off,
        };

        using JsonDocument json = JsonDocument.Parse(declaration.SettingsJson()!);

        var keys = new List<string>();
        foreach (JsonProperty property in json.RootElement.EnumerateObject())
        {
            keys.Add(property.Name);
        }

        Assert.Equal(2, keys.Count);
        Assert.Contains("claude_source", keys);
        Assert.Contains("codex_source", keys);
    }

    /// <summary>
    /// Gemini CLI and Cline are offered but cannot block: the daemon's own
    /// start gate is two-conjunct, and an unanswered optional source is
    /// "never asked", which constructs no adapter. So neither one appears in
    /// the payload until it is answered, and neither one holds Continue.
    /// </summary>
    [Fact]
    public void AnUnansweredOptionalSourceNeitherBlocksNorTravels()
    {
        var declaration = new SessionRootsDeclaration
        {
            Claude = SourceDecision.Off,
            Codex = SourceDecision.Off,
        };

        Assert.True(declaration.IsComplete);

        using JsonDocument json = JsonDocument.Parse(declaration.SettingsJson()!);
        Assert.False(json.RootElement.TryGetProperty("gemini_source", out _));
        Assert.False(json.RootElement.TryGetProperty("cline_source", out _));
    }

    /// <summary>
    /// Once answered, an optional source travels exactly as the required two
    /// do: an explicit "off" for a refusal, a path for a folder.
    /// </summary>
    [Fact]
    public void AnAnsweredOptionalSourceTravelsLikeTheRequiredOnes()
    {
        var declaration = new SessionRootsDeclaration
        {
            Claude = SourceDecision.Off,
            Codex = SourceDecision.Off,
            Gemini = SourceDecision.Off,
            Cline = SourceDecision.Watch(@"C:\Users\z\.cline\tasks"),
        };

        using JsonDocument json = JsonDocument.Parse(declaration.SettingsJson()!);
        JsonElement root = json.RootElement;

        Assert.Equal("off", root.GetProperty("gemini_source").GetProperty("mode").GetString());
        JsonElement cline = root.GetProperty("cline_source");
        Assert.Equal("watch", cline.GetProperty("mode").GetString());
        Assert.Equal(@"C:\Users\z\.cline\tasks", cline.GetProperty("path").GetString());
    }

    /// <summary>
    /// The roots screen is the one place a shell maps a source id to a name
    /// itself, mirroring how <c>gemini-cli</c> is already mapped.
    /// </summary>
    [Fact]
    public void TheOptionalSourcesHaveDisplayNames()
    {
        Assert.Equal("Gemini CLI", SessionRootsCopy.AgentName(SourceDiscovery.GeminiCli));
        Assert.Equal("Cline", SessionRootsCopy.AgentName(SourceDiscovery.Cline));
    }

    /// <summary>
    /// Windows paths are full of backslashes, and a folder may contain a
    /// quote. Hand-built JSON would corrupt the first store with either, which
    /// is why the declaration is serialized rather than concatenated.
    /// </summary>
    [Theory]
    [InlineData(@"C:\Users\z\.claude\projects")]
    [InlineData(@"C:\Users\Ann ""The Boss"" Lee\.claude\projects")]
    [InlineData(@"\\server\share\sessions")]
    public void APathSurvivesSerializationExactly(string path)
    {
        var declaration = new SessionRootsDeclaration
        {
            Claude = SourceDecision.Watch(path),
            Codex = SourceDecision.Off,
        };

        using JsonDocument json = JsonDocument.Parse(declaration.SettingsJson()!);

        Assert.Equal(
            path,
            json.RootElement.GetProperty("claude_source").GetProperty("path").GetString());
    }

    /// <summary>
    /// A blank folder is not an answer. Collapsing it to Undecided keeps
    /// Continue disabled rather than sending a declaration the daemon rejects.
    /// </summary>
    [Theory]
    [InlineData("")]
    [InlineData("   ")]
    public void ABlankFolderIsNotADeclaration(string path)
    {
        Assert.Equal(SourceDecision.Undecided, SourceDecision.Watch(path));
        Assert.False(SourceDecision.Watch(path).IsDecided);
    }

    [Fact]
    public void AWatchedPathIsTrimmedRatherThanSentWithItsWhitespace()
    {
        Assert.Equal(@"C:\x", SourceDecision.Watch("  C:\\x  ").Path);
    }

    /// <summary>
    /// Off and Undecided are different states, and nothing may treat them as
    /// the same one.
    /// </summary>
    [Fact]
    public void OffIsAnAnswerAndUndecidedIsNot()
    {
        Assert.True(SourceDecision.Off.IsDecided);
        Assert.False(SourceDecision.Undecided.IsDecided);
        Assert.NotEqual(SourceDecision.Off, SourceDecision.Undecided);
    }

    /// <summary>
    /// The three discovery outcomes stay distinct. Collapsing "not there" and
    /// "there but empty" into one sentence would hide which of them the
    /// contributor is looking at.
    /// </summary>
    [Fact]
    public void EvidenceDistinguishesMissingEmptyAndPopulatedStores()
    {
        const string Path = @"C:\Users\z\.codex\sessions";

        string missing = SessionRootsCopy.Evidence(
            new SourceCandidate { Source = SourceDiscovery.Codex, Path = Path, Exists = false });
        string empty = SessionRootsCopy.Evidence(new SourceCandidate
        {
            Source = SourceDiscovery.Codex,
            Path = Path,
            Exists = true,
            SessionCount = 0,
        });
        string populated = SessionRootsCopy.Evidence(new SourceCandidate
        {
            Source = SourceDiscovery.Codex,
            Path = Path,
            Exists = true,
            SessionCount = 3066,
            MostRecent = DateTimeOffset.UtcNow.AddMinutes(-120),
        });

        Assert.NotEqual(missing, empty);
        Assert.NotEqual(empty, populated);
        Assert.Contains("3066", populated, StringComparison.Ordinal);
    }

    /// <summary>
    /// Discovery describing nothing at all is its own case.
    ///
    /// Distinct from "the conventional folder is not on this machine": there
    /// is not even a guess to show, so the sentence has to ask for a folder
    /// rather than report that one is missing.
    /// </summary>
    [Fact]
    public void ASourceDiscoveryCouldNotDescribeAsksForAFolder()
    {
        string undiscovered = SessionRootsCopy.Evidence(
            new SourceCandidate { Source = SourceDiscovery.Codex, Path = string.Empty });
        string absent = SessionRootsCopy.Evidence(new SourceCandidate
        {
            Source = SourceDiscovery.Codex,
            Path = @"C:\Users\z\.codex\sessions",
            Exists = false,
        });

        Assert.NotEqual(undiscovered, absent);
        Assert.Contains("Type the folder", undiscovered, StringComparison.Ordinal);
    }

    /// <summary>
    /// Typing a folder is a first-class answer, so its affordance is not
    /// conditional on discovery having failed.
    /// </summary>
    [Fact]
    public void TheManualPathHintIsUnconditional()
    {
        Assert.False(string.IsNullOrWhiteSpace(SessionRootsCopy.ManualHint));
    }

    /// <summary>
    /// A store that exists is never described as having no location.
    ///
    /// Regression: the no-location branch originally tested the path alone and
    /// ran first, so a discovered store with thousands of sessions was
    /// reported as "no location for this agent" -- which would have turned the
    /// one line that makes this screen a consent prompt back into an empty
    /// box. Exists can only be true if a directory was stat'd, so it outranks
    /// an empty path rather than the other way round.
    /// </summary>
    [Fact]
    public void AStoreThatExistsIsNeverReportedAsHavingNoLocation()
    {
        string sentence = SessionRootsCopy.Evidence(new SourceCandidate
        {
            Source = SourceDiscovery.Codex,
            Path = string.Empty,
            Exists = true,
            SessionCount = 3066,
            MostRecent = DateTimeOffset.UtcNow,
        });

        Assert.DoesNotContain("no location", sentence, StringComparison.Ordinal);
        Assert.Contains("3066", sentence, StringComparison.Ordinal);
    }

    [Fact]
    public void OneSessionIsNotPluralised()
    {
        string sentence = SessionRootsCopy.Evidence(new SourceCandidate
        {
            Source = SourceDiscovery.ClaudeCode,
            Path = @"C:\Users\z\.claude\projects",
            Exists = true,
            SessionCount = 1,
            MostRecent = DateTimeOffset.UtcNow,
        });

        Assert.Contains("1 session,", sentence, StringComparison.Ordinal);
    }

    /// <summary>
    /// The bands match <c>human_when</c> in the GTK shell's model.rs, so the
    /// two shells describe the same instant the same way.
    /// </summary>
    [Theory]
    [InlineData(0, "just now")]
    [InlineData(1, "just now")]
    [InlineData(2, "2 minutes ago")]
    [InlineData(59, "59 minutes ago")]
    [InlineData(60, "an hour ago")]
    [InlineData(119, "an hour ago")]
    [InlineData(120, "2 hours ago")]
    [InlineData(1439, "23 hours ago")]
    [InlineData(1440, "yesterday")]
    [InlineData(2879, "yesterday")]
    [InlineData(2880, "2 days ago")]
    public void ElapsedTimeIsDescribedInTheSameBandsAsTheLinuxShell(int minutes, string expected)
    {
        DateTimeOffset now = new(2026, 8, 19, 12, 0, 0, TimeSpan.Zero);

        Assert.Equal(expected, SessionRootsCopy.HumanWhen(now.AddMinutes(-minutes), now));
    }

    /// <summary>
    /// A store with no sessions has no timestamp, and answering "just now"
    /// about it would be a lie. The GTK helper says "just now" for None
    /// because it is describing a queue row that necessarily exists.
    /// </summary>
    [Fact]
    public void AMissingTimestampIsUnknownRatherThanJustNow()
    {
        Assert.Equal("unknown", SessionRootsCopy.HumanWhen(null));
    }

    /// <summary>
    /// A relocated store says which variable moved it, so an unfamiliar path
    /// reads as explained rather than as a mistake.
    /// </summary>
    [Fact]
    public void ARelocatedStoreNamesTheVariableThatMovedIt()
    {
        Assert.Contains(
            "CLAUDE_CONFIG_DIR",
            SessionRootsCopy.RelocatedNote(SourceDiscovery.ClaudeCode),
            StringComparison.Ordinal);
        Assert.Contains(
            "CODEX_HOME",
            SessionRootsCopy.RelocatedNote(SourceDiscovery.Codex),
            StringComparison.Ordinal);
    }

    [Fact]
    public void TheRefusalLabelIsRecognisedAndNothingElseIs()
    {
        Assert.True(
            new TcException("tc_daemon_start failed: roots-not-declared", "roots-not-declared")
                .IsRootsNotDeclared);
        Assert.False(
            new TcException("tc_daemon_start failed: daemon-start-failed", "daemon-start-failed")
                .IsRootsNotDeclared);
        Assert.False(new TcException("daemon-not-started").IsRootsNotDeclared);
    }
}
