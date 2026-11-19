using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// The Devices section's words, decoded from <c>tc_account_devices_copy_json</c>
/// (<c>AccountDevicesCopy</c>, AccountDevicesSurface.swift:3-90). Every string
/// is a draft awaiting owner approval and none is written here: a payload
/// with a missing or empty field is refused whole. Templates carry one
/// placeholder each, <c>{device}</c>, <c>{code}</c> or <c>{when}</c>.
/// </summary>
public sealed record AccountDevicesCopy(
    string Title,
    string Body,
    string ThisDevice,
    string Unnamed,
    string LastSeen,
    string LinkButton,
    string LinkAccessibilityName,
    string LinkBody,
    string CodeCommand,
    string CopyButton,
    string Copied,
    string Waiting,
    string Linked,
    string Expired,
    string DoneButton,
    string UnlinkButton,
    string UnlinkAccessibilityName,
    string UnlinkConfirm,
    string UnlinkConfirmButton,
    string UnlinkConfirmAccessibilityName,
    string CancelButton,
    string SessionEnded,
    string TooMany,
    string Failed)
{
    /// <summary>The payload fields this shell decodes, by wire name (<c>consumedFields</c>).</summary>
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "title", "body", "this_device", "unnamed", "last_seen", "link_button",
        "link_accessibility_name", "link_body", "code_command", "copy_button", "copied",
        "waiting", "linked", "expired", "done_button", "unlink_button",
        "unlink_accessibility_name", "unlink_confirm", "unlink_confirm_button",
        "unlink_confirm_accessibility_name", "cancel_button", "session_ended", "too_many", "failed",
    };

    /// <summary>Null, never a partly filled value, for an unparseable payload or an empty field.</summary>
    public static AccountDevicesCopy? Decode(string? json)
    {
        Dictionary<string, string>? f = CopyFields.Read(json, ConsumedFields);
        return f is null
            ? null
            : new AccountDevicesCopy(
                f["title"], f["body"], f["this_device"], f["unnamed"], f["last_seen"], f["link_button"],
                f["link_accessibility_name"], f["link_body"], f["code_command"], f["copy_button"], f["copied"],
                f["waiting"], f["linked"], f["expired"], f["done_button"], f["unlink_button"],
                f["unlink_accessibility_name"], f["unlink_confirm"], f["unlink_confirm_button"],
                f["unlink_confirm_accessibility_name"], f["cancel_button"], f["session_ended"], f["too_many"],
                f["failed"]);
    }

    /// <summary>The core's words, read once.</summary>
    public static AccountDevicesCopy? Live => LiveCopy.Value;

    private static readonly Lazy<AccountDevicesCopy?> LiveCopy = new(() =>
        Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_account_devices_copy_json())));

    /// <summary><paramref name="template"/> with <c>{placeholder}</c> filled in.</summary>
    public static string Render(string template, string placeholder, string value) =>
        template.Replace("{" + placeholder + "}", value, StringComparison.Ordinal);
}

/// <summary>Reads a flat JSON object of strings, refusing it whole when a field is missing or blank.</summary>
internal static class CopyFields
{
    public static Dictionary<string, string>? Read(string? json, IEnumerable<string> fields)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind != JsonValueKind.Object)
            {
                return null;
            }

            var read = new Dictionary<string, string>(StringComparer.Ordinal);
            foreach (string field in fields)
            {
                if (!doc.RootElement.TryGetProperty(field, out JsonElement value)
                    || value.ValueKind != JsonValueKind.String
                    || value.GetString() is not { } word
                    || IsBlank(word))
                {
                    return null;
                }

                read[field] = word;
            }

            return read;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>
    /// Blank as Swift's <c>trimmingCharacters(in: .whitespaces)</c> reads it:
    /// only space separators and tab, so a line break is not blank.
    /// </summary>
    internal static bool IsBlank(string word) =>
        word.All(c => c == '\t' || char.GetUnicodeCategory(c) == UnicodeCategory.SpaceSeparator);
}

