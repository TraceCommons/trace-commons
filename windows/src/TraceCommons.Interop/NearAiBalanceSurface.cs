using System;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// What <c>near_ai_balance</c> reported.
/// </summary>
/// <remarks>
/// The state is carried as the daemon's own string and handed to the shared
/// table, for the reason <see cref="NearAiCredentialStatus"/> carries its
/// label that way: a state a later daemon grows would otherwise have to be
/// spelled in this shell before it could be shown, and the shared table
/// already answers an unfamiliar label safely.
///
/// <para>
/// EVERY FIGURE IS NULLABLE, AND THE NULLS ARE LOAD-BEARING. The wire sends
/// each numeric key present-and-null in every state it cannot fill, so a null
/// here is always "we know we do not know" and never "this daemon is too old
/// to say". Defaulting one to zero on the way in would defeat every
/// distinction the shared table makes on the way out.
/// </para>
///
/// <para>
/// <see cref="Scale"/> is nullable for a different reason: it is the units,
/// not a figure. A payload that carried no readable scale carries no figure
/// this shell may draw, because a missing scale read as zero turns 8.5
/// dollars into $8,500,000,000.00. Absent units means no figure at all.
/// </para>
/// </remarks>
public readonly record struct NearAiBalance(
    string State,
    byte? Scale,
    long? RemainingNanos,
    long? SpendLimitNanos,
    long? TotalSpentNanos,
    DateTimeOffset? ObservedAt)
{
    /// <summary>
    /// Nothing was reported. The empty state, which the shared table answers
    /// with the sentence saying this daemon does not report a balance --
    /// never with one saying no sign-in is kept here, and never with one
    /// about what is in the account.
    /// </summary>
    public static NearAiBalance Unreported => new(string.Empty, null, null, null, null, null);
}

