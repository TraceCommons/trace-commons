using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;

namespace TraceCommons.Interop.Map;

/// <summary>A tool's source declaration (TracesTree.swift:117-145).</summary>
public enum SourceMode
{
    Watch,
    Off,
    Unset,

    /// <summary>Settings that could not be read: never drawn as an answer.</summary>
    Unknown,
}

/// <summary>
/// The Traces tree, a port of the Mac's <c>TracesTree</c>
/// (macos/Sources/TraceCommonsApp/Views/Monitor/TracesTree.swift), built only
/// from what the core reports. <see cref="Folders"/> is what the tree pane
/// draws; <see cref="Tools"/> and <see cref="Unplaced"/> are the per-tool
/// aggregate the map draws.
/// </summary>
public sealed record TracesTree(
    IReadOnlyList<TracesTree.ToolNode> Tools,
    IReadOnlyList<TracesTree.FolderNode> Unplaced,
    IReadOnlyList<TracesTree.FolderNode> Folders,
    int HiddenIgnored = 0)
{
    public static readonly TracesTree Empty = new(System.Array.Empty<ToolNode>(), System.Array.Empty<FolderNode>(), System.Array.Empty<FolderNode>());

    /// <summary>TracesTree.swift:81-88.</summary>
    public sealed record ToolNode(string Kind, SourceMode Mode, IReadOnlyList<FolderNode> Folders)
    {
        public string Id => Kind;

        public int Waiting => Folders.Sum(f => f.Sessions.Count);
    }

    /// <summary>TracesTree.swift:90-115.</summary>
    public sealed record FolderNode(string Id, string Label, string? Mode, IReadOnlyList<MapEntry> Sessions)
    {
        public IReadOnlyList<string> OfferableModes { get; init; } = System.Array.Empty<string>();

        public string? Disclosure { get; init; }

        public bool IsBucket { get; init; }

        public int? PendingCount { get; init; }

        public int? ContributableCount { get; init; }

        public string? Path { get; init; }

        /// <summary>When its newest waiting trace started, or was queued; null with none.</summary>
        public DateTimeOffset? MostRecent
        {
            get
            {
                DateTimeOffset? newest = null;
                foreach (MapEntry s in Sessions)
                {
                    if ((s.StartedAt ?? s.DiscoveredAt) is { } at && (newest is null || at > newest))
                    {
                        newest = at;
                    }
                }

                return newest;
            }
        }
    }

    /// <summary>The <c>*_source_mode</c> value in its Mac reading (TracesTree.swift:121-128).</summary>
    public static SourceMode ModeOf(string? raw) => raw switch
    {
        "watch" => SourceMode.Watch,
        "off" => SourceMode.Off,
        _ => SourceMode.Unset,
    };

    /// <summary>TracesTree.swift:145-150.</summary>
    public static bool DrawsTool(string kind, SourceMode mode, bool hasFolders, IReadOnlySet<string> scansWhenUnset) =>
        mode is SourceMode.Watch or SourceMode.Off || hasFolders || scansWhenUnset.Contains(kind);

    /// <summary>
    /// TracesTree.swift:162-226. <paramref name="sourceModes"/> is
    /// <c>get_settings</c>'s modes by tool, null when settings were not read.
    /// </summary>
    public static TracesTree Build(
        IReadOnlyList<MapEntry> entries,
        IReadOnlyList<MapProject> projects,
        IReadOnlyDictionary<string, string?>? sourceModes,
        IReadOnlySet<string> scansWhenUnset,
        bool showsIgnored = false,
        bool keepsOrder = false,
        bool onlyWithSessions = false)
    {
        var folders = new Dictionary<string, FolderNode>(StringComparer.Ordinal);
        var sessions = new Dictionary<string, List<MapEntry>>(StringComparer.Ordinal);
        var order = new List<string>();
        void Add(FolderNode node)
        {
            if (folders.ContainsKey(node.Id))
            {
                return;
            }

            folders[node.Id] = node;
            sessions[node.Id] = new List<MapEntry>();
            order.Add(node.Id);
        }

        foreach (MapProject project in projects)
        {
            string? mode = project.KnownMode;
            Add(new FolderNode(project.ProjectId, project.DisplayLabel, mode, System.Array.Empty<MapEntry>())
            {
                OfferableModes = project.OfferableModes,
                Disclosure = mode == MapModes.AutoUpload ? project.AutomaticDisclosure : null,
                IsBucket = project.IsUnresolvedBucket,
                PendingCount = project.PendingCount,
                ContributableCount = project.ContributableCount,
                Path = project.ProjectPath,
            });
        }

        foreach (MapEntry entry in entries)
        {
            Add(new FolderNode(entry.ProjectId, entry.ProjectLabel, null, System.Array.Empty<MapEntry>()));
            sessions[entry.ProjectId].Add(entry);
        }

        var byTool = new Dictionary<string, List<FolderNode>>(StringComparer.Ordinal);
        var unplaced = new List<FolderNode>();
        var drawn = new List<FolderNode>();
        int hiddenIgnored = 0;
        foreach (string id in order)
        {
            List<MapEntry> list = sessions[id];
            if (onlyWithSessions && list.Count == 0)
            {
                continue;
            }

            if (!showsIgnored && folders[id].Mode == MapModes.Ignore)
            {
                hiddenIgnored++;
                continue;
            }

            IReadOnlyList<MapEntry> held = keepsOrder
                ? list
                : list.OrderByDescending(s => s.StartedAt ?? DateTimeOffset.MinValue).ToList();
            FolderNode node = folders[id] with { Sessions = held };
            drawn.Add(node);
            if (MajorityTool(node.Sessions) is { } kind)
            {
                if (!byTool.TryGetValue(kind, out List<FolderNode>? under))
                {
                    byTool[kind] = under = new List<FolderNode>();
                }

                under.Add(node);
            }
            else
            {
                unplaced.Add(node);
            }
        }

        var tools = new List<ToolNode>();
        foreach (string kind in MapTools.All)
        {
            SourceMode mode = sourceModes is null
                ? SourceMode.Unknown
                : ModeOf(sourceModes.TryGetValue(kind, out string? raw) ? raw : null);
            List<FolderNode> nodes = (byTool.TryGetValue(kind, out List<FolderNode>? under) ? under : new List<FolderNode>())
                .OrderByDescending(f => f.Sessions.Count).ToList();
            if (DrawsTool(kind, mode, nodes.Count > 0, scansWhenUnset))
            {
                tools.Add(new ToolNode(kind, mode, nodes));
            }
        }

        return new TracesTree(
            tools.OrderByDescending(t => t.Waiting).ToList(),
            unplaced,
            keepsOrder ? ListedOrder(drawn, entries) : Ordered(drawn),
            hiddenIgnored);
    }

    /// <summary>TracesTree.swift:230-238.</summary>
    public static IReadOnlyList<FolderNode> ListedOrder(IReadOnlyList<FolderNode> folders, IReadOnlyList<MapEntry> entries)
    {
        var rank = new Dictionary<string, int>(StringComparer.Ordinal);
        for (int i = 0; i < entries.Count; i++)
        {
            rank.TryAdd(entries[i].ProjectId, i);
        }

        var listed = folders.Where(f => rank.ContainsKey(f.Id)).OrderBy(f => rank[f.Id]).ToList();
        listed.AddRange(Ordered(folders.Where(f => !rank.ContainsKey(f.Id)).ToList()));
        return listed;
    }

    /// <summary>The tree with its folders in the person's arrangement (TracesTree.swift:243-257).</summary>
    public TracesTree Arranged(IReadOnlyList<string> order) => this with { Folders = Arranged(Folders, order) };

    public static IReadOnlyList<FolderNode> Arranged(IReadOnlyList<FolderNode> folders, IReadOnlyList<string> order)
    {
        var rank = new Dictionary<string, int>(StringComparer.Ordinal);
        for (int i = 0; i < order.Count; i++)
        {
            rank.TryAdd(order[i], i);
        }

        var placed = folders.Where(f => rank.ContainsKey(f.Id)).OrderBy(f => rank[f.Id]).ToList();
        placed.AddRange(folders.Where(f => !rank.ContainsKey(f.Id)));
        return placed;
    }

    /// <summary>
    /// The arrangement after <paramref name="moved"/> is dropped on
    /// <paramref name="target"/> (TracesTree.swift:264-272).
    /// </summary>
    public static IReadOnlyList<string> Moving(string moved, string target, IReadOnlyList<string> shown, IReadOnlyList<string> order)
    {
        var ids = shown.ToList();
        ids.AddRange(order.Where(id => !shown.Contains(id)));
        int from = ids.IndexOf(moved), to = ids.IndexOf(target);
        if (moved == target || from < 0 || to < 0)
        {
            return ids;
        }

        ids.RemoveAt(from);
        ids.Insert(to, moved);
        return ids;
    }

    /// <summary>
    /// Newest waiting first; folders with nothing waiting after them; ties by
    /// name as Finder sorts, then by id (TracesTree.swift:277-289).
    /// </summary>
    public static IReadOnlyList<FolderNode> Ordered(IReadOnlyList<FolderNode> folders)
    {
        var list = folders.ToList();
        list.Sort((a, b) =>
        {
            DateTimeOffset? x = a.MostRecent, y = b.MostRecent;
            if (x is { } xa && y is { } yb && xa != yb)
            {
                return yb.CompareTo(xa);
            }

            if (x is not null && y is null)
            {
                return -1;
            }

            if (x is null && y is not null)
            {
                return 1;
            }

            int byName = NaturalCompare(a.Label, b.Label);
            return byName != 0 ? byName : string.CompareOrdinal(a.Id, b.Id);
        });
        return list;
    }

    /// <summary>
    /// <c>localizedStandardCompare</c>'s reading of names: case ignored, runs
    /// of digits compared by value ("folder 2" before "folder 10").
    /// </summary>
    public static int NaturalCompare(string a, string b)
    {
        CompareInfo compare = CultureInfo.InvariantCulture.CompareInfo;
        int i = 0, j = 0;
        while (i < a.Length && j < b.Length)
        {
            if (char.IsDigit(a[i]) && char.IsDigit(b[j]))
            {
                int si = i, sj = j;
                while (i < a.Length && char.IsDigit(a[i]))
                {
                    i++;
                }

                while (j < b.Length && char.IsDigit(b[j]))
                {
                    j++;
                }

                string da = a[si..i].TrimStart('0'), db = b[sj..j].TrimStart('0');
                if (da.Length != db.Length)
                {
                    return da.Length.CompareTo(db.Length);
                }

                int digits = string.CompareOrdinal(da, db);
                if (digits != 0)
                {
                    return digits;
                }

                continue;
            }

            int c = compare.Compare(a, i, 1, b, j, 1, CompareOptions.IgnoreCase);
            if (c != 0)
            {
                return c;
            }

            i++;
            j++;
        }

        return (a.Length - i).CompareTo(b.Length - j);
    }

    /// <summary>
    /// The tool most of a folder's traces came from; null with none or with
    /// no known tool. Ties go to the alphabetically first adapter name, as
    /// the Mac's <c>max(by:)</c> over its comparator does (TracesTree.swift:293-304).
    /// </summary>
    public static string? MajorityTool(IReadOnlyList<MapEntry> sessions)
    {
        var counts = new Dictionary<string, int>(StringComparer.Ordinal);
        foreach (MapEntry s in sessions)
        {
            string? kind = MapTools.Known(s.DeclaredSource ?? s.Source) ? s.DeclaredSource ?? s.Source
                : MapTools.Known(s.Source) ? s.Source : null;
            if (kind is not null)
            {
                counts[kind] = counts.TryGetValue(kind, out int n) ? n + 1 : 1;
            }
        }

        string? best = null;
        int bestCount = 0;
        foreach ((string kind, int count) in counts)
        {
            if (best is null || count > bestCount || (count == bestCount && string.CompareOrdinal(kind, best) < 0))
            {
                best = kind;
                bestCount = count;
            }
        }

        return best;
    }

    /// <summary>
    /// The tree narrowed to <paramref name="query"/>, case and diacritics
    /// ignored; a blank query is the whole tree (TracesTree.swift:41-65).
    /// </summary>
    public TracesTree Matching(string query)
    {
        string needle = query.Trim();
        if (needle.Length == 0)
        {
            return this;
        }

        CompareInfo compare = CultureInfo.InvariantCulture.CompareInfo;
        bool Has(string? text) =>
            text is not null && compare.IndexOf(text, needle, CompareOptions.IgnoreCase | CompareOptions.IgnoreNonSpace) >= 0;
        FolderNode? Narrowed(FolderNode folder)
        {
            if (Has(folder.Label) || Has(folder.Path))
            {
                return folder;
            }

            var kept = folder.Sessions.Where(s =>
                Has(s.Title) || Has(s.ProjectLabel) || (MapTools.Known(s.Source) && Has(MapTools.DisplayName(s.Source)))).ToList();
            return kept.Count == 0 ? null : folder with { Sessions = kept };
        }

        return this with
        {
            Folders = Folders.Select(Narrowed).OfType<FolderNode>().ToList(),
            Unplaced = Unplaced.Select(Narrowed).OfType<FolderNode>().ToList(),
            Tools = Tools.Select(t => t with { Folders = t.Folders.Select(Narrowed).OfType<FolderNode>().ToList() })
                .Where(t => t.Folders.Count > 0).ToList(),
        };
    }

    /// <summary>The tree with only one tool, for the map's focus (TracesTree.swift:339-348).</summary>
    public TracesTree Focused(string? toolId)
    {
        if (toolId is null || !Tools.Any(t => t.Id == toolId))
        {
            return this;
        }

        return new TracesTree(
            Tools.Where(t => t.Id == toolId).ToList(),
            System.Array.Empty<FolderNode>(),
            Folders.Where(f => MajorityTool(f.Sessions) == toolId).ToList());
    }

    /// <summary>Every trace in the tree, in drawing order.</summary>
    public IReadOnlyList<MapEntry> AllSessions => Folders.SelectMany(f => f.Sessions).ToList();
}
