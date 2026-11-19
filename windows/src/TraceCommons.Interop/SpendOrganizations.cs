using System;
using System.Collections.Generic;
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

/// <summary>
/// The Private AI screen's prompt to choose a spending organization (spec
/// W19-D2 (a), <c>SpendOrganizationPrompt</c>, SpendOrganizationPrompt.swift,
/// with AppModel.swift:1064-1114). Drawn only when the daemon asks. Its action
/// opens Settings; "Not now" records the organization already in use, which
/// mints nothing, and the prompt leaves only on a confirmed reply.
/// </summary>
public sealed class SpendOrganizationPromptModel : INotifyPropertyChanged
{
    public const string ListMethod = "near_ai_spend_organizations";
    public const string SelectMethod = "near_ai_spend_organization_select";

    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly Func<long> _connection;
    private long _reads;
    private bool _dismissing;

    public SpendOrganizationPromptModel(Func<string, string, Task<DaemonResponse>> call, Func<long> connection)
    {
        _call = call;
        _connection = connection;
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public SpendOrganizations Status { get; private set; } = SpendOrganizations.Unreported;

    public bool IsShown => Status.ShowsPrompt;

    public string Title => Status.View?.PromptTitle ?? string.Empty;

    public string Line => Status.View?.PromptLine ?? string.Empty;

    public string Action => Status.View?.PromptAction ?? string.Empty;

    public string ActionName => Status.View?.PromptActionAccessibility ?? string.Empty;

    public string Dismiss => Status.View?.PromptDismiss ?? string.Empty;

    public string DismissName => Status.View?.PromptDismissAccessibility ?? string.Empty;

    /// <summary>Reads the listing; a failed read is unreported and draws nothing.</summary>
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
    /// "Not now": select the organization already current. A refusal keeps the
    /// prompt and says nothing; the fresh re-read shows the truth.
    /// </summary>
    public async Task DismissAsync()
    {
        if (_dismissing || Status.CurrentOrganizationId is not { } current) return;
        _dismissing = true;
        long connection = _connection();
        bool confirmed;
        try
        {
            DaemonResponse response = await _call(SelectMethod, JsonSerializer.Serialize(new Dictionary<string, string> { ["organization_id"] = current }))
                .ConfigureAwait(true);
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

    private void Publish(SpendOrganizations status)
    {
        Status = status;
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(string.Empty));
    }
}
