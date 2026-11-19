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
