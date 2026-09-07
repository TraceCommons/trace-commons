using System;
using System.Collections.Generic;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The harness list, the plan and the commit, decoded.
///
/// <para>
/// These mirror the macOS shell's cases on purpose: two shells decoding the
/// same payload by different rules would produce two behaviours nobody could
/// compare. Where the plan document's snippets quote an older field set
/// (<c>wired</c>, <c>facades</c>, <c>noop</c>, <c>digest</c>), the cases below
/// quote what <c>daemon/harness.rs</c> actually puts on the wire.
/// </para>
/// </summary>
public class HarnessSurfaceTests
{
    private const string OneRow =
        """
        {"catalog_present":false,"destination_port":8787,
         "harnesses":[{"id":"claude","name":"Claude Code","installed":true,
           "connected":false,"config_path":"C:\\Users\\x\\.claude\\settings.json",
           "connect_command":"ironwire connect claude","family":"anthropic",
           "state":"not_connected","last_call_at":null,
           "can_connect":true,"can_disconnect":false}],
         "activity":{"readable":true,"window_hours":24,"last_call_at":null,"families":[]}}
        """;

    [Fact]
    public void AMalformedRowIsDroppedRatherThanGuessed()
    {
        Assert.Empty(HarnessSurface.ParseListing("""{"harnesses":[{"name":"x"}]}""").Harnesses);
    }

    [Fact]
    public void AMalformedPayloadIsAnEmptyListRatherThanAThrow()
    {
        Assert.Empty(HarnessSurface.ParseListing("{not json").Harnesses);
        Assert.Empty(HarnessSurface.ParseListing(null).Harnesses);
        Assert.Empty(HarnessSurface.ParseListing("  ").Harnesses);
    }

    [Fact]
    public void TheRowCarriesTheFileItWouldChange()
    {
        HarnessListing listing = HarnessSurface.ParseListing(OneRow);
        Assert.Single(listing.Harnesses);
        Assert.Equal(@"C:\Users\x\.claude\settings.json", listing.Harnesses[0].ConfigPath);
        Assert.Equal("ironwire connect claude", listing.Harnesses[0].ConnectCommand);
        Assert.Equal("Claude Code", listing.Harnesses[0].Name);
    }

    /// <summary>
    /// The catalog channel is inert in this build, so the list is the tools
    /// this app was compiled knowing about. A surface that hid that would
    /// imply the machine has only these.
    /// </summary>
    [Fact]
    public void TheListSaysWhetherItCameFromACatalog()
    {
        Assert.False(HarnessSurface.ParseListing(OneRow).CatalogPresent);
    }

    [Fact]
    public void ANoopPlanIsDistinctFromAChange()
    {
        HarnessPlan? noop = HarnessSurface.ParsePlan(
            """{"id":"claude","action":"connect","outcome":"noop","plan_id":null,"path":"/p","changes":[],"occupied":[]}""");
        Assert.Equal(HarnessPlanOutcome.Noop, noop!.Outcome);
        Assert.False(noop.IsCommittable);

        HarnessPlan? real = HarnessSurface.ParsePlan(
            """{"id":"claude","action":"connect","outcome":"changes","plan_id":"a5f0e4a7-2a1f-4f3e-8b1e-0f6c1d2b3a44","path":"/p","changes":["set a thing"],"occupied":[]}""");
        Assert.Equal(HarnessPlanOutcome.Changes, real!.Outcome);
        Assert.True(real.IsCommittable);
        Assert.Equal(new[] { "set a thing" }, real.Changes);
        Assert.Equal("a5f0e4a7-2a1f-4f3e-8b1e-0f6c1d2b3a44", real.PlanId);
    }

    /// <summary>
    /// An unparseable config is a refusal that needs a human, not "nothing to
    /// change". A shell that collapsed the two would tell a contributor with a
    /// broken settings file that everything was fine.
    /// </summary>
    [Fact]
    public void ARefusedFileIsNotANoop()
    {
        HarnessPlan? refused = HarnessSurface.ParsePlan(
            """{"id":"claude","action":"connect","outcome":"unparseable","plan_id":null,"path":"/p","changes":[],"occupied":[]}""");
        Assert.Equal(HarnessPlanOutcome.Unparseable, refused!.Outcome);
        Assert.NotEqual(HarnessPlanOutcome.Noop, refused.Outcome);
        Assert.False(refused.IsCommittable);
    }

