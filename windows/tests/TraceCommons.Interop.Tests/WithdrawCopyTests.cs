using System;
using System.Collections.Generic;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Withdrawal's copy and its one decision.
///
/// This is the reason <see cref="WithdrawCopy"/> lives in the interop
/// assembly rather than in a view model. Withdrawal is the one place in this
/// product where a plausible-sounding phrase becomes a false promise about
/// erasure, and the three confirmation bodies belong to
/// <c>docs/contributor-daemon-ipc-v1_1.md</c>'s "Canonical confirmation copy"
/// table rather than to this shell. Checked here, they are checked on a
/// machine that cannot build WinUI at all.
///
/// The Linux shell holds the identical assertions in
/// <c>crates/trace-commons-contributor-gtk/src/copy.rs</c>. The two shells
/// must not diverge, and these tests are what stops one of them drifting.
/// </summary>
public sealed class WithdrawCopyTests
{
    [Fact]
    public void TheCanonicalBodiesAreStillTheDocumentsOwnWords()
    {
        // Transcribed from the "Canonical confirmation copy" table.
        // Compared WHOLE rather than by keyword: a paraphrase that kept every
        // keyword would still be a paraphrase, and paraphrasing is the exact
        // failure the table exists to prevent.
        Assert.Equal(
            "This trace never entered the commons. Withdrawing deletes it. Nothing was "
            + "distributed and nothing needs recalling.",
            WithdrawCopy.BodyNotDistributed);

        Assert.Equal(
            "This trace is in the commons but has not been included in any published export or "
            + "benchmark yet. Withdrawing deletes it and excludes it from everything published "
            + "from here on.",
            WithdrawCopy.BodyCommonsNotDistributed);

        Assert.Equal(
            "This trace has already been included in a published export or benchmark. "
            + "Withdrawing deletes our copy and excludes it from everything published from here "
            + "on, but copies that have already been distributed cannot be recalled. Withdrawing "
            + "does not undo that.",
            WithdrawCopy.BodyCommonsDistributed);
    }

    [Fact]
    public void EachTierLooksUpItsOwnBodyAndAFutureTierLooksUpNothing()
    {
        Assert.Equal(
            WithdrawCopy.BodyNotDistributed,
            WithdrawCopy.CanonicalBody(WithdrawCopy.ReachNotDistributed));
        Assert.Equal(
            WithdrawCopy.BodyCommonsNotDistributed,
            WithdrawCopy.CanonicalBody(WithdrawCopy.ReachCommonsNotDistributed));
        Assert.Equal(
            WithdrawCopy.BodyCommonsDistributed,
            WithdrawCopy.CanonicalBody(WithdrawCopy.ReachCommonsDistributed));

        // A tier from a newer server is unrecognised, not mis-assigned. This
        // is why the reach is carried as a wire string rather than parsed
        // into an enum that would have to guess.
        Assert.Null(WithdrawCopy.CanonicalBody("a-tier-from-the-future"));
        Assert.Null(WithdrawCopy.CanonicalBody(null));
    }

    [Theory]
    [InlineData("submitted", WithdrawStage.NotInTheCommons)]
    [InlineData("quarantined", WithdrawStage.NotInTheCommons)]
    [InlineData("accepted", WithdrawStage.InTheCommons)]
    [InlineData("withdrawn", WithdrawStage.Unknown)]
    [InlineData("something-new", WithdrawStage.Unknown)]
    [InlineData(null, WithdrawStage.Unknown)]
    public void TheStageIsReadOffTheLocalStatusAndNothingElse(
        string? status,
        WithdrawStage expected)
    {
        // The client holds only `status`; the tier is computed on the server
        // DURING the call. This mapping is the whole of what a client may
        // honestly claim beforehand.
        Assert.Equal(expected, WithdrawStageExtensions.StageOf(status));
    }

