using System;
using System.Collections.Generic;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// The two request shapes for one-click <c>approve</c>: one entry, or a whole
/// project.
///
/// Mutually exclusive on the wire -- the daemon refuses a call naming neither
/// as <c>bad_params</c> -- so this offers exactly one way to build each and no
/// way to build both at once. Building the parameter string here rather than
/// inline at each call site is what keeps the row and the project-group
/// buttons from drifting into two different spellings of the same request,
/// and it is testable on a machine that cannot build WinUI at all.
/// </summary>
public static class SubmitParams
{
    /// <summary>One queue row: <c>{"entry_id": "..."}</c>.</summary>
    public static string ForEntry(string entryId) => ForEntry(entryId, null);

    /// <summary>
    /// One queue row, carrying the contributor's verdict:
    /// <c>{"entry_id": "...", "outcome": "worked"}</c>.
    /// </summary>
    /// <param name="outcome">
    /// One of <see cref="Verdict"/>'s three values, or <c>null</c> when the
    /// contributor did not answer -- in which case the key is OMITTED
    /// entirely rather than sent as <c>null</c> or <c>""</c>. The daemon
    /// distinguishes an absent parameter (recorded as unknown, approval
    /// proceeds) from an unrecognised one (<c>outcome-invalid</c>, approves
    /// nothing), and those two are not the same event.
    /// </param>
    public static string ForEntry(string entryId, string? outcome) =>
        ForEntry(entryId, outcome, null);

    /// <summary>
    /// One queue row, carrying the contributor's verdict and the correction
    /// they wrote for it:
    /// <c>{"entry_id": "...", "outcome": "failed", "correction": "..."}</c>.
    /// </summary>
    /// <param name="correction">
    /// What the contributor typed, or <c>null</c> when they wrote nothing.
    /// Blank counts as nothing: an empty string is not the absence of a
    /// correction, and sending one would declare
    /// <c>correction_included</c> on the envelope for content that is not
    /// there. The key is OMITTED in that case.
    /// </param>
    /// <remarks>
    /// A correction is only meaningful under <c>partly</c> or <c>failed</c>
    /// -- you cannot correct a run you have just called successful -- and
    /// the daemon refuses the other combinations as
    /// <c>correction-needs-outcome</c>, approving nothing. Refused here as
    /// well rather than over the pipe, for the reason
    /// <see cref="Verdict.Require"/> gives: a sheet that never offers the
    /// field outside those two verdicts cannot reach this, so an argument
    /// that does is a bug that should be a test failure rather than a
    /// submit that silently did not happen.
    /// </remarks>
    public static string ForEntry(string entryId, string? outcome, string? correction)
    {
        if (string.IsNullOrWhiteSpace(entryId))
        {
            throw new ArgumentException("entryId must not be empty.", nameof(entryId));
        }

        return Serialize("entry_id", entryId, outcome, correction);
    }

    /// <summary>
    /// One project group: <c>{"project_id": "..."}</c>.
    ///
    /// The id an <c>entry_value</c> publishes as <c>project_id</c> -- never
    /// <c>project_label</c>, which is a display string the daemon does not
    /// treat as an identifier.
    /// </summary>
    public static string ForProject(string projectId) => ForProject(projectId, null);

    /// <summary>
    /// One project group, carrying the contributor's verdict. A value
    /// supplied here applies to every entry the approval covers, not just
    /// one; <c>null</c> omits the key, exactly as in
    /// <see cref="ForEntry(string, string?)"/>.
    /// </summary>
    public static string ForProject(string projectId, string? outcome)
    {
        if (string.IsNullOrWhiteSpace(projectId))
        {
            throw new ArgumentException("projectId must not be empty.", nameof(projectId));
        }

        // Never a correction: one written for a group would describe
        // sessions it was not written about, and every one of them would
        // carry it into the corpus as the contributor's own words. The
        // daemon refuses the combination as
        // `correction-needs-entry-id`; there is simply no way to express
        // it from here.
        return Serialize("project_id", projectId, outcome, null);
    }

    /// <summary>
    /// Builds the request, adding <c>outcome</c> only when there is one to
    /// add. An unrecognised value throws rather than reaching the socket --
    /// see <see cref="Verdict.Require"/>.
    /// </summary>
    private static string Serialize(
        string targetKey,
        string targetValue,
        string? outcome,
        string? correction)
    {
        var payload = new Dictionary<string, string> { [targetKey] = targetValue };
        if (!Verdict.IsAbsent(outcome))
        {
            payload["outcome"] = Verdict.Require(outcome!);
        }

        string? written = correction?.Trim();
        if (!string.IsNullOrEmpty(written))
        {
            if (outcome is not (Verdict.Partly or Verdict.Failed))
            {
                throw new ArgumentException(
                    "a correction may only accompany the partly or failed outcome.",
                    nameof(correction));
            }

            if (written.Length > CorrectionCopy.MaxCharacters)
            {
                throw new ArgumentException(
                    "a correction must be at most "
                        + CorrectionCopy.MaxCharacters
                        + " characters.",
                    nameof(correction));
            }

            payload["correction"] = written;
        }

        return JsonSerializer.Serialize(payload);
    }
}
