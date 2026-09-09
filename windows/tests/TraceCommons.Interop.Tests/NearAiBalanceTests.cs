using System;
using System.Globalization;
using System.Linq;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The NEAR AI balance surface as it really crosses the C ABI.
///
/// <para>
/// What is under test is that this shell RENDERS and does not DECIDE. The
/// sentence per state, the tone, the one action and the money formatting all
/// live in <c>private_inference_copy.rs</c>, and every one of them has a way
/// of going wrong that a shell reimplementing it would not notice: an absent
/// amount printed as <c>$0.00</c> tells a contributor the money is gone, and
/// a no-limit account told it has $0.00 left is the same lie with a different
/// cause.
/// </para>
/// </summary>
public class NearAiBalanceTests
{
    /// <summary>The five states the daemon reports, spelled as it spells them.</summary>
    private static readonly string[] Reported =
    {
        "known", "no_session", "session_expired", "no_organization", "unavailable",
    };

    /// <summary>The wire's own scale today. Read from the payload, never assumed.</summary>
    private const byte Scale = 9;

    private static PrivateInferenceCopy Copy()
    {
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        Assert.NotNull(copy);
        return copy!;
    }

    private static NearAiBalance Of(string state) =>
        NearAiBalance.Unreported with { State = state, Scale = Scale };

    /// <summary>
    /// AN EMPTY AMOUNT IS NEVER <c>$0.00</c>, and a zero is never nothing.
    /// </summary>
    /// <remarks>
    /// The distinction the whole surface turns on. These amounts are signed,
    /// so absence cannot ride on an out-of-range integer the way every other
    /// money export here encodes it -- which is why presence is its own
    /// argument, and why a shell that folded the two together would render a
    /// real debt as no figure and an unknown figure as an empty account.
    /// </remarks>
    [Fact]
    public void ANullAmountIsNoFigureAndAZeroAmountIsARealZero()
    {
        Assert.Equal(string.Empty, NearAiBalanceSurface.Amount(null, Scale));
        Assert.Equal("$0.00", NearAiBalanceSurface.Amount(0, Scale));
        Assert.NotEqual(
            NearAiBalanceSurface.Amount(null, Scale),
            NearAiBalanceSurface.Amount(0, Scale));
    }

    /// <summary>
    /// An overdrawn account renders as a negative figure rather than
    /// disappearing.
    /// </summary>
    [Fact]
    public void ANegativeAmountIsAFigureAndNotAnAbsence()
    {
        string owed = NearAiBalanceSurface.Amount(-1_500_000_000, Scale);
        Assert.Equal("-$1.50", owed);
        Assert.NotEqual(string.Empty, owed);
    }

    /// <summary>
    /// <c>scale</c> comes off the wire. Nothing here divides by a constant.
    /// </summary>
    /// <remarks>
    /// The same integer under two scales must not print the same money. A
    /// shell holding its own 1_000_000_000 would be wrong by a factor of a
    /// thousand the day the daemon changed the field, and would go on looking
    /// right until then.
    /// </remarks>
    [Fact]
    public void TheScaleComesOffTheWireRatherThanFromAConstant()
    {
        Assert.Equal("$8.50", NearAiBalanceSurface.Amount(8_500_000_000, 9));
        Assert.Equal("$8500.00", NearAiBalanceSurface.Amount(8_500_000_000, 6));
        Assert.NotEqual(
            NearAiBalanceSurface.Amount(8_500_000_000, 9),
            NearAiBalanceSurface.Amount(8_500_000_000, 6));
    }