    [Fact]
    public void ATraceAlreadyInTheCommonsIsNeverShownOnlyTheGentlerTier()
    {
        // Rule 2. `accepted` may resolve to EITHER commons tier and this
        // window cannot tell which, so showing only the gentler body would be
        // claiming more erasure than may have been achieved.
        WithdrawConfirmation commons = WithdrawCopy.Confirmation(WithdrawStage.InTheCommons);

        Assert.Equal(
            new[]
            {
                WithdrawCopy.BodyCommonsNotDistributed,
                WithdrawCopy.BodyCommonsDistributed,
            },
            commons.Bodies);

        // And the distributed one is the one given the greater weight.
        Assert.Equal(1, commons.Gravest);

        // The contributor is told plainly that the outcome is decided on the
        // server, rather than being shown two bodies with no explanation of
        // why there are two.
        Assert.NotNull(commons.Ambiguity);
        Assert.Contains("decided on the server", commons.Ambiguity!, StringComparison.Ordinal);

        Assert.Equal(WithdrawCopy.WithdrawAnyway, commons.ConfirmLabel);
    }

    [Fact]
    public void ATraceThatNeverEnteredTheCommonsIsNotToldItWasExcluded()
    {
        // The other half of rule 2: `submitted`/`quarantined` maps to
        // `not_distributed` exactly -- that is the server's own rule -- so the
        // gentlest body is shown alone and no export it was never in is
        // mentioned.
        WithdrawConfirmation outside = WithdrawCopy.Confirmation(WithdrawStage.NotInTheCommons);

        Assert.Equal(new[] { WithdrawCopy.BodyNotDistributed }, outside.Bodies);
        Assert.Null(outside.Gravest);
        Assert.Null(outside.Ambiguity);
        Assert.Equal(WithdrawCopy.Withdraw, outside.ConfirmLabel);
    }

    [Fact]
    public void AnUnrecognisedStageCannotRuleOutTheFurthestReach()
    {
        WithdrawConfirmation unknown = WithdrawCopy.Confirmation(WithdrawStage.Unknown);

        Assert.Equal(new[] { WithdrawCopy.BodyCommonsDistributed }, unknown.Bodies);
        Assert.Equal(0, unknown.Gravest);
        Assert.Equal(WithdrawCopy.WithdrawAnyway, unknown.ConfirmLabel);
    }

    [Fact]
    public void EveryConfirmationCarriesTheCannotBeRecalledClauseUnlessTheTierRulesItOut()
    {
        // Only the one stage the server's own rule pins to `not_distributed`
        // may omit it. Both others must carry it, because both others could
        // turn out to be `commons_distributed`.
        foreach (WithdrawStage stage in new[] { WithdrawStage.InTheCommons, WithdrawStage.Unknown })
        {
            Assert.Contains(WithdrawCopy.BodyCommonsDistributed, WithdrawCopy.Confirmation(stage).Bodies);
        }

        Assert.DoesNotContain(
            WithdrawCopy.BodyCommonsDistributed,
            WithdrawCopy.Confirmation(WithdrawStage.NotInTheCommons).Bodies);
    }

    [Fact]
    public void EveryTierStatesTheSameVerifiedThingAboutCredit()
    {
        // Rule 3. Credit already recorded stays recorded, and no tier says
        // anything else about it. Nothing here may imply a claw-back.
        foreach (WithdrawStage stage in new[]
                 {
                     WithdrawStage.NotInTheCommons,
                     WithdrawStage.InTheCommons,
                     WithdrawStage.Unknown,
                 })
        {
            Assert.Equal(WithdrawCopy.CreditNote, WithdrawCopy.Confirmation(stage).Credit);
        }

        Assert.Equal("Credit already recorded stays.", WithdrawCopy.CreditNote);
    }

    [Fact]
    public void NoOutcomeIsEverReportedAsABareWithdrawn()
    {
        // Rule 1, the one the whole table exists to enforce. Each tier's
        // report carries that tier's canonical body whole.
        foreach (string reach in new[]
                 {
                     WithdrawCopy.ReachNotDistributed,
                     WithdrawCopy.ReachCommonsNotDistributed,
                     WithdrawCopy.ReachCommonsDistributed,
                 })
        {
            string sentence = WithdrawCopy.ResultSentence(reach);
            Assert.Contains(WithdrawCopy.CanonicalBody(reach)!, sentence, StringComparison.Ordinal);

            // "Withdrawn." alone is never the whole message.
            Assert.NotEqual("Withdrawn.", sentence);
        }

        // And an unknown tier is not smoothed into the mild answer: the
        // withdrawal happened, but how far the trace travelled cannot be
        // stated, so the furthest reach is not ruled out.
        Assert.Contains("cannot be recalled", WithdrawCopy.ResultSentence(null), StringComparison.Ordinal);
        Assert.Contains(
            "cannot be recalled",
            WithdrawCopy.ResultSentence("a-tier-from-the-future"),
            StringComparison.Ordinal);
    }

