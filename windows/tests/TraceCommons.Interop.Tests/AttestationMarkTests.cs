using System;
using System.Linq;
using System.Reflection;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The attestation mark surface as it really crosses the C ABI.
/// </summary>
/// <remarks>
/// The mark answers a different question from eligibility, and the difference
/// is the whole reason it exists. Eligibility asks <i>may this contributor
/// send this session</i>, and says nothing at all when nobody asked. The mark
/// states whether the session carries proof of the model call that produced
/// it, which is a fact about the trace and is owed to everybody, invited
/// contributors included.
///
/// <para>
/// So the two rules are opposites, and the tests below pin both: an absent
/// <c>eligibility</c> draws nothing, while a queue entry always draws a mark.
/// </para>
/// </remarks>
public class AttestationMarkTests
{
    private const string Attested = "attested";
    private const string UnattestedPermanent = "unattested_permanent";
    private const string UnattestedConfiguration = "unattested_configuration";
    private const string Unknown = "unknown";

    /// <summary>The one reason label an <c>unknown</c> mark can carry.</summary>
    private const string ReceiptUnavailable = "receipt_unavailable";

    private static PrivateInferenceCopy Copy()
    {
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        Assert.NotNull(copy);
        return copy!;
    }

    private static AttestationMarkDecision Decide(string? mark, string? reason = null) =>
        AttestationMarkSurface.Decide(new AttestationMark(mark, reason));

    /// <summary>
    /// Each of the four marks draws the sentence the Rust exported for it, and
    /// no two of them draw the same one.
    /// </summary>
    [Fact]
    public void EveryMarkDrawsItsOwnExportedSentence()
    {
        PrivateInferenceCopy copy = Copy();
        Assert.Equal(copy.AttestationAttested, Decide(Attested).MarkLine);
        Assert.Equal(
            copy.AttestationUnattestedPermanent, Decide(UnattestedPermanent).MarkLine);
        Assert.Equal(
            copy.AttestationUnattestedConfiguration,
            Decide(UnattestedConfiguration).MarkLine);
        Assert.Equal(copy.AttestationUnknown, Decide(Unknown).MarkLine);

        string?[] drawn = new[]
        {
            Decide(Attested).MarkLine,
            Decide(UnattestedPermanent).MarkLine,
            Decide(UnattestedConfiguration).MarkLine,
            Decide(Unknown).MarkLine,
        };
        Assert.Equal(4, drawn.Distinct(StringComparer.Ordinal).Count());
        Assert.All(drawn, line => Assert.False(string.IsNullOrWhiteSpace(line)));
    }

    /// <summary>
    /// THE MARK IS RENDERED FOR AN INVITED CONTRIBUTOR. This is the case the
    /// whole feature exists for.
    /// </summary>
    /// <remarks>
    /// An invited contributor's entries carry no <c>eligibility</c> key at
    /// all, and the eligibility surface correctly draws nothing for them. The
    /// mark is not gated on that: it arrives on the same entry, it is about
    /// the trace rather than about the contributor's permission, and a shell
    /// that hid it alongside the eligibility line would withhold from an
    /// invited contributor a fact affecting what their work is worth.
    /// </remarks>
    [Fact]
    public void TheMarkIsDrawnForAnInvitedContributor()
    {
        QueueEntry invited = JsonSerializer.Deserialize<QueueEntry>(
            "{\"entry_id\":\"e1\",\"attestation\":\"attested\"}")!;

        // Nobody asked this contributor an eligibility question.
        Assert.Null(invited.Eligibility);
        Assert.False(ContributionEligibilitySurface.Decide(invited).IsAnswered);

        // The mark is drawn anyway.
        AttestationMarkDecision decision = AttestationMarkSurface.Decide(invited);
        Assert.True(decision.HasMarkLine);
        Assert.Equal(Copy().AttestationAttested, decision.MarkLine);
        Assert.Equal(PrivateInferenceTone.Clear, decision.Tone);
    }

