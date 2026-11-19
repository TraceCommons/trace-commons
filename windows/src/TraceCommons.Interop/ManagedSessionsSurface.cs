using System;
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
/// <param name="TerminalChosen">Whether the person has chosen the terminal; null
/// from an older daemon, which reads as never asking.</param>
/// <param name="TerminalDestination">The chosen terminal's name.</param>
/// <param name="LaunchDestination">Where a session opens: herdr while sessions
/// are grouped there, else the chosen terminal. Absent from an older
/// daemon.</param>
public sealed record ManagedCapabilities(
    [property: JsonPropertyName("managed_launch")] bool ManagedLaunch,
    [property: JsonPropertyName("terminal_launch")] bool TerminalLaunch,
    [property: JsonPropertyName("terminal_destination")] string? TerminalDestination,
    [property: JsonPropertyName("near_ai_signin")] bool NearAiSignin = false,
    [property: JsonPropertyName("terminal_current")] string? TerminalCurrent = null,
    [property: JsonPropertyName("terminal_choices")] ManagedTerminalChoice[]? TerminalChoices = null,
    [property: JsonPropertyName("launch_destination")] string? LaunchDestination = null,
    [property: JsonPropertyName("terminal_chosen")] bool? TerminalChosen = null)
{
    /// <summary>Where a session opens, for the launch lines and a launch's
    /// refusal: <see cref="LaunchDestination"/>, or the chosen terminal from
    /// an older daemon (owner, 2026-11-03).</summary>
    public string? SessionDestination => LaunchDestination ?? TerminalDestination;
}

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

    /// <summary>Whether a launch dialog asks which terminal to open in: on
    /// every launch, whether or not one was chosen before, and only when more
    /// than one is installed. An older daemon that does not say never asks
    /// (#1891, as macOS <c>asksForTerminal</c>).</summary>
    [JsonIgnore] public bool AsksForTerminal =>
        Capabilities.TerminalChosen is not null && TerminalChoices.Length > 1;

    /// <summary>The position a launch dialog that asks starts on: the current
    /// terminal when it is installed, else the first installed; -1 with none.</summary>
    [JsonIgnore] public int ProposedTerminalIndex =>
        TerminalSelectedIndex >= 0 ? TerminalSelectedIndex : (TerminalChoices.Length > 0 ? 0 : -1);

    /// <summary>The terminal a launch saves as the default before it starts:
    /// the one picked in a dialog that asked, else null.</summary>
    public string? TerminalToSave(int pickedIndex) =>
        AsksForTerminal && pickedIndex >= 0 && pickedIndex < TerminalChoices.Length
            ? TerminalChoices[pickedIndex].Id : null;

    /// <summary>The tool's NEAR AI sign-in entry, only while signed in.</summary>
    public ManagedAccount? SignInEntry(string tool) => NearAiSignedIn && CanLaunch(tool)
        ? System.Array.Find(SignInAccounts ?? [], a => a.Tool == tool && a.Connection == "near_ai_signin")
        : null;

    /// <summary>
    /// The tools the sign-in card offers a Start button for, in order: each
    /// with a sign-in entry that can launch. OpenCode only while the daemon's
    /// <c>tools</c> row says it is installed; an older daemon that sends no
    /// <c>tools</c> offers no OpenCode button (fail closed). Then Red Team,
    /// when its entry can launch (ManagedSessionsSurface.swift:1082-1098).
    /// </summary>
    [JsonIgnore] public string[] SignInStartTools
    {
        get
        {
            var list = ManagedSignIn.StartTools.Where(tool =>
                SignInEntry(tool) is not null && CanLaunch(tool)
                && (tool != "opencode" || Tools?.FirstOrDefault(t => t.Tool == tool)?.Installed == true)).ToList();
            if (SignInEntry("red_team") is not null && CanLaunch("red_team")) list.Add("red_team");
            return list.ToArray();
        }
    }

    /// <summary>
    /// The <c>managed_launch_prepare</c> request that starts <paramref name="tool"/>
    /// on the NEAR AI sign-in in <paramref name="cwd"/>; null unless the card
    /// offers that tool's Start button. A null <paramref name="cwd"/> names no
    /// folder, and the daemon starts the session in the person's home folder
    /// (owner, 2026-10-29; ManagedSessionsSurface.swift:1100-1114). A folder
    /// that is not fully qualified is refused here (null), never sent.
    /// </summary>
    public Dictionary<string, object>? SignInLaunchParams(string tool, string? cwd, string requestId)
    {
        if (!SignInStartTools.Contains(tool) || SignInEntry(tool) is not { } entry) return null;
        if (cwd is not null && !ManagedLaunchFolder.IsAcceptable(cwd)) return null;
        var request = new Dictionary<string, object>
        {
            ["request_id"] = requestId, ["purpose"] = "coding", ["tool"] = entry.Tool,
            ["connection"] = entry.Connection, ["account_id"] = entry.Id,
            ["expected_generation"] = Generations.GetValueOrDefault(entry.Tool), ["save_default"] = false,
        };
        if (cwd is not null) request["cwd"] = cwd;
        return request;
    }

    /// <summary>
    /// The <c>managed_launch_prepare</c> request the saved-account Launch
    /// dialog sends: <paramref name="account"/> in <paramref name="cwd"/>,
    /// decided at send time. Null when the folder is not fully qualified, or
    /// for OpenCode without a model (checklist L4, the sibling of
    /// <see cref="SignInLaunchParams"/>). The wire shape is the one the
    /// dialog sent before: <c>model</c> null and no collection for any other
    /// tool.
    /// </summary>
    public Dictionary<string, object?>? AccountLaunchParams(
        ManagedAccount account, string? cwd, string? model, bool collectTraces, string requestId)
    {
        if (!ManagedLaunchFolder.IsAcceptable(cwd)) return null;
        bool openCode = account.Tool == "opencode";
        string? trimmed = openCode ? model?.Trim() : null;
        if (openCode && string.IsNullOrEmpty(trimmed)) return null;
        return new Dictionary<string, object?>
        {
            ["request_id"] = requestId, ["purpose"] = "coding", ["tool"] = account.Tool,
            ["connection"] = account.Connection, ["account_id"] = account.Id, ["cwd"] = cwd,
            ["expected_generation"] = Generations.GetValueOrDefault(account.Tool), ["save_default"] = false,
            ["model"] = trimmed, ["collect_traces"] = openCode && collectTraces,
        };
    }

    /// <summary>What the sign-in card holds at its foot (ManagedSessionsView.swift:119-129).</summary>
    [JsonIgnore] public ManagedToolArea ToolArea =>
        !Capabilities.TerminalLaunch ? ManagedToolArea.NoTerminal
        : NeedsToolInstallation ? ManagedToolArea.Install
        : ManagedToolArea.Buttons;

    /// <summary>
    /// Whether no tool this shell launches is installed: the daemon sends a
    /// <c>tools</c> row and none of the launched tools in it is installed
    /// (<c>needsToolInstallation</c>, ManagedSessionsSurface.swift:1004-1007).
    /// An older daemon that sends no row never asks for an install.
    /// </summary>
    [JsonIgnore] public bool NeedsToolInstallation =>
        Tools is not null && !Tools.Any(t => LaunchedTools.Contains(t.Tool) && t.Installed);

    /// <summary>
    /// The launched tools the daemon reports as not installed, in the
    /// shell's order: the Install links beside the Start buttons, and the
    /// signed-out install row (<c>missingInstallTools</c>,
    /// ManagedSessionsSurface.swift:990-994).
    /// </summary>
    [JsonIgnore] public ManagedTool[] MissingInstallTools =>
        LaunchedTools.Select(tool => Tools?.FirstOrDefault(t => t.Tool == tool && !t.Installed))
            .OfType<ManagedTool>().ToArray();

    public static ManagedSnapshot? Parse(string json) => JsonSerializer.Deserialize<ManagedSnapshot>(json,
        new JsonSerializerOptions { PropertyNameCaseInsensitive = true });
}