    [Fact]
    public void TheDistributedReportNeverImpliesDistributedCopiesWereRetrieved()
    {
        // Rule 2 again, on the after-the-fact side. The clause from "but
        // copies" onward is the one sentence in this feature that must never
        // be softened, shortened, or quietly dropped.
        string sentence = WithdrawCopy.ResultSentence(WithdrawCopy.ReachCommonsDistributed);

        Assert.Contains(
            "copies that have already been distributed cannot be recalled",
            sentence,
            StringComparison.Ordinal);
        Assert.Contains("Withdrawing does not undo that.", sentence, StringComparison.Ordinal);

        string lower = sentence.ToLowerInvariant();
        foreach (string forbidden in new[] { "recalled all", "retrieved", "erased everywhere", "fully deleted" })
        {
            Assert.DoesNotContain(forbidden, lower, StringComparison.Ordinal);
        }
    }

    [Fact]
    public void AFailedWithdrawalOpensBySayingNothingHappened()
    {
        // A contributor must not walk away from a failure believing their
        // trace was taken back, whichever failure it was.
        foreach (string sentence in new[]
                 {
                     WithdrawCopy.AccountSessionRequired,
                     WithdrawCopy.NotFound,
                     WithdrawCopy.FailureSentence("withdraw-failed"),
                     WithdrawCopy.FailureSentence("account-session-required"),
                     WithdrawCopy.FailureSentence("not-found"),
                     WithdrawCopy.FailureSentence("not_found"),
                     WithdrawCopy.FailureSentence("submission-not-found"),
                     WithdrawCopy.FailureSentence(null),
                     WithdrawCopy.FailureSentence(string.Empty),
                 })
        {
            Assert.StartsWith("Nothing was withdrawn", sentence, StringComparison.Ordinal);
        }
    }

    [Fact]
    public void TheFailureContributorsActuallyHitIsExplainedInFull()
    {
        // `daemon/withdraw.rs` answers `account-session-required` before ever
        // attempting the call, always, because the daemon holds a device key
        // and never an account session. So this is the path contributors will
        // actually take, and it gets the whole explanatory sentence rather
        // than a bare label.
        string sentence = WithdrawCopy.FailureSentence(WithdrawCopy.AccountSessionRequiredLabel);

        Assert.Equal(WithdrawCopy.AccountSessionRequired, sentence);
        Assert.DoesNotContain("account-session-required", sentence, StringComparison.Ordinal);
        Assert.Contains("authenticated by your Trace Commons account", sentence, StringComparison.Ordinal);
        Assert.Contains("no account sign-in yet", sentence, StringComparison.Ordinal);
    }

    [Fact]
    public void TheNotFoundSentenceDisclosesNeitherExistenceNorOwnership()
    {
        // Rule 4: the server answers identically whether a submission belongs
        // to somebody else or does not exist, so that accounts cannot be
        // enumerated. This window must not undo that by guessing out loud.
        string lower = WithdrawCopy.NotFound.ToLowerInvariant();

        Assert.DoesNotContain("belongs to", lower, StringComparison.Ordinal);
        Assert.DoesNotContain("does not exist", lower, StringComparison.Ordinal);
        Assert.DoesNotContain("someone else", lower, StringComparison.Ordinal);
        Assert.Contains("no trace with that id under your account", lower, StringComparison.Ordinal);
    }

    [Fact]
    public void BulkWithdrawalIsRefusedInWordsRatherThanLeftSilentlyMissing()
    {
        // Rule 6. `withdraw_bulk` reports only counts, so rule 1 cannot be
        // honoured for it at all -- and the shared design draws a button
        // here, so its absence is explained rather than left as a gap.
        Assert.Contains("only", WithdrawCopy.NoBulk, StringComparison.Ordinal);
        Assert.Contains("how many succeeded", WithdrawCopy.NoBulk, StringComparison.Ordinal);
        Assert.Contains("one at a time", WithdrawCopy.NoBulk, StringComparison.Ordinal);
    }