    /// <summary>
    /// A plan id is minted for a committable plan and for nothing else, so a
    /// shell cannot construct a write out of an outcome that refused one.
    /// </summary>
    [Fact]
    public void OnlyAChangeCarriesAPlanId()
    {
        foreach (string outcome in new[] { "noop", "unparseable", "not_installed", "entry_unusable", "no_config_path" })
        {
            HarnessPlan? plan = HarnessSurface.ParsePlan(
                $$"""{"id":"claude","action":"connect","outcome":"{{outcome}}","plan_id":null,"path":null,"changes":[],"occupied":[]}""");
            Assert.False(plan!.IsCommittable);
            Assert.Null(plan.PlanId);
        }
    }

    /// <summary>An outcome this build never heard of is not committable.</summary>
    [Fact]
    public void AnOutcomeFromALaterDaemonIsNotCommittable()
    {
        HarnessPlan? plan = HarnessSurface.ParsePlan(
            """{"id":"claude","action":"connect","outcome":"a_thing_invented_later","plan_id":"a5f0e4a7-2a1f-4f3e-8b1e-0f6c1d2b3a44","path":"/p","changes":[],"occupied":[]}""");
        Assert.Equal(HarnessPlanOutcome.Unknown, plan!.Outcome);
        Assert.False(plan.IsCommittable);
    }

    [Fact]
    public void AnOccupiedSlotSurvivesToTheScreen()
    {
        HarnessPlan? plan = HarnessSurface.ParsePlan(
            """{"id":"claude","action":"connect","outcome":"noop","plan_id":null,"path":"/p","changes":[],"occupied":[{"slot":"env.ANTHROPIC_BASE_URL","current":"https://theirs.example"}]}""");
        Assert.Equal("env.ANTHROPIC_BASE_URL", plan!.Occupied[0].Slot);
        Assert.Equal("https://theirs.example", plan.Occupied[0].Current);
    }

    /// <summary>
    /// Occupied rides alongside the outcome; it is not one. A plan can carry
    /// changes AND a slot that was left alone.
    /// </summary>
    [Fact]
    public void AnOccupiedSlotRidesAlongsideAChange()
    {
        HarnessPlan? plan = HarnessSurface.ParsePlan(
            """{"id":"claude","action":"connect","outcome":"changes","plan_id":"a5f0e4a7-2a1f-4f3e-8b1e-0f6c1d2b3a44","path":"/p","changes":["set a thing"],"occupied":[{"slot":"env.ANTHROPIC_BASE_URL","current":"https://theirs.example"}]}""");
        Assert.True(plan!.IsCommittable);
        Assert.Single(plan.Occupied);
    }

    [Fact]
    public void ANotInstalledToolCannotBeConnected()
    {
        var row = new HarnessRow { Id = "codex", Name = "Codex", Installed = false };
        Assert.False(row.CanConnect);
    }

    /// <summary>
    /// Uninstalling a coding tool does not remove the line we put in its
    /// config file, so a connected tool may always be disconnected.
    /// </summary>
    [Fact]
    public void AConnectedToolMayBeDisconnectedEvenUninstalled()
    {
        Assert.True(HarnessSurface.ActionAvailable(HarnessSurface.Disconnect, installed: false, connected: true));
        Assert.False(HarnessSurface.ActionAvailable(HarnessSurface.Connect, installed: false, connected: true));
        Assert.False(HarnessSurface.ActionAvailable("rewire", installed: true, connected: true));
    }

    /// <summary>The daemon's own answer and the ABI's agree, per row.</summary>
    [Fact]
    public void TheRowsAnswerMatchesTheSharedBranchTable()
    {
        HarnessRow row = HarnessSurface.ParseListing(OneRow).Harnesses[0];
        Assert.Equal(
            HarnessSurface.ActionAvailable(HarnessSurface.Connect, row.Installed, row.Connected),
            row.CanConnect);
        Assert.Equal(
            HarnessSurface.ActionAvailable(HarnessSurface.Disconnect, row.Installed, row.Connected),
            row.CanDisconnect);
    }

