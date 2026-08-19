using System;
using System.Collections.Generic;

namespace TraceCommons.Interop;

/// <summary>
/// The words of the "What gets removed?" dialog on onboarding screen 1, and
/// the prettification of the scrubber's detector slugs.
///
/// THE LIST IS GENERATED, NOT WRITTEN HERE. Its contents come from the
/// scrubber's own detector table, reached through
/// <c>tc_scrub_detector_names()</c>, because a hand-maintained list of what is
/// removed is exactly the kind of claim that silently stops being true. Only
/// the prettification below is a lookup, and an unrecognised slug still renders
/// de-slugged rather than vanishing from a list a contributor is reading to
/// decide whether to trust the scrubber.
///
/// NAMES ONLY, NEVER THE PATTERNS. Publishing the regexes would tell someone
/// trying to slip a secret past the scrubber exactly what to avoid. The dialog
/// says what is caught, not how.
/// </summary>
public static class ScrubDetectorCopy
{
    /// <summary>The link that opens the dialog, under the scrubbing paragraph.</summary>
    public const string LinkLabel = "What gets removed?";

    /// <summary>The dialog's heading. No question mark: it is answering one.</summary>
    public const string Heading = "What gets removed";

    /// <summary>
    /// Introduces the list without claiming to enumerate everything, because
    /// the concession below says plainly that it does not.
    /// </summary>
    public const string Intro = "Before a trace leaves this machine, these are found and replaced:";

    /// <summary>
    /// The concession, shown beneath the list. A list of what is caught is not
    /// a guarantee, and this product's credibility rests on conceding that
    /// first rather than being caught out by it later.
    /// </summary>
    public const string ResidualRisk =
        "Scrubbing is pattern-based. It misses things it hasn't seen before.";

    /// <summary>The dialog's dismissal.</summary>
    public const string Close = "Close";

    /// <summary>
    /// Every detector slug the scrubber's table carried when this screen was
    /// built, as read from <c>trace_contribution.rs</c>.
    ///
    /// This is NOT what the dialog renders -- that comes from the live export.
    /// It exists so <c>EveryKnownDetectorHasAHumanLabel</c> can run on a machine
    /// that cannot reach the cdylib's newest exports, and it is deliberately a
    /// weaker guard than GTK's, which iterates the live table. When the export
    /// is available here, this set should be replaced by a call to it.
    /// </summary>
    internal static readonly string[] KnownDetectorsAtTimeOfWriting =
    {
        "openai_api_key",
        "github_token",
        "aws_access_key",
        "provider_token",
        "jwt",
        "npm_token",
        "google_api_key",
        "pem_header_orphan",
    };

    /// <summary>
    /// A named detector, in words. <paramref name="slug"/> is a name from the
    /// protocol's table.
    /// </summary>
    public static string LabelFor(string slug)
    {
        return slug switch
        {
            "openai_api_key" => "OpenAI API keys",
            "github_token" => "GitHub tokens",
            "aws_access_key" => "AWS access keys",

            // The regex behind this one covers Stripe, GitLab and Slack
            // prefixes. Naming them beats "provider tokens", which tells a
            // contributor nothing about whether their own provider is covered.
            "provider_token" => "Stripe, GitLab and Slack tokens",
            "jwt" => "JSON Web Tokens",
            "npm_token" => "npm tokens",
            "google_api_key" => "Google API keys",
            "pem_header_orphan" => "Private keys in PEM blocks",
            _ => Deslug(slug),
        };
    }

    /// <summary>
    /// Renders the given slugs as display labels, in the order the scrubber
    /// reports them.
    /// </summary>
    public static IReadOnlyList<string> LabelsFor(IEnumerable<string> slugs)
    {
        ArgumentNullException.ThrowIfNull(slugs);

        var labels = new List<string>();
        foreach (string slug in slugs)
        {
            if (!string.IsNullOrWhiteSpace(slug))
            {
                labels.Add(LabelFor(slug));
            }
        }

        return labels;
    }

    /// <summary>
    /// The fallback for a detector this shell has not been taught to name. A
    /// safety net, not the plan: a new detector should get a real label, and
    /// the guard test is what makes that happen. Vanishing from the list would
    /// be worse -- it would understate what the scrubber catches.
    /// </summary>
    private static string Deslug(string slug) =>
        string.IsNullOrWhiteSpace(slug) ? slug : slug.Replace('_', ' ');
}
