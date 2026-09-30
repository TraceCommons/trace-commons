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
    /// Only an explicit <c>true</c> on the wire is a yes.
    /// </summary>
    /// <remarks>
    /// The test above hands <c>Available</c> snapshots built in memory. This
    /// one comes through the deserializer the sheet actually uses, because
    /// the refusing answers arrive as JSON: <c>add_admission_setting</c>
    /// answers null when the daemon could not read its config, and a daemon
    /// that predates the key answers nothing at all. An unreadable config
    /// quietly becoming "evidence not required" is the same bug pointed the
    /// other way. GTK and macOS hold this in their own suites.
    /// </remarks>
    [Fact]
    public void AnUnreadableOrSilentDaemonIsNotAYes()
    {
        var hello = DaemonResponse.Parse("{\"result\":{\"methods\":[\"prepare_admission_session\"]}}");
        static DaemonSettingsSnapshot? Wire(string admissionEvidence) =>
            DaemonResponse
                .Parse("{\"result\":{\"near_ai_configured\":false" + admissionEvidence + "}}")
                .ResultAs<DaemonSettingsSnapshot>();

        Assert.True(AdmissionPreparation.Available(hello, Wire(",\"admission_evidence_required\":true")));
        foreach (string answer in new[]
                 {
                     ",\"admission_evidence_required\":false",
                     ",\"admission_evidence_required\":null",
                     "",
                 })
        {
            Assert.NotEqual(true, Wire(answer)!.AdmissionEvidenceRequired);
            Assert.False(
                AdmissionPreparation.Available(hello, Wire(answer)),
                $"the answer {(answer.Length == 0 ? "(absent)" : answer)} was read as eligibility");
        }
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
        // A deserialized daemon reply, not a snapshot the sheet made up: a
        // literal `new DaemonSettingsSnapshot { AdmissionEvidenceRequired =
        // true }` would satisfy a check for the type name alone and offer
        // the control to everyone.
        Assert.Contains("ResultAs<DaemonSettingsSnapshot>", assignments[0], StringComparison.Ordinal);
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

    /// <summary>
    /// The control belongs in the sheet footer, drawn on every preview.
    /// </summary>
    /// <remarks>
    /// It used to sit inside the panel bound to <c>HasFailed</c> -- a preview
    /// that could not be opened or read. That is not the same condition as
    /// "this session has no inference evidence": a session without evidence
    /// previews perfectly well, because it is only a transcript, and the
    /// refusal comes later. So a contributor who wanted evidence had to fail
    /// first to discover the control that produces it. GTK has always drawn
    /// it in the footer; this is that placement.
    ///
    /// The footer is the last row of the sheet, so everything after its
    /// opening tag is inside it.
    /// </remarks>
    [Fact]
    public void TheAdmissionControlIsDrawnInTheFooterAndNotOnlyOnAFailedPreview()
    {
        string markup = Regex.Replace(
            Regex.Replace(Uncommented("PreviewSheet.xaml.txt"), "<!--.*?-->", " ", RegexOptions.Singleline),
            @"\s+",
            " ");

        int footer = markup.IndexOf("<StackPanel Grid.Row=\"3\"", StringComparison.Ordinal);
        Assert.True(footer >= 0, "the sheet no longer has a footer row to draw it in");

        int failureNotice = markup.IndexOf("This one can't be shown.", StringComparison.Ordinal);
        Assert.True(failureNotice >= 0, "the failed-preview notice was not found");

        int control = markup.IndexOf("Click=\"OnPrepareAdmission\"", StringComparison.Ordinal);
        Assert.True(control >= 0, "the preparation control was not found in the sheet markup");

        Assert.True(
            control > footer,
            "the preparation control is drawn before the footer, which on this sheet means "
            + "it is inside the failed-preview notice a contributor should not have to reach.");
        Assert.True(
            failureNotice < footer,
            "the failed-preview notice moved into the footer; this test's ordering argument "
            + "no longer holds and needs rewriting rather than adjusting.");
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
