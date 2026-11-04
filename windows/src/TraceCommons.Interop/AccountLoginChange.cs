using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

// Changing the NEAR AI login an account uses (F10), ported from the macOS
// shell's AccountLoginChange.swift and AccountLoginChangeModel.swift. The
// daemon runs the ceremony (account_login_change); this file is the shell's
// pure half: the words, the dialog's page for each state, the IPC calls, and
// the driver that opens the browser for each leg and asks until it ends, all
// testable without Windows.

/// <summary>
/// The change words, decoded from what the Rust exported
/// (<c>tc_account_change_copy_json</c>).
/// </summary>
/// <remarks>
/// Every string is owner-approved (2026-11-03; the mismatch body
/// 2026-11-02). None is written in C#: a payload with a missing or blank
/// field is refused whole. Templates carry <c>{provider}</c>,
/// <c>{old_provider}</c> or <c>{new_provider}</c>, a login's title, filled by
/// <see cref="Render"/>.
/// </remarks>
public sealed record AccountChangeCopy
{
    [JsonPropertyName("mismatch_body")] public string? MismatchBody { get; init; }
    [JsonPropertyName("mismatch_sign_in")] public string? MismatchSignIn { get; init; }
    [JsonPropertyName("mismatch_change")] public string? MismatchChange { get; init; }
    [JsonPropertyName("change")] public string? Change { get; init; }
    [JsonPropertyName("payouts_needs_login_body")] public string? PayoutsNeedsLoginBody { get; init; }
    [JsonPropertyName("payouts_needs_login_add")] public string? PayoutsNeedsLoginAdd { get; init; }
    /// <summary>Under "Add a NEAR AI login" when it could not start (DRAFT,
    /// NEEDS APPROVAL (2026-11-04)). Decoded for parity; this shell shows one
    /// start-failed line for both entry buttons, so it is not drawn yet.</summary>
    [JsonPropertyName("payouts_needs_login_add_failed")] public string? PayoutsNeedsLoginAddFailed { get; init; }
    [JsonPropertyName("title")] public string? Title { get; init; }
    [JsonPropertyName("body")] public string? Body { get; init; }
    [JsonPropertyName("body_old_unknown")] public string? BodyOldUnknown { get; init; }
    [JsonPropertyName("body_wallet")] public string? BodyWallet { get; init; }
    [JsonPropertyName("private_ai")] public string? PrivateAi { get; init; }
    [JsonPropertyName("confirm")] public string? Confirm { get; init; }
    [JsonPropertyName("cancel")] public string? Cancel { get; init; }
    [JsonPropertyName("step_current")] public string? StepCurrent { get; init; }
    [JsonPropertyName("step_current_wallet")] public string? StepCurrentWallet { get; init; }
    [JsonPropertyName("step_new")] public string? StepNew { get; init; }
    /// <summary>Steps 1 and 2's buttons, apart from their headings
    /// (copy-guide rule 4). DRAFT, NEEDS APPROVAL (2026-11-04).</summary>
    [JsonPropertyName("step_current_button")] public string? StepCurrentButton { get; init; }
    [JsonPropertyName("step_new_button")] public string? StepNewButton { get; init; }
    [JsonPropertyName("error_current_login")] public string? ErrorCurrentLogin { get; init; }
    [JsonPropertyName("error_wallet")] public string? ErrorWallet { get; init; }
    [JsonPropertyName("error_in_use")] public string? ErrorInUse { get; init; }
    [JsonPropertyName("error_too_soon")] public string? ErrorTooSoon { get; init; }
    [JsonPropertyName("error_unavailable")] public string? ErrorUnavailable { get; init; }
    [JsonPropertyName("error_retired")] public string? ErrorRetired { get; init; }
    [JsonPropertyName("step_current_accessibility_name")] public string? StepCurrentAccessibilityName { get; init; }
    [JsonPropertyName("step_new_accessibility_name")] public string? StepNewAccessibilityName { get; init; }
    [JsonPropertyName("confirm_accessibility_name")] public string? ConfirmAccessibilityName { get; init; }
    [JsonPropertyName("provider_near")] public string? ProviderNear { get; init; }
    [JsonPropertyName("provider_github")] public string? ProviderGithub { get; init; }
    [JsonPropertyName("provider_google")] public string? ProviderGoogle { get; init; }

