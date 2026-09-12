using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

public sealed record InsightMutationEffects(IReadOnlyList<string> InvalidatedEpisodeIds)
{
    public static InsightMutationEffects Decode(JsonElement response)
    {
        if (!response.TryGetProperty("mutation_effects", out var effects))
            return new(Array.Empty<string>());
        return new(effects.GetProperty("invalidated_episode_ids").EnumerateArray()
            .Select(id => id.GetString() ?? throw new JsonException("insights-response-invalid"))
            .ToArray());
    }
}
