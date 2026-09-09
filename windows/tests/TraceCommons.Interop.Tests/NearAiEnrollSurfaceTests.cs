using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The login-enrolment sentences, from the live ABI rather than a fixture.
/// </summary>
public sealed class NearAiEnrollSurfaceTests
{
    private static readonly string[] Labels =
    {
        "near_ai_enroll_already_enrolled",
        "near_ai_enroll_no_session",
        "near_ai_enroll_endpoint_refused",
        "near_ai_enroll_token_unavailable",
        "near_ai_enroll_start_failed",
        "near_ai_enroll_commons_unreachable",
        "near_ai_enroll_commons_unsupported",
        "near_ai_enroll_invalid",
        "near_ai_enroll_verification_failed",
        "near_ai_enroll_unavailable",
    };

    [Fact]
    public void EveryRefusalReachesItsOwnSentence()
    {
        var seen = new HashSet<string>(StringComparer.Ordinal);
        foreach (string label in Labels)
        {
            string? line = NearAiEnrollSurface.Line(label);
            Assert.False(string.IsNullOrEmpty(line), $"{label} reached no sentence");
            Assert.True(seen.Add(line!), $"{label} shares a sentence with another refusal");
        }
    }

    /// <summary>
    /// An unfamiliar label reaches the generic sentence, never nothing.
    /// </summary>
    /// <remarks>
    /// Unlike an attestation reason, silence is not honest here: a refusal
    /// this build cannot name is the whole of what a contributor is told.
    /// </remarks>
    [Fact]
    public void AnUnknownLabelClaimsNothingSpecific()
    {
        string generic = NearAiEnrollSurface.Line(NearAiEnrollSurface.Unavailable)!;
        Assert.False(string.IsNullOrEmpty(generic));
        foreach (string unknown in new[] { "", "near_ai_enroll_from_a_newer_daemon" })
        {
            Assert.Equal(generic, NearAiEnrollSurface.Line(unknown));
        }
        Assert.Equal(generic, NearAiEnrollSurface.Line(null));
    }

    /// <summary>
    /// The two outcomes a contributor would misread if the tone were wrong.
    /// </summary>
    [Fact]
    public void TheTonesPointAtTheStepRatherThanTheWall()
    {
        Assert.Equal(
            PrivateInferenceTone.Attention,
            NearAiEnrollSurface.Tone("near_ai_enroll_no_session"));
        Assert.Equal(
            PrivateInferenceTone.Clear,
            NearAiEnrollSurface.Tone("near_ai_enroll_already_enrolled"));
        Assert.Equal(
            PrivateInferenceTone.Refused,
            NearAiEnrollSurface.Tone("near_ai_enroll_commons_unreachable"));
    }

    /// <summary>
    /// The onboarding screen asks the shared surface and spells no control
    /// name of its own.
    /// </summary>
    /// <remarks>
    /// TraceCommons.App is WinUI and cannot be referenced from a test
    /// assembly, so the view model and the markup are read as the text the
    /// csproj copies beside us. Comment lines are stripped first: those
    /// comments name the control names on purpose, and a guard that failed
    /// them would teach the next reader to delete the explanation.
    /// </remarks>
    [Fact]
    public void TheScreenAsksTheSharedSurfaceAndNamesNoRefusal()
    {
        string source = Uncommented("OnboardingViewModel.cs.txt");
        Assert.Contains("NearAiEnrollSurface.Line(_nearAiRefusal)", source, StringComparison.Ordinal);
        Assert.Contains("NearAiEnrollSurface.CredentialStatePresent", source, StringComparison.Ordinal);
        foreach (string label in new[]
                 {
                     "near_ai_enroll_no_session",
                     "near_ai_enroll_commons_unreachable",
                     "near_ai_enroll_commons_unsupported",
                     "near_ai_enroll_already_enrolled",
                 })
        {
            Assert.DoesNotContain(label, source, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// With no sign-in the control is withheld and the step is said.
    /// </summary>
    [Fact]
    public void WithoutASignInTheStepIsOfferedRatherThanAControlThatCanOnlyFail()
    {
        string source = Uncommented("OnboardingViewModel.cs.txt");
        Assert.Contains("CanOfferNearAiJoin => _nearAiSignedIn", source, StringComparison.Ordinal);
        Assert.Contains("_nearAiSignedIn || _nearAiJoined ? string.Empty", source, StringComparison.Ordinal);

        // Read from the answer, not assumed from the absence of a refusal.
        Assert.Contains("enrolled.ValueKind == System.Text.Json.JsonValueKind.True", source, StringComparison.Ordinal);

        string markup = Regex.Replace(
            Regex.Replace(Uncommented("OnboardingWindow.xaml.txt"), "<!--.*?-->", " ", RegexOptions.Singleline),
            @"\s+",
            " ");
        // Withheld, not disabled: Visibility carries the offer.
        Assert.Contains(
            "Visibility=\"{x:Bind ViewModel.CanOfferNearAiJoin, Mode=OneWay}\"",
            markup,
            StringComparison.Ordinal);
        Assert.Contains("ViewModel.NearAiNeedsLogin", markup, StringComparison.Ordinal);
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