    /// <summary>
    /// A scale the payload never carried yields NO figure at all, rather than
    /// a figure divided by a guess.
    /// </summary>
    /// <remarks>
    /// The dangerous direction: a missing <c>scale</c> defaulted to zero
    /// makes 8.5 dollars print as $8,500,000,000.00. Nothing on this row may
    /// be drawn from units nobody sent.
    /// </remarks>
    [Fact]
    public void AnAmountWithNoScaleIsNotDrawnAtAll()
    {
        Assert.Equal(string.Empty, NearAiBalanceSurface.Amount(8_500_000_000, null));
        Assert.Equal(string.Empty, NearAiBalanceSurface.RemainingLine(8_500_000_000, null));
        Assert.Equal(string.Empty, NearAiBalanceSurface.RemainingLine(null, null));
        Assert.Equal(string.Empty, NearAiBalanceSurface.LimitLine(0, null));
        Assert.Equal(string.Empty, NearAiBalanceSurface.SpentLine(0, null));
    }

    /// <summary>
    /// A NULL REMAINING FIGURE IS NOT AN EMPTY LINE AND NOT A ZERO. It is the
    /// ordinary state of an account nobody capped, and it has its own
    /// sentence.
    /// </summary>
    [Fact]
    public void AnAbsentRemainingFigureGivesTheNoLimitSentence()
    {
        PrivateInferenceCopy copy = Copy();
        string absent = NearAiBalanceSurface.RemainingLine(null, Scale);

        Assert.Equal(copy.BalanceNoRemaining, absent);
        Assert.NotEqual(string.Empty, absent);
        Assert.DoesNotContain("$0.00", absent, StringComparison.Ordinal);

        // And a real zero is a real zero, said in the other words.
        string zero = NearAiBalanceSurface.RemainingLine(0, Scale);
        Assert.Contains("$0.00", zero, StringComparison.Ordinal);
        Assert.NotEqual(absent, zero);
    }

    /// <summary>
    /// The other two figures go the other way: absent is no line, and zero is
    /// a real <c>$0.00</c>.
    /// </summary>
    /// <remarks>
    /// Deliberately not the remaining line's rule. A ceiling nobody set has
    /// already been explained by the sentence above it, and an account that
    /// has spent nothing really has spent $0.00.
    /// </remarks>
    [Fact]
    public void TheLimitAndSpentLinesAreEmptyWhenAbsentAndZeroWhenZero()
    {
        Assert.Equal(string.Empty, NearAiBalanceSurface.LimitLine(null, Scale));
        Assert.Equal(string.Empty, NearAiBalanceSurface.SpentLine(null, Scale));
        Assert.Contains("$0.00", NearAiBalanceSurface.LimitLine(0, Scale), StringComparison.Ordinal);
        Assert.Contains("$0.00", NearAiBalanceSurface.SpentLine(0, Scale), StringComparison.Ordinal);
    }

    /// <summary>
    /// <c>known</c> answers the EMPTY STRING, and the emptiness is the point:
    /// on that state the figures are the content.
    /// </summary>
    [Fact]
    public void TheKnownStateCarriesNoSentenceOfItsOwn()
    {
        Assert.Equal(string.Empty, NearAiBalanceSurface.StateLine(Of("known"), Copy()));
    }

    /// <summary>
    /// Every other reported state gets its own finished sentence, and no two
    /// share one.
    /// </summary>
    [Fact]
    public void EveryUnreadStateGetsItsOwnFinishedSentence()
    {
        PrivateInferenceCopy copy = Copy();
        string[] lines = Reported
            .Where(state => state != "known")
            .Select(state => NearAiBalanceSurface.StateLine(Of(state), copy))
            .ToArray();

        foreach (string line in lines)
        {
            Assert.NotEqual(string.Empty, line);
            Assert.EndsWith(".", line.Trim(), StringComparison.Ordinal);
        }

        Assert.Equal(lines.Length, lines.Distinct(StringComparer.Ordinal).Count());
    }

