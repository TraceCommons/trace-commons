using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop.Map;

/// <summary>
/// What the Traces tree has selected (<c>MonitorSelection</c>,
/// TracesTree.swift:323-385): a folder or a trace, stored as
/// <c>folder:&lt;id&gt;</c> or <c>session:&lt;id&gt;</c>. Nothing selected is null.
/// </summary>
public abstract record TreeSelection
{
    private const string FolderTag = "folder:";
    private const string SessionTag = "session:";

    public sealed record Folder(string ProjectId) : TreeSelection
    {
        public override string RawValue => FolderTag + ProjectId;
    }

    public sealed record Session(string EntryId) : TreeSelection
    {
        public override string RawValue => SessionTag + EntryId;
    }

    /// <summary>The stored form, also the row's id (TracesViews.swift:554).</summary>
    public abstract string RawValue { get; }

    /// <summary>A stored selection, or null for anything else (TracesTree.swift:333-341).</summary>
    public static TreeSelection? Parse(string? raw)
    {
        if (raw is null)
        {
            return null;
        }

        if (raw.StartsWith(FolderTag, StringComparison.Ordinal) && raw.Length > FolderTag.Length)
        {
            return new Folder(raw[FolderTag.Length..]);
        }

        if (raw.StartsWith(SessionTag, StringComparison.Ordinal) && raw.Length > SessionTag.Length)
        {
            return new Session(raw[SessionTag.Length..]);
        }

        return null;
    }
}

/// <summary>One row of the tree as drawn (TracesViews.swift:244-256).</summary>
public abstract record TreeRow
{
    public sealed record FolderRow(TracesTree.FolderNode Folder) : TreeRow
    {
        public override string Id => new TreeSelection.Folder(Folder.Id).RawValue;
    }

    /// <summary><paramref name="Off"/>: under an ignored folder, so drawn off too.</summary>
    public sealed record SessionRow(MapEntry Entry, bool Off) : TreeRow
    {
        public override string Id => new TreeSelection.Session(Entry.EntryId).RawValue;
    }

    public abstract string Id { get; }
}

/// <summary>A row's open menu (TracesViews.swift:107-116).</summary>
public abstract record TreeRowMenu
{
    /// <summary>A folder's "..." menu: Ignore.</summary>
    public sealed record FolderMenu(string ProjectId) : TreeRowMenu;

    /// <summary>A folder's Submit split button's menu: Submit all as.</summary>
    public sealed record SubmitMenu(string ProjectId) : TreeRowMenu;

    /// <summary>A trace's menu: Dismiss trace.</summary>
    public sealed record SessionMenu(string EntryId) : TreeRowMenu;
}

/// <summary>
/// The tree's words, the core's (<c>monitor_traces_copy</c>'s <c>tree</c> and
/// <c>counts</c> tables). Empty where the core said nothing, never typed here.
/// </summary>
public sealed record TreeWords(IReadOnlyDictionary<string, string> Tree, IReadOnlyDictionary<string, string> Counts, IReadOnlyDictionary<string, string> Top)
{
    public static readonly Lazy<TreeWords> Core = new(() => Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_monitor_traces_copy_json())));

    public string this[string key] => Tree.TryGetValue(key, out string? w) ? w : string.Empty;

    public string Count(string key) => Counts.TryGetValue(key, out string? w) ? w : string.Empty;

    public string Word(string key) => Top.TryGetValue(key, out string? w) ? w : string.Empty;

    public static TreeWords Parse(string? json)
    {
        var tree = new Dictionary<string, string>(StringComparer.Ordinal);
        var counts = new Dictionary<string, string>(StringComparer.Ordinal);
        var top = new Dictionary<string, string>(StringComparer.Ordinal);
        if (!string.IsNullOrEmpty(json))
        {
            try
            {
                using JsonDocument doc = JsonDocument.Parse(json);
                foreach (JsonProperty p in doc.RootElement.EnumerateObject())
                {
                    if (p.Value.ValueKind == JsonValueKind.String)
                    {
                        top[p.Name] = p.Value.GetString() ?? string.Empty;
                    }
                    else if (p.Value.ValueKind == JsonValueKind.Object && (p.Name == "tree" || p.Name == "counts"))
                    {
                        Dictionary<string, string> into = p.Name == "tree" ? tree : counts;
                        foreach (JsonProperty w in p.Value.EnumerateObject())
                        {
                            if (w.Value.ValueKind == JsonValueKind.String)
                            {
                                into[w.Name] = w.Value.GetString() ?? string.Empty;
                            }
                        }
                    }
                }
            }
            catch (JsonException)
            {
                // No words: every row then says nothing rather than something typed here.
            }
        }

        return new TreeWords(tree, counts, top);
    }

    /// <summary><c>{name}</c> filled, as the Mac's <c>FirstRunCopy.fill</c>.</summary>
    public static string Fill(string template, string name, string value) =>
        template.Replace("{" + name + "}", value, StringComparison.Ordinal);
}