    /// Only the clear tone may be painted as working, on a harness row as anywhere.
    [Fact]
    public void OnlyAnAnsweringHarnessReadsAsWorking()
    {
        Assert.True(HarnessSurface.ReadsAsWorking(HarnessSurface.State("answering")));
        foreach (string label in new[]
        {
            "not_connected", "connected_no_calls", "activity_shared", "unknown",
            "", "a_state_from_a_later_daemon",
        })
        {
            Assert.False(HarnessSurface.ReadsAsWorking(HarnessSurface.State(label)));
        }
    }

    /// <summary>
    /// A call arrived in this tool's protocol family and more than one
    /// connected tool speaks it. The sentence that says a call was answered
    /// is still true of this computer; what may not happen is painting the
    /// row as this tool working.
    /// </summary>
    [Fact]
    public void ASharedFamilyIsItsOwnStateAndNotAFlavourOfAnswering()
    {
        Assert.Equal(HarnessState.ActivityShared, HarnessSurface.State("activity_shared"));
        Assert.NotEqual(HarnessState.Answering, HarnessSurface.State("activity_shared"));
    }

    [Fact]
    public void AStateFromALaterDaemonIsUnknown()
    {
        Assert.Equal(HarnessState.Unknown, HarnessSurface.State("a_state_from_a_later_daemon"));
        Assert.Equal(HarnessState.Unknown, HarnessSurface.State(null));
    }

    /// <summary>
    /// The shell cannot construct a write. A commit carries the plan id the
    /// daemon minted and nothing else -- no id, no action, no path.
    /// </summary>
    [Fact]
    public void ACommitCarriesOnlyThePlanTheDaemonMinted()
    {
        string body = HarnessSurface.SerializeCommit("a5f0e4a7-2a1f-4f3e-8b1e-0f6c1d2b3a44");
        Assert.Equal("""{"plan_id":"a5f0e4a7-2a1f-4f3e-8b1e-0f6c1d2b3a44"}""", body);
    }

    [Fact]
    public void APlanRequestNamesTheToolAndTheDirection()
    {
        Assert.Equal(
            """{"id":"claude","action":"connect"}""",
            HarnessSurface.SerializePlan("claude", HarnessSurface.Connect));
        Assert.Equal(
            """{"id":"codex","action":"disconnect"}""",
            HarnessSurface.SerializePlan("codex", HarnessSurface.Disconnect));
    }

    /// <summary>
    /// Every commit refusal is the same refusal, and none of them leaves
    /// anything to commit again.
    /// </summary>
    /// <remarks>
    /// The daemon takes the plan out of its store BEFORE it re-checks the
    /// file digest and before it writes, so an expired plan, a spent one, a
    /// file that moved underneath it and a write that failed all end with no
    /// plan and nothing written. A shell that told these apart would be
    /// offering a retry against nothing, or re-planning a write on the
    /// contributor's behalf that they were never shown.
    /// </remarks>
    [Fact]
    public void NoCommitRefusalIsRetryable()
    {
        foreach (string message in new[]
        {
            "harness-plan-unknown", "harness-config-changed", "harness-commit-failed",
            "something-a-later-daemon-says",
        })
        {
            Assert.False(HarnessSurface.CommitIsRetryable(
                new DaemonError { Code = "unavailable", Message = message }));
        }

        Assert.False(HarnessSurface.CommitIsRetryable(null));
    }

    /// <summary>
    /// A connect cannot be planned when nothing answers here. The daemon says
    /// so by name rather than by an outcome, because it is a fact about this
    /// computer and not about the tool's file.
    /// </summary>
    [Fact]
    public void AConnectWithNoDestinationIsRecognised()
    {
        Assert.True(HarnessSurface.NoDestination(new DaemonError { Code = "bad-params", Message = "harness-no-destination" }));
        Assert.False(HarnessSurface.NoDestination(new DaemonError { Code = "bad-params", Message = "harness-unknown" }));
    }

