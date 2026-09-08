using System;

namespace TraceCommons.Interop;

/// <summary>
/// The one control a shell may offer for one queue entry's eligibility state.
/// </summary>
/// <remarks>
/// The ABI numbering these decode from is a range of its own, disjoint from
/// <see cref="CredentialAction"/>'s and from every tone range, for the reason
/// those are disjoint from each other: one numbering shared between them is
/// one renumbering away from drawing a sign-in button on a queue row.
///
/// <para>
/// <see cref="None"/> is not "hide the row". Every session is shown, because
/// hiding a contributor's own work is its own dishonesty and makes this app
/// look as though it had not noticed files the contributor knows it can see.
/// The row is present, unoffered, and carries its sentence.
/// </para>
/// </remarks>
public enum ContributionControl
{
    /// <summary>Draw no send control. The row still draws.</summary>
    None,

    /// <summary>Offer to send this session.</summary>
    Contribute,
}

/// <summary>
/// What a queue entry said about whether it can be contributed.
/// </summary>
/// <remarks>
/// <b>A NULL <see cref="State"/> IS AN ABSENT FIELD, WHICH IS NOT
/// <c>unknown</c>.</b> The daemon omits <c>eligibility</c> entirely for a
/// contributor who was invited rather than admitted on evidence: they have no
/// eligibility question, and a row answering one they do not have puts a
/// caveat on work that carries none. <c>unknown</c> is a real state that
/// arrives on the wire and is rendered as one.
///
/// <para>
/// The state is carried as the daemon's own string and handed to the shared
/// table, for the reason <see cref="PrivateInferenceState"/> carries its
/// label that way: a state a later daemon grows would otherwise have to be
/// spelled in this shell before it could be shown, and the shared table
/// already answers an unfamiliar label safely.
/// </para>
/// </remarks>
public readonly record struct ContributionEligibility(string? State, string? Reason)
{
    /// <summary>
    /// No eligibility question was asked of this row. Not a state, and never
    /// rendered as one.
    /// </summary>
    public static ContributionEligibility Absent => new(null, null);

    /// <summary>What one queue entry carried, or <see cref="Absent"/>.</summary>
    public static ContributionEligibility From(QueueEntry entry)
    {
        ArgumentNullException.ThrowIfNull(entry);
        return new ContributionEligibility(entry.Eligibility, entry.EligibilityReason);
    }

    /// <summary>
    /// Whether the daemon answered the question at all. False is the invited
    /// contributor and the unreadable setting; it is NOT a negative answer.
    /// </summary>
    public bool IsAnswered => State is not null;
}

/// <summary>
/// What one queue row draws for its eligibility, resolved once.
/// </summary>
/// <remarks>
/// Every member below either came from the shared crate across the C ABI or
/// is the deliberate no-question case. Nothing here was computed from the
/// state string in this shell.
/// </remarks>
public readonly record struct ContributionEligibilityDecision(
    bool IsAnswered,
    string? StateLine,
    PrivateInferenceTone? Tone,
    ContributionControl Control,
    string? ReasonLine)
{
    /// <summary>Whether a sentence about eligibility belongs on this row.</summary>
    public bool HasStateLine => StateLine is { Length: > 0 };

    /// <summary>Whether a second sentence naming the reason belongs on it.</summary>
    public bool HasReasonLine => ReasonLine is { Length: > 0 };

    /// <summary>Whether a send control may be drawn.</summary>
    public bool OffersContribute => Control == ContributionControl.Contribute;
}