/// <summary>
/// The Traces order control's words, the core's nudge words
/// (<c>tc_nudge_copy_json</c>, NudgeCopy.swift): its name, "Oldest first" and
/// "Your order". Empty where the core said nothing.
/// </summary>
public static class TreeOrderWords
{
    private static readonly Lazy<IReadOnlyDictionary<string, string>> Words = new(() =>
    {
        var words = new Dictionary<string, string>(StringComparer.Ordinal);
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_nudge_copy_json());
        if (!string.IsNullOrEmpty(json))
        {
            try
            {
                using JsonDocument doc = JsonDocument.Parse(json);
                foreach (JsonProperty p in doc.RootElement.EnumerateObject())
                {
                    if (p.Value.ValueKind == JsonValueKind.String)
                    {
                        words[p.Name] = p.Value.GetString() ?? string.Empty;
                    }
                }
            }
            catch (JsonException)
            {
                // No words: the control is not drawn.
            }
        }

        return words;
    });

    public static string Label => Word("LIST_ORDER_LABEL");

    public static string Queue => Word("LIST_ORDER_QUEUE");

    public static string Arranged => Word("LIST_ORDER_ARRANGED");

    private static string Word(string key) => Words.Value.TryGetValue(key, out string? w) ? w : string.Empty;
}

/// <summary>
/// The Traces tree's rules, as the Mac's <c>TracesTreeView</c> statics
/// (macos/Sources/TraceCommonsApp/Views/Monitor/TracesViews.swift). Pure, so
/// the window only draws what these return.
/// </summary>
public static class TracesTreeRows
{
    /// <summary>How far a press travels before it drags a folder (:717).</summary>
    public const double FolderDragThreshold = 6;

    /// <summary>#1146's inset for the lines under a folder row, <c>px-14</c> (:599).</summary>
    public const double NoteInset = 38;

    /// <summary>A row scrolled into view: GlassMotion.fast, the token curve over 0.15 s (:503).</summary>
    public static readonly BezierCurve ScrollToRow = BezierCurve.Token(MonitorMotion.Fast);

    /// <summary>A dragged folder settling where it was dropped: spring(0.3, 0.85) (:764).</summary>
    public static readonly SpringCurve FolderDragRelease = new(0.3, 0.85);

    /// <summary>The search field moving in from the trailing edge with a fade, fast (NudgeViews.swift:150-161).</summary>
    public static readonly BezierCurve SearchFieldSlide = BezierCurve.Token(MonitorMotion.Fast);

    /// <summary>Every folder, and the traces of the open ones (:261-272).</summary>
    public static IReadOnlyList<TreeRow> DrawnRows(TracesTree tree, IReadOnlySet<string> expanded)
    {
        var rows = new List<TreeRow>();
        foreach (TracesTree.FolderNode folder in tree.Folders)
        {
            rows.Add(new TreeRow.FolderRow(folder));
            if (expanded.Contains(folder.Id))
            {
                bool off = folder.Mode == MapModes.Ignore;
                rows.AddRange(folder.Sessions.Select(s => new TreeRow.SessionRow(s, off)));
            }
        }

        return rows;
    }