    [Fact]
    public void ACommitAnswerCarriesTheFileAndTheCopyItPreserved()
    {
        HarnessCommit? commit = HarnessSurface.ParseCommit(
            """{"id":"claude","action":"connect","committed":true,"path":"/p","backup_path":"/p.ironwire-backup"}""");
        Assert.True(commit!.Committed);
        Assert.Equal("/p", commit.Path);
        Assert.Equal("/p.ironwire-backup", commit.BackupPath);
        Assert.Null(HarnessSurface.ParseCommit("{nope"));
    }

    /// <summary>
    /// Every sentence a harness row shows comes off the payload, through the
    /// one shared table. Nothing here composes one, so an unknown state shows
    /// no sentence rather than a guessed one.
    /// </summary>
    /// <remarks>
    /// The expected values are read off <see cref="PrivateInferenceCopy"/>
    /// rather than typed here, so this test cannot become the place a
    /// sentence is written. What it proves is that
    /// <c>tc_harness_state_line</c> -- the same call the macOS and GNOME
    /// shells make -- answers the payload's own field for each state.
    /// </remarks>
    [Fact]
    public void EveryStateSentenceComesFromThePayload()
    {
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        Assert.NotNull(copy);

        var expected = new Dictionary<string, string>
        {
            ["not_connected"] = copy!.HarnessNotConnected,
            ["connected_no_calls"] = copy.HarnessConnectedNothingSeen,
            ["answering"] = copy.HarnessAnswering,
        };

        foreach (KeyValuePair<string, string> pair in expected)
        {
            Assert.Equal(pair.Value, HarnessSurface.StateSentence(pair.Key));
        }
    }

    /// <summary>
    /// Every outcome that writes nothing explains itself, and it is the
    /// shared table that says how.
    /// </summary>
    /// <remarks>
    /// The expected values are read off <see cref="PrivateInferenceCopy"/>
    /// rather than typed here, so this cannot become the place a sentence is
    /// written. Four of these five outcomes had no sentence at all before,
    /// and their preview opened with a title, a path, no changes, no
    /// explanation and a way out.
    /// </remarks>
    [Fact]
    public void EveryOutcomeThatChangesNothingSaysWhy()
    {
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        Assert.NotNull(copy);

        var expected = new Dictionary<string, string>
        {
            ["noop"] = copy!.HarnessPlanNothingToChange,
            ["unparseable"] = copy.HarnessUnreadableConfig,
            ["not_installed"] = copy.HarnessNotInstalled,
            ["entry_unusable"] = copy.HarnessPlanEntryUnusable,
            ["no_config_path"] = copy.HarnessPlanNoConfigPath,
        };

        foreach (KeyValuePair<string, string> pair in expected)
        {
            Assert.False(string.IsNullOrWhiteSpace(HarnessSurface.OutcomeSentence(pair.Key)));
            Assert.Equal(pair.Value, HarnessSurface.OutcomeSentence(pair.Key));
        }

        // A plan with changes shows them; a sentence above them would be this
        // app narrating its own list. An outcome from a later daemon may not
        // borrow the nearest refusal either.
        foreach (string silent in new[] { "changes", "an_outcome_from_a_later_daemon", "" })
        {
            Assert.Equal(string.Empty, HarnessSurface.OutcomeSentence(silent));
        }
    }

    /// <summary>
    /// A tool that is not on this machine says so, and does not keep a
    /// sentence about settings it does not have.
    /// </summary>
    /// <remarks>
    /// Before this the two rendered identically: same not-connected sentence,
    /// connect button simply absent, nothing saying why.
    /// </remarks>
    [Fact]
    public void AMissingToolSaysSoRatherThanBorrowingTheNotConnectedSentence()
    {
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        Assert.NotNull(copy);

        HarnessListing present = HarnessSurface.ParseListing(OneRow);
        Assert.True(present.Harnesses[0].Installed);
        Assert.Equal(copy!.HarnessNotConnected, HarnessSurface.RowSentence(present.Harnesses[0], copy));

        HarnessListing missing = HarnessSurface.ParseListing(
            OneRow.Replace("\"installed\":true", "\"installed\":false", StringComparison.Ordinal));
        Assert.False(missing.Harnesses[0].Installed);
        Assert.Equal(copy.HarnessNotInstalled, HarnessSurface.RowSentence(missing.Harnesses[0], copy));
        Assert.NotEqual(copy.HarnessNotConnected, HarnessSurface.RowSentence(missing.Harnesses[0], copy));
    }