/// <summary>
/// One device linked to the account, as <c>account_devices_list</c> relays it
/// (<c>AccountDevice</c>, AccountDevicesSurface.swift:92-140). Dates are the
/// server's RFC 3339 text; a count the server did not give is null, never 0.
/// </summary>
public sealed record AccountDevice(
    string DeviceKeyId,
    string? Label = null,
    string? Origin = null,
    string? LastSeenAt = null,
    bool ThisDevice = false,
    long? ContributedTraces = null,
    string? LastContributedAt = null)
{
    internal static AccountDevice? From(JsonElement e)
    {
        if (e.ValueKind != JsonValueKind.Object
            || !e.TryGetProperty("device_key_id", out JsonElement id) || id.ValueKind != JsonValueKind.String)
        {
            return null;
        }

        static string? Text(JsonElement o, string key) =>
            o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.String ? v.GetString() : null;

        bool thisDevice = e.TryGetProperty("this_device", out JsonElement t) && t.ValueKind == JsonValueKind.True;
        long? traces = e.TryGetProperty("contributed_traces", out JsonElement c) && c.ValueKind == JsonValueKind.Number
            && c.TryGetInt64(out long n) ? n : null;
        return new AccountDevice(
            id.GetString()!, Text(e, "label"), Text(e, "origin"), Text(e, "last_seen_at"), thisDevice, traces,
            Text(e, "last_contributed_at"));
    }
}

/// <summary>A minted one-time code. Shown once; never stored (<c>DeviceCode</c>, :142-166).</summary>
public sealed record DeviceCode(string CodeId, string Code, string ExpiresAt)
{
    /// <summary><see cref="ExpiresAt"/> read as a time, or null when it is not RFC 3339.</summary>
    public DateTimeOffset? ExpiryDate =>
        DateTimeOffset.TryParseExact(
            ExpiresAt,
            new[] { "yyyy-MM-dd'T'HH:mm:ss.FFFFFFFK", "yyyy-MM-dd'T'HH:mm:ssK" },
            CultureInfo.InvariantCulture,
            DateTimeStyles.None,
            out DateTimeOffset at)
            ? at
            : null;
}

/// <summary><c>account_device_code_status</c>: <c>pending</c>, <c>redeemed</c> or <c>expired</c>.</summary>
public sealed record DeviceCodeStatus(string State, string? DeviceKeyId = null);

/// <summary>A refusal, routed from the daemon's fixed label (<c>AccountDevicesRefusal</c>, :184-205).</summary>
public enum AccountDevicesRefusal
{
    /// <summary>No account session, and the daemon could not renew one.</summary>
    SessionEnded,
    /// <summary>Too many codes outstanding or minted recently.</summary>
    TooMany,
    /// <summary>Anything else. Nothing changed.</summary>
    Failed,
}

public static class AccountDevicesRefusals
{
    public static AccountDevicesRefusal From(string label) => label switch
    {
        "account-session-required" => AccountDevicesRefusal.SessionEnded,
        "device_code_too_many" or "account_devices_rate_limited" => AccountDevicesRefusal.TooMany,
        _ => AccountDevicesRefusal.Failed,
    };
}

/// <summary>Where linking a server stands, one code at a time (<c>DeviceLinkPhase</c>, :207-255).</summary>
public abstract record DeviceLinkPhase
{
    public sealed record Idle : DeviceLinkPhase;

    public sealed record Minting : DeviceLinkPhase;

    /// <summary>The code is on screen and the server has not used it yet.</summary>
    public sealed record Waiting(DeviceCode Code) : DeviceLinkPhase;

    /// <summary>The server used the code.</summary>
    public sealed record Linked(string? DeviceKeyId) : DeviceLinkPhase;

    public sealed record Expired : DeviceLinkPhase;

    public sealed record Refused(AccountDevicesRefusal Refusal) : DeviceLinkPhase;

    /// <summary>
    /// How many status asks in a row may go unanswered before the shell stops
    /// asking and says it failed.
    /// </summary>
    public const int MaxMissedAnswers = 5;

    /// <summary>The code on screen, so the shell keeps asking about it.</summary>
    public DeviceCode? ShownCode => this is Waiting w ? w.Code : null;

    /// <summary>A code past its expiry is expired whether or not the daemon answered.</summary>
    public DeviceLinkPhase Checked(DateTimeOffset now, string codeId) =>
        ShownCode is { } code && code.CodeId == codeId && code.ExpiryDate is { } expiry && now >= expiry
            ? new Expired()
            : this;

    /// <summary>The phase after <paramref name="missed"/> status asks in a row went unanswered.</summary>
    public DeviceLinkPhase AfterMissed(int missed, string codeId) =>
        ShownCode is { } code && code.CodeId == codeId && missed >= MaxMissedAnswers
            ? new Refused(AccountDevicesRefusal.Failed)
            : this;