    /// <summary>
    /// The payload fields this shell decodes, by wire name; compared with
    /// the live export by the interop tests.
    /// </summary>
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "mismatch_body", "mismatch_sign_in", "mismatch_change", "change",
        "payouts_needs_login_body", "payouts_needs_login_add", "payouts_needs_login_add_failed",
        "title", "body", "body_old_unknown", "body_wallet", "private_ai", "confirm", "cancel",
        "step_current", "step_current_wallet", "step_new", "step_current_button",
        "step_new_button", "error_current_login", "error_wallet", "error_in_use",
        "error_too_soon", "error_unavailable", "error_retired", "step_current_accessibility_name",
        "step_new_accessibility_name", "confirm_accessibility_name", "provider_near",
        "provider_github", "provider_google",
    };

    /// <summary>Every field, in <see cref="ConsumedFields"/> order.</summary>
    public IReadOnlyList<string?> Sentences => new[]
    {
        MismatchBody, MismatchSignIn, MismatchChange, Change, PayoutsNeedsLoginBody,
        PayoutsNeedsLoginAdd, PayoutsNeedsLoginAddFailed, Title, Body, BodyOldUnknown,
        BodyWallet, PrivateAi, Confirm, Cancel, StepCurrent, StepCurrentWallet, StepNew,
        StepCurrentButton, StepNewButton, ErrorCurrentLogin, ErrorWallet,
        ErrorInUse, ErrorTooSoon, ErrorUnavailable, ErrorRetired, StepCurrentAccessibilityName,
        StepNewAccessibilityName, ConfirmAccessibilityName, ProviderNear, ProviderGithub,
        ProviderGoogle,
    };

    /// <summary>
    /// Null, never a partly-filled value, for an unparseable payload or a
    /// missing or blank field.
    /// </summary>
    public static AccountChangeCopy? Decode(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            AccountChangeCopy? copy = JsonSerializer.Deserialize<AccountChangeCopy>(json);
            return copy is null || copy.Sentences.Any(string.IsNullOrWhiteSpace) ? null : copy;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>The words through the real ABI, or null when they did not arrive.</summary>
    public static AccountChangeCopy? Copy()
    {
        try
        {
            return Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_account_change_copy_json()));
        }
        catch (Exception e) when (e is DllNotFoundException or EntryPointNotFoundException)
        {
            // A library without the export draws no entry point at all.
            return null;
        }
    }

    /// <summary>
    /// A login's title by its tag (<c>near</c>, <c>github</c>,
    /// <c>google</c>), or null for a tag this build does not know.
    /// </summary>
    public string? ProviderTitle(string? tag) => tag switch
    {
        "near" => ProviderNear,
        "github" => ProviderGithub,
        "google" => ProviderGoogle,
        _ => null,
    };

    /// <summary>
    /// <paramref name="template"/> with its login placeholders filled by
    /// title. Null when the template names a login that is missing or
    /// unknown, so a raw tag or a placeholder is never shown.
    /// </summary>
    public string? Render(string? template, string? provider = null, string? old = null, string? @new = null)
    {
        if (template is null)
        {
            return null;
        }

        string text = template;
        foreach (var (placeholder, tag) in new[]
        {
            ("{provider}", provider), ("{old_provider}", old), ("{new_provider}", @new),
        })
        {
            if (!text.Contains(placeholder, StringComparison.Ordinal))
            {
                continue;
            }

            if (ProviderTitle(tag) is not { } title)
            {
                return null;
            }

            text = text.Replace(placeholder, title, StringComparison.Ordinal);
        }

        return text;
    }
}

/// <summary>
/// The change fields of <c>status</c> (spec 2.4). Absent on an older daemon,
/// which then offers nothing.
/// </summary>
public sealed record AccountLoginChangeStatus
{
    /// <summary>The status keys read here.</summary>
    public static readonly IReadOnlyList<string> StatusKeys = new[]
    {
        "login_change_available", "payouts_need_login", "login_change_held", "login_mismatch",
    };

    public static readonly AccountLoginChangeStatus None = new(false, false, false);

