using System;

namespace TraceCommons.Interop;

/// <summary>
/// What one queue entry said about whether it carries proof of the model call
/// that produced it.
/// </summary>
/// <remarks>
/// <b>THERE IS NO ABSENT CASE, WHICH IS THE OPPOSITE OF
/// <see cref="ContributionEligibility"/>.</b> Eligibility asks whether this
/// contributor may send this session and correctly says nothing when nobody
/// asked; the mark states a fact about the trace, and that is owed to
/// everybody. An invited contributor's rows carry no eligibility and every
/// one of them carries a mark.
///
/// <para>
/// A null <see cref="Mark"/> is therefore not a fifth state and not a reason
/// to draw nothing: it is a daemon too old to have sent the field, and the
/// shared table answers <c>unknown</c> for it. Not having been told is not
/// evidence that a session carries no proof.
/// </para>
///
/// <para>
/// The mark is carried as the daemon's own string and handed to the shared
/// table, for the reason <see cref="PrivateInferenceState"/> carries its
/// label that way: a mark a later daemon grows would otherwise have to be
/// spelled in this shell before it could be shown, and the shared table
/// already answers an unfamiliar label safely.
/// </para>
/// </remarks>
public readonly record struct AttestationMark(string? Mark, string? Reason)
{
    /// <summary>What one queue entry carried.</summary>
    public static AttestationMark From(QueueEntry entry)
    {
        ArgumentNullException.ThrowIfNull(entry);
        return new AttestationMark(entry.Attestation, entry.AttestationReason);
    }
}

/// <summary>
/// What one queue row draws for its attestation mark, resolved once.
/// </summary>
/// <remarks>
/// Every member below came from the shared crate across the C ABI. Nothing
/// here was computed from the mark string in this shell.
///
/// <para>
/// <b>No control, deliberately.</b> There is no
/// <c>tc_contribution_attestation_control</c> to ask, because the mark
/// describes the trace and offers nothing to press. Sendability remains
/// <see cref="ContributionEligibilitySurface"/>'s question, and a button
/// drawn off the mark would be an action invented out of a description.
/// </para>
/// </remarks>
public readonly record struct AttestationMarkDecision(
    string MarkLine,
    PrivateInferenceTone Tone,
    string? ReasonLine)
{
    /// <summary>
    /// Whether the sentence is drawable. True for every mark the ABI can
    /// answer, which is all of them; false only where a caught panic left
    /// nothing to say, and a blank line is worse than none.
    /// </summary>
    public bool HasMarkLine => MarkLine is { Length: > 0 };

    /// <summary>Whether a second sentence naming the reason belongs under it.</summary>
    public bool HasReasonLine => ReasonLine is { Length: > 0 };
}

/// <summary>
/// The attestation mark surface, across the C ABI.
///
/// Holds no words and owns no branch. Every sentence and the tone come from
/// <c>crates/trace-commons-contributor/src/private_inference_copy.rs</c>.
/// </summary>
public static class AttestationMarkSurface
{
    /// <summary>
    /// Everything one row draws, in one call.
    /// </summary>
    /// <remarks>
    /// Unconditional. The eligibility surface has an absent arm that skips
    /// the ABI entirely; this one must not grow the same shape, because the
    /// contributor whose rows would fall into it is precisely the one this
    /// mark was added for.
    /// </remarks>
    public static AttestationMarkDecision Decide(AttestationMark mark) =>
        new(
            MarkLine: MarkLine(mark.Mark),
            Tone: PrivateInferenceSurface.FromAbiTone(
                NativeMethods.tc_contribution_attestation_tone(mark.Mark)),
            ReasonLine: ReasonLine(mark.Reason));

    /// <summary>Everything one queue entry's row draws.</summary>
    public static AttestationMarkDecision Decide(QueueEntry entry) =>
        Decide(AttestationMark.From(entry));

    /// <summary>
    /// The sentence for one mark. A caught panic falls back to the empty
    /// string, which draws no line -- never to a borrowed sentence.
    /// </summary>
    private static string MarkLine(string? mark) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_contribution_attestation_line(mark))
        ?? string.Empty;

    /// <summary>
    /// The sentence for one reason label, or null where there is nothing
    /// honest to say.
    /// </summary>
    /// <remarks>
    /// <b>ASKED OF THE REASON KEY, NEVER OF THE MARK.</b> The presence of a
    /// reason varies within a single mark: <c>unknown</c> carries none when
    /// the row was never evaluated, and carries <c>receipt_unavailable</c>
    /// when a send was refused because the receipt service was down. That
    /// second one is a retraction rather than a refusal, and suppressing it
    /// drops the only signal telling a contributor the session may attest
    /// later.
    ///
    /// <para>
    /// The ABI answers the EMPTY STRING for an absent, null or unfamiliar
    /// reason and this renders nothing for it. Deliberately unlike the mark
    /// line's fallback: the mark sentence has already said what is true, and
    /// a second sentence guessing at a reason this build does not know would
    /// add a detail nobody established.
    /// </para>
    ///
    /// <para>
    /// The thirteen labels are shared with
    /// <see cref="ContributionEligibilitySurface.ReasonLine"/> and the
    /// sentences are not. Substituting one accessor for the other compiles,
    /// returns something plausible, and tells a contributor their session was
    /// turned down when nothing was ever asked of it.
    /// </para>
    /// </remarks>
    public static string? ReasonLine(string? reason)
    {
        string? line = NativeMethods.TakeOwnedString(
            NativeMethods.tc_contribution_attestation_reason_line(reason));
        return line is { Length: > 0 } ? line : null;
    }
}
