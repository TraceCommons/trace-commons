using System;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class InferenceEvidenceConsentTests
{
    [Fact]
    public void EnablingRequiresDisclosureButDisablingDoesNot()
    {
        Assert.Throws<InvalidOperationException>(() => InferenceEvidenceConsent.Serialize(true, false));
        using JsonDocument enabled = JsonDocument.Parse(InferenceEvidenceConsent.Serialize(true, true));
        JsonProperty field = Assert.Single(enabled.RootElement.EnumerateObject());
        Assert.Equal("ironwire_attested_bodies", field.Name);
        Assert.True(field.Value.GetBoolean());
        using JsonDocument disabled = JsonDocument.Parse(InferenceEvidenceConsent.Serialize(false, false));
        Assert.False(disabled.RootElement.GetProperty("ironwire_attested_bodies").GetBoolean());
    }

    [Fact]
    public void OlderDaemonDoesNotGrantConsentOrConfirmAWrite()
    {
        var settings = JsonSerializer.Deserialize<DaemonSettingsSnapshot>("{\"near_ai_configured\":true,\"ironwire\":{\"mode\":\"watch\",\"port\":8463}}");
        Assert.NotNull(settings);
        Assert.False(settings!.InferenceEvidenceEnabled);
        Assert.Null(settings.IronwireAttestedBodies);
        Assert.False(InferenceEvidenceConsent.ConfirmsWrite(settings, true));
        Assert.False(InferenceEvidenceConsent.ConfirmsWrite(settings, false));
    }

    [Theory]
    [InlineData(true)]
    [InlineData(false)]
    public void OnlyTheReturnedValueConfirmsTheWrite(bool requested)
    {
        Assert.True(InferenceEvidenceConsent.ConfirmsWrite(new DaemonSettingsSnapshot { IronwireAttestedBodies = requested }, requested));
        Assert.False(InferenceEvidenceConsent.ConfirmsWrite(new DaemonSettingsSnapshot { IronwireAttestedBodies = !requested }, requested));
        Assert.False(InferenceEvidenceConsent.ConfirmsWrite(null, requested));
    }
}
