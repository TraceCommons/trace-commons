using System;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;
namespace TraceCommons.Interop.Tests;
public sealed class AdmissionPreparationTests
{
    [Fact]
    public void PreparationNeedsBothConfiguredRequirementAndAvailableMethod()
    {
        var hello = DaemonResponse.Parse("{\"result\":{\"methods\":[\"prepare_admission_session\"]}}");
        Assert.False(AdmissionPreparation.Available(hello, new DaemonSettingsSnapshot()));
        Assert.False(AdmissionPreparation.Available(hello, null));
        Assert.True(AdmissionPreparation.Available(hello, new DaemonSettingsSnapshot { AdmissionEvidenceRequired = true }));
        Assert.False(AdmissionPreparation.Available(DaemonResponse.Parse("{\"result\":{}}"), new DaemonSettingsSnapshot { AdmissionEvidenceRequired = true }));
    }
    [Fact]
    public void PreparationRequiresCoreReadinessInsteadOfRecomputingExpiry()
    {
        var response = new DaemonResponse { Result = JsonSerializer.SerializeToElement(new { status = "ready_for_next_inference", view = new { ready = true } }) };
        Assert.True(AdmissionPreparation.IsReady(response));
        foreach (var json in new[] { "{}", "{\"status\":\"ready_for_next_inference\"}", "{\"status\":\"ready_for_next_inference\",\"expires_at\":1}", "{\"status\":\"ready_for_next_inference\",\"expires_at\":\"tomorrow\"}" })
            Assert.False(AdmissionPreparation.IsReady(DaemonResponse.Parse("{\"result\":" + json + "}")));
        using var request = JsonDocument.Parse(AdmissionPreparation.Request("selected-entry", " nearai "));
        Assert.Equal("selected-entry", request.RootElement.GetProperty("entry_id").GetString());
        Assert.Equal("nearai", request.RootElement.GetProperty("backend").GetString());
        Assert.True(request.RootElement.GetProperty("confirmed").GetBoolean());
    }
}
