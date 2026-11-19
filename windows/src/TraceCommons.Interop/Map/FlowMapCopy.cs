using System;
using System.Collections.Generic;
using System.Text.Json;

namespace TraceCommons.Interop.Map;

/// <summary>
/// The flow map's words (<c>preview_copy::MonitorFlowMapCopy</c>, the
/// <c>flow_map</c> table of <c>tc_monitor_screens_copy_json</c>) and the
/// fills the Mac applies to them (<c>MonitorFlowMapCopy</c>,
/// macos/Sources/TCShellCore/MonitorGraphMapCopy.swift:65-330), plus the
/// table's top-level words the map names its nodes with
/// (<c>MonitorWords</c>, MonitorWords.swift:14-30).
/// </summary>
/// <remarks>
/// Where the Mac says "this Mac", Windows takes the core's "this computer"
/// variant from the same table (<c>legend_memory_computer</c> and its four
/// siblings), which the core exports for every computer but a Mac.
/// </remarks>
public sealed class FlowMapCopy
{
    private readonly IReadOnlyDictionary<string, string> _map;
    private readonly IReadOnlyDictionary<string, string> _top;

    private FlowMapCopy(IReadOnlyDictionary<string, string> map, IReadOnlyDictionary<string, string> top)
    {
        _map = map;
        _top = top;
    }