    /// <summary>
    /// Every mark is painted with the tone the contract names for it.
    /// </summary>
    /// <remarks>
    /// Only <c>unattested_configuration</c> is <c>_ATTENTION</c>: a setting
    /// decides whether future sessions carry proof, so there is something to
    /// do about it. A permanently unattested session is <c>_NEUTRAL</c>
    /// rather than a refusal, because nothing was refused and nothing went
    /// wrong: the session simply has no attached call.
    /// </remarks>
    [Fact]
    public void EveryMarkIsPaintedWithItsContractedTone()
    {
        Assert.Equal(PrivateInferenceTone.Clear, Decide(Attested).Tone);
        Assert.Equal(PrivateInferenceTone.Neutral, Decide(UnattestedPermanent).Tone);
        Assert.Equal(
            PrivateInferenceTone.Attention, Decide(UnattestedConfiguration).Tone);
        Assert.Equal(PrivateInferenceTone.Neutral, Decide(Unknown).Tone);
    }

    /// <summary>
    /// A mark this build has never heard of degrades to the <c>unknown</c>
    /// sentence and the neutral tone, and never borrows another mark's words.
    /// </summary>
    /// <remarks>
    /// The failure that matters is the opposite one: a later daemon growing a
    /// mark, and this shell reporting a contributor's session as carrying no
    /// proof because it could not read the word. Saying the answer is not
    /// known is the only honest reading of a label nobody here can parse.
    /// </remarks>
    [Fact]
    public void AnUnfamiliarMarkDegradesToUnknown()
    {
        PrivateInferenceCopy copy = Copy();
        foreach (string? unfamiliar in new[] { null, string.Empty, "  ", "attested_v2", "ATTESTED" })
        {
            AttestationMarkDecision decision = Decide(unfamiliar);
            Assert.Equal(copy.AttestationUnknown, decision.MarkLine);
            Assert.Equal(PrivateInferenceTone.Neutral, decision.Tone);
            Assert.NotEqual(copy.AttestationAttested, decision.MarkLine);
            Assert.NotEqual(copy.AttestationUnattestedPermanent, decision.MarkLine);
            Assert.NotEqual(copy.AttestationUnattestedConfiguration, decision.MarkLine);
        }
    }

    /// <summary>
    /// THE REASON IS BRANCHED ON THE KEY'S PRESENCE, NEVER ON THE MARK.
    /// </summary>
    /// <remarks>
    /// The rule most easily got wrong. <c>attested</c> never carries a
    /// reason and both unattested marks always do, so a shell can appear
    /// correct while deciding from the mark. <c>unknown</c> is where that
    /// breaks: it carries no reason when the row was simply never evaluated,
    /// and carries <c>receipt_unavailable</c> when a send was refused because
    /// the receipt service was down. That second one is a retraction rather
    /// than a refusal, and suppressing it drops the only signal telling a
    /// contributor the session may attest later.
    /// </remarks>
    [Fact]
    public void TheReasonFollowsTheKeyAndNotTheMark()
    {
        // Never evaluated: no reason key, so no second line.
        AttestationMarkDecision unevaluated = Decide(Unknown);
        Assert.False(unevaluated.HasReasonLine);
        Assert.Null(unevaluated.ReasonLine);

        // Refused for an unavailable receipt: the same mark, WITH its reason.
        AttestationMarkDecision retracted = Decide(Unknown, ReceiptUnavailable);
        Assert.True(retracted.HasReasonLine);
        Assert.Equal(Copy().AttestationReasonReceiptUnavailable, retracted.ReasonLine);

        // And the mark itself is identical either way, which is exactly why
        // branching on it cannot work.
        Assert.Equal(unevaluated.MarkLine, retracted.MarkLine);
    }

    /// <summary>
    /// The unattested marks carry their reasons, and <c>attested</c> draws
    /// none because none arrives.
    /// </summary>
    [Fact]
    public void TheUnattestedMarksCarryReasonsAndAttestedDoesNot()
    {
        PrivateInferenceCopy copy = Copy();
        Assert.Equal(
            copy.AttestationReasonNoCall,
            Decide(UnattestedPermanent, "no_inference_call").ReasonLine);
        Assert.Equal(
            copy.AttestationReasonEvidenceCaptureOff,
            Decide(UnattestedConfiguration, "evidence_capture_off").ReasonLine);
        Assert.False(Decide(Attested).HasReasonLine);
    }

