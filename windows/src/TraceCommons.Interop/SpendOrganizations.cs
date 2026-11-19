using System;
using System.Collections.Generic;
using System.Linq;
using System.ComponentModel;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// <c>near_ai_spend_organizations</c>, as the Mac reads it
/// (<c>SpendOrganizations</c>, macos/Sources/TCShellCore/SpendOrganizationSurface.swift).
/// Anything unreadable is <see cref="Unreported"/>, never a guess.
/// </summary>
public sealed record SpendOrganizations(
    string State,
    IReadOnlyList<SpendOrganizations.Organization> Organizations,
    string? CurrentOrganizationId,
    bool Explicit,
    bool Prompt,
    bool RevokePending,
    SpendOrganizations.Words? View)
{
    public sealed record Organization(string Id, string Name, bool Current);

    /// <summary>The daemon's words; all present or the surface draws nothing.</summary>
    public sealed record Words(
        string Title, string What, string Picker, string Switching, string Failed, string SessionExpired,
        string Refused, string RevokePending, string PromptTitle, string PromptLine, string PromptAction,
        string PromptActionAccessibility, string PromptDismiss, string PromptDismissAccessibility);

    public const string StateListed = "listed";

    public static readonly SpendOrganizations Unreported = new(string.Empty, Array.Empty<Organization>(), null, false, false, false, null);

    /// <summary>Settings' picker: a complete listing of two or more, a current one, and the words.</summary>
    public bool ShowsPicker => State == StateListed && Organizations.Count >= 2 && CurrentOrganizationId is not null && View is not null;

    /// <summary>The Private AI screen's prompt: the daemon asks, and there is a picker to send the reader to.</summary>
    public bool ShowsPrompt => ShowsPicker && Prompt;

    /// <summary>This answer with the prompt ended, once a choice was accepted and before the re-read.</summary>
    public SpendOrganizations WithPromptEnded() => this with { Prompt = false };

    /// <summary>A refused switch's sentence by the daemon's label; null for anything that is not a refusal.</summary>
    public string? FailureLine(string label)
    {
        if (View is not { } words || !label.StartsWith("near_ai_spend_org_", StringComparison.Ordinal)) return null;
        return label switch
        {
            "near_ai_spend_org_session_expired" or "near_ai_spend_org_no_session" => words.SessionExpired,
            "near_ai_spend_org_refused" => words.Refused,
            _ => words.Failed,
        };
    }

    public static SpendOrganizations Parse(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object || !root.TryGetProperty("state", out JsonElement state)
            || state.ValueKind != JsonValueKind.String)
        {
            return Unreported;
        }

        var organizations = new List<Organization>();
        if (root.TryGetProperty("organizations", out JsonElement rows) && rows.ValueKind == JsonValueKind.Array)
        {
            foreach (JsonElement row in rows.EnumerateArray())
            {
                if (Text(row, "id") is not { } id || Text(row, "name") is not { } name) return Unreported;
                organizations.Add(new Organization(id, name, Flag(row, "current")));
            }
        }

        string? current = root.TryGetProperty("current_organization_id", out JsonElement c) && c.ValueKind == JsonValueKind.String ? c.GetString() : null;
        Words? words = root.TryGetProperty("view", out JsonElement view) && view.ValueKind == JsonValueKind.Object ? ParseWords(view) : null;
        return new SpendOrganizations(state.GetString()!, organizations, current, Flag(root, "explicit"), Flag(root, "prompt"),
            Flag(root, "revoke_pending"), words);
    }

    private static Words? ParseWords(JsonElement view)
    {
        string? T(string key) => Text(view, key);
        // `what` may arrive empty (owner, 2026-11-04); every other word is required.
        string? what = view.TryGetProperty("what", out JsonElement w) && w.ValueKind == JsonValueKind.String ? w.GetString() : null;
        if (T("title") is not { } title || what is null || T("picker") is not { } picker || T("switching") is not { } switching
            || T("failed") is not { } failed || T("session_expired") is not { } expired || T("refused") is not { } refused
            || T("revoke_pending") is not { } revoke || T("prompt_title") is not { } promptTitle
            || T("prompt_line") is not { } promptLine || T("prompt_action") is not { } promptAction
            || T("prompt_action_accessibility") is not { } promptActionName || T("prompt_dismiss") is not { } promptDismiss
            || T("prompt_dismiss_accessibility") is not { } promptDismissName)
        {
            return null;
        }

        return new Words(title, what, picker, switching, failed, expired, refused, revoke, promptTitle, promptLine,
            promptAction, promptActionName, promptDismiss, promptDismissName);
    }

    private static string? Text(JsonElement o, string key) =>
        o.ValueKind == JsonValueKind.Object && o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.String
        && v.GetString() is { Length: > 0 } s ? s : null;

    private static bool Flag(JsonElement o, string key) =>
        o.ValueKind == JsonValueKind.Object && o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.True;
}

