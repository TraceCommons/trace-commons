using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// The Flow 1 grant screens' sentences for this configuration
/// (macos/Sources/TCShellCore/AutomaticGrantCopy.swift): the scrub
/// disclosure's lines, in the core's words, for the disclosure it chose.
/// </summary>
public sealed record AutomaticGrantCopy
{
    public sealed record Scrub
    {
        [JsonPropertyName("scope")] public string Scope { get; init; } = "";
        [JsonPropertyName("limit")] public string Limit { get; init; } = "";
    }

    [JsonPropertyName("disclosure")] public string Disclosure { get; init; } = "";
    [JsonPropertyName("patterns_only")] public Scrub? PatternsOnly { get; init; }
    [JsonPropertyName("model_scrubbed")] public Scrub? ModelScrubbed { get; init; }
    [JsonPropertyName("summary")] public string Summary { get; init; } = "";
    [JsonPropertyName("no_review")] public string NoReview { get; init; } = "";

    /// <summary>The scrub the core chose (AutomaticGrantCopy.swift:52-58).</summary>
    public Scrub? Chosen => Disclosure switch
    {
        "patterns_only" => PatternsOnly,
        "model_scrubbed" => ModelScrubbed,
        _ => null,
    };

    /// <summary>The scrub sheet's lines: scope, limit, no review (AutomaticGrantCopy.swift:60-63).</summary>
    public IReadOnlyList<string> Lines => Chosen is { } s ? new[] { s.Scope, s.Limit, NoReview } : Array.Empty<string>();

    /// <summary>Decoded, or null for anything the Mac refuses: no lines, an empty line, or no summary.</summary>
    public static AutomaticGrantCopy? Decode(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            AutomaticGrantCopy? copy = JsonSerializer.Deserialize<AutomaticGrantCopy>(json);
            if (copy is null || copy.Lines.Count == 0 || copy.Lines.Any(string.IsNullOrEmpty) || copy.Summary.Length == 0)
            {
                return null;
            }

            return copy;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary><c>tc_automatic_contribution_copy_json</c> for the daemon's config directory.</summary>
    public static AutomaticGrantCopy? Load(string configDir) =>
        Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_automatic_contribution_copy_json(configDir)));
}

/// <summary>The core's answer to whether the grant may be asked for (<c>flow1::grant_request</c>).</summary>
public sealed record Flow1GrantRequest(bool Ready, IReadOnlyList<string> Blockers, string? WitnessSigningAddress)
{
    public static Flow1GrantRequest? Decode(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            JsonElement root = doc.RootElement;
            if (root.ValueKind != JsonValueKind.Object
                || !root.TryGetProperty("ready", out JsonElement ready) || ready.ValueKind is not (JsonValueKind.True or JsonValueKind.False)
                || !root.TryGetProperty("blockers", out JsonElement blockers) || blockers.ValueKind != JsonValueKind.Array)
            {
                return null;
            }

            var list = new List<string>();
            foreach (JsonElement b in blockers.EnumerateArray())
            {
                if (b.ValueKind != JsonValueKind.String)
                {
                    return null;
                }

                list.Add(b.GetString()!);
            }

            string? witness = root.TryGetProperty("witness_signing_address", out JsonElement w) && w.ValueKind == JsonValueKind.String ? w.GetString() : null;
            return new Flow1GrantRequest(ready.GetBoolean(), list, witness);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary><c>tc_flow1_grant_request_json</c>: the core decides, from what the person was shown.</summary>
    public static Flow1GrantRequest? Ask(string progressJson) =>
        Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_flow1_grant_request_json(progressJson)));
}

/// <summary>Which disclosure sheet is up (SharingDisclosures.swift <c>Step</c>).</summary>
public enum DisclosureStep
{
    Scrub,
    Witness,
    Done,
}

/// <summary>
/// The Automatic path's two disclosures, in order: the scrub disclosure,
/// then the witness disclosure (macos/Sources/TraceCommonsApp/Views/FirstRun/SharingDisclosures.swift:1-102).
/// Pure; it records what the person was shown, and the core decides from
/// that whether the grant may be asked for.
/// </summary>
public sealed class SharingDisclosureFlow
{
    /// <summary>The label a not-ready answer carries when the core's answer could not be read at all.</summary>
    public const string UnreadableLabel = "flow1-grant-request-unreadable";

    public DisclosureStep Step { get; private set; } = DisclosureStep.Scrub;

    /// <summary>The signing address the witness sheet showed, null for none.</summary>
    public string? WitnessShown { get; private set; }

    /// <summary>Start goes through the disclosures on Automatic, on an account that can choose it.</summary>
    public static bool IsNeeded(FirstRunState state) =>
        state.Sharing == SharingPath.Automatic && FirstRunNavigation.CanChooseAutomatic(state);

    /// <summary>The scrub sheet's Continue; ignored out of order.</summary>
    public void AcknowledgeScrub()
    {
        if (Step == DisclosureStep.Scrub)
        {
            Step = DisclosureStep.Witness;
        }
    }

    /// <summary>The witness sheet's Continue with the address it showed; ignored before the scrub sheet.</summary>
    public void AcknowledgeWitness(string? shown)
    {
        if (Step == DisclosureStep.Witness)
        {
            WitnessShown = shown;
            Step = DisclosureStep.Done;
        }
    }

    /// <summary>
    /// The progress the core reads (SharingDisclosures.swift:66-75):
    /// <paramref name="connected"/> from the daemon's status, never a step
    /// remembered here; the scopes Start saves first.
    /// </summary>
    public string ProgressJson(bool connected, IEnumerable<string> scopes)
    {
        var o = new Dictionary<string, object?>(StringComparer.Ordinal)
        {
            ["connected"] = connected,
            ["scopes_saved"] = scopes.OrderBy(s => s, StringComparer.Ordinal).ToList(),
            ["path"] = "automatic",
            ["scrub_disclosure_seen"] = Step != DisclosureStep.Scrub,
            ["witness_disclosure_seen"] = Step == DisclosureStep.Done,
            ["witness_shown"] = WitnessShown,
        };
        return JsonSerializer.Serialize(o);
    }

    /// <summary>
    /// The state Start commits, and the refusal to show once it has run
    /// (SharingDisclosures.swift:84-101). Ready: Automatic stands with the
    /// core's witness and <see cref="FirstRunState.GrantReady"/>, the only
    /// place it is set. Anything else finishes on Ask and says why.
    /// </summary>
    public static (FirstRunState State, FirstRunFailure? Refused) Resolve(FirstRunState state, Flow1GrantRequest? request)
    {
        FirstRunState next = state.Clone();
        if (request is { Ready: true })
        {
            next.WitnessSigningAddress = request.WitnessSigningAddress;
            next.GrantReady = true;
            return (next, null);
        }

        next.Sharing = SharingPath.AskMe;
        next.WitnessSigningAddress = null;
        next.GrantReady = false;
        return (next, new FirstRunFailure.GrantRefused(request?.Blockers.FirstOrDefault() ?? UnreadableLabel));
    }
}
