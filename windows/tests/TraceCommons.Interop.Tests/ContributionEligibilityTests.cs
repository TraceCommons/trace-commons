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
/// The contribution eligibility surface as it really crosses the C ABI.
///
/// The rule this whole surface exists for is <b>show every session, offer
/// only the eligible ones</b>. The three ways to get that wrong all have a
/// test here: rendering an absent field as a state, letting an unfamiliar
/// state borrow a sentence that was written about something else, and
/// deciding in this shell which rows get a send button.
/// </summary>
public class ContributionEligibilityTests
{
    private const string Eligible = "eligible";
    private const string IneligiblePermanent = "ineligible_permanent";
    private const string IneligibleConfiguration = "ineligible_configuration";
    private const string Unknown = "unknown";

    private static PrivateInferenceCopy Copy()
    {
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        Assert.NotNull(copy);
        return copy!;
    }

    private static ContributionEligibilityDecision Decide(string? state, string? reason = null) =>
        ContributionEligibilitySurface.Decide(new ContributionEligibility(state, reason));

    /// <summary>
    /// Each of the four states draws the sentence the Rust exported for it,
    /// and no two of them draw the same one.
    /// </summary>
    [Fact]
    public void EveryStateDrawsItsOwnExportedSentence()
    {
        PrivateInferenceCopy copy = Copy();
        Assert.Equal(copy.EligibilityEligible, Decide(Eligible).StateLine);
        Assert.Equal(copy.EligibilityIneligiblePermanent, Decide(IneligiblePermanent).StateLine);
        Assert.Equal(
            copy.EligibilityIneligibleConfiguration,
            Decide(IneligibleConfiguration).StateLine);
        Assert.Equal(copy.EligibilityUnknown, Decide(Unknown).StateLine);

        var drawn = new[]
        {
            Decide(Eligible).StateLine,
            Decide(IneligiblePermanent).StateLine,
            Decide(IneligibleConfiguration).StateLine,
            Decide(Unknown).StateLine,
        };
        Assert.Equal(4, drawn.Distinct(StringComparer.Ordinal).Count());
        Assert.All(drawn, line => Assert.False(string.IsNullOrWhiteSpace(line)));
    }

    /// <summary>
    /// AN ABSENT <c>eligibility</c> KEY IS NOT THE <c>unknown</c> STATE.
    /// </summary>
    /// <remarks>
    /// The daemon omits the field for a contributor who was invited rather
    /// than admitted on evidence. They have no eligibility question, so their
    /// rows carry no sentence, no tone and no caveat, and keep the send
    /// control they already had. A shell that read the missing key as
    /// <c>unknown</c> would put "this has not been worked out" on every row
    /// of a queue that was never in question, and take the send button off
    /// all of them.
    /// </remarks>
    [Fact]
    public void AnAbsentFieldIsNotTheUnknownState()
    {
        ContributionEligibilityDecision absent =
            ContributionEligibilitySurface.Decide(ContributionEligibility.Absent);

        Assert.False(absent.IsAnswered);
        Assert.Null(absent.StateLine);
        Assert.Null(absent.Tone);
        Assert.Null(absent.ReasonLine);
        Assert.False(absent.HasStateLine);
        Assert.False(absent.HasReasonLine);
        Assert.Equal(ContributionControl.Contribute, absent.Control);

        ContributionEligibilityDecision unknown = Decide(Unknown);
        Assert.True(unknown.IsAnswered);
        Assert.Equal(Copy().EligibilityUnknown, unknown.StateLine);
        Assert.Equal(ContributionControl.None, unknown.Control);

        Assert.NotEqual(unknown, absent);
    }

    /// <summary>
    /// A queue entry with no <c>eligibility</c> key deserialises to the
    /// absent decision, not to a state.
    /// </summary>
    /// <remarks>
    /// The end of the same wire the daemon writes. <c>System.Text.Json</c>
    /// leaves a missing string property null, and this pins that a null
    /// stays absent all the way to the decision rather than being read as a
    /// label somewhere in between.
    /// </remarks>
    [Fact]
    public void AnEntryWithoutTheKeyDecodesAsAbsent()
    {
        QueueEntry entry = JsonSerializer.Deserialize<QueueEntry>(
            "{\"entry_id\":\"e1\",\"size_bytes\":10}")!;
        Assert.Null(entry.Eligibility);
        Assert.Null(entry.EligibilityReason);

        ContributionEligibilityDecision decision =
            ContributionEligibilitySurface.Decide(entry);
        Assert.False(decision.IsAnswered);
        Assert.Equal(ContributionControl.Contribute, decision.Control);
        Assert.Null(decision.StateLine);
    }

    /// <summary>
    /// An entry that carries the key decodes as that state, reason and all.
    /// </summary>
    [Fact]
    public void AnEntryWithTheKeyDecodesAsThatState()
    {
        QueueEntry entry = JsonSerializer.Deserialize<QueueEntry>(
            "{\"entry_id\":\"e1\",\"eligibility\":\"ineligible_permanent\","
            + "\"eligibility_reason\":\"no_inference_call\"}")!;

        ContributionEligibilityDecision decision =
            ContributionEligibilitySurface.Decide(entry);
        Assert.True(decision.IsAnswered);
        Assert.Equal(Copy().EligibilityIneligiblePermanent, decision.StateLine);
        Assert.Equal(Copy().EligibilityReasonNoCall, decision.ReasonLine);
        Assert.Equal(ContributionControl.None, decision.Control);
    }

