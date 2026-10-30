using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

public sealed record ManagedAccount(
    string Id, string Tool, string Connection, string Label,
    [property: JsonPropertyName("auth_state")] string AuthState);

public sealed record ManagedSession(
    string Id, string Tool, string Connection,
    [property: JsonPropertyName("account_id")] string AccountId,
    [property: JsonPropertyName("account_label")] string AccountLabel,
    [property: JsonPropertyName("project_label")] string ProjectLabel,
    string State, string Purpose, string? Model = null,
    [property: JsonPropertyName("collect_traces")] bool CollectTraces = false,
    string Collection = "off")
{
    [JsonIgnore] public bool HoldsAccount => State != "exited" && State != "failed";
}

/// <summary>A terminal a managed session can open in, as the daemon names
/// it: its wire id and the app's own name.</summary>
public sealed record ManagedTerminalChoice(string Id, string Name);
public sealed record ManagedTool(string Tool, bool Installed,
    [property: JsonPropertyName("install_url")] string InstallUrl);

/// <param name="NearAiSignin">Whether the daemon holds a NEAR AI sign-in: a
/// bool, never the credential. Absent from an older daemon, which reads as no.</param>
/// <param name="TerminalCurrent">The chosen terminal's id; null from an older
/// daemon, or for a stored choice the daemon does not know.</param>
/// <param name="TerminalChoices">The terminals the daemon found on this
/// computer; absent from an older daemon.</param>
public sealed record ManagedCapabilities(
    [property: JsonPropertyName("managed_launch")] bool ManagedLaunch,
    [property: JsonPropertyName("terminal_launch")] bool TerminalLaunch,
    [property: JsonPropertyName("terminal_destination")] string? TerminalDestination,
    [property: JsonPropertyName("near_ai_signin")] bool NearAiSignin = false,
    [property: JsonPropertyName("terminal_current")] string? TerminalCurrent = null,
    [property: JsonPropertyName("terminal_choices")] ManagedTerminalChoice[]? TerminalChoices = null);

/// <param name="SignInAccounts">One entry per tool, derived from the NEAR AI
/// sign-in and never saved. Empty, or absent from an older daemon, without one.</param>
public sealed record ManagedSnapshot(
    ulong Revision, ManagedAccount[] Accounts, ManagedSession[] Sessions,
    Dictionary<string, ulong> Generations, ManagedCapabilities Capabilities,
    Dictionary<string, string> Copy,
    [property: JsonPropertyName("sign_in_accounts")] ManagedAccount[]? SignInAccounts = null, ManagedTool[]? Tools = null)
{
    /// <summary>The tools this shell launches, in the order the Add and
    /// Launch dialogs offer them. OpenCode is one since #1418.</summary>
    public static readonly string[] LaunchedTools = ["claude", "codex", "opencode"];
    // Older snapshots retain every launched tool. New inventories
    // must positively report an installed managed tool.
    public bool CanLaunch(string tool) => LaunchedTools.Contains(tool) &&
        (Tools is null || Tools.Any(t => t.Tool == tool && t.Installed));
    public string[] SetupTools => LaunchedTools.Where(CanLaunch).ToArray();
    /// <summary>The tools the install block offers: every tool this shell
    /// launches, and only while none is installed (Ron, 2026-10-15).</summary>
    public ManagedTool[] InstallTools => Tools is null || SetupTools.Length > 0 ? []
        : Tools.Where(t => LaunchedTools.Contains(t.Tool)).ToArray();
    /// <summary>Whether the caption and Install belong on an account row: the
    /// inventory reports the tool missing. Keys on installation, not on
    /// launchability, so an installed OpenCode account is not "missing".</summary>
    public bool ToolMissing(string tool) =>
        Tools is not null && !Tools.Any(t => t.Tool == tool && t.Installed);
    /// <summary>Reconnect is hidden, not disabled, while the tool is missing.
    /// OpenCode has no subscription, so it never offers one.</summary>
    public bool OffersReconnect(ManagedAccount account) =>
        account.Connection == "subscription" && ManagedSignIn.Tools.Contains(account.Tool) && CanLaunch(account.Tool);
    /// <summary><c>tool_unavailable</c> with <c>{tool}</c> filled with the tool's name.</summary>
    public string ToolUnavailable(string tool) => Text("tool_unavailable").Replace("{tool}", Text(tool));
    /// <summary>The tools to offer an update for after an unsupported-version
    /// refusal: the attempted tool, or every installed one for an old payload.</summary>
    public ManagedTool[] VersionRecoveryTools(string? attempted) => (Tools ?? []).Where(t => t.Installed && (attempted is null || t.Tool == attempted)).ToArray();
    /// <summary>Connections offered when adding an account. OpenCode has no
    /// subscription and no NEAR AI sign-in entry, so it keeps its NEAR AI key;
    /// Claude Code and Codex follow <see cref="ManagedSignIn.ConnectionOptions"/>.</summary>
    public static string[] Connections(string tool, bool signedIn = false) =>
        tool == "opencode" ? ["api_key", "near_ai"] : ManagedSignIn.ConnectionOptions(signedIn);
    public string Text(string key) => Copy.GetValueOrDefault(key, string.Empty);

    /// <summary>Whether the daemon holds a NEAR AI sign-in.</summary>
    [JsonIgnore] public bool NearAiSignedIn => Capabilities.NearAiSignin;

    /// <summary>The terminals the Open sessions in picker offers: the ones
    /// the daemon found, none from an older daemon.</summary>
    [JsonIgnore] public ManagedTerminalChoice[] TerminalChoices => Capabilities.TerminalChoices ?? [];

    /// <summary>The position of the current choice among
    /// <see cref="TerminalChoices"/>, or -1 when it is not among them, so a
    /// choice no longer found selects nothing rather than the first.</summary>
    [JsonIgnore] public int TerminalSelectedIndex =>
        System.Array.FindIndex(TerminalChoices, c => c.Id == Capabilities.TerminalCurrent);

    /// <summary>The tool's NEAR AI sign-in entry, only while signed in.</summary>
    public ManagedAccount? SignInEntry(string tool) => NearAiSignedIn && CanLaunch(tool)
        ? System.Array.Find(SignInAccounts ?? [], a => a.Tool == tool && a.Connection == "near_ai_signin")
        : null;

    /// <summary>
    /// The <c>managed_launch_prepare</c> request that starts <paramref name="tool"/>
    /// on the NEAR AI sign-in in <paramref name="cwd"/>; null without an entry for it.
    /// </summary>
    public Dictionary<string, object>? SignInLaunchParams(string tool, string cwd, string requestId) =>
        SignInEntry(tool) is { } entry
            ? new()
            {
                ["request_id"] = requestId, ["purpose"] = "coding", ["tool"] = entry.Tool,
                ["connection"] = entry.Connection, ["account_id"] = entry.Id, ["cwd"] = cwd,
                ["expected_generation"] = Generations.GetValueOrDefault(entry.Tool), ["save_default"] = false,
            }
            : null;
    public static ManagedSnapshot? Parse(string json) => JsonSerializer.Deserialize<ManagedSnapshot>(json,
        new JsonSerializerOptions { PropertyNameCaseInsensitive = true });
}

