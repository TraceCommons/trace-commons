using System;
using System.Collections.Generic;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// The scrubber's detector names, read from the daemon rather than kept here.
///
/// This is the half of the "What gets removed?" dialog that must not be a
/// hand-maintained list. <see cref="ScrubDetectorCopy"/> turns these slugs into
/// display labels; what the set CONTAINS is the scrubber's business, and a
/// detector added upstream appears here without anyone remembering to add it.
/// </summary>
public static class ScrubDetectors
{
    /// <summary>
    /// The detector slugs, in the order the scrubber reports them.
    ///
    /// Returns an empty list rather than throwing when the call fails or the
    /// payload will not parse. The dialog is reference material: a contributor
    /// who opens it during a transient fault should see the concession and an
    /// empty list, not a crash on the first screen of onboarding. The caller
    /// decides what to render for empty.
    /// </summary>
    public static IReadOnlyList<string> Names() =>
        ParseNames(NativeMethods.TakeOwnedString(NativeMethods.tc_scrub_detector_names()));

    /// <summary>
    /// The payload half of <see cref="Names"/>, split out so it is testable
    /// without the cdylib. The native call is a one-liner; this is where the
    /// behaviour that can actually be wrong lives.
    /// </summary>
    internal static IReadOnlyList<string> ParseNames(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return Array.Empty<string>();
        }

        try
        {
            string[]? names = JsonSerializer.Deserialize<string[]>(json);
            if (names is null)
            {
                return Array.Empty<string>();
            }

            var kept = new List<string>(names.Length);
            foreach (string? name in names)
            {
                if (!string.IsNullOrWhiteSpace(name))
                {
                    kept.Add(name);
                }
            }

            return kept;
        }
        catch (JsonException)
        {
            return Array.Empty<string>();
        }
    }

    /// <summary>
    /// The detector names as display labels, ready for the dialog.
    /// </summary>
    public static IReadOnlyList<string> Labels() => ScrubDetectorCopy.LabelsFor(Names());
}