/// <summary>The tone of the line under the picker (<c>GlassNotice</c> tones the Mac's card uses).</summary>
public enum SpendOrganizationTone
{
    Ask,
    Outside,
}

/// <summary>One line under the picker, at most (<c>SpendOrganizationCard.Line</c>).</summary>
public sealed record SpendOrganizationLine(string Text, SpendOrganizationTone Tone);

/// <summary>
/// Which organization Private AI spends from: the Private AI screen's prompt
/// (spec W19-D2 (a), SpendOrganizationPrompt.swift) and the Settings picker
/// it leads to (SpendOrganizationCard.swift), over one state, as the Mac's
/// <c>AppModel</c> holds it (AppModel.swift:1043-1115). The window owns one,
/// so a choice made in Settings ends the prompt on the tab.
/// </summary>
/// <remarks>
/// Nothing is chosen for the contributor: reading or re-reading the listing
/// sends nothing, and the picker's echo of the organization already current
/// is not a pick. A switch is sent only for a different organization the
/// contributor picked, or by "Not now", which records the one in use and
/// mints nothing. Every word is the daemon's <c>view</c>.
/// </remarks>
public sealed class SpendOrganizationModel : INotifyPropertyChanged
{
    public const string ListMethod = "near_ai_spend_organizations";
    public const string SelectMethod = "near_ai_spend_organization_select";

    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly Func<long> _connection;
    private long _reads;
    private bool _dismissing;