    /// <summary>
    /// The when-line crosses the ABI, so this shell draws it at all.
    /// </summary>
    /// <remarks>
    /// It used to exist in Rust and stop there: the GNOME shell links the
    /// crate and rendered it, and this one rendered nothing for the same row.
    /// An absent or unparseable timestamp crosses as a negative number, which
    /// is the shared "nothing to report" and comes back empty.
    /// </remarks>
    [Fact]
    public void TheWhenLineIsAssembledOnTheFarSide()
    {
        var now = new DateTimeOffset(2026, 1, 1, 0, 2, 0, TimeSpan.Zero);
        Assert.Contains(
            "2 minutes ago",
            HarnessSurface.LastCallSentence("2026-01-01T00:00:00Z", now),
            StringComparison.Ordinal);
        Assert.Contains(
            "just now",
            HarnessSurface.LastCallSentence("2026-01-01T00:01:30Z", now),
            StringComparison.Ordinal);

        foreach (string? absent in new[] { null, string.Empty, "not a timestamp" })
        {
            Assert.Equal(string.Empty, HarnessSurface.LastCallSentence(absent, now));
        }

        // A timestamp in the future is nothing to report, not a call at time
        // zero: a clock that ran backwards must not read as "just now".
        Assert.Equal(
            string.Empty, HarnessSurface.LastCallSentence("2027-01-01T00:00:00Z", now));
    }

    /// <summary>
    /// The amount is assembled on the far side, and an unmeasured figure
    /// draws nothing rather than a zero.
    /// </summary>
    /// <remarks>
    /// The assertion that carries the weight is the last one. A listing whose
    /// <c>spend.known</c> is false must produce the empty string, and it must
    /// NOT produce the sentence a measured zero gets: "nobody could tell" and
    /// "nothing was spent" are different facts, and rendering the first as
    /// $0.00 is the defect this whole block exists to prevent.
    /// </remarks>
    [Fact]
    public void TheAmountIsAssembledOnTheFarSideAndUnknownDrawsNothing()
    {
        HarnessListing measured = HarnessSurface.ParseListing(
            """
            {"catalog_present":false,"harnesses":[],
             "activity":{"readable":true,"window_hours":24,"last_call_at":null,"families":[]},
             "spend":{"known":true,"micros":1230000}}
            """);
        Assert.True(measured.SpendKnown);
        Assert.Equal(1_230_000UL, measured.SpendMicros);
        Assert.Contains("$1.23", HarnessSurface.SpendSentence(measured), StringComparison.Ordinal);
        Assert.Contains(
            "since midnight", HarnessSurface.SpendSentence(measured), StringComparison.Ordinal);

        HarnessListing zero = HarnessSurface.ParseListing(
            """
            {"catalog_present":false,"harnesses":[],
             "activity":{"readable":true,"window_hours":24,"last_call_at":null,"families":[]},
             "spend":{"known":true,"micros":0}}
            """);
        Assert.Contains("$0.00", HarnessSurface.SpendSentence(zero), StringComparison.Ordinal);

        foreach (string body in new[]
        {
            """{"harnesses":[],"spend":{"known":false,"micros":null}}""",
            """{"harnesses":[],"spend":{"known":true,"micros":null}}""",
            """{"harnesses":[]}""",
        })
        {
            HarnessListing unknown = HarnessSurface.ParseListing(body);
            Assert.False(unknown.SpendKnown);
            Assert.Equal(string.Empty, HarnessSurface.SpendSentence(unknown));
        }

        Assert.NotEqual(HarnessSurface.SpendSentence(zero), HarnessSurface.SpendSentence(HarnessListing.Empty));
    }

