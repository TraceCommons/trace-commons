using System;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// Settings > Updates (W17-D3 (a); <c>UpdatesSection</c>,
/// StartupSection.swift:180-253) over the Windows updater. A packaged copy
/// is self-updating: App Installer owns the swap, so the card says the
/// schedule it keeps, when this app last checked, and when it installs,
/// then offers Check Now, which asks the deployment service and raises the
/// update banner. An unpackaged copy has no updater: the Mac's disabled
/// branch, the "Updates unavailable" tag (:221-223) with its default
/// sentence (:251).
///
/// The packaged copy's tag and caption are the core's Windows-only words
/// (<c>checks_every_eight_hours</c>, <c>installs_when_closed</c>), approved
/// by the owner 2026-11-19, in place of the Mac's "Checks daily" and "Asks
/// before installing an update.", which would be untrue under App
/// Installer's 8-hour background task. The last-check line is the Mac's
/// <c>last_checked</c>, timed from this app's own last check that got an
/// answer (at launch or Check Now; <see cref="UpdateCheckClock"/>), and
/// absent until one has this run. Homebrew's branch has no Windows
/// counterpart.
/// </summary>
public sealed record UpdatesCard(string VersionLabel, string Version, string? Tag, bool TagOn, string? LastChecked, string? Caption, string? CheckNow)
{
    public static UpdatesCard Of(bool packaged, string version, Func<string, string> word, DateTimeOffset? lastCheck, DateTimeOffset now)
    {
        string label = word("version");
        if (!packaged)
        {
            return new UpdatesCard(label, version, NonEmpty(word("updates_unavailable")), false, null, NonEmpty(word("updates_off")), null);
        }

        return new UpdatesCard(
            label,
            version,
            NonEmpty(word("checks_every_eight_hours")),
            true,
            LastCheckedLine(word("last_checked"), lastCheck, now),
            NonEmpty(word("installs_when_closed")),
            NonEmpty(word("check_now")));
    }

    /// <summary>The card from the core's Settings words, timed against the clock now.</summary>
    public static UpdatesCard Live(bool packaged, string version, DateTimeOffset? lastCheck) =>
        Of(packaged, version, SettingsLegacyWords.Word, lastCheck, DateTimeOffset.UtcNow);

    /// <summary>
    /// "Last checked {relative}." with the hole filled the way the routing
    /// card's last-check line is (<see cref="SessionRootsCopy.HumanWhen(DateTimeOffset?, DateTimeOffset)"/>);
    /// null before any check has completed, or without the word.
    /// </summary>
    private static string? LastCheckedLine(string template, DateTimeOffset? lastCheck, DateTimeOffset now) =>
        lastCheck is null || template.Length == 0
            ? null
            : SettingsLegacyWords.Fill(template, "relative", SessionRootsCopy.HumanWhen(lastCheck, now));

    private static string? NonEmpty(string word) => word.Length == 0 ? null : word;
}

/// <summary>
/// When this app's own update check last got an answer this run: the
/// launch check and Check Now both record here, and Settings > Updates
/// reads it for its last-check line. App Installer's own background checks
/// are not visible to the app, so they are not counted.
/// </summary>
public sealed class UpdateCheckClock
{
    /// <summary>The last completed check, or null before any this run.</summary>
    public DateTimeOffset? LastCompleted { get; private set; }

    /// <summary>Raised after a completed check moves <see cref="LastCompleted"/>.</summary>
    public event Action? Changed;

    /// <summary>
    /// A check is out: Check Now is off until it answers, as the Mac's is
    /// while Sparkle cannot check (<c>canCheckNow</c>; W17 review F3).
    /// </summary>
    public bool Checking { get; private set; }

    /// <summary>Starts a check; false when one is already out, which then starts nothing.</summary>
    public bool TryBegin()
    {
        if (Checking)
        {
            return false;
        }

        Checking = true;
        Changed?.Invoke();
        return true;
    }

    /// <summary>
    /// Whether the deployment service answered: up to date, or an update
    /// offered. A failed check, or a package with no App Installer
    /// association, checked nothing.
    /// </summary>
    public static bool Completed(TcUpdateAvailability availability) =>
        availability is TcUpdateAvailability.NoUpdates
            or TcUpdateAvailability.Available
            or TcUpdateAvailability.Required;

    /// <summary>Records a check made at <paramref name="at"/> if it completed.</summary>
    public void Record(TcUpdateAvailability availability, DateTimeOffset at)
    {
        bool wasChecking = Checking;
        Checking = false;
        if (!Completed(availability))
        {
            // Any answer ends the check; only one that got an answer is recorded.
            if (wasChecking)
            {
                Changed?.Invoke();
            }

            return;
        }

        LastCompleted = at;
        Changed?.Invoke();
    }
}
