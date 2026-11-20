using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Map;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// The daemon's <c>automatic_grant</c> answer (macos/Sources/TCShellCore/Flow1.swift
/// <c>AutomaticGrant</c>): an ungranted answer carries only <c>granted</c>.
/// </summary>
public sealed record AutomaticGrantState(bool Granted, bool? IncludeExisting)
{
    /// <summary>Decoded, or null for anything without a boolean <c>granted</c>.</summary>
    public static AutomaticGrantState? Decode(JsonElement? result)
    {
        if (result is not { ValueKind: JsonValueKind.Object } o
            || !o.TryGetProperty("granted", out JsonElement g) || g.ValueKind is not (JsonValueKind.True or JsonValueKind.False))
        {
            return null;
        }

        bool? existing = o.TryGetProperty("include_existing", out JsonElement e) && e.ValueKind is JsonValueKind.True or JsonValueKind.False
            ? e.GetBoolean()
            : null;
        return new AutomaticGrantState(g.GetBoolean(), existing);
    }
}

/// <summary>What pressing the eye on shows first (SharingChoiceCard.swift <c>Step</c>).</summary>
public abstract record SharingStep
{
    /// <summary>The question about the folders set to Ask.</summary>
    public sealed record KeepAsks(int Count) : SharingStep;

    /// <summary>Setup's disclosures, then the grant.</summary>
    public sealed record Disclose(bool KeepFolderAsks) : SharingStep;
}

/// <summary>The titles over the tree's columns (TracesViews.swift <c>TracesColumnTitles</c>).</summary>
public sealed record TracesColumnTitles(string Folders, string Submit, string Automatic);

/// <summary>The core's two refusal lines under the eye (<c>settings_sharing_refused</c>, <c>settings_sharing_save_failed</c>).</summary>
public sealed record SharingRefusals(string Refused, string SaveFailed);

/// <summary>What a press asks of the window and the daemon, one step each.</summary>
public interface ISharingChoiceSteps
{
    /// <summary>The keep-asks question: true keeps them asking, false makes them Automatic, null cancelled.</summary>
    Task<bool?> AskKeepFolderAsks(int count);

    /// <summary>Setup's two disclosures; the finished flow, or null when the person cancelled.</summary>
    Task<SharingDisclosureFlow?> Disclose();

    /// <summary>The core's answer to whether the grant may be asked for, from what the flow showed.</summary>
    Task<Flow1GrantRequest?> GrantRequest(SharingDisclosureFlow flow);

    Task<GrantAnswer> GrantAutomatic(string? witness, bool keepFolderAsks);

    Task<bool> SetAskEachTime();
}

/// <summary>
/// The Traces toolbar's "Automatic for every folder" eye (owner,
/// 2026-10-30 and 2026-11-02; macos/Sources/TraceCommonsApp/Views/Settings/SharingChoiceCard.swift
/// <c>SharingChoice</c> and <c>SharingChoiceCard.toolbarButton</c>), and
/// the column titles drawn while it shows (TracesViews.swift:193-198).
/// Pure, so the rules are tested apart from the window.
/// </summary>
public static class SharingChoice
{
    /// <summary>The eye (the Mac's SF Symbol <c>eye</c>; Segoe Fluent Icons View).</summary>
    public const string ToolbarGlyph = "";

    /// <summary>Submit's column title is centred over the usual split button (TracesViews.swift:1550-1552).</summary>
    public const double SubmitColumnMinWidth = 128;

    /// <summary>
    /// What is in force: Automatic while the include-existing grant is, Ask
    /// each time otherwise; null until <c>automatic_grant</c> has answered,
    /// so nothing is shown on a guess (SharingChoiceCard.swift:18-21).
    /// </summary>
    public static SharingPath? Current(AutomaticGrantState? grant) =>
        grant is null ? null
        : grant.Granted && grant.IncludeExisting == true ? SharingPath.Automatic
        : SharingPath.AskMe;

    /// <summary>
    /// Folders set to Ask by a rule of their own, from <c>list_projects</c>:
    /// configured, not the unresolved bucket, and <c>folder_mode</c> (else
    /// <c>mode</c>) Ask (SharingChoiceCard.swift:24-28).
    /// </summary>
    public static int AskingFolders(JsonElement listProjects)
    {
        if (listProjects.ValueKind != JsonValueKind.Object
            || !listProjects.TryGetProperty("projects", out JsonElement projects) || projects.ValueKind != JsonValueKind.Array)
        {
            return 0;
        }

        int count = 0;
        foreach (JsonElement p in projects.EnumerateArray())
        {
            if (p.ValueKind != JsonValueKind.Object
                || !(p.TryGetProperty("configured", out JsonElement c) && c.ValueKind == JsonValueKind.True)
                || (p.TryGetProperty("is_unresolved_bucket", out JsonElement b) && b.ValueKind == JsonValueKind.True))
            {
                continue;
            }

            string? mode = Str(p, "folder_mode") ?? Str(p, "mode");
            if (mode == MapModes.Ask)
            {
                count++;
            }
        }

        return count;
    }