    /// <summary>
    /// Two states have no sentence on the payload, and neither may borrow one.
    /// </summary>
    /// <remarks>
    /// <c>HarnessAnswering</c> would claim an attribution the ledger cannot
    /// make -- it records a protocol family, and a family two connected tools
    /// both speak names neither of them. <c>HarnessConnectedNothingSeen</c>
    /// would be flatly false: something did arrive. So the row draws no state
    /// line, at a tone that is not the working one. This test is what fails if
    /// someone later fills the gap by pointing at the nearest sentence instead
    /// of adding the missing one.
    /// </remarks>
    [Fact]
    public void TheTwoStatesWithNoSentenceBorrowNeither()
    {
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        Assert.NotNull(copy);

        // A label from a later daemon is the same shape, and must not fall
        // through to the nearest sentence either.
        foreach (string label in new[] { "activity_shared", "unknown", "a_state_from_a_later_daemon" })
        {
            string sentence = HarnessSurface.StateSentence(label);
            Assert.Equal(string.Empty, sentence);
            Assert.NotEqual(copy!.HarnessAnswering, sentence);
            Assert.NotEqual(copy.HarnessConnectedNothingSeen, sentence);
            Assert.False(HarnessSurface.ReadsAsWorking(HarnessSurface.State(label)));
        }
    }

    /// <summary>
    /// A timestamp with fractional seconds does not empty the list.
    /// </summary>
    /// <remarks>
    /// The daemon writes <c>to_rfc3339()</c>, which carries fractional
    /// seconds, and <c>last_call_at</c> sits INSIDE the list envelope -- so a
    /// decoder strict enough to reject it loses every tool, not one field.
    /// This shell carries the value as the string the daemon wrote and never
    /// parses it, because it renders no sentence about when: the one that
    /// exists lives in the Rust and is not exported across the ABI. That makes
    /// this a guard on the decision, not on a parser, and it is the thing that
    /// fails if someone later reaches for a strict date type here.
    /// </remarks>
    [Theory]
    [InlineData("2026-09-07T11:22:33.123456789+00:00")]
    [InlineData("2026-09-07T11:22:33.123456789Z")]
    [InlineData("2026-09-07T11:22:33+00:00")]
    [InlineData("2026-09-07T11:22:33.5-07:00")]
    public void AFractionalSecondTimestampKeepsTheWholeList(string stamp)
    {
        const string template =
            """
            {"catalog_present":false,"destination_port":8787,
             "harnesses":[{"id":"claude","name":"Claude Code","installed":true,
               "connected":true,"config_path":"/p","connect_command":"c",
               "family":"anthropic","state":"answering","last_call_at":"STAMP",
               "can_connect":false,"can_disconnect":true}],
             "activity":{"readable":true,"window_hours":24,
               "last_call_at":"STAMP","families":[]}}
            """;

        IReadOnlyList<HarnessRow> rows =
            HarnessSurface.ParseRows(template.Replace("STAMP", stamp, StringComparison.Ordinal));

        Assert.Single(rows);
        Assert.Equal(stamp, rows[0].LastCallAt);
        Assert.Equal(HarnessState.Answering, rows[0].State);
    }

    /// <summary>
    /// The exposure question is asked off the LISTENER, not off the marker,
    /// and is therefore wider than the first-run offer.
    /// </summary>
    /// <remarks>
    /// A contributor who accepted once and has since used the kill switch is
    /// making the exposure decision afresh: a connect would reopen the
    /// listener they turned off. The first-run offer's own branch stays
    /// exactly as it is -- this is a second, wider question, in the shape the
    /// other two shells use.
    /// </remarks>
    [Fact]
    public void AConnectThatWouldReopenTheListenerAsksAgain()
    {
        Assert.True(HarnessSurface.ConnectNeedsExposure(listenerOn: false));
        Assert.False(HarnessSurface.ConnectNeedsExposure(listenerOn: true));
    }

    /// <summary>
    /// The gate is wider than <c>ShouldOffer</c>, which is why it exists
    /// separately rather than calling it.
    /// </summary>
    [Fact]
    public void TheConnectGateIsWiderThanTheFirstRunOffer()
    {
        // Answered, and then switched off. The first-run offer is done with
        // this contributor; a connect is not.
        Assert.False(PrivateInferenceSurface.ShouldOffer(known: true, answered: true, on: false));
        Assert.True(HarnessSurface.ConnectNeedsExposure(listenerOn: false));
    }
}