    [Fact]
    public void AWithdrawnRecordIsNotOfferedWithdrawalAgain()
    {
        // A withdrawn record stays on the list reading as withdrawn: it is
        // never dropped and never re-labelled as something that failed. There
        // is simply nothing left for a button to do.
        Assert.False(WithdrawCopy.OffersWithdrawal("withdrawn", "sub-1"));

        // And a record with no submission id gets no button either: `withdraw`
        // takes exactly that id, so the button would have nothing to send and
        // would fail for a reason the contributor could do nothing about.
        Assert.False(WithdrawCopy.OffersWithdrawal("accepted", null));
        Assert.False(WithdrawCopy.OffersWithdrawal("accepted", "   "));

        foreach (string status in new[] { "submitted", "quarantined", "accepted", "something-new" })
        {
            Assert.True(WithdrawCopy.OffersWithdrawal(status, "sub-1"));
        }
    }

    [Fact]
    public void TheServersTierIsReadOffTheWireResponse()
    {
        // The shape `withdraw` actually answers with, so the after-the-fact
        // report is driven by what the server applied rather than by what the
        // client guessed.
        DaemonResponse response = DaemonResponse.Parse(
            "{\"id\":1,\"result\":{\"withdrawn\":true,\"distribution_reach\":\"commons_distributed\"}}");

        WithdrawResult? result = response.ResultAs<WithdrawResult>();

        Assert.NotNull(result);
        Assert.True(result!.Withdrawn);
        Assert.Equal(WithdrawCopy.ReachCommonsDistributed, result.DistributionReach);
        Assert.Equal(
            WithdrawCopy.BodyCommonsDistributed,
            WithdrawCopy.CanonicalBody(result.DistributionReach));
    }

    [Fact]
    public void AnErrorFrameCarriesTheLabelTheFailureSentenceIsKeyedOn()
    {
        // The daemon puts `account-session-required` in the error's MESSAGE;
        // the CODE is the generic `unavailable`. Keying the sentence on the
        // code would collapse every unavailable failure into one.
        DaemonResponse response = DaemonResponse.Parse(
            "{\"id\":1,\"error\":{\"code\":\"unavailable\",\"message\":\"account-session-required\"}}");

        Assert.True(response.IsError);
        Assert.Equal(WithdrawCopy.AccountSessionRequiredLabel, response.Error!.Message);
        Assert.Equal(
            WithdrawCopy.AccountSessionRequired,
            WithdrawCopy.FailureSentence(response.Error.Message));
    }

    [Fact]
    public void NoWithdrawalStringNamesAnInternalMechanism()
    {
        // The same rule the rest of the contributor copy holds to: "privacy
        // filter", "claim", "ingest", "canary" are internal words, and a
        // sentence about someone's own trace is the last place to start
        // using them.
        var everything = new List<string>
        {
            WithdrawCopy.BodyNotDistributed,
            WithdrawCopy.BodyCommonsNotDistributed,
            WithdrawCopy.BodyCommonsDistributed,
            WithdrawCopy.CreditNote,
            WithdrawCopy.AccountSessionRequired,
            WithdrawCopy.NotFound,
            WithdrawCopy.NoBulk,
            WithdrawCopy.AmbiguityInTheCommons,
            WithdrawCopy.AmbiguityUnknown,
            WithdrawCopy.ResultSentence(null),
        };

        foreach (string sentence in everything)
        {
            string lower = sentence.ToLowerInvariant();
            foreach (string forbidden in new[] { "privacy filter", "canary", "ingest", "claim" })
            {
                Assert.DoesNotContain(forbidden, lower, StringComparison.Ordinal);
            }
        }
    }
}

/// <summary>
/// The history surface: the wire shapes it reads and the words it puts on
/// them.
/// </summary>
public sealed class HistoryCopyTests
{
    [Fact]
    public void AWithdrawnRecordReadsAsWithdrawnAndNotAsAFailure()
    {
        // The rule this exists to hold: a withdrawn trace stays on the list
        // and says who withdrew it. It is never re-labelled as something that
        // failed, and it is never dropped.
        Assert.Equal(HistoryCopy.WithdrawnByYou, HistoryCopy.StatusWord("withdrawn"));
        Assert.Equal("Withdrawn by you", HistoryCopy.WithdrawnByYou);
    }