    /// <summary>The selectable rows in drawing order (:465-475).</summary>
    public static IReadOnlyList<TreeSelection> VisibleRows(TracesTree tree, IReadOnlySet<string> expanded) =>
        DrawnRows(tree, expanded).Select(r => r switch
        {
            TreeRow.FolderRow f => (TreeSelection)new TreeSelection.Folder(f.Folder.Id),
            TreeRow.SessionRow s => new TreeSelection.Session(s.Entry.EntryId),
            _ => throw new InvalidOperationException(),
        }).ToList();

    /// <summary>The open folders once <paramref name="selected"/> is shown (:479-483).</summary>
    public static IReadOnlySet<string> Revealing(TreeSelection? selected, TracesTree tree, IReadOnlySet<string> expanded)
    {
        if (selected is not TreeSelection.Session session || FolderOf(session.EntryId, tree) is not { } folder)
        {
            return expanded;
        }

        var open = new HashSet<string>(expanded, StringComparer.Ordinal) { folder };
        return open;
    }

    /// <summary>One step up or down; the top row when nothing drawn is selected; the ends hold (:510-514).</summary>
    public static TreeSelection? Moved(TreeSelection? current, bool down, IReadOnlyList<TreeSelection> rows)
    {
        if (rows.Count == 0)
        {
            return null;
        }

        int at = current is null ? -1 : IndexOf(rows, current);
        if (at < 0)
        {
            return rows[0];
        }

        return rows[down ? Math.Min(at + 1, rows.Count - 1) : Math.Max(at - 1, 0)];
    }

    /// <summary>A row click: the selected row again selects nothing (:518-520).</summary>
    public static TreeSelection? Toggled(TreeSelection clicked, TreeSelection? current) =>
        clicked == current ? null : clicked;

    /// <summary>
    /// Whether a selection opens the inspector (<c>MonitorWindowView.select</c>,
    /// MonitorWindowView.swift:828-831): a trace does, every time it is
    /// selected; a folder, or clearing the selection, never does.
    /// </summary>
    public static bool OpensInspector(TreeSelection? selected) => selected is TreeSelection.Session;

    /// <summary>The menu open after a row's control for <paramref name="menu"/> is pressed (:120-122).</summary>
    public static TreeRowMenu? Toggled(TreeRowMenu menu, TreeRowMenu? current) => menu == current ? null : menu;

    /// <summary>
    /// A row kebab's ink token (Controls.swift <c>GlassKebab.ink</c>): on a
    /// selected row, the row's own ink, as <c>GlassGround.selection.controlInk</c>;
    /// otherwise null, the button's own. The button's dark ink on the
    /// selection measured 2.72:1 (owner, 2026-11-20).
    /// </summary>
    public static string? KebabInk(bool selected) => selected ? "textOnAccent" : null;

    /// <summary>The folder holding a selection, or null when it is not in the tree (:545-552).</summary>
    public static string? FolderOf(TreeSelection selected, TracesTree tree) => selected switch
    {
        TreeSelection.Session s => FolderOf(s.EntryId, tree),
        TreeSelection.Folder f => tree.Folders.Any(x => x.Id == f.ProjectId) ? f.ProjectId : null,
        _ => null,
    };

    /// <summary>The folder holding a trace (:557-559).</summary>
    public static string? FolderOf(string entryId, TracesTree tree) =>
        tree.Folders.FirstOrDefault(f => f.Sessions.Any(s => s.EntryId == entryId))?.Id;