    public AccountLoginChangeStatus(bool available, bool payoutsNeedLogin, bool held, bool mismatch = false)
    {
        Available = available;
        PayoutsNeedLogin = payoutsNeedLogin;
        Held = held;
        Mismatch = mismatch || held;
    }

    /// <summary>This device may offer "Use a different account".</summary>
    public bool Available { get; }

    /// <summary>A wallet-origin account: its payouts need a NEAR AI login.</summary>
    public bool PayoutsNeedLogin { get; }

    /// <summary>
    /// A login refused as another account is held aside: the account
    /// section shows the mismatch block with the change button.
    /// </summary>
    public bool Held { get; }

    /// <summary>
    /// The last NEAR AI sign-in was refused as another account: the account
    /// section shows the approved mismatch body, whether or not the change
    /// is offered (it is offered only with <see cref="Held"/>).
    /// </summary>
    public bool Mismatch { get; }

    /// <summary>Whether the mismatch block offers "Use a different account".</summary>
    public bool OffersChange => Available && Held;

    /// <summary>Whether the payouts line shows "Add a NEAR AI login".</summary>
    public bool ShowsPayoutsLine => Available && PayoutsNeedLogin;

    /// <summary>
    /// Whether the account section draws its own "Use a different account":
    /// offered, and neither the mismatch block nor the payouts line offers
    /// it instead.
    /// </summary>
    public bool ShowsChangeButton => Available && !Held && !PayoutsNeedLogin;

    /// <summary>The four keys out of a status object; absent or not a bool reads false.</summary>
    public static AccountLoginChangeStatus From(JsonElement status)
    {
        if (status.ValueKind != JsonValueKind.Object)
        {
            return None;
        }

        bool Flag(string key) =>
            status.TryGetProperty(key, out JsonElement value) && value.ValueKind == JsonValueKind.True;

        return new AccountLoginChangeStatus(
            Flag(StatusKeys[0]), Flag(StatusKeys[1]), Flag(StatusKeys[2]), Flag(StatusKeys[3]));
    }
}

/// <summary>The ceremony's step, as the daemon names it.</summary>
public enum AccountLoginChangeStep
{
    Current,
    New,
    Confirm,
    Done,
}

/// <summary>One answer of <c>account_login_change</c>: the ceremony as the daemon holds it.</summary>
/// <param name="CurrentKind"><c>wallet</c> or <c>near_ai_login</c>.</param>
/// <param name="Busy">A browser leg or a commons call is running.</param>
/// <param name="Error"><c>current_login</c>, <c>wallet</c>, <c>in_use</c>, <c>too_soon</c> or <c>unavailable</c>.</param>
/// <param name="BrowserUrl">Only on the answer that began a browser leg.</param>
public sealed record AccountLoginChangeState(
    string ChangeId,
    AccountLoginChangeStep Step,
    string CurrentKind,
    string? OldProvider,
    string? NewProvider,
    bool Busy,
    string? Error,
    string? BrowserUrl)
{
    public bool IsWallet => CurrentKind == "wallet";

    /// <summary>Null for an answer without its change id, kind or a step this build knows.</summary>
    public static AccountLoginChangeState? Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            using JsonDocument document = JsonDocument.Parse(json);
            return From(document.RootElement);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    internal static AccountLoginChangeState? From(JsonElement root)
    {
        if (root.ValueKind != JsonValueKind.Object)
        {
            return null;
        }

        string? Text(string key) =>
            root.TryGetProperty(key, out JsonElement value) && value.ValueKind == JsonValueKind.String
                ? value.GetString()
                : null;

        AccountLoginChangeStep? step = Text("step") switch
        {
            "current" => AccountLoginChangeStep.Current,
            "new" => AccountLoginChangeStep.New,
            "confirm" => AccountLoginChangeStep.Confirm,
            "done" => AccountLoginChangeStep.Done,
            _ => null,
        };
        if (Text("change_id") is not { } id || step is not { } known || Text("current_kind") is not { } kind)
        {
            return null;
        }

        bool busy = root.TryGetProperty("busy", out JsonElement b) && b.ValueKind == JsonValueKind.True;
        return new AccountLoginChangeState(
            id, known, kind, Text("old_provider"), Text("new_provider"), busy, Text("error"), Text("browser_url"));
    }
}

