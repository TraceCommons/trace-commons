using System;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// The daemon's answer to <c>arming_suggestion</c>: the one project worth
/// offering to arm right now, and the evidence for offering it.
/// </summary>
/// <remarks>
/// The daemon answers with an empty object when there is nothing to suggest,
/// which <see cref="Parse"/> reports as null rather than as a zero-filled
/// offer -- a shell that receives no suggestion must draw no card, and a
/// zero-filled one would be a claim about a project the daemon never made.
/// </remarks>
public sealed record ArmingOffer(string ProjectId, string ProjectLabel, int ContributedCount)
{
    /// <summary>
    /// Reads an <c>arming_suggestion</c> result, or null when there is no
    /// suggestion or the payload does not fit.
    /// </summary>
    public static ArmingOffer? Parse(JsonElement data)
    {
        if (data.ValueKind != JsonValueKind.Object
            || !data.TryGetProperty("project_id", out JsonElement id)
            || id.ValueKind != JsonValueKind.String
            || id.GetString() is not { Length: > 0 } projectId)
        {
            return null;
        }

        string label = data.TryGetProperty("project_label", out JsonElement l)
            && l.ValueKind == JsonValueKind.String
                ? l.GetString() ?? string.Empty
                : string.Empty;

        int count = data.TryGetProperty("contributed_count", out JsonElement c)
            && c.TryGetInt32(out int parsed)
                ? parsed
                : 0;

        return new ArmingOffer(projectId, label, count);
    }
}

/// <summary>
/// The words for the arming offer.
/// </summary>
/// <remarks>
/// <para>
/// This is the offer, not the confirmation. It appears in the queue once a
/// project has been approved several times, and its whole job is to make the
/// case from evidence the contributor already has: they have read previews
/// from this project and kept approving them. Arming asks someone to stop
/// reading those previews, and the only honest basis for that question is the
/// history of them saying yes.
/// </para>
/// <para>
/// The macOS shell says the same thing in <c>ArmingOfferCopy</c> and the
/// Linux shell in <c>copy::arming_offer_evidence</c>. Three shells wording
/// the same offer differently is worse than any one of them.
/// </para>
/// </remarks>
public static class ArmingOfferCopy
{
    /// <summary>
    /// The evidence, stated before the question, so a contributor who reads
    /// only the first line still learns why they are being asked.
    /// </summary>
    public static string Evidence(string projectLabel, int count)
    {
        string times = count == 1 ? "once" : $"{count} times";
        return $"You've contributed from {projectLabel} {times}.";
    }

    public static string Question(string projectLabel) =>
        $"Contribute from {projectLabel} automatically?";

    /// <summary>Carries the action rather than agreeing in the abstract.</summary>
    public const string Confirm = "Turn on automatic contributing";

    /// <summary>
    /// "Not now" rather than "No": the daemon silences the offer for thirty
    /// days rather than forever, and this must not promise otherwise.
    /// Settings still arms the project at any point in between.
    /// </summary>
    public const string Decline = "Not now";
}
