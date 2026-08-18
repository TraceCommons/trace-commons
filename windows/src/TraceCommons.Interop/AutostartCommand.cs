using System;

namespace TraceCommons.Interop;

/// <summary>
/// The naming and the command-line quoting of the per-user run-at-login
/// entry, kept apart from the registry call that writes it so both can be
/// tested off Windows.
/// </summary>
/// <remarks>
/// <para>
/// The registry half lives in <c>TraceCommons.App.RunAtLogin</c>. What is
/// here is the part that is wrong-able without any Windows API involved: the
/// value name, which is what the entry is called in Task Manager's Startup
/// tab and in Settings -> Apps -> Startup, and the quoting of the executable
/// path, which decides whether the app starts at all from a folder whose name
/// contains a space -- <c>C:\Program Files</c>, or any user whose display
/// name has one.
/// </para>
/// <para>
/// Unquoted, <c>CreateProcess</c> would try <c>C:\Users\Ada</c> first and then
/// <c>C:\Users\Ada Lovelace\...</c>, and the failure only appears on machines
/// whose paths happen to contain a space -- which is to say, not on the
/// developer's.
/// </para>
/// </remarks>
public static class AutostartCommand
{
    /// <summary>
    /// The value name under the Run key.
    /// </summary>
    /// <remarks>
    /// The product name rather than the executable's, because this string is
    /// shown to the contributor by Windows itself in Task Manager's Startup
    /// tab. That list is where someone goes to audit what starts with their
    /// machine, and anything there that they cannot recognise reads as
    /// malware -- correctly.
    /// </remarks>
    public const string ValueName = "Trace Commons";

    /// <summary>
    /// The per-user Run key, relative to HKEY_CURRENT_USER.
    /// </summary>
    /// <remarks>
    /// HKCU, never HKLM. This app ships unpackaged and self-contained and
    /// installs per user with no administrator rights, so the machine-wide
    /// key would both need elevation to write and would keep starting the app
    /// for users who never installed it. The same reasoning, and the same
    /// hive, as <c>UrlSchemeRegistration</c>.
    /// </remarks>
    public const string RunKeyPath = @"Software\Microsoft\Windows\CurrentVersion\Run";

    /// <summary>
    /// The command to store, for an executable path.
    /// </summary>
    /// <remarks>
    /// No arguments are appended. A launch at login is an ordinary launch:
    /// the app has no "started by Windows" behaviour to switch on, and an
    /// argument here would be a second code path that only ever runs on a
    /// contributor's machine and never in a test.
    /// </remarks>
    public static string For(string executablePath)
    {
        ArgumentNullException.ThrowIfNull(executablePath);

        if (executablePath.Length == 0)
        {
            throw new ArgumentException("executable path is empty", nameof(executablePath));
        }

        if (executablePath.Contains('"', StringComparison.Ordinal))
        {
            // A quote in a Windows path is not legal, so this is not a path.
            // Refusing is better than quoting it and writing something that
            // would run a different program than the caller meant.
            throw new ArgumentException("executable path contains a quote", nameof(executablePath));
        }

        return $"\"{executablePath}\"";
    }

    /// <summary>
    /// Whether a stored value is this executable's entry, ignoring quoting.
    /// </summary>
    /// <remarks>
    /// Used to notice an entry left behind by a copy of the app that has
    /// since been moved or replaced. An unpackaged app lives in whatever
    /// folder its owner keeps it in, and that folder can be renamed between
    /// runs; a stale entry would silently stop starting anything.
    /// </remarks>
    public static bool PointsAt(string? storedValue, string executablePath)
    {
        ArgumentNullException.ThrowIfNull(executablePath);

        if (string.IsNullOrEmpty(storedValue))
        {
            return false;
        }

        return string.Equals(
            storedValue.Trim().Trim('"'),
            executablePath.Trim().Trim('"'),
            StringComparison.OrdinalIgnoreCase);
    }
}