    /// <summary>
    /// A state this build has never heard of gets the unknown sentence and
    /// BORROWS NOBODY'S.
    /// </summary>
    /// <remarks>
    /// Borrowing the no-session sentence would tell somebody who is signed in
    /// that they are not; borrowing the unavailable one would put a reason in
    /// their head that nobody worked out. The empty state is the other half:
    /// a daemon that does not answer this at all is not a read that failed.
    /// </remarks>
    [Fact]
    public void AnUnrecognisedStateBorrowsNobodysSentence()
    {
        PrivateInferenceCopy copy = Copy();
        string unknown = NearAiBalanceSurface.StateLine(Of("a_state_from_a_later_daemon"), copy);

        Assert.Equal(copy.BalanceUnknown, unknown);
        foreach (string state in Reported)
        {
            Assert.NotEqual(NearAiBalanceSurface.StateLine(Of(state), copy), unknown);
        }

        Assert.Equal(copy.BalanceUnreported, NearAiBalanceSurface.StateLine(Of(string.Empty), copy));
        Assert.NotEqual(copy.BalanceUnreported, unknown);
    }

    /// <summary>
    /// TONE MEANS THE READ SUCCEEDED, NOT THAT THE BALANCE IS HEALTHY.
    /// </summary>
    /// <remarks>
    /// <c>known</c> alone is clear, and it is clear for an empty account and
    /// an overdrawn one too. Nothing across this ABI judges an amount, so a
    /// shell painting a low figure red would be inventing a threshold nobody
    /// set, on an account whose ceiling may not exist.
    /// </remarks>
    [Fact]
    public void ToneSaysTheReadSucceededAndNeverJudgesTheAmount()
    {
        Assert.Equal(PrivateInferenceTone.Clear, NearAiBalanceSurface.Tone(Of("known")));
        Assert.Equal(PrivateInferenceTone.Refused, NearAiBalanceSurface.Tone(Of("session_expired")));
        Assert.Equal(
            PrivateInferenceTone.Attention, NearAiBalanceSurface.Tone(Of("no_organization")));

        foreach (string unread in new[] { "no_session", "unavailable", string.Empty, "later" })
        {
            Assert.Equal(PrivateInferenceTone.Neutral, NearAiBalanceSurface.Tone(Of(unread)));
        }

        // The amount cannot move the tone, because the tone never sees one.
        foreach (long? nanos in new long?[] { null, -1, 0, 1, long.MaxValue })
        {
            NearAiBalance broke = Of("known") with { RemainingNanos = nanos };
            Assert.Equal(PrivateInferenceTone.Clear, NearAiBalanceSurface.Tone(broke));
        }
    }

    /// <summary>
    /// ONE ACTION, ON EXACTLY TWO STATES, and it is the sign-in row's own.
    /// </summary>
    /// <remarks>
    /// A refused session gets obtain WITHOUT a forget first: the ceremony
    /// overwrites both records, and forgetting would throw away a working key
    /// to fix an unrelated sign-in. Everything else offers nothing -- an
    /// unread state included, because the offer is what mints the second key.
    /// </remarks>
    [Fact]
    public void ObtainIsOfferedOnExactlyTwoStates()
    {
        Assert.Equal(CredentialAction.Obtain, NearAiBalanceSurface.Action(Of("no_session")));
        Assert.Equal(CredentialAction.Obtain, NearAiBalanceSurface.Action(Of("session_expired")));

        foreach (string quiet in new[]
        {
            "known", "no_organization", "unavailable", string.Empty, "a_later_state",
        })
        {
            Assert.Equal(CredentialAction.None, NearAiBalanceSurface.Action(Of(quiet)));
        }
    }

    /// <summary>
    /// The action's words and its consequence sentence come from the sign-in
    /// row, because it is the sign-in row's action.
    /// </summary>
    [Fact]
    public void TheActionReusesTheSignInRowsWordsAndItsConsequence()
    {
        PrivateInferenceCopy copy = Copy();
        Assert.Equal(
            copy.CredentialObtain,
            NearAiCredentialSurface.ActionLabel(CredentialAction.Obtain, copy));
        Assert.Equal(
            copy.CredentialCost,
            NearAiCredentialSurface.ActionPreamble(CredentialAction.Obtain, copy));
    }