/// <summary>
/// The contribution eligibility surface, across the C ABI.
///
/// Holds no words and owns no branch. Every sentence, the tone and the control
/// come from
/// <c>crates/trace-commons-contributor/src/private_inference_copy.rs</c>.
/// </summary>
public static class ContributionEligibilitySurface
{
    /// <summary>
    /// Everything one row draws, in one call.
    /// </summary>
    /// <remarks>
    /// THE ABSENT CASE IS THE ONE THAT IS EASY TO GET WRONG, so it is decided
    /// here and nowhere else. An unanswered row reaches none of the three ABI
    /// entry points -- they are all defined over a state, and the state line
    /// in particular answers the "not worked out" sentence for anything it
    /// cannot read, which would put a caveat on an invited contributor's work
    /// where there is no question to caveat.
    ///
    /// <para>
    /// It keeps <see cref="ContributionControl.Contribute"/> because that is
    /// what the row already did before this field existed, and this slice
    /// must not take an offer away from a contributor whose queue was never
    /// in question. That is not a fifth arm of the state table: it is the
    /// absence of the table.
    /// </para>
    /// </remarks>
    public static ContributionEligibilityDecision Decide(ContributionEligibility eligibility)
    {
        if (!eligibility.IsAnswered)
        {
            return new ContributionEligibilityDecision(
                IsAnswered: false,
                StateLine: null,
                Tone: null,
                Control: ContributionControl.Contribute,
                ReasonLine: null);
        }

        return new ContributionEligibilityDecision(
            IsAnswered: true,
            StateLine: StateLine(eligibility.State),
            Tone: PrivateInferenceSurface.FromAbiTone(
                NativeMethods.tc_contribution_eligibility_tone(eligibility.State)),
            Control: FromAbiControl(
                NativeMethods.tc_contribution_eligibility_control(eligibility.State)),
            ReasonLine: ReasonLine(eligibility.Reason));
    }

    /// <summary>Everything one queue entry's row draws.</summary>
    public static ContributionEligibilityDecision Decide(QueueEntry entry) =>
        Decide(ContributionEligibility.From(entry));

    /// <summary>
    /// The sentence for one state. A caught panic falls back to the empty
    /// string, which draws no line -- never to a borrowed sentence.
    /// </summary>
    private static string StateLine(string? state) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_contribution_eligibility_line(state))
        ?? string.Empty;

    /// <summary>
    /// The sentence for one reason label, or null where there is nothing
    /// honest to say.
    /// </summary>
    /// <remarks>
    /// The ABI answers the EMPTY STRING for an absent, null or unfamiliar
    /// reason and this renders nothing for it. Deliberately unlike the state
    /// line's fallback: the state sentence has already said what is true, and
    /// a second sentence guessing at a reason this build does not know would
    /// add a detail nobody established.
    /// </remarks>
    public static string? ReasonLine(string? reason)
    {
        string? line = NativeMethods.TakeOwnedString(
            NativeMethods.tc_contribution_eligibility_reason_line(reason));
        return line is { Length: > 0 } ? line : null;
    }

    /// <summary>
    /// How many sessions a group submit is leaving behind, or null where
    /// there is nothing to say.
    /// </summary>
    /// <remarks>
    /// Null for zero and for a negative -- the shared crate answers the empty
    /// string for both, and a line reading "0 sessions are not being sent"
    /// invents a caveat where none exists. A negative is what a race between
    /// the daemon's count and the rows on screen produces; silence is the
    /// honest answer to a number that cannot be right.
    /// </remarks>
    public static string? WithheldLine(long withheld)
    {
        string? line = NativeMethods.TakeOwnedString(
            NativeMethods.tc_contribution_withheld_line(withheld));
        return line is { Length: > 0 } ? line : null;
    }

    /// <summary>
    /// The ABI value, spelled out rather than cast.
    ///
    /// Anything unknown is <see cref="ContributionControl.None"/>. That is
    /// the whole point of the range being its own: a tone value arriving
    /// here, through a cross-wiring or a later ABI, must offer nothing rather
    /// than land on contribute by arithmetic.
    /// </summary>
    internal static ContributionControl FromAbiControl(int value) =>
        value switch
        {
            AbiControlNone => ContributionControl.None,
            AbiControlContribute => ContributionControl.Contribute,
            _ => ContributionControl.None,
        };

    private const int AbiControlNone = 50;
    private const int AbiControlContribute = 51;
}