    /// <summary>
    /// THE MARK'S REASON SENTENCES ARE NOT THE ELIGIBILITY ONES.
    /// </summary>
    /// <remarks>
    /// The thirteen labels are shared and the two accessors are not.
    /// Substituting one for the other compiles and returns a plausible
    /// sentence, which is what makes it worth pinning.
    ///
    /// <para>
    /// Most of the shared labels name a property of the recorded session and
    /// are worded identically on both sides, so equality proves nothing about
    /// which accessor was reached. The labels where the vocabularies diverge
    /// are the ones that do: eligibility's read as refusals of a request, and
    /// for an invited contributor nothing was requested. The test asserts the
    /// attestation wording on every one of those, and fails loudly rather
    /// than silently passing if a copy change ever leaves none of them.
    /// </para>
    /// </remarks>
    [Fact]
    public void NoReasonSentenceIsBorrowedFromEligibility()
    {
        PrivateInferenceCopy copy = Copy();
        (string Label, string Attestation, string Eligibility)[] pairs = new[]
        {
            ("no_inference_call", copy.AttestationReasonNoCall, copy.EligibilityReasonNoCall),
            ("capture_off", copy.AttestationReasonCaptureOff, copy.EligibilityReasonCaptureOff),
            ("digest_absent", copy.AttestationReasonDigestAbsent, copy.EligibilityReasonDigestAbsent),
            ("upstream_id_absent", copy.AttestationReasonUpstreamIdAbsent, copy.EligibilityReasonUpstreamIdAbsent),
            ("digest_mismatch", copy.AttestationReasonDigestMismatch, copy.EligibilityReasonDigestMismatch),
            ("reference_malformed", copy.AttestationReasonReferenceMalformed, copy.EligibilityReasonReferenceMalformed),
            ("bodies_unreadable", copy.AttestationReasonBodiesUnreadable, copy.EligibilityReasonBodiesUnreadable),
            ("body_not_utf8", copy.AttestationReasonBodyNotUtf8, copy.EligibilityReasonBodyNotUtf8),
            ("body_too_large", copy.AttestationReasonBodyTooLarge, copy.EligibilityReasonBodyTooLarge),
            ("evidence_capture_off", copy.AttestationReasonEvidenceCaptureOff, copy.EligibilityReasonEvidenceCaptureOff),
            ("marker_absent", copy.AttestationReasonMarkerAbsent, copy.EligibilityReasonMarkerAbsent),
            ("request_malformed", copy.AttestationReasonRequestMalformed, copy.EligibilityReasonRequestMalformed),
            ("receipt_unavailable", copy.AttestationReasonReceiptUnavailable, copy.EligibilityReasonReceiptUnavailable),
        };

        Assert.Equal(13, pairs.Length);

        int discriminating = 0;
        foreach ((string label, string attestation, string eligibility) in pairs)
        {
            Assert.False(string.IsNullOrWhiteSpace(attestation));

            // The surface reaches the attestation accessor for every label.
            Assert.Equal(attestation, Decide(Unknown, label).ReasonLine);

            // On the labels the two vocabularies word differently, that is a
            // real discrimination rather than a coincidence.
            if (!string.Equals(attestation, eligibility, StringComparison.Ordinal))
            {
                discriminating++;
                Assert.NotEqual(eligibility, Decide(Unknown, label).ReasonLine);
            }
        }

        // If a copy change ever made all thirteen identical, the loop above
        // would stop proving anything and would go on passing. Fail instead.
        Assert.True(
            discriminating > 0,
            "no shared reason label is worded differently, so nothing here "
            + "distinguishes the attestation accessor from the eligibility one");
    }