    /// <summary>
    /// The phase after one status answer for the code on screen. An answer for
    /// another code, or for no code, changes nothing.
    /// </summary>
    public DeviceLinkPhase After(DeviceCodeStatus status, string codeId)
    {
        if (ShownCode is not { } code || code.CodeId != codeId)
        {
            return this;
        }

        return status.State switch
        {
            "redeemed" => new Linked(status.DeviceKeyId),
            "expired" => new Expired(),
            _ => this,
        };
    }
}

/// <summary>
/// What the Devices section shows (<c>AccountDevicesSurface</c>, :257-318).
/// Every sentence is the core's and every fact the daemon's.
/// </summary>
public sealed class AccountDevicesSurface
{
    public AccountDevicesSurface(AccountDevicesCopy copy)
    {
        Copy = copy;
    }

    public AccountDevicesCopy Copy { get; }

    /// <summary>The device's name: its label, or the unnamed line.</summary>
    public string Name(AccountDevice device)
    {
        string label = device.Label?.Trim() ?? string.Empty;
        return label.Length == 0 ? Copy.Unnamed : label;
    }

    /// <summary>"Last seen ..." with the time formatted by the shell, or null when never seen.</summary>
    public string? LastSeenLine(AccountDevice device, Func<string, string?> formatted) =>
        device.LastSeenAt is { } at && formatted(at) is { } when
            ? AccountDevicesCopy.Render(Copy.LastSeen, "when", when)
            : null;

    /// <summary>The command the person runs on the server.</summary>
    public string Command(DeviceCode code) => AccountDevicesCopy.Render(Copy.CodeCommand, "code", code.Code);

    /// <summary>This device signs out instead, so it gets no Unlink.</summary>
    public static bool CanUnlink(AccountDevice device) => !device.ThisDevice;

    public string UnlinkConfirm(AccountDevice device) =>
        AccountDevicesCopy.Render(Copy.UnlinkConfirm, "device", Name(device));

    public string UnlinkAccessibilityName(AccountDevice device) =>
        AccountDevicesCopy.Render(Copy.UnlinkAccessibilityName, "device", Name(device));

    /// <summary>The confirmation's Unlink, named apart from the row's.</summary>
    public string UnlinkConfirmAccessibilityName(AccountDevice device) =>
        AccountDevicesCopy.Render(Copy.UnlinkConfirmAccessibilityName, "device", Name(device));

    public string RefusalLine(AccountDevicesRefusal refusal) => refusal switch
    {
        AccountDevicesRefusal.SessionEnded => Copy.SessionEnded,
        AccountDevicesRefusal.TooMany => Copy.TooMany,
        _ => Copy.Failed,
    };

    /// <summary>This device first, then the others in the daemon's order.</summary>
    public static IReadOnlyList<AccountDevice> Ordered(IEnumerable<AccountDevice> devices)
    {
        AccountDevice[] all = devices.ToArray();
        return all.Where(d => d.ThisDevice).Concat(all.Where(d => !d.ThisDevice)).ToArray();
    }
}

/// <summary>A daemon refusal: its fixed label, safe to route on and never shown.</summary>
public sealed class DaemonRefusedException : Exception
{
    public DaemonRefusedException(string label)
        : base(label)
    {
        Label = label;
    }

    public string Label { get; }
}

/// <summary>
/// The device IPC calls over the daemon call, so the method names and
/// parameters are asserted without a daemon (<c>AccountDevicesClient</c>, :320-375).
/// </summary>
public sealed class AccountDevicesClient
{
    public const string ListMethod = "account_devices_list";
    public const string MintMethod = "account_device_code_mint";
    public const string StatusMethod = "account_device_code_status";
    public const string RevokeMethod = "account_device_revoke";
    public const string RenameMethod = "account_device_rename";

    private readonly Func<string, string, Task<DaemonResponse>> _call;

    public AccountDevicesClient(Func<string, string, Task<DaemonResponse>> call)
    {
        _call = call ?? throw new ArgumentNullException(nameof(call));
    }

    public async Task<IReadOnlyList<AccountDevice>> ListAsync()
    {
        JsonElement result = await DaemonCalls.ResultAsync(_call, ListMethod, new Dictionary<string, object?>()).ConfigureAwait(true);
        if (!result.TryGetProperty("devices", out JsonElement devices) || devices.ValueKind != JsonValueKind.Array)
        {
            throw new DaemonRefusedException(DaemonCalls.Unparseable);
        }

        var list = new List<AccountDevice>();
        foreach (JsonElement item in devices.EnumerateArray())
        {
            list.Add(AccountDevice.From(item) ?? throw new DaemonRefusedException(DaemonCalls.Unparseable));
        }

        return list;
    }

