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
    public static string ForEntry(string entryId)
    {
        if (string.IsNullOrWhiteSpace(entryId))
        {
            throw new ArgumentException("entryId must not be empty.", nameof(entryId));
        }

        return JsonSerializer.Serialize(
            new Dictionary<string, string> { ["entry_id"] = entryId });
    }

    /// <summary>
    /// One project group: <c>{"project_id": "..."}</c>.
    ///
    /// The id an <c>entry_value</c> publishes as <c>project_id</c> -- never
    /// <c>project_label</c>, which is a display string the daemon does not
    /// treat as an identifier.
    /// </summary>
    public static string ForProject(string projectId)
    {
        if (string.IsNullOrWhiteSpace(projectId))
        {
            throw new ArgumentException("projectId must not be empty.", nameof(projectId));
        }

        return JsonSerializer.Serialize(
            new Dictionary<string, string> { ["project_id"] = projectId });
    }
}
