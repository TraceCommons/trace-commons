using System;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.RegularExpressions;
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

    /// <summary>
    /// The sheet has to reach <see cref="AdmissionPreparation.Available"/>
    /// carrying the daemon's settings, because that is the only place the
    /// enrolment is consulted.
    /// </summary>
    /// <remarks>
    /// <c>CanPrepareAdmission</c> reads as though it only asks whether the
    /// daemon advertises the method; the enrolment check is one hop away, in
    /// the assignment to the backing field. An audit read the property and
    /// concluded contributors on an invite were being shown a button that
    /// could only refuse them. They are not -- but nothing failed if that
    /// hop were dropped, so this pins it.
    ///
    /// TraceCommons.App is WinUI and cannot be referenced from a test
    /// assembly, so the view model is read as the text the csproj copies
    /// beside us. Comment lines are stripped first: a guard that fails
    /// source for naming a symbol in prose teaches the next reader to delete
    /// the prose.
    /// </remarks>
    [Fact]
    public void TheSheetDecidesAdmissionFromTheDaemonSettingsAndNotTheMethodList()
    {
        string source = Uncommented("PreviewSheetViewModel.cs.txt");
        string[] assignments = source
            .Split('\n')
            .Where(line => Regex.IsMatch(line, @"(^|[^\w.])_admissionSupported\s*="))
            .ToArray();

        Assert.Single(assignments);
        Assert.Contains("AdmissionPreparation.Available(", assignments[0], StringComparison.Ordinal);
        Assert.Contains("DaemonSettingsSnapshot", assignments[0], StringComparison.Ordinal);
    }

    /// <summary>
    /// Absent, not present-and-refusing. macOS omits the whole preparation
    /// view for a contributor who cannot use it; a disabled control with
    /// nothing to explain it is worse than no control at all.
    /// </summary>
    [Fact]
    public void TheAdmissionControlIsWithheldRatherThanShownDisabled()
    {
        string markup = Regex.Replace(
            Regex.Replace(Uncommented("PreviewSheet.xaml.txt"), "<!--.*?-->", " ", RegexOptions.Singleline),
            @"\s+",
            " ");
        Match button = Regex.Match(markup, @"<Button[^>]*Click=""OnPrepareAdmission""[^>]*>");
        Assert.True(button.Success, "the preparation button was not found in the sheet markup");
        Assert.Contains(
            @"Visibility=""{x:Bind ViewModel.CanPrepareAdmission",
            button.Value,
            StringComparison.Ordinal);
        Assert.DoesNotContain("IsEnabled=", button.Value, StringComparison.Ordinal);
    }

    private static string Uncommented(string file)
    {
        string path = Path.Combine(AppContext.BaseDirectory, file);
        Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");
        return string.Join(
            "\n",
            File.ReadAllText(path)
                .Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));
    }
}
