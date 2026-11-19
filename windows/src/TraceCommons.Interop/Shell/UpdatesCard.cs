namespace TraceCommons.Interop.Shell;

/// <summary>
/// Settings > Updates (W17-D3 (a); <c>UpdatesSection</c>,
/// StartupSection.swift:180-253) over the Windows updater. A packaged copy
/// is self-updating: App Installer owns the swap, so the card offers Check
/// Now, which asks the deployment service and raises the update banner. An
/// unpackaged copy has no updater: the Mac's disabled branch, the
/// "Updates unavailable" tag (:221-223) with its default sentence (:251).
///
/// Three Mac lines are left out on Windows because they would be untrue
/// here and the core has no Windows wording for them: "Checks daily" (:195;
/// App Installer checks every 8 hours), "Asks before installing an update."
/// (:200; the background task installs without asking), and the last-check
/// line (:196; Windows has no system relative-date formatter and no
/// last-check time from App Installer). Homebrew's branch has no Windows
/// counterpart.
/// </summary>
public sealed record UpdatesCard(string VersionLabel, string Version, string? Tag, bool TagOn, string? Caption, string? CheckNow)
{
    public static UpdatesCard Of(bool packaged, string version, System.Func<string, string> word)
    {
        string label = word("version");
        if (!packaged)
        {
            return new UpdatesCard(label, version, NonEmpty(word("updates_unavailable")), false, NonEmpty(word("updates_off")), null);
        }

        return new UpdatesCard(label, version, null, true, null, NonEmpty(word("check_now")));
    }

    /// <summary>The card from the core's Settings words.</summary>
    public static UpdatesCard Live(bool packaged, string version) => Of(packaged, version, SettingsLegacyWords.Word);

    private static string? NonEmpty(string word) => word.Length == 0 ? null : word;
}