    [Fact]
    public void QuarantineNeverReadsAsARefusal()
    {
        // Held is held. A contributor who sees it grouped with failures reads
        // it as rejection, which is the misreading this word exists to stop.
        Assert.Equal(HistoryCopy.QuarantineHeading, HistoryCopy.StatusWord("quarantined"));
        Assert.Equal("Held for privacy review", HistoryCopy.QuarantineHeading);

        string text = (HistoryCopy.QuarantineHeading + " " + HistoryCopy.QuarantineBody)
            .ToLowerInvariant();

        // The word appears exactly once, in the sentence denying it.
        Assert.Contains("have not been rejected", text, StringComparison.Ordinal);

        // And no turnaround time is ever stated, because nobody can.
        foreach (string forbidden in new[] { "48 hours", "business days", "within a week", "usually takes" })
        {
            Assert.DoesNotContain(forbidden, text, StringComparison.Ordinal);
        }
    }

    [Theory]
    [InlineData("accepted", "In the commons")]
    [InlineData("submitted", "Waiting to be scored")]
    [InlineData("a-status-from-the-future", "Waiting to be scored")]
    [InlineData(null, "Waiting to be scored")]
    public void AnUnknownStatusDegradesToWaitingRatherThanToAFailure(
        string? status,
        string expected)
    {
        Assert.Equal(expected, HistoryCopy.StatusWord(status));
    }

    [Fact]
    public void CreditCopyCarriesNoCurrencyProjectionOrDate()
    {
        // Credit is a record, never a currency.
        foreach (string forbidden in new[] { "$", "USD", "worth", "value of", "by 20", "payout of" })
        {
            Assert.DoesNotContain(forbidden, HistoryCopy.CreditBody, StringComparison.Ordinal);
        }
    }

    [Fact]
    public void ASettledFigureIsReportedAsSettledAndAPendingOneIsNot()
    {
        // Recorded credit is final; anything still being scored is stated as
        // such rather than added to it, and neither carries a symbol.
        Assert.Equal("credit 4.5", HistoryCopy.CreditLine(4.5f, 9f));
        Assert.Equal("credit 2.0, still being scored", HistoryCopy.CreditLine(null, 2f));

        // Nothing to state is stated as nothing, not as a zero.
        Assert.Null(HistoryCopy.CreditLine(null, 0f));
    }

    [Fact]
    public void AWithdrawnRecordKeepsWhateverCreditItRecorded()
    {
        // Rule 3 on the history row: withdrawal does not reverse settled
        // credit, so the figure is drawn exactly as it would be on any other
        // record. There is no branch here on status, and there must not be.
        Assert.Equal("credit 7.5", HistoryCopy.CreditLine(7.5f, 0f));
    }

    [Fact]
    public void AskingForAnUpdateNeverClaimsOneArrived()
    {
        // `refresh_history` answers `requested: true` and nothing else -- the
        // poller owns the network call. Copy that said "Updated" would be a
        // claim about a round trip that has not happened yet.
        string lower = HistoryCopy.CheckForUpdatesAsked.ToLowerInvariant();

        Assert.StartsWith("asked", lower, StringComparison.Ordinal);
        Assert.DoesNotContain("updated", lower, StringComparison.Ordinal);
        Assert.DoesNotContain("refreshed", lower, StringComparison.Ordinal);
    }

    [Fact]
    public void TheOutcomeCountsAreNotPresentedAsExplainingUnqueuedSessions()
    {
        // The contract is explicit: `queue_outcome_counts` covers entries
        // that reached the queue, and cannot answer "I finished a session,
        // why is nothing pending?" for one the watcher discarded first. The
        // footnote says so rather than letting the heading overclaim.
        Assert.Contains("never offered is not counted", HistoryCopy.OutcomesFootnote, StringComparison.Ordinal);
    }

    [Fact]
    public void TheRollupsWaitingFigureNeverGoesNegative()
    {
        // The three figures come from one cache but not from one instant, so
        // a transient overshoot is possible. "-1 waiting" would be this
        // screen's first visibly wrong number.
        var rollup = new HistoryRollup
        {
            AllTime = new HistoryCounts { Submitted = 2, Accepted = 3 },
            Quarantined = 4,
        };

        Assert.Equal(0, rollup.WaitingToBeScored);

        rollup.AllTime = new HistoryCounts { Submitted = 10, Accepted = 3 };
        rollup.Quarantined = 2;
        Assert.Equal(5, rollup.WaitingToBeScored);
    }