    /// <summary>
    /// What a search does to the open folders (:222-232): it opens every
    /// folder it matched and remembers which were closed; clearing it closes
    /// exactly those, never <paramref name="keep"/>, the selection's folder.
    /// </summary>
    public static (IReadOnlySet<string> Expanded, IReadOnlySet<string> SearchOpened) SearchExpansion(
        string query, IReadOnlyList<string> matched, IReadOnlySet<string> expanded, IReadOnlySet<string> searchOpened,
        string? keep = null)
    {
        if (string.IsNullOrWhiteSpace(query))
        {
            var closing = new HashSet<string>(searchOpened, StringComparer.Ordinal);
            if (keep is not null)
            {
                closing.Remove(keep);
            }

            var left = new HashSet<string>(expanded, StringComparer.Ordinal);
            left.ExceptWith(closing);
            return (left, new HashSet<string>(StringComparer.Ordinal));
        }

        var opened = new HashSet<string>(matched, StringComparer.Ordinal);
        opened.ExceptWith(expanded);
        var open = new HashSet<string>(expanded, StringComparer.Ordinal);
        open.UnionWith(matched);
        var remembered = new HashSet<string>(searchOpened, StringComparer.Ordinal);
        remembered.UnionWith(opened);
        return (open, remembered);
    }

    /// <summary>Whether an empty list still draws its controls (:236-238).</summary>
    public static bool KeepsControlsWhenEmpty(bool idleOnly, int hiddenIgnored) => idleOnly || hiddenIgnored > 0;

    /// <summary>
    /// The folder a dragged folder trades places with: above once its middle
    /// is passed going up, below going down; null between them (:722-727).
    /// </summary>
    public static string? FolderSwap(double center, (string Id, double MidY)? above, (string Id, double MidY)? below)
    {
        if (above is { } a && center < a.MidY)
        {
            return a.Id;
        }

        if (below is { } b && center > b.MidY)
        {
            return b.Id;
        }

        return null;
    }

    /// <summary>
    /// A folder row's sub line (:944-953): the mode's word and the count
    /// waiting, or the core's line for an ignored folder.
    /// </summary>
    public static string FolderSub(TracesTree.FolderNode folder, TreeWords words, Func<string, string> modeLabel)
    {
        if (folder.Mode == MapModes.Ignore)
        {
            return words["ignored_folder"];
        }

        string count = folder.Sessions.Count == 1
            ? words.Count("sessions_waiting_one")
            : TreeWords.Fill(words.Count("sessions_waiting"), "count", folder.Sessions.Count.ToString(CultureInfo.InvariantCulture));
        string? mode = folder.Mode is { } m ? modeLabel(m) : null;
        return string.Join(" · ", new[] { mode, count }.Where(p => !string.IsNullOrEmpty(p)));
    }

    /// <summary>The folder's Submit pill, with the count the daemon says it sends (:956-958).</summary>
    public static string SubmitTitle(int count, TreeWords words) =>
        TreeWords.Fill(words["submit_count"], "count", count.ToString(CultureInfo.InvariantCulture));

    /// <summary>The pill's help: what it sends, then what it leaves behind (:962-964).</summary>
    public static string SubmitHelp(string? withheld, TreeWords words) =>
        string.Join(" ", new[] { words["submit_tip"], withheld }.Where(p => !string.IsNullOrEmpty(p)));

    /// <summary>The line under a folder Submit leaves traces behind in (:986-990).</summary>
    public static string WithheldNote(string withheld, int eligible, TreeWords words) =>
        TreeWords.Fill(words["eligible_count"], "count", eligible.ToString(CultureInfo.InvariantCulture)) + " · " + withheld;

    /// <summary>
    /// A trace's title, #1146's <c>sessionWhen</c> (:976-981): the short
    /// weekday and time it was found, or when it started, in the person's
    /// zone and culture; a dash with neither.
    /// </summary>
    public static string When(MapEntry entry, TimeZoneInfo zone, CultureInfo culture)
    {
        if ((entry.DiscoveredAt ?? entry.StartedAt) is not { } at)
        {
            return "—";
        }

        DateTime local = TimeZoneInfo.ConvertTime(at, zone).DateTime;
        DateTimeFormatInfo f = culture.DateTimeFormat;
        return f.AbbreviatedDayNames[(int)local.DayOfWeek] + " " + local.ToString(f.ShortTimePattern, culture);
    }