/// <summary>Copy keys for the NEAR AI sign-in launch. Every word is in the
/// shared table (<c>managed/copy.rs</c>); this only picks the key.</summary>
public static class ManagedSignIn
{
    /// <summary>The tools a sign-in session can start, in the order shown.</summary>
    public static readonly string[] Tools = ["claude", "codex"];

    /// <summary>The Start button's key, or null for a tool this shell does not know.</summary>
    public static string? StartKey(string tool) => System.Array.IndexOf(Tools, tool) >= 0 ? "start_" + tool : null;

    /// <summary>
    /// The Add dialog's sign-in types. Signed in, NEAR AI is not offered: the
    /// sign-in entries already run on it, with no key. Signed out, a pasted
    /// key stays the fallback.
    /// </summary>
    public static string[] ConnectionOptions(bool signedIn) =>
        signedIn ? ["subscription", "api_key"] : ["subscription", "api_key", "near_ai"];

    /// <summary>
    /// The sign-in line with <c>{destination}</c> filled from the terminal the
    /// daemon reports, as the launch line does; <paramref name="fallback"/>
    /// (the core's "your terminal") when it reports none.
    /// </summary>
    public static string Scope(string template, string? destination, string fallback) =>
        template.Replace("{destination}", destination ?? fallback);

    /// <summary>The line for a failed managed action: the sign-in prompt
    /// when the daemon refused for want of a NEAR AI sign-in, the not-found
    /// line when the chosen terminal was not found, and the add-funds line
    /// when the sign-in's balance cannot pay for a session's first calls.</summary>
    public static string ErrorKey(string? code) => code switch
    {
        "managed-near-ai-sign-in-required" => "sign_in_needed",
        "managed-terminal-not-found" => "terminal_not_found",
        "managed-near-ai-balance-too-low" => "balance_too_low",
        _ => "action_failed",
    };

    /// <summary>
    /// A line with <c>{terminal}</c> filled from the terminal the daemon
    /// reports; <paramref name="fallback"/> (the core's "your terminal") when
    /// it reports none.
    /// </summary>
    public static string FillTerminal(string template, string? terminal, string fallback) =>
        template.Replace("{terminal}", terminal ?? fallback);
}
