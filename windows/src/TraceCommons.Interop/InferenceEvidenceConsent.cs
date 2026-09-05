using System;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>Independent consent to send captured inference content to a witness.</summary>
public static class InferenceEvidenceConsent
{
    public static string Serialize(bool enabled, bool disclosureConfirmed)
    {
        if (enabled && !disclosureConfirmed)
        {
            throw new InvalidOperationException("inference-disclosure-required");
        }
        return JsonSerializer.Serialize(new { ironwire_attested_bodies = enabled });
    }

    public static bool ConfirmsWrite(DaemonSettingsSnapshot? settings, bool enabled) =>
        settings?.IronwireAttestedBodies == enabled;
}