    /// <summary>
    /// A size as the Mac's <c>ByteCountFormatter</c> with <c>.memory</c>
    /// writes it (:1060-1062): binary units, whole KB, MB to one place, GB
    /// and up to two, trailing zeros dropped. Below 1 KB the Mac spells
    /// "bytes" and "Zero", words of its own formatter; this shell writes the
    /// count with "B", as its queue always has, rather than author them.
    /// </summary>
    public static string Size(long bytes, CultureInfo culture)
    {
        if (bytes < 1024)
        {
            return bytes.ToString("N0", culture) + " B";
        }

        string[] units = { "KB", "MB", "GB", "TB" };
        int[] places = { 0, 1, 2, 2 };
        double value = bytes / 1024.0;
        int unit = 0;
        while (value >= 1024 && unit < units.Length - 1)
        {
            value /= 1024;
            unit++;
        }

        string format = places[unit] == 0 ? "#,0" : "#,0." + new string('#', places[unit]);
        return value.ToString(format, culture) + " " + units[unit];
    }

    /// <summary>Held when a person has to look before it can go (:1008-1010).</summary>
    public static bool IsHeld(bool heldForSecondLook, bool heldByManualScrubCheck, bool returnedFromKeep) =>
        heldForSecondLook || heldByManualScrubCheck || returnedFromKeep;

    /// <summary>
    /// The same, from the daemon's reason label: second-look review, Manual
    /// Scrub check, or back from Keep (DaemonDataModels.swift:324-328, :412-414).
    /// </summary>
    public static bool IsHeld(string? reasonLabel) => reasonLabel is "second-look-review-required" or "scrub-check-manual" or "returned-from-keep";

    /// <summary>
    /// A trace's sub line (:1031-1056): its size, then its state: held, the
    /// core's ineligible sentence, trimmed to fit, the attestation sentence,
    /// else waiting.
    /// </summary>
    public static string? SessionSub(
        long? sizeBytes, bool held, string? heldWord, string? ineligible, bool trimmed, string? attestation,
        TreeWords words, CultureInfo culture)
    {
        string? state;
        if (held && !string.IsNullOrEmpty(heldWord))
        {
            state = heldWord;
        }
        else if (!string.IsNullOrEmpty(ineligible))
        {
            state = ineligible;
        }
        else if (trimmed)
        {
            state = words["session_trimmed"];
        }
        else if (!string.IsNullOrEmpty(attestation))
        {
            state = attestation;
        }
        else
        {
            state = words["session_waiting"];
        }

        string? size = sizeBytes is { } b and >= 0 ? Size(b, culture) : null;
        var parts = new[] { size, state }.Where(p => !string.IsNullOrEmpty(p)).ToList();
        return parts.Count == 0 ? null : string.Join(" · ", parts);
    }

    /// <summary>
    /// The daemon's reasons a pending trace waits for a person
    /// (<c>ReasonLabel.needingAPerson</c>, DaemonDataModels.swift:428-441,
    /// <c>queue.rs</c> <c>REASONS_NEEDING_A_PERSON</c>).
    /// </summary>
    public static readonly IReadOnlySet<string> ReasonsNeedingAPerson = new HashSet<string>(StringComparer.Ordinal)
    {
        "token-distribution-review-required", "witness-risk-review-required", "privacy-filter-transient-exhausted",
        "second-look-review-required", "witness_timeout", "witness_unreachable", "witness_classifier_unavailable",
        "witness_redaction_failed", "witness_attestation_unavailable", "witness_collateral_unavailable",
        "witness-not-trusted", "witness-measurement-not-accepted",
    };

    /// <summary>Held for review: pending, for a reason that needs a person (DaemonDataModels.swift:333-335).</summary>
    public static bool HeldForReview(string? state, string? reasonLabel) =>
        state == "pending" && reasonLabel is not null && ReasonsNeedingAPerson.Contains(reasonLabel);