/// <summary>What a page's primary button does.</summary>
public enum AccountLoginChangeAction
{
    /// <summary>Step 1: prove the current identity (a browser leg).</summary>
    ProveCurrent,

    /// <summary>Step 2: sign in with the new login (a browser leg).</summary>
    SignInNew,

    /// <summary>Step 3: the person confirms.</summary>
    Confirm,

    /// <summary>Done: close the dialog.</summary>
    Close,
}

/// <summary>
/// What the dialog shows for one state. Pure: every sentence comes from the
/// core's copy, every fact from the daemon.
/// </summary>
public sealed record AccountLoginChangePage(
    string Title,
    string? Heading,
    IReadOnlyList<string> Lines,
    string Primary,
    string PrimaryAccessibilityName,
    AccountLoginChangeAction Action,
    bool AsksWalletAccount,
    string? Error,
    bool Busy,
    string Cancel)
{
    /// <summary>
    /// Whether Enter presses the primary button. Never for Switch: it moves
    /// the account.
    /// </summary>
    public bool IsDefault => Action != AccountLoginChangeAction.Confirm;
}

public static class AccountLoginChangeSurface
{
    /// <summary>
    /// The dialog's page for <paramref name="state"/>. Null only when the
    /// words for this state cannot be drawn (a template names a login nobody
    /// knows), so the shell closes rather than show a placeholder.
    /// </summary>
    public static AccountLoginChangePage? Page(AccountLoginChangeState state, AccountChangeCopy copy)
    {
        ArgumentNullException.ThrowIfNull(state);
        ArgumentNullException.ThrowIfNull(copy);
        string title = copy.Title!;
        string cancel = copy.Cancel!;
        string? error = ErrorLine(state, copy);
        switch (state.Step)
        {
            case AccountLoginChangeStep.Current:
            {
                string[] lines = state.IsWallet ? new[] { copy.StepCurrentWallet! } : Array.Empty<string>();
                string name = state.IsWallet
                    ? copy.StepCurrent!
                    : copy.Render(copy.StepCurrentAccessibilityName, old: state.OldProvider) ?? copy.StepCurrent!;
                return new AccountLoginChangePage(
                    title, copy.StepCurrent, lines, copy.StepCurrentButton!, name,
                    AccountLoginChangeAction.ProveCurrent, state.IsWallet, error, state.Busy, cancel);
            }

            case AccountLoginChangeStep.New:
                return new AccountLoginChangePage(
                    title, copy.StepNew, Array.Empty<string>(), copy.StepNewButton!, copy.StepNewAccessibilityName!,
                    AccountLoginChangeAction.SignInNew, false, error, state.Busy, cancel);

            case AccountLoginChangeStep.Confirm:
            {
                if (ConfirmBody(state, copy) is not { } body
                    || copy.Render(copy.PrivateAi, @new: state.NewProvider) is not { } privateAi
                    || copy.Render(copy.ConfirmAccessibilityName, @new: state.NewProvider) is not { } name)
                {
                    return null;
                }

                return new AccountLoginChangePage(
                    title, null, new[] { body, privateAi }, copy.Confirm!, name,
                    AccountLoginChangeAction.Confirm, false, error, state.Busy, cancel);
            }

            default:
                // No separate confirmation (spec 2.2): the account section
                // says which login is connected. The dialog closes.
                return new AccountLoginChangePage(
                    title, null, Array.Empty<string>(), cancel, cancel,
                    AccountLoginChangeAction.Close, false, null, false, cancel);
        }
    }

    /// <summary>
    /// The approved mismatch body while the last NEAR AI sign-in was refused
    /// as another account, else null. Shown where a NEAR AI sign-in's result
    /// is shown, in place of a generic failure.
    /// </summary>
    public static string? MismatchLine(AccountLoginChangeStatus status, AccountChangeCopy? copy) =>
        status.Mismatch ? copy?.MismatchBody : null;

    /// <summary>
    /// Step 3's body: the wallet body for a wallet-origin account, the body
    /// naming both logins for a NEAR AI account, or the body for an old
    /// login nobody knows.
    /// </summary>
    internal static string? ConfirmBody(AccountLoginChangeState state, AccountChangeCopy copy) =>
        state.IsWallet
            ? copy.Render(copy.BodyWallet, @new: state.NewProvider)
            : copy.Render(copy.Body, old: state.OldProvider, @new: state.NewProvider)
                ?? copy.Render(copy.BodyOldUnknown, @new: state.NewProvider);