    public async Task<DeviceCode> MintAsync(string? label)
    {
        var parameters = new Dictionary<string, object?>();
        if (label is not null)
        {
            parameters["label"] = label;
        }

        JsonElement r = await DaemonCalls.ResultAsync(_call, MintMethod, parameters).ConfigureAwait(true);
        return DaemonCalls.Text(r, "code_id") is { } id && DaemonCalls.Text(r, "code") is { } code
            && DaemonCalls.Text(r, "expires_at") is { } expires
            ? new DeviceCode(id, code, expires)
            : throw new DaemonRefusedException(DaemonCalls.Unparseable);
    }

    public async Task<DeviceCodeStatus> StatusAsync(string codeId)
    {
        JsonElement r = await DaemonCalls.ResultAsync(
            _call, StatusMethod, new Dictionary<string, object?> { ["code_id"] = codeId }).ConfigureAwait(true);
        return DaemonCalls.Text(r, "state") is { } state
            ? new DeviceCodeStatus(state, DaemonCalls.Text(r, "device_key_id"))
            : throw new DaemonRefusedException(DaemonCalls.Unparseable);
    }

    public Task RevokeAsync(string deviceKeyId) =>
        DaemonCalls.ResultAsync(_call, RevokeMethod, new Dictionary<string, object?> { ["device_key_id"] = deviceKeyId });

    public Task RenameAsync(string deviceKeyId, string? label)
    {
        var parameters = new Dictionary<string, object?> { ["device_key_id"] = deviceKeyId };
        if (label is not null)
        {
            parameters["label"] = label;
        }

        return DaemonCalls.ResultAsync(_call, RenameMethod, parameters);
    }
}

/// <summary>One daemon call read the way the Mac's clients read it: an error's label, or the result.</summary>
internal static class DaemonCalls
{
    public const string Unparseable = "unparseable-response";

    public static async Task<JsonElement> ResultAsync(
        Func<string, string, Task<DaemonResponse>> call, string method, Dictionary<string, object?> parameters)
    {
        DaemonResponse response;
        try
        {
            response = await call(method, JsonSerializer.Serialize(parameters)).ConfigureAwait(true);
        }
        catch (Exception e) when (e is not DaemonRefusedException)
        {
            // Any failure to get an answer is a refusal, as the Mac's
            // clients read every error: the flows then say "failed" rather
            // than leave Busy set or crash the handler that awaited them.
            throw new DaemonRefusedException("unavailable");
        }

        if (response.Error is { } error)
        {
            throw new DaemonRefusedException(string.IsNullOrEmpty(error.Message) ? "unavailable" : error.Message);
        }

        return response.Result is { } result ? result.Clone() : throw new DaemonRefusedException(Unparseable);
    }

    public static string? Text(JsonElement o, string key) =>
        o.ValueKind == JsonValueKind.Object && o.TryGetProperty(key, out JsonElement v) && v.ValueKind == JsonValueKind.String
            ? v.GetString()
            : null;
}

/// <summary>
/// The Devices section's state over time (<c>AccountDevicesModel</c>,
/// AccountDevicesModel.swift): a list read that supersedes earlier ones, a
/// minted code followed every three seconds until it is used, expires or
/// goes unanswered five times, and an unlink that re-reads the list.
/// Continuations resume on the caller's context; drive it from the UI thread.
/// </summary>
public sealed class AccountDevicesFlow
{
    /// <summary>How often a code on screen is asked about (<c>pollEvery</c>).</summary>
    public static readonly TimeSpan PollEvery = TimeSpan.FromSeconds(3);

    private readonly AccountDevicesClient _client;
    private readonly Func<TimeSpan, Task> _delay;
    private readonly Func<DateTimeOffset> _now;
    private ulong _generation;

    public AccountDevicesFlow(
        AccountDevicesClient client,
        AccountDevicesSurface? surface,
        Func<TimeSpan, Task>? delay = null,
        Func<DateTimeOffset>? now = null)
    {
        _client = client;
        Surface = surface;
        _delay = delay ?? Task.Delay;
        _now = now ?? (() => DateTimeOffset.UtcNow);
    }

    public event Action? Changed;

    public AccountDevicesSurface? Surface { get; }
    public IReadOnlyList<AccountDevice>? Devices { get; private set; }
    public AccountDevicesRefusal? ListRefusal { get; private set; }
    public DeviceLinkPhase Link { get; private set; } = new DeviceLinkPhase.Idle();
    public AccountDevicesRefusal? Refusal { get; private set; }
    public bool Busy { get; private set; }
    public AccountDevice? Confirming { get; private set; }
    public string? CopiedCode { get; private set; }