    /// <summary>
    /// Whether a trace row's Submit may send (<c>maySubmitSession</c>,
    /// TracesStore.swift:801-806): a core that answered and is signed in, a
    /// folder that is not ignored, a trace not held for review, and the
    /// shared eligibility table offering Contribute. Asked again at the press.
    /// </summary>
    public static bool MaySubmitTrace(bool offersContribute, bool? loggedIn, bool ready, bool folderIgnored, string? state, string? reasonLabel) =>
        ready && loggedIn == true && !folderIgnored && !HeldForReview(state, reasonLabel) && offersContribute;

    /// <summary>
    /// <see cref="MaySubmitTrace"/> for the trace <paramref name="entryId"/>
    /// of <paramref name="tree"/>, its folder's mode read from the tree.
    /// </summary>
    public static bool MaySubmitIn(TracesTree tree, bool? loggedIn, bool ready, string entryId, bool offersContribute, string? state, string? reasonLabel)
    {
        bool ignored = tree.Folders.FirstOrDefault(f => f.Sessions.Any(s => s.EntryId == entryId))?.Mode == MapModes.Ignore;
        return MaySubmitTrace(offersContribute, loggedIn, ready, ignored, state, reasonLabel);
    }

    /// <summary>A trace's flag (:1016-1024): amber held or ineligible or a refused mark, green for a clear one.</summary>
    public static TreeFlag? Flag(bool held, bool ineligible, TreeAttestationTone? attestation)
    {
        if (held || ineligible)
        {
            return TreeFlag.Ask;
        }

        return attestation switch
        {
            TreeAttestationTone.Attention or TreeAttestationTone.Refused => TreeFlag.Ask,
            TreeAttestationTone.Clear => TreeFlag.On,
            _ => null,
        };
    }

    /// <summary>
    /// The selection's tool, what the graph counts and the binoculars focus
    /// (MonitorWindowView.swift:711-724): a trace's own tool, declared first;
    /// a folder's tool on the map, else its traces' majority; null otherwise.
    /// </summary>
    public static string? ToolOf(TreeSelection? selection, TracesTree tree)
    {
        switch (selection)
        {
            case TreeSelection.Session s when tree.AllSessions.FirstOrDefault(e => e.EntryId == s.EntryId) is { } entry:
                return MapTools.Known(entry.DeclaredSource ?? entry.Source) ? entry.DeclaredSource ?? entry.Source
                    : MapTools.Known(entry.Source) ? entry.Source : null;
            case TreeSelection.Folder f when tree.Folders.FirstOrDefault(x => x.Id == f.ProjectId) is { } folder:
                return tree.Tools.FirstOrDefault(t => t.Folders.Any(x => x.Id == folder.Id))?.Id
                    ?? TracesTree.MajorityTool(folder.Sessions);
            default:
                return null;
        }
    }

    private static int IndexOf(IReadOnlyList<TreeSelection> rows, TreeSelection current)
    {
        for (int i = 0; i < rows.Count; i++)
        {
            if (rows[i] == current)
            {
                return i;
            }
        }

        return -1;
    }
}

/// <summary>A row's flag colour (<c>GlassListRow.Flag</c>).</summary>
public enum TreeFlag
{
    Ask,
    On,
}

/// <summary>The attestation tones the flag reads (<c>PrivateInferenceTone</c>).</summary>
public enum TreeAttestationTone
{
    Clear,
    Attention,
    Refused,
    Plain,
}

/// <summary>
/// The tree's own state, as the Mac's <c>TracesTreeView</c> keeps it
/// (:59-100) and <c>TracesStore</c>'s folder order (TracesStore.swift:73-125):
/// open folders, the folders a search opened, the selection and the one row
/// menu open. The window draws <see cref="Rows"/> and forwards its clicks,
/// keys and drags here; nothing about what changes is decided in the window.
/// </summary>
public sealed class TracesTreeState
{
    private HashSet<string> _expanded = new(StringComparer.Ordinal);
    private HashSet<string> _searchOpened = new(StringComparer.Ordinal);
    private TreeSelection? _revealed;
    private TracesTree _tree = TracesTree.Empty;
    private List<string> _folderOrder = new();