    /// <summary>
    /// A reason label this build does not know draws nothing at all.
    /// </summary>
    /// <remarks>
    /// Deliberately unlike the mark's fallback. The mark sentence has already
    /// said what is true and a row must say something; a second sentence
    /// guessing at a reason nobody established would add a detail this build
    /// invented.
    /// </remarks>
    [Fact]
    public void AnUnfamiliarReasonDrawsNothing()
    {
        foreach (string? unfamiliar in new[] { null, string.Empty, "no_inference_call_v2" })
        {
            Assert.Null(Decide(UnattestedPermanent, unfamiliar).ReasonLine);
            Assert.False(Decide(UnattestedPermanent, unfamiliar).HasReasonLine);
        }
    }

    /// <summary>
    /// A queue entry from a daemon predating the fields reports
    /// <c>unknown</c>, never an unattested mark.
    /// </summary>
    /// <remarks>
    /// The opposite of the eligibility rule, and the reason there is no
    /// "absent" decision here. A missing key is not evidence that a session
    /// carries no proof; it is evidence that this build was never told.
    /// </remarks>
    [Fact]
    public void AnEntryWithoutTheKeysReportsUnknown()
    {
        QueueEntry entry = JsonSerializer.Deserialize<QueueEntry>(
            "{\"entry_id\":\"e1\",\"size_bytes\":10}")!;
        Assert.Null(entry.Attestation);
        Assert.Null(entry.AttestationReason);

        AttestationMarkDecision decision = AttestationMarkSurface.Decide(entry);
        Assert.True(decision.HasMarkLine);
        Assert.Equal(Copy().AttestationUnknown, decision.MarkLine);
        Assert.Equal(PrivateInferenceTone.Neutral, decision.Tone);
        Assert.False(decision.HasReasonLine);
    }

    /// <summary>An entry that carries the keys decodes as that mark and reason.</summary>
    [Fact]
    public void AnEntryWithTheKeysDecodesAsThatMark()
    {
        QueueEntry entry = JsonSerializer.Deserialize<QueueEntry>(
            "{\"entry_id\":\"e1\",\"attestation\":\"unattested_configuration\","
            + "\"attestation_reason\":\"evidence_capture_off\"}")!;

        Assert.Equal("unattested_configuration", entry.Attestation);
        Assert.Equal("evidence_capture_off", entry.AttestationReason);

        AttestationMarkDecision decision = AttestationMarkSurface.Decide(entry);
        Assert.Equal(Copy().AttestationUnattestedConfiguration, decision.MarkLine);
        Assert.Equal(Copy().AttestationReasonEvidenceCaptureOff, decision.ReasonLine);
        Assert.Equal(PrivateInferenceTone.Attention, decision.Tone);
    }

    /// <summary>
    /// THE MARK OFFERS NO CONTROL, AND THAT IS THE CONTRACT.
    /// </summary>
    /// <remarks>
    /// There is no <c>tc_contribution_attestation_control</c> to ask. The
    /// mark describes the trace and offers nothing to press; whether a
    /// session may be sent stays
    /// <see cref="ContributionEligibilitySurface"/>'s question. A control
    /// derived from the mark would be an action invented out of a
    /// description, so nothing on this decision may carry one.
    /// </remarks>
    [Fact]
    public void TheDecisionCarriesNoControl()
    {
        MemberInfo[] members = typeof(AttestationMarkDecision)
            .GetMembers(BindingFlags.Public | BindingFlags.Instance);
        Assert.DoesNotContain(
            members, m => m.Name.Contains("Control", StringComparison.Ordinal));
        Assert.DoesNotContain(
            members, m => m.Name.Contains("Contribute", StringComparison.Ordinal));

        Assert.DoesNotContain(
            typeof(AttestationMarkSurface).GetMembers(BindingFlags.Public | BindingFlags.Static),
            m => m.Name.Contains("Control", StringComparison.Ordinal));

        // Nor may any member of the decision be typed as one.
        Assert.DoesNotContain(
            typeof(AttestationMarkDecision).GetProperties(),
            p => p.PropertyType == typeof(ContributionControl));
    }
}
