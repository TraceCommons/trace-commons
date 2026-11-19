using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.RegularExpressions;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The login-enrollment sentences, from the live ABI rather than a fixture.
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
}