    /// <summary>
    /// A state this build has never heard of borrows no other state's
    /// sentence: it reports that the answer has not been worked out, and
    /// never an ineligibility.
    /// </summary>
    /// <remarks>
    /// The first slice never emits <c>unknown</c> and never emits anything
    /// outside the four, so this is the guard for the day one of them
    /// arrives. Degrading an unreadable state into a refusal would stop a
    /// contributor offering work that is fine.
    /// </remarks>
    [Fact]
    public void AnUnrecognisedStateBorrowsNoOtherStatesSentence()
    {
        PrivateInferenceCopy copy = Copy();
        foreach (string state in new[]
        {
            string.Empty,
            "   ",
            "ELIGIBLE",
            "eligible ",
            "ineligible",
            "ineligible_transient",
            "a state a later daemon grew",
        })
        {
            ContributionEligibilityDecision decision = Decide(state);
            Assert.True(decision.IsAnswered);
            Assert.Equal(copy.EligibilityUnknown, decision.StateLine);
            Assert.NotEqual(copy.EligibilityEligible, decision.StateLine);
            Assert.NotEqual(copy.EligibilityIneligiblePermanent, decision.StateLine);
            Assert.NotEqual(copy.EligibilityIneligibleConfiguration, decision.StateLine);
            Assert.Equal(PrivateInferenceTone.Neutral, decision.Tone);
            Assert.Equal(ContributionControl.None, decision.Control);
        }
    }

    /// <summary>
    /// The tone of every state is the ABI's answer, not this shell's.
    /// </summary>
    /// <remarks>
    /// Asserted against the raw ABI call rather than against a table written
    /// here: a table would agree with itself forever, including on the day
    /// the Rust changed one arm. The literal expectations beside it are the
    /// two that carry a claim -- eligible reads clear, and a permanent
    /// ineligibility is deliberately NOT painted as a refusal, because
    /// nothing was refused and a contributor's ordinary older work is not a
    /// failure.
    /// </remarks>
    [Fact]
    public void ToneComesFromTheAbi()
    {
        foreach (string state in new[]
        {
            Eligible, IneligiblePermanent, IneligibleConfiguration, Unknown, "unheard-of",
        })
        {
            Assert.Equal(
                PrivateInferenceSurface.FromAbiTone(
                    NativeMethods.tc_contribution_eligibility_tone(state)),
                Decide(state).Tone);
        }

        Assert.Equal(PrivateInferenceTone.Clear, Decide(Eligible).Tone);
        Assert.Equal(PrivateInferenceTone.Attention, Decide(IneligibleConfiguration).Tone);
        Assert.Equal(PrivateInferenceTone.Neutral, Decide(IneligiblePermanent).Tone);
        Assert.Equal(PrivateInferenceTone.Neutral, Decide(Unknown).Tone);
        Assert.NotEqual(PrivateInferenceTone.Refused, Decide(IneligiblePermanent).Tone);
    }

    /// <summary>
    /// Which rows get a send control is the ABI's answer, not this shell's,
    /// and only <c>eligible</c> gets one.
    /// </summary>
    /// <remarks>
    /// This is the defect the surface exists to remove. Offering a send
    /// beside a session the server will refuse is worse than an inert
    /// button: pressing it sends a contributor's work and has it turned
    /// away.
    /// </remarks>
    [Fact]
    public void ControlComesFromTheAbiAndOnlyEligibleIsOffered()
    {
        foreach (string state in new[]
        {
            Eligible, IneligiblePermanent, IneligibleConfiguration, Unknown, "unheard-of",
        })
        {
            Assert.Equal(
                ContributionEligibilitySurface.FromAbiControl(
                    NativeMethods.tc_contribution_eligibility_control(state)),
                Decide(state).Control);
        }

        Assert.Equal(ContributionControl.Contribute, Decide(Eligible).Control);
        Assert.True(Decide(Eligible).OffersContribute);
        foreach (string state in new[]
        {
            IneligiblePermanent, IneligibleConfiguration, Unknown, "unheard-of",
        })
        {
            Assert.Equal(ContributionControl.None, Decide(state).Control);
            Assert.False(Decide(state).OffersContribute);
        }
    }

    /// <summary>
    /// A control value from outside its own range offers nothing.
    /// </summary>
    /// <remarks>
    /// The range is disjoint from the credential actions' and from every
    /// tone's precisely so a cross-wiring cannot land on contribute by
    /// arithmetic. Every one of these is a real value from a neighbouring
    /// ABI block.
    /// </remarks>
    [Fact]
    public void AValueFromAnotherAbiRangeOffersNothing()
    {
        foreach (int value in new[] { 0, 1, 20, 22, 23, 30, 31, 32, 33, 49, 52, -1 })
        {
            Assert.Equal(
                ContributionControl.None,
                ContributionEligibilitySurface.FromAbiControl(value));
        }

        Assert.Equal(
            ContributionControl.Contribute,
            ContributionEligibilitySurface.FromAbiControl(51));
        Assert.Equal(
            ContributionControl.None,
            ContributionEligibilitySurface.FromAbiControl(50));
    }