    /// <summary>
    /// The daemon's step error as the approved sentence. A current-login
    /// error whose old login is not known has no approved words naming it,
    /// and says the generic line instead.
    /// </summary>
    internal static string? ErrorLine(AccountLoginChangeState state, AccountChangeCopy copy) => state.Error switch
    {
        null => null,
        "current_login" => copy.Render(copy.ErrorCurrentLogin, old: state.OldProvider) ?? copy.ErrorUnavailable,
        "wallet" => copy.ErrorWallet,
        "in_use" => copy.ErrorInUse,
        "too_soon" => copy.ErrorTooSoon,
        _ => copy.ErrorUnavailable,
    };
}

/// <summary>A daemon refusal: its fixed label, safe to route on and never shown.</summary>
public sealed class AccountLoginChangeRefusedException : Exception
{
    public AccountLoginChangeRefusedException(string label)
        : base(label)
    {
        Label = label;
    }

    public string Label { get; }
}

/// <summary>
/// The two IPC methods over the daemon call, so the method names and
/// parameters are asserted without a daemon.
/// </summary>
public sealed class AccountLoginChangeClient
{
    private const string Unparseable = "unparseable-response";

    private readonly Func<string, string, Task<DaemonResponse>> _call;

    public AccountLoginChangeClient(Func<string, string, Task<DaemonResponse>> call)
    {
        _call = call ?? throw new ArgumentNullException(nameof(call));
    }

    public Task<AccountLoginChangeState> StartAsync() =>
        StepAsync(new Dictionary<string, object> { ["step"] = "start" });

    public Task<AccountLoginChangeState> StatusAsync(string changeId) =>
        StepAsync(new Dictionary<string, object> { ["step"] = "status", ["change_id"] = changeId });

    /// <summary>Step 1. A wallet-origin account sends the NEAR account name typed in the dialog.</summary>
    public Task<AccountLoginChangeState> ProveCurrentAsync(string changeId, string? walletAccountId)
    {
        var parameters = new Dictionary<string, object> { ["step"] = "prove_current", ["change_id"] = changeId };
        if (walletAccountId is not null)
        {
            parameters["wallet_account_id"] = walletAccountId;
        }

        return StepAsync(parameters);
    }

    public Task<AccountLoginChangeState> SignInNewAsync(string changeId) =>
        StepAsync(new Dictionary<string, object> { ["step"] = "sign_in_new", ["change_id"] = changeId });

    /// <summary>Step 3: only from the person's Switch.</summary>
    public Task<AccountLoginChangeState> ConfirmAsync(string changeId) =>
        StepAsync(new Dictionary<string, object>
        {
            ["step"] = "confirm", ["change_id"] = changeId, ["confirmed"] = true,
        });

    public async Task CancelAsync(string? changeId)
    {
        var parameters = new Dictionary<string, object>();
        if (changeId is not null)
        {
            parameters["change_id"] = changeId;
        }

        await RawAsync(DaemonProtocol.Methods.AccountLoginChangeCancel, parameters).ConfigureAwait(true);
    }

    private async Task<AccountLoginChangeState> StepAsync(Dictionary<string, object> parameters)
    {
        JsonElement result = await RawAsync(DaemonProtocol.Methods.AccountLoginChange, parameters).ConfigureAwait(true);
        return AccountLoginChangeState.From(result) ?? throw new AccountLoginChangeRefusedException(Unparseable);
    }

    private async Task<JsonElement> RawAsync(string method, Dictionary<string, object> parameters)
    {
        DaemonResponse response = await _call(method, JsonSerializer.Serialize(parameters)).ConfigureAwait(true);
        if (response.Error is { } error)
        {
            throw new AccountLoginChangeRefusedException(error.Message);
        }

        return response.Result ?? throw new AccountLoginChangeRefusedException(Unparseable);
    }
}