    public async Task RefreshAsync()
    {
        ulong current = ++_generation;
        if (Surface is null)
        {
            Devices = null;
            ListRefusal = null;
            Changed?.Invoke();
            return;
        }

        try
        {
            IReadOnlyList<AccountDevice> listed = await _client.ListAsync().ConfigureAwait(true);
            if (current != _generation)
            {
                return;
            }

            Devices = AccountDevicesSurface.Ordered(listed);
            ListRefusal = null;
        }
        catch (DaemonRefusedException e)
        {
            if (current != _generation)
            {
                return;
            }

            Devices = null;
            ListRefusal = AccountDevicesRefusals.From(e.Label) == AccountDevicesRefusal.SessionEnded
                ? AccountDevicesRefusal.SessionEnded
                : null;
        }

        Changed?.Invoke();
    }

    private ulong _follow;

    /// <summary>
    /// Stops asking about the code on screen, as the Mac's task is cancelled
    /// when its view goes away. The code stays shown; a later
    /// <see cref="ResumeFollowing"/> asks again.
    /// </summary>
    public void StopFollowing() => _follow++;

    /// <summary>Asks about the code on screen again after <see cref="StopFollowing"/>.</summary>
    public async Task ResumeFollowing()
    {
        if (Link.ShownCode is { } code)
        {
            await FollowAsync(code).ConfigureAwait(true);
        }
    }

    public async Task LinkServerAsync()
    {
        if (Busy || Link.ShownCode is not null)
        {
            return;
        }

        Busy = true;
        Refusal = null;
        CopiedCode = null;
        Link = new DeviceLinkPhase.Minting();
        Changed?.Invoke();
        DeviceCode code;
        try
        {
            code = await _client.MintAsync(null).ConfigureAwait(true);
        }
        catch (DaemonRefusedException e)
        {
            Busy = false;
            Link = new DeviceLinkPhase.Refused(AccountDevicesRefusals.From(e.Label));
            Changed?.Invoke();
            return;
        }

        Busy = false;
        Link = new DeviceLinkPhase.Waiting(code);
        Changed?.Invoke();
        await FollowAsync(code).ConfigureAwait(true);
    }

    private async Task FollowAsync(DeviceCode code)
    {
        string codeId = code.CodeId;
        ulong follow = ++_follow;
        int missed = 0;
        while (Link.ShownCode?.CodeId == codeId)
        {
            await _delay(PollEvery).ConfigureAwait(true);
            if (follow != _follow)
            {
                return;
            }

            // Past its expiry the code is spent whatever the daemon says.
            Link = Link.Checked(_now(), codeId);
            if (Link.ShownCode?.CodeId != codeId)
            {
                Changed?.Invoke();
                break;
            }

            DeviceCodeStatus status;
            try
            {
                status = await _client.StatusAsync(codeId).ConfigureAwait(true);
            }
            catch (DaemonRefusedException)
            {
                missed++;
                Link = Link.AfterMissed(missed, codeId);
                Changed?.Invoke();
                continue;
            }

            missed = 0;
            Link = Link.After(status, codeId);
            Changed?.Invoke();
        }

        if (Link is DeviceLinkPhase.Linked)
        {
            await RefreshAsync().ConfigureAwait(true);
        }
    }

    public void DismissLink()
    {
        Link = new DeviceLinkPhase.Idle();
        CopiedCode = null;
        Changed?.Invoke();
    }

    /// <summary>Records that the command for the code on screen was copied.</summary>
    public void Copied(DeviceCode code)
    {
        CopiedCode = code.CodeId;
        Changed?.Invoke();
    }

    public void Confirm(AccountDevice? device)
    {
        Confirming = device;
        Changed?.Invoke();
    }

    public async Task UnlinkAsync(AccountDevice device)
    {
        if (Busy)
        {
            return;
        }

        Confirming = null;
        Busy = true;
        Changed?.Invoke();
        try
        {
            await _client.RevokeAsync(device.DeviceKeyId).ConfigureAwait(true);
            Refusal = null;
        }
        catch (DaemonRefusedException e)
        {
            Refusal = AccountDevicesRefusals.From(e.Label);
        }

        Busy = false;
        Changed?.Invoke();
        await RefreshAsync().ConfigureAwait(true);
    }
}
