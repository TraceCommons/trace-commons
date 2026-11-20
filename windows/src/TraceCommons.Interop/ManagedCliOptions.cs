using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// The CLI options drafts (<c>ManagedCLIOptionsDrafts</c>,
/// ManagedSessionsSurface.swift:1117-1137): a field keeps what was typed
/// until a save of exactly that text succeeds; a tool with no saved value
/// is never "changed", so Save stays off.
/// </summary>
public sealed class ManagedCliOptionsDrafts
{
    private readonly Dictionary<string, string> _values = new();

    public string Value(string tool, IReadOnlyDictionary<string, string>? saved) =>
        _values.TryGetValue(tool, out string? typed) ? typed
        : saved is not null && saved.TryGetValue(tool, out string? stored) ? stored
        : string.Empty;

    public bool IsChanged(string tool, IReadOnlyDictionary<string, string>? saved) =>
        saved is not null && saved.TryGetValue(tool, out string? stored) && Value(tool, saved) != stored;

    public void Edit(string tool, string options) => _values[tool] = options;

    public void Saved(string tool, string options)
    {
        if (_values.TryGetValue(tool, out string? typed) && typed == options)
        {
            _values.Remove(tool);
        }
    }
}

/// <summary>
/// Settings > Tools' CLI options card (<c>ManagedCLIOptionsSection</c>,
/// ManagedCLIOptionsSection.swift, and <c>AppModel.saveManagedCLIOptions</c>
/// with <c>managedAction</c>, AppModel.swift:155-232). The daemon owns the
/// saved arguments; a row is drawn for each supported tool the snapshot
/// carries options for. A save stands only when the reply echoes the tool
/// and the options and carries a revision; anything else is a refusal,
/// shown under that tool's row as the managed words' line for it. Each
/// action then reads the snapshot again. One action at a time.
/// </summary>
public sealed class ManagedCliOptionsFlow
{
    /// <summary>The tools whose options are offered, in order (<c>ManagedSurface.supportedTools</c>, :299).</summary>
    public static readonly string[] SupportedTools = ["claude", "codex", "opencode"];

    private readonly Func<string, string, Task<DaemonResponse>> _call;

    public ManagedCliOptionsFlow(Func<string, string, Task<DaemonResponse>> call)
    {
        _call = call;
    }

    public ManagedCliOptionsDrafts Drafts { get; } = new();

    /// <summary>The last snapshot; null until one answered.</summary>
    public ManagedSnapshot? Snapshot { get; private set; }

    /// <summary>The latest snapshot read failed.</summary>
    public bool ReadFailed { get; private set; }

    public bool Busy { get; private set; }

    /// <summary>The tool whose save was refused, and the managed words' key for the line.</summary>
    public (string Tool, string Key)? Refusal { get; private set; }

    /// <summary>The rows to draw: supported tools the snapshot has options for.</summary>
    public IReadOnlyList<string> Rows()
    {
        var rows = new List<string>();
        if (Snapshot?.CliOptions is { } options)
        {
            foreach (string tool in SupportedTools)
            {
                if (options.ContainsKey(tool))
                {
                    rows.Add(tool);
                }
            }
        }

        return rows;
    }

    public bool CanSave(string tool) => !Busy && Drafts.IsChanged(tool, Snapshot?.CliOptions);

    public async Task ReadAsync()
    {
        try
        {
            DaemonResponse response = await _call("managed_snapshot", "{}").ConfigureAwait(true);
            ManagedSnapshot? snapshot = !response.IsError && response.Result is { } result
                ? ManagedSnapshot.Parse(result.GetRawText())
                : null;
            ReadFailed = snapshot is null;
            if (snapshot is not null)
            {
                Snapshot = snapshot;
            }
        }
        catch (Exception)
        {
            ReadFailed = true;
        }
    }

    /// <summary>Saves the field's text for <paramref name="tool"/> when it differs from the saved value.</summary>
    public async Task SaveAsync(string tool)
    {
        if (!CanSave(tool))
        {
            return;
        }

        Busy = true;
        Refusal = null;
        string options = Drafts.Value(tool, Snapshot?.CliOptions);
        string? code = null;
        string? message = null;
        bool confirmed = false;
        ulong revision = 0;
        try
        {
            DaemonResponse response = await _call(
                "managed_cli_options_set",
                JsonSerializer.Serialize(new Dictionary<string, string> { ["tool"] = tool, ["options"] = options })).ConfigureAwait(true);
            if (response.Error is { } error)
            {
                code = error.Code;
                message = error.Message;
            }
            else if (response.Result is { } result
                && DaemonCalls.Text(result, "tool") == tool
                && DaemonCalls.Text(result, "options") == options
                && result.TryGetProperty("revision", out JsonElement r) && r.TryGetUInt64(out revision))
            {
                confirmed = true;
            }
            else
            {
                code = "unavailable";
                message = "managed-cli-options-unconfirmed";
            }
        }
        catch (Exception)
        {
            code = "unavailable";
        }

        Busy = false;
        if (confirmed)
        {
            if (Snapshot?.CliOptions is { } saved && revision >= Snapshot.Revision)
            {
                saved[tool] = options;
            }

            Drafts.Saved(tool, options);
        }
        else
        {
            Refusal = (tool, ErrorKey(code, message));
        }

        await ReadAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// The managed words' key for a refused action (<c>ManagedSurface.errorKey</c>,
    /// ManagedSessionsSurface.swift:371-408), matched on the code or the message.
    /// </summary>
    public static string ErrorKey(string? code, string? message)
    {
        bool Is(string label) => code == label || message == label;
        if (Is("managed-collection-account-busy")) return "collection_account_busy";
        if (Is("managed-cli-options-invalid")) return "cli_options_invalid";
        if (Is("managed-unsupported-version")) return "unsupported_version";
        if (Is("launch-unknown") || Is("managed-launch-unknown")) return "launch_unknown";
        if (Is("managed-near-ai-sign-in-required")) return "sign_in_needed";
        if (Is("managed-terminal-not-found")) return "terminal_not_found";
        if (Is("managed-herdr-socket-untrusted")) return "herdr_socket_untrusted";
        if (Is("managed-rex-not-ready")) return "rex_not_ready";
        if (Is("managed-model-unavailable")) return "model_unavailable";
        if (Is("managed-near-ai-balance-too-low")) return "balance_too_low";
        if (Is("managed-project-chooses-model")) return "project_chooses_model";
        if (Is("managed-project-sets-auto-mode-server")) return "project_sets_auto_mode_server";
        if (Is("red_team_unavailable")) return "red_team_unavailable";
        if (Is("red_team_needs_near_ai")) return "red_team_needs_near_ai";
        return "action_failed";
    }
}

/// <summary>The managed words (<c>tc_managed_copy_json</c>, <c>AppModel.managedText</c>); empty when missing.</summary>
public static class ManagedWords
{
    private static readonly Lazy<string?> Json = new(() =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_managed_copy_json()));

    public static string Text(string key) => Shell.SettingsNavWords.Field(Json.Value, key) ?? string.Empty;
}