/// <summary>
/// The change dialog's state, ported from macOS's
/// <c>AccountLoginChangeModel</c>. The daemon runs the ceremony and every
/// network call; this opens the browser for the two browser legs and asks
/// how they went, about every second, until they end. Every sentence is the
/// core's (<see cref="AccountChangeCopy"/>).
/// </summary>
/// <remarks>
/// One instance serves the three entry points (the mismatch block, the
/// account section's button and the payouts line), so they can never
/// disagree about whether a change is running. Continuations resume on the
/// caller's context, so the app drives it from the UI thread.
/// </remarks>
public sealed class AccountLoginChangeFlow
{
    /// <summary>How often a browser leg is asked about.</summary>
    public static readonly TimeSpan DefaultPollInterval = TimeSpan.FromSeconds(1);

    /// <summary>How long before asking stops (the daemon's own leg ends sooner).</summary>
    public static readonly TimeSpan DefaultPollLimit = TimeSpan.FromMinutes(6);

    private readonly AccountLoginChangeClient _client;
    private readonly Func<Uri, Task<bool>> _open;
    private readonly Func<TimeSpan, CancellationToken, Task> _delay;
    private readonly TimeSpan _interval;
    private readonly int _maxPolls;
    private ulong _generation;
    private string _walletAccount = string.Empty;

    public AccountLoginChangeFlow(
        Func<string, string, Task<DaemonResponse>> call,
        Func<Uri, Task<bool>> open,
        AccountChangeCopy? copy,
        Func<TimeSpan, CancellationToken, Task>? delay = null,
        TimeSpan? interval = null,
        TimeSpan? limit = null)
    {
        _client = new AccountLoginChangeClient(call);
        _open = open ?? throw new ArgumentNullException(nameof(open));
        Copy = copy;
        _delay = delay ?? ((span, token) => Task.Delay(span, token));
        _interval = interval ?? DefaultPollInterval;
        TimeSpan cap = limit ?? DefaultPollLimit;
        _maxPolls = Math.Max(1, (int)Math.Ceiling(cap.TotalMilliseconds / Math.Max(1, _interval.TotalMilliseconds)));
    }

    /// <summary>Anything below changed.</summary>
    public event EventHandler? Changed;

    /// <summary>The change was confirmed and the dialog closed: re-read the account.</summary>
    public event EventHandler? Finished;

    /// <summary>The core's words, null if the export did not arrive (no entry point then draws).</summary>
    public AccountChangeCopy? Copy { get; }

    /// <summary>The ceremony as the daemon last answered it; null while no dialog is up.</summary>
    public AccountLoginChangeState? State { get; private set; }

    public bool Presented { get; private set; }

    /// <summary>A call to the daemon is running.</summary>
    public bool Calling { get; private set; }

    /// <summary>The daemon refused to start a change.</summary>
    public bool StartFailed { get; private set; }

    /// <summary>The generic line under the entry point after a refused start, else null.</summary>
    public string? StartFailedLine => StartFailed ? Copy?.ErrorUnavailable : null;

    /// <summary>The NEAR account name, typed in step 1 of a wallet-origin account.</summary>
    public string WalletAccount
    {
        get => _walletAccount;
        set
        {
            string next = value ?? string.Empty;
            if (next == _walletAccount)
            {
                return;
            }

            _walletAccount = next;
            Raise();
        }
    }

    /// <summary>The dialog's page for the current state.</summary>
    public AccountLoginChangePage? Page =>
        Copy is { } copy && State is { } state ? AccountLoginChangeSurface.Page(state, copy) : null;

    /// <summary>A browser leg or a daemon call is running.</summary>
    public bool Working => Calling || Page?.Busy == true;

    /// <summary>Whether the page's primary button may be pressed.</summary>
    public bool CanPressPrimary =>
        Page is { } page && !Working && (!page.AsksWalletAccount || WalletAccount.Trim().Length > 0);

    /// <summary>Open the dialog and start (or resume) the ceremony. A login held aside moves into it.</summary>
    public async Task BeginAsync()
    {
        if (Copy is null || Calling)
        {
            return;
        }

        _generation++;
        StartFailed = false;
        _walletAccount = string.Empty;
        Calling = true;
        Raise();
        try
        {
            State = await _client.StartAsync().ConfigureAwait(true);
            Presented = true;
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            StartFailed = true;
        }
        finally
        {
            Calling = false;
            Raise();
        }
    }