    /// <param name="credential">
    /// The sign-in or credential action in flight, shared with the credential
    /// card; a pick and Not now wait for it, as the Mac's do
    /// (AppModel.swift:1076 and :1104).
    /// </param>
    public SpendOrganizationModel(
        Func<string, string, Task<DaemonResponse>> call, Func<long> connection, CredentialActivity credential)
    {
        _call = call;
        _connection = connection;
        Credential = credential ?? throw new ArgumentNullException(nameof(credential));
        Credential.Changed += _ => Raise();
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>
    /// A switch settled, refused or not: the key may have moved, so the
    /// balance is read afresh (AppModel.swift:1093). Not now raises nothing.
    /// </summary>
    public event Action? SwitchSettled;

    public CredentialActivity Credential { get; }

    public SpendOrganizations Status { get; private set; } = SpendOrganizations.Unreported;

    // ---- The prompt ----

    public bool IsShown => Status.ShowsPrompt;

    public string Title => Status.View?.PromptTitle ?? string.Empty;

    public string Line => Status.View?.PromptLine ?? string.Empty;

    public string Action => Status.View?.PromptAction ?? string.Empty;

    public string ActionName => Status.View?.PromptActionAccessibility ?? string.Empty;

    public string Dismiss => Status.View?.PromptDismiss ?? string.Empty;

    public string DismissName => Status.View?.PromptDismissAccessibility ?? string.Empty;

    // ---- The picker ----

    /// <summary>Two or more organizations, a current one, and the words.</summary>
    public bool ShowsPicker => Status.ShowsPicker;

    public string PickerTitle => Status.View?.Title ?? string.Empty;

    /// <summary>Cut by the owner 2026-11-04 (P188): an empty line draws nothing.</summary>
    public string What => Status.View?.What ?? string.Empty;

    public bool HasWhat => What.Length > 0;

    /// <summary>The picker's accessible name (<c>GlassSelect(words.picker, ...)</c>).</summary>
    public string PickerName => Status.View?.Picker ?? string.Empty;

    /// <summary>The organizations by name, in the daemon's order; none without a picker.</summary>
    public IReadOnlyList<string> OptionNames =>
        ShowsPicker ? Status.Organizations.Select(o => o.Name).ToArray() : Array.Empty<string>();

    /// <summary>The current organization's row, or -1 when it is not listed.</summary>
    public int CurrentIndex
    {
        get
        {
            for (int i = 0; i < Status.Organizations.Count; i++)
            {
                if (Status.Organizations[i].Id == Status.CurrentOrganizationId) return i;
            }

            return -1;
        }
    }

    /// <summary>A switch is in flight; it takes about ten seconds.</summary>
    public bool Switching { get; private set; }

    /// <summary>The last switch's refusal, in the daemon's words. Every refusal changed nothing.</summary>
    public string? Failure { get; private set; }

    /// <summary>The picker is held while a switch or a credential action is in flight.</summary>
    public bool CanPick => !Switching && !Credential.Busy;

    public SpendOrganizationLine? StateLine => LineFor(Status, Switching, Failure);

    /// <summary>
    /// One line under the picker, at most: switching, else the last refusal,
    /// else a replaced key still being revoked (<c>SpendOrganizationCard.stateLine</c>).
    /// </summary>
    public static SpendOrganizationLine? LineFor(SpendOrganizations status, bool switching, string? failure)
    {
        if (status.View is not { } words) return null;
        if (switching) return new SpendOrganizationLine(words.Switching, SpendOrganizationTone.Ask);
        if (failure is not null) return new SpendOrganizationLine(failure, SpendOrganizationTone.Outside);
        if (status.RevokePending) return new SpendOrganizationLine(words.RevokePending, SpendOrganizationTone.Ask);
        return null;
    }

    /// <summary>Reads the listing; a failed read is unreported and draws nothing. Sends no choice.</summary>
    public async Task RefreshAsync(bool fresh = false)
    {
        long mine = ++_reads;
        long connection = _connection();
        SpendOrganizations status;
        try
        {
            DaemonResponse response = await _call(ListMethod, fresh ? "{\"fresh\":true}" : "{}").ConfigureAwait(true);
            status = response is { IsError: false, Result: { } result } ? SpendOrganizations.Parse(result) : SpendOrganizations.Unreported;
        }
        catch (Exception)
        {
            status = SpendOrganizations.Unreported;
        }

        if (mine != _reads || connection != _connection()) return;
        Publish(status);
    }

    /// <summary>
    /// The picker's row <paramref name="index"/> was chosen. Only a different
    /// organization than the current one is a pick: the picker echoing its
    /// own selection, or a row that is not there, sends nothing.
    /// </summary>
    public Task PickAsync(int index)
    {
        if (!ShowsPicker || index < 0 || index >= Status.Organizations.Count) return Task.CompletedTask;
        string wanted = Status.Organizations[index].Id;
        return wanted == Status.CurrentOrganizationId ? Task.CompletedTask : SelectAsync(wanted);
    }

    /// <summary>
    /// Switches Private AI to <paramref name="organizationId"/>
    /// (<c>selectSpendOrganization</c>, AppModel.swift:1075-1096).
    /// </summary>
    public async Task SelectAsync(string organizationId)
    {
        if (!CanPick) return;
        Switching = true;
        Failure = null;
        Raise();
        long connection = _connection();
        string? label;
        try
        {
            DaemonResponse response = await _call(SelectMethod, Choice(organizationId)).ConfigureAwait(true);
            label = response.IsError ? response.Error!.Message : null;
        }
        catch (Exception)
        {
            // Not the daemon's refusal: the switch may have happened, so no
            // line; the re-read shows which organization is current.
            label = string.Empty;
        }

        Switching = false;
        if (connection != _connection())
        {
            Raise();
            return;
        }

        if (label is null)
        {
            // The choice is recorded, so the prompt ends now, not on the re-read.
            Status = Status.WithPromptEnded();
        }
        else
        {
            Failure = Status.FailureLine(label);
        }

        Raise();
        SwitchSettled?.Invoke();
        await RefreshAsync(fresh: true).ConfigureAwait(true);
    }

    /// <summary>
    /// "Not now": select the organization already current. A refusal keeps the
    /// prompt and says nothing; the fresh re-read shows the truth. Waits for a
    /// switch or a credential action in flight (AppModel.swift:1104).
    /// </summary>
    public async Task DismissAsync()
    {
        if (_dismissing || !CanPick || Status.CurrentOrganizationId is not { } current) return;
        _dismissing = true;
        long connection = _connection();
        bool confirmed;
        try
        {
            DaemonResponse response = await _call(SelectMethod, Choice(current)).ConfigureAwait(true);
            confirmed = !response.IsError;
        }
        catch (Exception)
        {
            confirmed = false;
        }
        finally
        {
            _dismissing = false;
        }

        if (connection != _connection()) return;
        if (confirmed) Publish(Status.WithPromptEnded());
        await RefreshAsync(fresh: true).ConfigureAwait(true);
    }

    private static string Choice(string organizationId) =>
        JsonSerializer.Serialize(new Dictionary<string, string> { ["organization_id"] = organizationId });

    private void Publish(SpendOrganizations status)
    {
        Status = status;
        Raise();
    }

    private void Raise() => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(string.Empty));
}