    private static string? Str(JsonElement o, string key) =>
        o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

    /// <summary>The keep-asks question when any folder is set to Ask; straight to the disclosures otherwise.</summary>
    public static SharingStep FirstStep(int asking) =>
        asking > 0 ? new SharingStep.KeepAsks(asking) : new SharingStep.Disclose(true);

    /// <summary>
    /// The eye's tooltip line: how many folders the tab lists send on their
    /// own, of every folder it lists with a mode (Never counted;
    /// SharingChoiceCard.swift:34-39).
    /// </summary>
    public static string AutomaticCount(IEnumerable<TracesTree.FolderNode> folders, string words)
    {
        var listed = folders.Where(f => f.Mode is not null).ToList();
        int automatic = listed.Count(f => f.Mode == MapModes.AutoUpload);
        return words
            .Replace("{count}", automatic.ToString(System.Globalization.CultureInfo.CurrentCulture), StringComparison.Ordinal)
            .Replace("{total}", listed.Count.ToString(System.Globalization.CultureInfo.CurrentCulture), StringComparison.Ordinal);
    }

    /// <summary>The tooltip: its name, then the count line (SharingChoiceCard.swift:54-56).</summary>
    public static string ToolbarHelp(string name, string line) => line.Length == 0 ? name : name + "\n" + line;

    /// <summary>
    /// Drawn only once the grant and its words are known: never a guess,
    /// never dimmed for a state it cannot act on (SharingChoiceCard.swift:61-63).
    /// </summary>
    public static bool DrawsToolbarButton(bool grantKnown, bool wordsKnown) => grantKnown && wordsKnown;

    /// <summary>
    /// The column titles, while the eye's words exist (TracesViews.swift:193-198):
    /// the folders in the core's words for every computer but a Mac
    /// (<c>column_folders_computer</c>, owner 2026-11-18, as the Linux shell
    /// draws them), Submit and Auto. Null when any word is missing.
    /// </summary>
    public static TracesColumnTitles? ColumnTitles(TreeWords words)
    {
        string folders = words["column_folders_computer"], submit = words["column_submit"], automatic = words["column_automatic"];
        if (words["automatic_all"].Length == 0 || words["automatic_count"].Length == 0
            || folders.Length == 0 || submit.Length == 0 || automatic.Length == 0)
        {
            return null;
        }

        return new TracesColumnTitles(folders, submit, automatic);
    }

    /// <summary>
    /// A press of the eye (SharingChoiceCard.swift:217-260). On (Automatic
    /// in force) it is Ask each time. Off, it asks about folders set to Ask,
    /// shows setup's two disclosures, lets the core decide whether the grant
    /// may be asked for, then grants. A cancelled step does nothing. Returns
    /// the refusal to show under the eye, or null.
    /// </summary>
    public static async Task<string?> PressAsync(SharingPath? current, int asking, ISharingChoiceSteps steps, SharingRefusals refusals)
    {
        ArgumentNullException.ThrowIfNull(steps);
        ArgumentNullException.ThrowIfNull(refusals);
        switch (current)
        {
            case null:
                return null;
            case SharingPath.Automatic:
                return await steps.SetAskEachTime().ConfigureAwait(true) ? null : refusals.SaveFailed;
        }

        bool keep = true;
        if (FirstStep(asking) is SharingStep.KeepAsks ask)
        {
            if (await steps.AskKeepFolderAsks(ask.Count).ConfigureAwait(true) is not bool answer)
            {
                return null;
            }

            keep = answer;
        }

        if (await steps.Disclose().ConfigureAwait(true) is not { } flow)
        {
            return null;
        }

        if (await steps.GrantRequest(flow).ConfigureAwait(true) is not { Ready: true } request)
        {
            return refusals.Refused;
        }

        GrantAnswer granted = await steps.GrantAutomatic(request.WitnessSigningAddress, keep).ConfigureAwait(true);
        return granted.Granted ? null : refusals.Refused;
    }
}