    /// <summary>The page's primary button.</summary>
    public async Task PrimaryAsync()
    {
        if (State is not { } state || Page is not { } page || Calling || page.Busy)
        {
            return;
        }

        string id = state.ChangeId;
        switch (page.Action)
        {
            case AccountLoginChangeAction.Close:
                Close();
                return;
            case AccountLoginChangeAction.ProveCurrent:
            {
                string? account = state.IsWallet ? WalletAccount.Trim() : null;
                if (state.IsWallet && string.IsNullOrEmpty(account))
                {
                    return;
                }

                await LegAsync(() => _client.ProveCurrentAsync(id, account)).ConfigureAwait(true);
                return;
            }

            case AccountLoginChangeAction.SignInNew:
                await LegAsync(() => _client.SignInNewAsync(id)).ConfigureAwait(true);
                return;
            case AccountLoginChangeAction.Confirm:
            {
                ulong current = _generation;
                AccountLoginChangeState? next = await CallAsync(() => _client.ConfirmAsync(id)).ConfigureAwait(true);
                if (current != _generation)
                {
                    return;
                }

                if (next is not null)
                {
                    State = next;
                    Raise();
                }
                else
                {
                    await RereadAsync().ConfigureAwait(true);
                }

                if (State?.Step == AccountLoginChangeStep.Done)
                {
                    Close();
                    Finished?.Invoke(this, EventArgs.Empty);
                }

                return;
            }
        }
    }

    /// <summary>Cancel: the daemon drops the ceremony and any login held for it.</summary>
    public async Task CancelAsync()
    {
        string? id = State?.ChangeId;
        Close();
        try
        {
            await _client.CancelAsync(id).ConfigureAwait(true);
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            // Nothing to show: the dialog is gone, and the daemon ends an
            // abandoned ceremony on its own.
        }
    }

    private void Close()
    {
        _generation++;
        Presented = false;
        State = null;
        _walletAccount = string.Empty;
        Raise();
    }

    /// <summary>Begin a browser leg, open the browser, and ask until it ends.</summary>
    private async Task LegAsync(Func<Task<AccountLoginChangeState>> step)
    {
        ulong current = _generation;
        AccountLoginChangeState? next = await CallAsync(step).ConfigureAwait(true);
        if (current != _generation)
        {
            return;
        }

        if (next is null)
        {
            await RereadAsync().ConfigureAwait(true);
            return;
        }

        State = next;
        Raise();
        if (next.BrowserUrl is { } text && Uri.TryCreate(text, UriKind.Absolute, out Uri? url))
        {
            try
            {
                await _open(url).ConfigureAwait(true);
            }
            catch
            {
                // Browser-launch errors can include the URL; the leg ends on
                // its own and the dialog says so when asked.
                System.Diagnostics.Trace.TraceWarning(nameof(AccountLoginChangeFlow));
            }
        }

        await FollowAsync(next.ChangeId, current).ConfigureAwait(true);
    }

    private async Task FollowAsync(string id, ulong current)
    {
        for (int poll = 0; poll < _maxPolls && State?.Busy == true; poll++)
        {
            try
            {
                await _delay(_interval, CancellationToken.None).ConfigureAwait(true);
            }
            catch (OperationCanceledException)
            {
                return;
            }

            if (current != _generation)
            {
                return;
            }

            AccountLoginChangeState? next = await CallAsync(() => _client.StatusAsync(id)).ConfigureAwait(true);
            if (current != _generation || next is null)
            {
                return;
            }

            State = next;
            Raise();
        }
    }

    private async Task RereadAsync()
    {
        if (State?.ChangeId is not { } id)
        {
            return;
        }

        ulong current = _generation;
        AccountLoginChangeState? next = await CallAsync(() => _client.StatusAsync(id)).ConfigureAwait(true);
        if (current == _generation && next is not null)
        {
            State = next;
            Raise();
        }
    }

    private async Task<AccountLoginChangeState?> CallAsync(Func<Task<AccountLoginChangeState>> call)
    {
        Calling = true;
        Raise();
        try
        {
            return await call().ConfigureAwait(true);
        }
        catch (Exception e) when (e is not OperationCanceledException)
        {
            return null;
        }
        finally
        {
            Calling = false;
            Raise();
        }
    }

    private void Raise() => Changed?.Invoke(this, EventArgs.Empty);
}
