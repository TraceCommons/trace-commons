using System;
using System.Collections.Generic;
using System.Linq;

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
    /// The value name under the Run key: the app's name, from
    /// <c>brand.rs</c> (<c>tc_app_name</c>).
    /// </summary>
    /// <remarks>
    /// The product name rather than the executable's, because Windows shows
    /// a startup entry by name to someone auditing what starts with their
    /// machine, and anything there that they cannot recognise reads as
    /// malware -- correctly. It is the name every other surface Windows
    /// shows uses (owner rule 2026-11-20).
    /// </remarks>
    public static string ValueName => Shell.MonitorWords.AppName;

    /// <summary>
    /// Names the entry was written under before <see cref="ValueName"/>,
    /// newest first. <see cref="Migrate"/> moves this executable's entry off
    /// them; nothing ever writes one.
    /// </summary>
    public static IReadOnlyList<string> FormerValueNames { get; } = new[] { "Trace Commons" };

    /// <summary>
    /// Where Windows records that the contributor disabled a startup entry
    /// (Settings -> Apps -> Startup, Task Manager), keyed by the Run value's
    /// name; relative to HKEY_CURRENT_USER.
    /// </summary>
    public const string ApprovedKeyPath = @"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run";

    /// <summary>
    /// The Run key and Windows' approval record beside it, by value name:
    /// the registry in the app, plain maps in the tests.
    /// </summary>
    public interface IStartupEntries
    {
        /// <summary>The Run value under <paramref name="name"/>; null when absent.</summary>
        string? Command(string name);

        /// <summary>The StartupApproved value under <paramref name="name"/>; null when absent.</summary>
        byte[]? Approval(string name);

        void SetCommand(string name, string command);

        void SetApproval(string name, byte[] approval);

        /// <summary>Removes <paramref name="name"/> from both keys.</summary>
        void Remove(string name);
    }

    /// <summary>
    /// Moves this executable's entry from a former name to
    /// <see cref="ValueName"/>, with Windows' record of whether the
    /// contributor disabled it. Returns whether anything moved.
    /// </summary>
    /// <remarks>
    /// <para>
    /// The value name is the entry's identity. Left under the old name, the
    /// entry would still start the app, and a new one written beside it
    /// would start it twice; turning the toggle off would remove only the
    /// new one.
    /// </para>
    /// <para>
    /// The approval record moves with it, unchanged. Windows keeps
    /// "disabled" under the same name, so a move that dropped it would turn
    /// back on what the contributor turned off. This is the only time the
    /// app touches that key, and it never changes what it says.
    /// </para>
    /// <para>
    /// Another copy's entry is left alone, as <c>RunAtLogin.Refresh</c>
    /// leaves it, and so is ours when the new name already belongs to
    /// another copy: moving it then would overwrite that copy's entry.
    /// </para>
    /// </remarks>
    public static bool Migrate(IStartupEntries entries, string executablePath)
    {
        ArgumentNullException.ThrowIfNull(entries);
        ArgumentNullException.ThrowIfNull(executablePath);

        bool moved = false;
        foreach (string former in FormerValueNames)
        {
            string? stored = entries.Command(former);
            if (!PointsAt(stored, executablePath))
            {
                continue;
            }

            string? current = entries.Command(ValueName);
            if (current is not null && !PointsAt(current, executablePath))
            {
                continue;
            }

            if (current is null)
            {
                entries.SetCommand(ValueName, stored!);
                if (entries.Approval(former) is byte[] approval && entries.Approval(ValueName) is null)
                {
                    entries.SetApproval(ValueName, approval);
                }
            }

            entries.Remove(former);
            moved = true;
        }

        return moved;
    }

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
    /// The argument the entry starts the app with, so the launch knows it
    /// is at login and opens no window: the Mac's quiet login-item launch
    /// (<c>LaunchRouting.launchKind</c>, MonitorNavigation.swift). Windows
    /// tells an unpackaged app nothing about why it started; the packaged
    /// build's startup task says so through its activation kind instead.
    /// </summary>
    public const string LoginArgument = "--at-login";

    /// <summary>
    /// The command to store, for an executable path: the quoted path and
    /// <see cref="LoginArgument"/>.
    /// </summary>
    /// <remarks>
    /// The argument is the only difference between a launch at login and
    /// any other, and <see cref="IsLoginLaunch"/> and
    /// <see cref="OpensWindow"/> decide what it changes, off Windows.
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

        return $"\"{executablePath}\" {LoginArgument}";
    }

    /// <summary>
    /// Whether this launch is at login: the entry's argument after the
    /// program (<paramref name="args"/> as
    /// <c>Environment.GetCommandLineArgs</c> gives them), or the packaged
    /// build's startup task.
    /// </summary>
    public static bool IsLoginLaunch(IReadOnlyList<string> args, bool startupTaskActivation)
    {
        ArgumentNullException.ThrowIfNull(args);
        return startupTaskActivation
            || args.Skip(1).Any(a => string.Equals(a, LoginArgument, StringComparison.OrdinalIgnoreCase));
    }

    /// <summary>
    /// Whether the launch opens the window. A launch at login stays in the
    /// tray, unless there is no tray icon to reach the app by or the launch
    /// carries an invite the person just followed.
    /// </summary>
    public static bool OpensWindow(bool loginLaunch, bool trayPresent, bool hasInvite) =>
        !loginLaunch || !trayPresent || hasInvite;

    /// <summary>
    /// Whether a launch that stayed in the tray opens the window once its
    /// daemon start is over (the Mac's <c>LaunchRouting.launchOpening</c>
    /// for a quiet launch): a refused start, undeclared folders included,
    /// or setup not yet finished is something to do, and a window opens as
    /// an ordinary launch would.
    /// </summary>
    public static bool OpensAfterQuietStart(bool started, bool setupComplete) =>
        !started || !setupComplete;

    /// <summary>
    /// Whether a stored value is exactly what <see cref="For"/> writes for
    /// this executable today. An entry written before the argument existed
    /// still points here (<see cref="PointsAt"/>) but would open the window
    /// at every login, so the app rewrites it.
    /// </summary>
    public static bool IsCurrent(string? storedValue, string executablePath)
    {
        ArgumentNullException.ThrowIfNull(executablePath);
        return PointsAt(storedValue, executablePath)
            && string.Equals(Arguments(storedValue!), LoginArgument, StringComparison.OrdinalIgnoreCase);
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
            Program(storedValue),
            executablePath.Trim().Trim('"'),
            StringComparison.OrdinalIgnoreCase);
    }

    /// <summary>The program part of a stored command: inside its quotes, or the whole value unquoted.</summary>
    private static string Program(string stored)
    {
        string value = stored.Trim();
        if (value.StartsWith('"'))
        {
            int close = value.IndexOf('"', 1);
            return close > 0 ? value[1..close] : value.Trim('"');
        }

        return value;
    }

    /// <summary>What follows the quoted program in a stored command, trimmed.</summary>
    private static string Arguments(string stored)
    {
        string value = stored.Trim();
        if (!value.StartsWith('"'))
        {
            return string.Empty;
        }

        int close = value.IndexOf('"', 1);
        return close > 0 ? value[(close + 1)..].Trim() : string.Empty;
    }
}