    [Fact]
    public void AbsentCommunityStandingParsesAsNoSectionRatherThanAZeroedOne()
    {
        // "Absent means no standing, and absent is not null." The object is
        // omitted entirely in every case where there is nothing to say, and a
        // client renders all of them identically by drawing no section.
        HistoryRollup? rollup = JsonSerializer.Deserialize<HistoryRollup>(
            "{\"week\":{\"submitted\":0,\"accepted\":0,\"quarantined\":0,\"other\":0},"
            + "\"month\":{\"submitted\":0,\"accepted\":0,\"quarantined\":0,\"other\":0},"
            + "\"all_time\":{\"submitted\":3,\"accepted\":1,\"quarantined\":1,\"other\":0},"
            + "\"credit_pending\":1.5,\"credit_final\":2.5,\"quarantined\":1,"
            + "\"last_refreshed_at\":null}");

        Assert.NotNull(rollup);
        Assert.Null(rollup!.Community);
        Assert.Null(rollup.LastRefreshedAt);
        Assert.Equal(1, rollup.WaitingToBeScored);
    }

    [Fact]
    public void APresentCommunityStandingKeepsItsNullableFieldsNullable()
    {
        // `rank` and `accept_rate` may be null inside an otherwise present
        // object; a client draws a dash rather than `#0` or `0%`, which it
        // can only do if the parse preserves the difference.
        HistoryRollup? rollup = JsonSerializer.Deserialize<HistoryRollup>(
            "{\"week\":{},\"month\":{},\"all_time\":{},\"credit_pending\":0,\"credit_final\":0,"
            + "\"quarantined\":0,\"community\":{\"rank\":null,\"novelty_credit\":1240.0,"
            + "\"accepted_in_window\":12,\"accept_rate\":null,\"window_label\":\"7d\","
            + "\"analytics_withheld\":true}}");

        Assert.NotNull(rollup?.Community);
        Assert.Null(rollup!.Community!.Rank);
        Assert.Null(rollup.Community.AcceptRate);
        Assert.Equal(12, rollup.Community.AcceptedInWindow);
        Assert.True(rollup.Community.AnalyticsWithheld);
    }

    [Fact]
    public void AHistoryRecordCarriesNoLocalPath()
    {
        // The Rust type says this at the top of its own file, and it is the
        // reason to say it again here: history is the surface most likely to
        // be screenshotted, exported, or shared. A field added because it was
        // convenient is a field that ends up in a screenshot.
        foreach (System.Reflection.PropertyInfo property in typeof(HistoryRecord).GetProperties())
        {
            string name = property.Name.ToLowerInvariant();
            Assert.DoesNotContain("path", name, StringComparison.Ordinal);
            Assert.DoesNotContain("url", name, StringComparison.Ordinal);
            Assert.DoesNotContain("token", name, StringComparison.Ordinal);
        }
    }

    [Fact]
    public void AHistoryPageParsesFromTheWireShapeTheDaemonSends()
    {
        DaemonResponse response = DaemonResponse.Parse(
            "{\"id\":1,\"result\":{\"history\":[{\"submission_id\":\"11111111-1111-1111-1111-111111111111\","
            + "\"submitted_at\":\"2026-08-01T12:00:00Z\",\"project_label\":\"myproj\","
            + "\"source\":\"claude-code\",\"session_hash\":\"abcd\",\"status\":\"quarantined\","
            + "\"consent_scopes\":[\"debugging_evaluation\"],\"credit_points_pending\":1.5,"
            + "\"credit_points_final\":null,\"explanations\":[\"Held because a passage looked personal.\"],"
            + "\"last_refreshed_at\":\"2026-08-01T12:05:00Z\",\"withdrawn_at\":null}]}}");

        HistoryList? page = response.ResultAs<HistoryList>();

        Assert.NotNull(page);
        HistoryRecord record = Assert.Single(page!.History);
        Assert.Equal("quarantined", record.Status);
        Assert.Null(record.CreditPointsFinal);
        Assert.Equal("credit 1.5, still being scored", HistoryCopy.CreditLine(record.CreditPointsFinal, record.CreditPointsPending));
        Assert.True(WithdrawCopy.OffersWithdrawal(record.Status, record.SubmissionId));

        // A held record maps to the not-in-the-commons stage: the server's own
        // rule is that it resolves to `not_distributed` exactly.
        Assert.Equal(WithdrawStage.NotInTheCommons, WithdrawStageExtensions.StageOf(record.Status));
    }
}
