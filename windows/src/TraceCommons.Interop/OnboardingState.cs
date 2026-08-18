using System;
using System.Collections.Generic;
using System.IO;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// Which tenants this device has finished onboarding for.
/// </summary>
/// <remarks>
/// <para>
/// The reason this exists rather than the app reading
/// <c>status.logged_in</c>: <c>enroll</c> succeeds on the Connect screen and
/// flips <c>logged_in</c> true there, three screens before consent is
/// chosen. Resuming on <c>logged_in</c> would drop a contributor who quit
/// mid-flow straight into the main window carrying <c>enroll</c>'s
/// floor-only scope default -- silently narrower consent than the one they
/// were in the middle of choosing, and no prompt to finish choosing it.
/// </para>
/// <para>
/// Keyed by tenant rather than a single global flag, because re-enrolling
/// into a different commons is a different consent decision. A global
/// boolean would let a new tenant inherit the old one's "done" and skip the
/// scopes screen entirely.
/// </para>
/// <para>
/// Every failure here is swallowed and answers "not complete". The cost of
/// wrongly believing onboarding is unfinished is that a contributor is shown
/// a screen again; the cost of wrongly believing it is finished is skipping
/// the consent decision. Those are not symmetric, so an unreadable file
/// resolves toward asking.
/// </para>
/// </remarks>
public sealed class OnboardingState
{
    private readonly string _path;

    /// <summary>
    /// Uses an explicit file, for tests.
    /// </summary>
    public OnboardingState(string path)
    {
        _path = path ?? throw new ArgumentNullException(nameof(path));
    }

    /// <summary>
    /// The default location under the user's local application data.
    /// </summary>
    public static OnboardingState Default() =>
        new(Path.Combine(
            Environment.GetFolderPath(Environment.SpecialFolder.LocalApplicationData),
            "TraceCommons",
            "onboarded.json"));

    /// <summary>
    /// Whether onboarding was walked to its end for this tenant.
    /// </summary>
    public bool IsComplete(string? tenantId)
    {
        if (string.IsNullOrEmpty(tenantId))
        {
            // No tenant means `enroll` has not happened, so there is nothing
            // that could have been finished.
            return false;
        }

        return Read().Contains(tenantId);
    }

    /// <summary>
    /// Records that onboarding finished for this tenant.
    /// </summary>
    public void MarkComplete(string? tenantId)
    {
        if (string.IsNullOrEmpty(tenantId))
        {
            return;
        }

        List<string> tenants = Read();
        if (tenants.Contains(tenantId))
        {
            return;
        }

        tenants.Add(tenantId);

        try
        {
            string? directory = Path.GetDirectoryName(_path);
            if (!string.IsNullOrEmpty(directory))
            {
                Directory.CreateDirectory(directory);
            }

            File.WriteAllText(_path, JsonSerializer.Serialize(tenants));
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            // Onboarding still finished for this run; only the memory of it
            // is lost, and the next launch asks again. That is the safe
            // direction, so it is not worth failing the flow over.
        }
    }

    private List<string> Read()
    {
        try
        {
            if (!File.Exists(_path))
            {
                return new List<string>();
            }

            return JsonSerializer.Deserialize<List<string>>(File.ReadAllText(_path))
                   ?? new List<string>();
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException or JsonException)
        {
            return new List<string>();
        }
    }
}