    private static readonly Lazy<FlowMapCopy?> Core = new(() => Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_monitor_screens_copy_json())));

    /// <summary>The core's table, read once; null when the core produced none.</summary>
    public static FlowMapCopy? Table => Core.Value;

    /// <summary>
    /// Decodes the monitor table. Null unless it is an object holding a
    /// <c>flow_map</c> object: the Mac decodes the table whole or not at all.
    /// </summary>
    public static FlowMapCopy? Parse(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind != JsonValueKind.Object
                || !doc.RootElement.TryGetProperty("flow_map", out JsonElement flow) || flow.ValueKind != JsonValueKind.Object)
            {
                return null;
            }

            var map = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (JsonProperty field in flow.EnumerateObject())
            {
                if (field.Value.ValueKind == JsonValueKind.String)
                {
                    map[field.Name] = field.Value.GetString()!;
                }
            }

            var top = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (JsonProperty field in doc.RootElement.EnumerateObject())
            {
                if (field.Value.ValueKind == JsonValueKind.String)
                {
                    top[field.Name] = field.Value.GetString()!;
                }
            }

            foreach (string key in Consumed)
            {
                if (!map.ContainsKey(key))
                {
                    return null;
                }
            }

            return new FlowMapCopy(map, top);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>The fields this shell draws; a table missing one is no table.</summary>
    public static readonly IReadOnlyList<string> Consumed = new[]
    {
        "map_label", "zoom_label", "sessions_one", "sessions", "traces_one", "traces", "folders_one", "folders",
        "tools_one", "tools", "hub", "library", "tool_title", "tool_watched", "tool_waiting", "tool_nothing_waiting",
        "tool_off", "tool_unset", "folder_rule", "folder_rule_unset", "folder_counts", "connected", "connected_line",
        "legend_not_watched", "reveal_folders_one", "reveal_folders", "fold_folders", "fold_folders_label",
        "tools_heading", "coin_waiting", "transfer_contributing", "transfer_keeping", "transfer_dismissed",
        "transfer_trace", "transfer_contribute_note", "transfer_contribute_note_mixed", "transfer_dismiss_note",
        "transfer_traces", "memory_claude_code", "memory_codex", "memory_opencode", "memory_gemini_cli", "memory_cline",
        "collected", "legend_memory_computer", "memory_stays_computer", "transfer_keep_note_computer",
        "kept_here_one_computer", "kept_here_computer", "style_title", "style_toggle", "style_caption", "credential",
        "none_found",
    };

    /// <summary>A word of the <c>flow_map</c> table.</summary>
    public string this[string key] => _map.TryGetValue(key, out string? word) ? word : string.Empty;

    /// <summary>A top-level word of the monitor table (<c>computer</c>, <c>commons</c>, ...).</summary>
    public string Top(string key) => _top.TryGetValue(key, out string? word) ? word : string.Empty;

    public string MapLabel => this["map_label"];
    public string ZoomLabel => this["zoom_label"];
    public string LegendNotWatched => this["legend_not_watched"];
    public string LegendMemory => this["legend_memory_computer"];
    public string ToolsHeading => this["tools_heading"];
    public string FoldFolders => this["fold_folders"];
    public string FoldFoldersLabel => this["fold_folders_label"];
    public string Credential => this["credential"];
    public string NoneFound => this["none_found"];
    public string StyleTitle => this["style_title"];
    public string StyleToggle => this["style_toggle"];
    public string StyleCaption => this["style_caption"];
    public string TransferContributing => this["transfer_contributing"];
    public string TransferKeeping => this["transfer_keeping"];
    public string TransferDismissed => this["transfer_dismissed"];
    public string TransferTrace => this["transfer_trace"];
    public string MemoryStays => this["memory_stays_computer"];
    public string TransferDismissNote => this["transfer_dismiss_note"];
    public string TransferContributeNoteMixed => this["transfer_contribute_note_mixed"];

    /// <summary>A number, or a dash for one the core did not report (MonitorGraphMapCopy.swift:198-200).</summary>
    public static string Figure(int? count) => count?.ToString(System.Globalization.CultureInfo.InvariantCulture) ?? "—";

    /// <summary>A counted noun; the singular is its own line (:203-205).</summary>
    public static string Counted(int? count, string one, string many) =>
        count == 1 ? one : many.Replace("{count}", Figure(count), StringComparison.Ordinal);

    private static string Fill(string word, string label) => word.Replace("{label}", label, StringComparison.Ordinal);

    private static string Fill(string word, string label, int? count) =>
        Fill(word, label).Replace("{count}", Figure(count), StringComparison.Ordinal);

    private string Sessions(int? n) => Counted(n, this["sessions_one"], this["sessions"]);

    /// <summary>This computer's card (:214-216).</summary>
    public string Hub(int waiting, int? contributed) => Fill(this["hub"], Sessions(waiting), contributed);

    /// <summary>"1 trace", "12 traces" (:220-222).</summary>
    public string TracesCounted(int count) => Counted(count, this["traces_one"], this["traces"]);

    /// <summary>The library's card (:224-226).</summary>
    public string Library(int? contributed) => Fill(this["library"], Counted(contributed, this["traces_one"], this["traces"]));

    /// <summary>A tool's card title (:229-232).</summary>
    public string ToolTitle(string tool, int folders) =>
        Fill(this["tool_title"], Counted(folders, this["folders_one"], this["folders"])).Replace("{tool}", tool, StringComparison.Ordinal);

    /// <summary>A tool's card (:251-260).</summary>
    public string Tool(SourceMode state, int waiting)
    {
        string waits = waiting > 0
            ? this["tool_waiting"].Replace("{count}", waiting.ToString(System.Globalization.CultureInfo.InvariantCulture), StringComparison.Ordinal)
            : this["tool_nothing_waiting"];
        return state switch
        {
            SourceMode.Watch => this["tool_watched"] + " " + waits,
            SourceMode.Off => this["tool_off"],
            SourceMode.Unset => this["tool_unset"],
            _ => waits,
        };
    }

    /// <summary>A folder's card (:264-270).</summary>
    public string Folder(string? path, string? rule, int waiting, int? contributed)
    {
        string ruleLine = rule is null ? this["folder_rule_unset"] : Fill(this["folder_rule"], rule);
        string counts = Fill(this["folder_counts"], Sessions(waiting), contributed);
        var parts = new List<string>();
        if (!string.IsNullOrEmpty(path))
        {
            parts.Add(path + ".");
        }

        parts.Add(ruleLine);
        parts.Add(counts);
        return string.Join(" ", parts);
    }

    /// <summary>The "+N" pill's accessible name (:273-275).</summary>
    public string RevealFolders(int count) =>
        count == 1 ? this["reveal_folders_one"] : this["reveal_folders"].Replace("{count}", N(count), StringComparison.Ordinal);

    /// <summary>A coin's line under its name (:278-280).</summary>
    public string CoinWaiting(int count) => this["coin_waiting"].Replace("{count}", N(count), StringComparison.Ordinal);

    /// <summary>
    /// The transfer card's note (:283-285); a kept trace's in the core's
    /// "this computer" words.
    /// </summary>
    public string TransferNote(bool contributing, string tool) =>
        contributing ? this["transfer_contribute_note"].Replace("{tool}", tool, StringComparison.Ordinal) : this["transfer_keep_note_computer"];

    /// <summary>A batch card's count line (:288-290).</summary>
    public string TransferTraces(int count) => this["transfer_traces"].Replace("{count}", N(count), StringComparison.Ordinal);

    /// <summary>A tool's own memory files (:294-303).</summary>
    public string? MemoryFiles(string source) => source switch
    {
        MapTools.ClaudeCode => this["memory_claude_code"],
        MapTools.Codex => this["memory_codex"],
        MapTools.OpenCode => this["memory_opencode"],
        MapTools.GeminiCli => this["memory_gemini_cli"],
        MapTools.Cline => this["memory_cline"],
        _ => null,
    };

    /// <summary>Under this computer while a contribution gathers (:306-309).</summary>
    public string Collected(int count, int total) =>
        this["collected"].Replace("{count}", N(count), StringComparison.Ordinal).Replace("{total}", N(total), StringComparison.Ordinal);

    /// <summary>Under this computer once traces are kept (:312-314), in the "this computer" words.</summary>
    public string KeptHere(int count) =>
        count == 1 ? this["kept_here_one_computer"] : this["kept_here_computer"].Replace("{count}", N(count), StringComparison.Ordinal);

    /// <summary>How many tools are connected (:318-320).</summary>
    public string Connected(int tools, bool sentence) =>
        Fill(sentence ? this["connected"] : this["connected_line"], Counted(tools, this["tools_one"], this["tools"]));

    private static string N(int n) => n.ToString(System.Globalization.CultureInfo.InvariantCulture);
}