    /// <summary>
    /// A read this shell could not make is UNREPORTED, never a claim about
    /// the account.
    /// </summary>
    [Fact]
    public void AMalformedAnswerIsUnreportedAndCarriesNoFigures()
    {
        foreach (string? bad in new[] { null, string.Empty, "   ", "[]", "\"known\"", "{" })
        {
            NearAiBalance balance = NearAiBalanceSurface.Parse(bad);
            Assert.Equal(NearAiBalance.Unreported, balance);
            Assert.Null(balance.Scale);
            Assert.Null(balance.RemainingNanos);
            Assert.Equal(Copy().BalanceUnreported, NearAiBalanceSurface.StateLine(balance, Copy()));
        }
    }

    /// <summary>
    /// The wire's nulls survive the parse as nulls. THIS IS THE PARSE'S WHOLE
    /// JOB: a decoder that defaulted a missing figure to zero would defeat
    /// every distinction above before the ABI was ever called.
    /// </summary>
    [Fact]
    public void TheWireNullsSurviveTheParse()
    {
        NearAiBalance balance = NearAiBalanceSurface.Parse(
            """
            {
              "state": "known",
              "currency": "USD",
              "scale": 9,
              "remaining_nanos": null,
              "spend_limit_nanos": null,
              "total_spent_nanos": 0,
              "observed_at": null
            }
            """);

        Assert.Equal("known", balance.State);
        Assert.Equal((byte)9, balance.Scale);
        Assert.Null(balance.RemainingNanos);
        Assert.Null(balance.SpendLimitNanos);
        Assert.Equal(0, balance.TotalSpentNanos);
        Assert.Null(balance.ObservedAt);
    }

    /// <summary>A full reading decodes every field, signs included.</summary>
    [Fact]
    public void AFullReadingDecodesEveryField()
    {
        NearAiBalance balance = NearAiBalanceSurface.Parse(
            """
            {
              "state": "known",
              "currency": "USD",
              "scale": 9,
              "remaining_nanos": -8500000000,
              "spend_limit_nanos": 10000000000,
              "total_spent_nanos": 1500000000,
              "observed_at": "2026-09-08T12:00:00Z"
            }
            """);

        Assert.Equal(-8_500_000_000, balance.RemainingNanos);
        Assert.Equal(10_000_000_000, balance.SpendLimitNanos);
        Assert.Equal(1_500_000_000, balance.TotalSpentNanos);
        Assert.Equal(
            DateTimeOffset.Parse("2026-09-08T12:00:00Z", CultureInfo.InvariantCulture),
            balance.ObservedAt);

        Assert.Equal("-$8.50", NearAiBalanceSurface.Amount(balance.RemainingNanos, balance.Scale));
    }

    /// <summary>
    /// A scale the payload never carried leaves every figure undrawable
    /// rather than divided by a guess.
    /// </summary>
    [Fact]
    public void AReadingWithNoScaleCarriesNoDrawableFigures()
    {
        NearAiBalance balance = NearAiBalanceSurface.Parse(
            """
            {"state": "known", "remaining_nanos": 8500000000}
            """);

        Assert.Equal("known", balance.State);
        Assert.Null(balance.Scale);
        Assert.Equal(
            string.Empty, NearAiBalanceSurface.Amount(balance.RemainingNanos, balance.Scale));
    }

    /// <summary>
    /// The age line says when THIS COMPUTER asked, and says nothing at all
    /// when nobody recorded an asking.
    /// </summary>
    [Fact]
    public void TheObservedLineIsEmptyWithoutATimestamp()
    {
        DateTimeOffset now = DateTimeOffset.UtcNow;
        Assert.Equal(string.Empty, NearAiBalanceSurface.ObservedLine(null, now));
        Assert.NotEqual(
            string.Empty, NearAiBalanceSurface.ObservedLine(now.AddSeconds(-5), now));

        // A clock that ran backwards is not an age. Never a sentence claiming
        // the question was put in the future.
        Assert.Equal(
            string.Empty, NearAiBalanceSurface.ObservedLine(now.AddMinutes(5), now));
    }
}