    public TracesTreeState(IReadOnlyList<string>? folderOrder = null, bool arranged = false)
    {
        _folderOrder = folderOrder?.ToList() ?? new List<string>();
        Arranged = arranged;
    }

    /// <summary>The tree as read, before the search and the person's order.</summary>
    public TracesTree Tree => _tree;

    /// <summary>The tree drawn: in the person's order while arranged, narrowed to the search (TracesStore.swift:70).</summary>
    public TracesTree Shown => (Arranged ? _tree.Arranged(_folderOrder) : _tree).Matching(Search);

    public IReadOnlySet<string> Expanded => _expanded;

    /// <summary>Whether the core answered the tree's reads (the Mac's phase not failed).</summary>
    public bool Ready => _read?.Ready == true;

    /// <summary><c>status.logged_in</c> from the same read; null when unread.</summary>
    public bool? LoggedIn => _read?.Status?.LoggedIn;

    /// <summary>
    /// Whether a trace's Submit may send, wherever it is drawn: the tree's
    /// row, the inspector's trace card, and the press of either
    /// (<see cref="TracesTreeRows.MaySubmitIn"/> over this read).
    /// </summary>
    public bool MaySubmit(string entryId, bool offersContribute, string? stateCode, string? reasonCode) =>
        TracesTreeRows.MaySubmitIn(_tree, LoggedIn, Ready, entryId, offersContribute, stateCode, reasonCode);

    public TreeSelection? Selection { get; private set; }

    public TreeRowMenu? Menu { get; private set; }

    public string Search { get; private set; } = string.Empty;

    public IReadOnlyList<string> FolderOrder => _folderOrder;

    public bool Arranged { get; private set; }

    /// <summary>A row to scroll to once its folder has opened, or now.</summary>
    public string? ScrollTarget { get; private set; }

    public IReadOnlyList<TreeRow> Rows => TracesTreeRows.DrawnRows(Shown, _expanded);

    /// <summary>Whether ignored folders are drawn: the order menu's switch, on by default (TracesStore.swift:32).</summary>
    public bool ShowsIgnored { get; private set; } = true;

    /// <summary>Folders in the order the core listed them: "Oldest first" chosen (TracesTree.swift:156-161).</summary>
    public bool KeepsOrder { get; private set; }

    private MapSnapshot? _read;
    private IReadOnlySet<string> _scans = new HashSet<string>();

    /// <summary>A read from the core, built into the tree with this list's options (TracesStore.swift:194-199).</summary>
    public void Read(MapSnapshot snapshot, IReadOnlySet<string> scansWhenUnset)
    {
        _read = snapshot;
        _scans = scansWhenUnset;
        Rebuild();
    }

    /// <summary>"Show ignored folders" (TracesStore.swift:32-36).</summary>
    public void ShowIgnored(bool shown)
    {
        ShowsIgnored = shown;
        Rebuild();
    }

    /// <summary>
    /// "Oldest first": the core's queue order, folders by their first listed
    /// trace, and the person's order off (TracesStore.swift setOrder). The
    /// read is already in queue order, the core's default for <c>list_pending</c>.
    /// </summary>
    public void ChooseQueueOrder()
    {
        KeepsOrder = true;
        Arranged = false;
        Rebuild();
    }

    /// <summary>"Your order": the folders as drawn become the order if none was kept (TracesStore.swift:113-120).</summary>
    public void ChooseArranged()
    {
        if (_folderOrder.Count == 0)
        {
            _folderOrder = Shown.Folders.Select(f => f.Id).ToList();
        }

        Arranged = true;
    }

    private void Rebuild()
    {
        if (_read is { Entries: { } entries, Projects: { } projects } read)
        {
            SetTree(TracesTree.Build(entries, projects, read.SourceModes, _scans, ShowsIgnored, KeepsOrder));
        }
        else if (_read is not null)
        {
            SetTree(TracesTree.Empty);
        }
    }