/// <summary>Copy keys for the NEAR AI sign-in launch. Every word is in the
/// shared table (<c>managed/copy.rs</c>); this only picks the key.</summary>
public static class ManagedSignIn
{
    /// <summary>The tools a sign-in session can start, in the order shown.</summary>
    public static readonly string[] Tools = ["claude", "codex"];

    /// <summary>The tools a sign-in session can start, in the order the
    /// card shows their buttons (<c>ManagedSurface.signInTools</c>,
    /// ManagedSessionsSurface.swift:323); <see cref="ManagedSnapshot.SignInStartTools"/>
    /// filters them.</summary>
    public static readonly string[] StartTools = ["claude", "codex", "opencode"];

    /// <summary>The Start button's key, or null for a tool this shell does
    /// not know (ManagedSessionsSurface.swift:327-329).</summary>
    public static string? StartKey(string tool) =>
        tool is "claude" or "codex" or "opencode" or "red_team" ? "start_" + tool : null;

    /// <summary>The launch dialog's Open button: "Open {tool}" with the
    /// core's tool name (ManagedFolderExplainer.swift:51-55).</summary>
    public static string OpenLabel(string template, string toolName) => template.Replace("{tool}", toolName);

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
    /// line when the chosen terminal was not found, the add-funds line
    /// when the sign-in's balance cannot pay for a session's first calls,
    /// and the line that says why when a NEAR AI project chooses a
    /// model.</summary>
    public static string ErrorKey(string? code) => code switch
    {
        "managed-near-ai-sign-in-required" => "sign_in_needed",
        "managed-terminal-not-found" => "terminal_not_found",
        "managed-near-ai-balance-too-low" => "balance_too_low",
        "managed-project-chooses-model" => "project_chooses_model",
        "managed-project-sets-auto-mode-server" => "project_sets_auto_mode_server",
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

/// <summary>What the sign-in card's foot holds (ManagedSessionsView.swift:119-123).</summary>
public enum ManagedToolArea
{
    Buttons,
    NoTerminal,
    Install,
}

/// <summary>
/// The folder a launch names. A launch decides from the folder at send time
/// and sends only a fully qualified path: never drive-relative
/// (<c>C:proj</c>), rooted without a drive (<c>\proj</c>) or relative, which
/// the terminal's own current folder would resolve (checklist L4).
/// </summary>
public static class ManagedLaunchFolder
{
    public static bool IsAcceptable(string? folder) =>
        !string.IsNullOrWhiteSpace(folder) && folder.Trim() == folder && IsFullyQualified(folder);

    /// <summary><c>Path.IsPathFullyQualified</c> under Windows rules on every
    /// host, so the rule is tested on the Mac too.</summary>
    private static bool IsFullyQualified(string path)
    {
        if (path.Length >= 3 && char.IsAsciiLetter(path[0]) && path[1] == ':' && path[2] is '\\' or '/') return true;
        if (path.Length >= 3 && path[0] is '\\' or '/' && path[1] is '\\' or '/' && path[2] is not ('\\' or '/')) return true;
        return false;
    }
}

/// <summary>
/// A managed action's failure, and where it is said: under the NEAR AI
/// card's Start buttons for a launch begun there, as the Mac's
/// <c>failureOnSignInCard</c> (ManagedSessionsView.swift:131-136), and under
/// the sessions otherwise. The next action clears it (checklist L3).
/// </summary>
public sealed class ManagedFailure
{
    /// <summary>The core key of the line that says why, or null with no failure.</summary>
    public string? Key { get; private set; }

    /// <summary>Whether the failure came from a launch begun on the card.</summary>
    public bool FromCard { get; private set; }

    /// <summary>An action begins: the last failure is cleared.</summary>
    public void Begin()
    {
        Key = null;
        FromCard = false;
    }

    /// <summary>The action was refused with <paramref name="key"/>.</summary>
    public void Refuse(string key, bool fromCard)
    {
        Key = key;
        FromCard = fromCard;
    }

    /// <summary>Whether the card says it: a card launch's failure while the card is drawn.</summary>
    public bool OnCard(bool cardShown) => Key is not null && FromCard && cardShown;

    /// <summary>
    /// The card's lines (ManagedSessionsView.swift:215-228): "That didn't
    /// work.", then the line that says why unless that is the same line,
    /// with <c>{terminal}</c> filled. Empty with no failure.
    /// </summary>
    public string[] CardLines(Func<string, string> text, string? terminal)
    {
        if (Key is null)
        {
            return [];
        }

        string failed = text("action_failed");
        return Key == "action_failed"
            ? [failed]
            : [failed, ManagedSignIn.FillTerminal(text(Key), terminal, text("terminal"))];
    }
}

/// <summary>
/// Keeps keyboard focus on the same button across a rebuild of the managed
/// sessions view: a button is known by its name and its place among buttons
/// of that name, since every read rebuilds every button.
/// </summary>
public static class ManagedFocus
{
    /// <summary>The key of the button at <paramref name="focused"/> among
    /// <paramref name="names"/> (every button's name, in order); null when
    /// no button there has focus.</summary>
    public static (string Name, int Index)? KeyOf(IReadOnlyList<string> names, int focused)
    {
        if (focused < 0 || focused >= names.Count)
        {
            return null;
        }

        string name = names[focused];
        int index = 0;
        for (int i = 0; i < focused; i++)
        {
            if (names[i] == name)
            {
                index++;
            }
        }

        return (name, index);
    }

    /// <summary>The position of the button <paramref name="key"/> names
    /// among <paramref name="names"/>, or -1 when it is gone.</summary>
    public static int Find(IReadOnlyList<string> names, (string Name, int Index) key)
    {
        int seen = 0;
        for (int i = 0; i < names.Count; i++)
        {
            if (names[i] == key.Name && seen++ == key.Index)
            {
                return i;
            }
        }

        return -1;
    }

    /// <summary>
    /// Which button a rebuild focuses, and the return kept for the next one.
    /// The focused button keeps focus. While a dialog is open
    /// (<paramref name="busy"/>) the pressed button's return is kept for
    /// when it closes; once it has closed, focus goes back to that button
    /// (box, 2026-11-19: it had dropped to the Settings gear).
    /// </summary>
    public static ((string Name, int Index)? Target, (string Name, int Index)? Keep) Rebuild(
        (string Name, int Index)? focused, bool busy, (string Name, int Index)? returnTo) =>
        busy ? (focused, returnTo) : (focused ?? returnTo, null);
}