    /// <summary>
    /// Each of the thirteen reason labels draws the sentence the Rust
    /// exported for it, and no two draw the same one.
    /// </summary>
    [Fact]
    public void EveryReasonLabelDrawsItsOwnExportedSentence()
    {
        PrivateInferenceCopy copy = Copy();
        var expected = new Dictionary<string, string>(StringComparer.Ordinal)
        {
            ["no_inference_call"] = copy.EligibilityReasonNoCall,
            ["capture_off"] = copy.EligibilityReasonCaptureOff,
            ["digest_absent"] = copy.EligibilityReasonDigestAbsent,
            ["upstream_id_absent"] = copy.EligibilityReasonUpstreamIdAbsent,
            ["digest_mismatch"] = copy.EligibilityReasonDigestMismatch,
            ["reference_malformed"] = copy.EligibilityReasonReferenceMalformed,
            ["bodies_unreadable"] = copy.EligibilityReasonBodiesUnreadable,
            ["body_not_utf8"] = copy.EligibilityReasonBodyNotUtf8,
            ["body_too_large"] = copy.EligibilityReasonBodyTooLarge,
            ["evidence_capture_off"] = copy.EligibilityReasonEvidenceCaptureOff,
            ["marker_absent"] = copy.EligibilityReasonMarkerAbsent,
            ["request_malformed"] = copy.EligibilityReasonRequestMalformed,
            ["receipt_unavailable"] = copy.EligibilityReasonReceiptUnavailable,
            ["receipt_not_issued"] = copy.EligibilityReasonReceiptNotIssued,
        };

        Assert.Equal(14, expected.Count);
        foreach (KeyValuePair<string, string> pair in expected)
        {
            string? drawn = ContributionEligibilitySurface.ReasonLine(pair.Key);
            Assert.False(string.IsNullOrWhiteSpace(drawn));
            Assert.Equal(pair.Value, drawn);
        }

        Assert.Equal(13, expected.Values.Distinct(StringComparer.Ordinal).Count());
    }

    /// <summary>
    /// An unfamiliar, empty or absent reason draws NOTHING.
    /// </summary>
    /// <remarks>
    /// Deliberately different from the state line's fallback. An unknown
    /// state still has to say something, because the row is being shown
    /// either way; an unknown reason has nothing honest to say, and a
    /// sentence guessing at it would add a detail nobody established.
    /// </remarks>
    [Fact]
    public void AnUnfamiliarReasonDrawsNothing()
    {
        PrivateInferenceCopy copy = Copy();
        foreach (string? reason in new[]
        {
            null, string.Empty, "  ", "NO_INFERENCE_CALL", "no_call", "some_later_variant",
        })
        {
            Assert.Null(ContributionEligibilitySurface.ReasonLine(reason));
            foreach (string sentence in copy.Sentences)
            {
                Assert.NotEqual(sentence, ContributionEligibilitySurface.ReasonLine(reason));
            }
        }

        Assert.Null(Decide(Eligible).ReasonLine);
        Assert.False(Decide(Eligible).HasReasonLine);
    }

    /// <summary>
    /// A row that was never asked the question draws no reason either, even
    /// if a reason label somehow rode along with the absent state.
    /// </summary>
    [Fact]
    public void AnUnansweredRowDrawsNoReason()
    {
        ContributionEligibilityDecision decision = Decide(null, "no_inference_call");
        Assert.False(decision.IsAnswered);
        Assert.Null(decision.ReasonLine);
        Assert.Null(decision.StateLine);
    }

    /// <summary>
    /// No wording is authored in this surface. Every string it can put on
    /// screen crossed the ABI already finished.
    /// </summary>
    /// <remarks>
    /// The strict rule, with an EMPTY allow-list rather than a set of wire
    /// values: this file reads two wire fields off a decoded
    /// <see cref="QueueEntry"/> and never names them itself, so it needs no
    /// literal at all. Asserted about the source because a hand-written
    /// sentence that happened to match the Rust would pass every behavioural
    /// test above and then survive a rename in exactly one of the three
    /// shells.
    /// </remarks>
    [Fact]
    public void NoWordingIsAuthoredInTheEligibilitySurface()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "ContributionEligibility.cs.txt");
        Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");

        string uncommented = string.Join(
            "\n",
            File.ReadAllText(path).Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal))
                .Where(line => !line.TrimStart().StartsWith("///", StringComparison.Ordinal)));

        foreach (Match match in Regex.Matches(uncommented, "\"([^\"\\\\]|\\\\.)*\""))
        {
            Assert.Fail(
                $"{match.Value} is a string literal in ContributionEligibility.cs. Every "
                + "sentence on this surface comes from private_inference_copy.rs across the "
                + "ABI, and this file needs no literal of its own.");
        }
    }
}
