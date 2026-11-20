using System;
using System.Globalization;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;

namespace TraceCommons.Interop;

/// <summary>
/// The hidden helper that starts the packaged app again after an in-app
/// update.
/// </summary>
/// <remarks>
/// The deployment ends the app's own process and installs the new version,
/// and nothing starts it again: on tc-win-az3 (2026-11-20) the app stayed
/// closed after every in-app update, with or without
/// <c>RegisterApplicationRestart</c>, because the deployment ends it through
/// <c>TerminateApplications</c> rather than Restart Manager. A process the
/// app starts runs outside the package's container and survives that ending
/// (measured on the same box). So the app starts this helper with Windows
/// PowerShell (<c>UpdateRelaunch.ps1</c>) just before the hand-off. It polls for this package family to
/// be registered at a different version and then starts the app through its
/// AUMID, the way the Start menu does. If the update never lands it gives up
/// after <see cref="WaitSeconds"/> and starts nothing.
/// <para>
/// The identity values are checked against a strict character set before
/// they go into the script, so nothing in them can end a quoted string.
/// </para>
/// </remarks>
public static class UpdateRelaunch
{
    /// <summary>How long the helper waits for the new version before giving up.</summary>
    public const int WaitSeconds = 180;

    /// <summary>The application id in Package.appxmanifest.</summary>
    public const string ApplicationId = "App";

    private static readonly Regex SafeName = new("^[A-Za-z0-9._-]+$", RegexOptions.CultureInvariant);
    private static readonly Regex QuadVersion = new(@"^\d+\.\d+\.\d+\.\d+$", RegexOptions.CultureInvariant);

    /// <summary>The helper's PowerShell script.</summary>
    /// <param name="packageName">Package identity name, e.g. Iqlusion.TraceCommons.</param>
    /// <param name="packageFamilyName">The package family name.</param>
    /// <param name="currentVersion">The running version, as a quad.</param>
    /// <exception cref="ArgumentException">A value outside the safe character set.</exception>
    public static string Script(string packageName, string packageFamilyName, string currentVersion)
    {
        Require(SafeName, packageName, nameof(packageName));
        Require(SafeName, packageFamilyName, nameof(packageFamilyName));
        Require(QuadVersion, currentVersion, nameof(currentVersion));

        return Template()
            .Replace("@@WAIT_SECONDS@@", WaitSeconds.ToString(CultureInfo.InvariantCulture), StringComparison.Ordinal)
            .Replace("@@PACKAGE_NAME@@", packageName, StringComparison.Ordinal)
            .Replace("@@PACKAGE_FAMILY@@", packageFamilyName, StringComparison.Ordinal)
            .Replace("@@CURRENT_VERSION@@", currentVersion, StringComparison.Ordinal)
            .Replace("@@APP_ID@@", ApplicationId, StringComparison.Ordinal);
    }

    /// <summary>Windows PowerShell arguments that run <paramref name="script"/> hidden.</summary>
    public static string Arguments(string script) =>
        "-NoProfile -NonInteractive -WindowStyle Hidden -EncodedCommand "
        + Convert.ToBase64String(Encoding.Unicode.GetBytes(script));

    /// <summary>
    /// The script, from <c>UpdateRelaunch.ps1</c> embedded beside this file:
    /// PowerShell kept as PowerShell, with LF line endings whatever the
    /// checkout did to it.
    /// </summary>
    private static string Template()
    {
        using Stream stream = typeof(UpdateRelaunch).Assembly.GetManifestResourceStream(ResourceName)
            ?? throw new InvalidOperationException(ResourceName + " is not embedded");
        using var reader = new StreamReader(stream, Encoding.UTF8);
        return reader.ReadToEnd().Replace("\r\n", "\n", StringComparison.Ordinal).TrimEnd('\n');
    }

    private const string ResourceName = "TraceCommons.Interop.UpdateRelaunch.ps1";

    private static void Require(Regex pattern, string value, string name)
    {
        if (string.IsNullOrEmpty(value) || !pattern.IsMatch(value))
        {
            throw new ArgumentException("not a value the relaunch helper accepts", name);
        }
    }
}