    /// <summary>A new read. A selection waiting for its trace is revealed now (TracesViews.swift:322-325).</summary>
    public void SetTree(TracesTree tree)
    {
        _tree = tree;
        Reveal();
    }

    /// <summary>A row clicked: selects it, or nothing when it was selected (:518-520).</summary>
    public void Click(TreeSelection row) => Select(TracesTreeRows.Toggled(row, Selection));

    /// <summary>The selection moved, by the keyboard or from elsewhere: a trace's folder opens and the row is scrolled to (:300-309).</summary>
    public void Select(TreeSelection? selection)
    {
        Selection = selection;
        Reveal();
        if (selection is not null)
        {
            ScrollTarget = selection.RawValue;
        }
    }

    /// <summary>The scroll target was scrolled to.</summary>
    public void Scrolled() => ScrollTarget = null;

    /// <summary>Up or down from the selection through the rows as drawn (:522-528).</summary>
    public void Move(bool down)
    {
        TreeSelection? next = TracesTreeRows.Moved(Selection, down, TracesTreeRows.VisibleRows(Shown, _expanded));
        if (next is not null && next != Selection)
        {
            Select(next);
        }
    }

    /// <summary>Left closes the selection's folder, right opens it; the selection stays (:535-543).</summary>
    public void Disclose(bool open)
    {
        if (Selection is null || TracesTreeRows.FolderOf(Selection, Shown) is not { } folder)
        {
            return;
        }

        _searchOpened.Remove(folder);
        if (open)
        {
            _expanded.Add(folder);
        }
        else
        {
            _expanded.Remove(folder);
        }
    }

    /// <summary>A folder's chevron (:585-588).</summary>
    public void ToggleOpen(string folderId)
    {
        _searchOpened.Remove(folderId);
        if (!_expanded.Remove(folderId))
        {
            _expanded.Add(folderId);
        }
    }

    public void ToggleMenu(TreeRowMenu menu) => Menu = TracesTreeRows.Toggled(menu, Menu);

    public void CloseMenu() => Menu = null;

    /// <summary>The applied search: opens what it matched, clearing closes what it opened (:328-335).</summary>
    public void SetSearch(string query)
    {
        Search = query;
        (IReadOnlySet<string> expanded, IReadOnlySet<string> opened) = TracesTreeRows.SearchExpansion(
            query, Shown.Folders.Select(f => f.Id).ToList(), _expanded, _searchOpened,
            Selection is null ? null : TracesTreeRows.FolderOf(Selection, _tree));
        _expanded = new HashSet<string>(expanded, StringComparer.Ordinal);
        _searchOpened = new HashSet<string>(opened, StringComparer.Ordinal);
    }

    /// <summary>
    /// Moves <paramref name="moved"/> onto <paramref name="target"/> and draws
    /// the person's order from now on, against every folder, not only those a
    /// search leaves (TracesStore.swift:93-98). True when the order changed.
    /// </summary>
    public bool MoveFolder(string moved, string target)
    {
        var shown = (Arranged ? _tree.Arranged(_folderOrder) : _tree).Folders.Select(f => f.Id).ToList();
        if (moved == target || !shown.Contains(moved) || !shown.Contains(target))
        {
            return false;
        }

        _folderOrder = TracesTree.Moving(moved, target, shown, _folderOrder).ToList();
        Arranged = true;
        KeepsOrder = false;
        return true;
    }

    private void Reveal()
    {
        if (Selection == _revealed)
        {
            return;
        }

        // Wait for a trace the tree does not hold yet (:487-488).
        if (Selection is TreeSelection.Session s && TracesTreeRows.FolderOf(s.EntryId, Shown) is null)
        {
            return;
        }

        _revealed = Selection;
        _expanded = new HashSet<string>(TracesTreeRows.Revealing(Selection, Shown, _expanded), StringComparer.Ordinal);
    }
}