/// <summary>
/// The NEAR AI balance surface, across the C ABI.
///
/// Holds no words, owns no branch and formats no money. Every sentence, every
/// figure, the tone and the action come from
/// <c>crates/trace-commons-contributor/src/private_inference_copy.rs</c>.
/// </summary>
public static class NearAiBalanceSurface
{
    /// <summary>
    /// What the daemon reported, or <see cref="NearAiBalance.Unreported"/>
    /// when the answer could not be read.
    /// </summary>
    /// <remarks>
    /// A malformed or missing reply is UNREPORTED and never a figure. The
    /// alternative failure -- a read that did not land rendering as an empty
    /// account -- is the one thing on this surface a contributor would act
    /// on, and it would be false.
    /// </remarks>
    public static NearAiBalance Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return NearAiBalance.Unreported;
        }

        try
        {
            using JsonDocument document = JsonDocument.Parse(json);
            if (document.RootElement.ValueKind != JsonValueKind.Object)
            {
                return NearAiBalance.Unreported;
            }

            JsonElement root = document.RootElement;
            return new NearAiBalance(
                ReadString(root, StateField) ?? string.Empty,
                ReadScale(root),
                ReadInt64(root, RemainingField),
                ReadInt64(root, SpendLimitField),
                ReadInt64(root, TotalSpentField),
                ReadTimestamp(root, ObservedAtField));
        }
        catch (JsonException)
        {
            return NearAiBalance.Unreported;
        }
    }

    /// <summary>
    /// The sentence for this state, or the EMPTY STRING for <c>known</c>,
    /// whose row is figures.
    /// </summary>
    /// <remarks>
    /// Falls back to the payload's could-not-read sentence when the Rust
    /// caught a panic -- never to the one about this machine, and never to
    /// one about the account.
    /// </remarks>
    public static string StateLine(NearAiBalance balance, PrivateInferenceCopy copy)
    {
        ArgumentNullException.ThrowIfNull(copy);
        return NativeMethods.TakeOwnedString(
                NativeMethods.tc_near_ai_balance_state_line(balance.State))
            ?? copy.BalanceUnknown;
    }

    /// <summary>
    /// How firmly the row reads.
    /// </summary>
    /// <remarks>
    /// THE TONE MEANS THE READ SUCCEEDED, NOT THAT THE BALANCE IS HEALTHY. It
    /// is a function of the state alone and never sees an amount, which is
    /// what keeps a shell from inventing a threshold nobody set.
    /// </remarks>
    public static PrivateInferenceTone Tone(NearAiBalance balance) =>
        PrivateInferenceSurface.FromAbiTone(
            NativeMethods.tc_near_ai_balance_state_tone(balance.State));

    /// <summary>
    /// The one action a shell may offer beside this state.
    /// </summary>
    /// <remarks>
    /// The sign-in row's own enum, decoded by the sign-in row's own decoder.
    /// Two states answer obtain and everything else answers none, including
    /// every state this build cannot read: the offer is what mints the second
    /// key.
    /// </remarks>
    public static CredentialAction Action(NearAiBalance balance) =>
        NearAiCredentialSurface.FromAbiAction(
            NativeMethods.tc_near_ai_balance_action(balance.State));

    /// <summary>
    /// One figure as money, or the EMPTY STRING where there is no figure to
    /// draw.
    /// </summary>
    /// <remarks>
    /// AN EMPTY STRING IS NEVER <c>$0.00</c>. Presence crosses as its own
    /// argument because these amounts are signed: an overdrawn account is a
    /// negative figure, so absence cannot ride on an out-of-range integer the
    /// way it does for every other money export here. A null means we know we
    /// do not know; a zero is a real balance and means the money is gone.
    ///
    /// <para>
    /// A reading with no scale has no drawable figure, for the reason
    /// <see cref="NearAiBalance.Scale"/> gives.
    /// </para>
    /// </remarks>
    public static string Amount(long? nanos, byte? scale) =>
        scale is not { } units
            ? string.Empty
            : NativeMethods.TakeOwnedString(
                NativeMethods.tc_near_ai_balance_amount(Present(nanos), nanos ?? 0, units))
                ?? string.Empty;

    /// <summary>
    /// What is left, as a finished sentence.
    /// </summary>
    /// <remarks>
    /// AN ABSENT FIGURE DOES NOT GIVE THE EMPTY STRING HERE. It gives the
    /// sentence for an account with no spending limit set -- the ordinary
    /// case for an account nobody has capped -- because the remaining figure
    /// is nullable even when the state is <c>known</c>, and a contributor
    /// with no cap must not be told they have $0.00 left.
    /// </remarks>
    public static string RemainingLine(long? nanos, byte? scale) =>
        Line(NativeMethods.tc_near_ai_balance_remaining_line, nanos, scale);

    /// <summary>
    /// The configured ceiling, or the empty string. An absent ceiling is no
    /// line: the remaining sentence has already said the part that matters
    /// about an uncapped account.
    /// </summary>
    public static string LimitLine(long? nanos, byte? scale) =>
        Line(NativeMethods.tc_near_ai_balance_limit_line, nanos, scale);

    /// <summary>
    /// What the WHOLE ACCOUNT has spent, or the empty string. A zero is not
    /// that: an account that has spent nothing has spent $0.00.
    /// </summary>
    public static string SpentLine(long? nanos, byte? scale) =>
        Line(NativeMethods.tc_near_ai_balance_spent_line, nanos, scale);

    /// <summary>
    /// How long ago THIS COMPUTER asked, or the empty string.
    /// </summary>
    /// <remarks>
    /// The age is computed here because the wire carries an instant, and the
    /// ABI takes an elapsed count with absence encoded as any negative value.
    /// A timestamp in the future is not an age and is passed as an absence:
    /// a clock that disagrees with the daemon's is not evidence that a
    /// question will be put later.
    /// </remarks>
    public static string ObservedLine(DateTimeOffset? observedAt, DateTimeOffset now)
    {
        if (observedAt is not { } asked)
        {
            return string.Empty;
        }

        double seconds = (now - asked).TotalSeconds;
        long elapsed = seconds is >= 0 and <= long.MaxValue ? (long)seconds : -1;
        return NativeMethods.TakeOwnedString(
            NativeMethods.tc_near_ai_balance_observed_line(elapsed)) ?? string.Empty;
    }

    private static string Line(
        Func<int, long, byte, IntPtr> export, long? nanos, byte? scale) =>
        scale is not { } units
            ? string.Empty
            : NativeMethods.TakeOwnedString(export(Present(nanos), nanos ?? 0, units))
                ?? string.Empty;

    /// <summary>
    /// The wire's null as the ABI's <c>present</c>: 0 for absent, non-zero
    /// otherwise. The value passed alongside it is never read when this is 0.
    /// </summary>
    private static int Present(long? nanos) => nanos.HasValue ? 1 : 0;

    private static string? ReadString(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement value)
        && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    private static long? ReadInt64(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement value)
        && value.ValueKind == JsonValueKind.Number
        && value.TryGetInt64(out long number)
            ? number
            : null;

    private static byte? ReadScale(JsonElement element) =>
        element.TryGetProperty(ScaleField, out JsonElement value)
        && value.ValueKind == JsonValueKind.Number
        && value.TryGetByte(out byte scale)
            ? scale
            : null;

    private static DateTimeOffset? ReadTimestamp(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement value)
        && value.ValueKind == JsonValueKind.String
        && value.TryGetDateTimeOffset(out DateTimeOffset stamp)
            ? stamp
            : null;

    private const string StateField = "state";
    private const string ScaleField = "scale";
    private const string RemainingField = "remaining_nanos";
    private const string SpendLimitField = "spend_limit_nanos";
    private const string TotalSpentField = "total_spent_nanos";
    private const string ObservedAtField = "observed_at";
}
