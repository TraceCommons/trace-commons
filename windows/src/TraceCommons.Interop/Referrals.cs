using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>
/// The referral words (<c>tc_referral_copy_json</c>, <c>ReferralCopy</c>,
/// ReferralSurface.swift), every one a DRAFT for owner review. Templates
/// carry <c>{code}</c>, <c>{n}</c>, <c>{date}</c> or <c>{amount}</c>. A
/// payload with a missing or blank field is refused whole.
/// </summary>
public sealed record ReferralCopy(IReadOnlyDictionary<string, string> Words)
{
    public static readonly IReadOnlyList<string> ConsumedFields = new[]
    {
        "title", "body", "create_button", "create_accessibility_name", "limit_reached", "unavailable",
        "copy_button", "copy_accessibility_name", "copied", "disable_button", "disable_accessibility_name",
        "disable_scope", "state_disabled", "state_suspended", "state_voided", "code_hidden", "joined_one",
        "joined_many", "people_joined", "credit", "credit_sent", "enter_title", "field_label",
        "field_placeholder", "apply_button", "apply_accessibility_name", "enter_scope", "applied_status",
        "not_applied", "invalid_code", "sign_up_label", "failed", "session_ended", "insights_own",
        "insights_referrals", "insights_on_hold",
    };

    public string this[string key] => Words[key];

    public static ReferralCopy? Decode(string? json) =>
        CopyFields.Read(json, ConsumedFields) is { } words ? new ReferralCopy(words) : null;

    public static ReferralCopy? Live => LiveCopy.Value;

    private static readonly Lazy<ReferralCopy?> LiveCopy = new(() =>
        Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_referral_copy_json())));

    public static string Render(string template, IReadOnlyDictionary<string, string> values) =>
        values.Aggregate(template, (text, pair) => text.Replace("{" + pair.Key + "}", pair.Value, StringComparison.Ordinal));

    /// <summary>A typed code as it is shown (<c>tc_referral_code_display</c>), or null for text that is not one.</summary>
    public static string? Display(string typed) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_referral_code_display(typed));
}

/// <summary>One referral code (<c>ReferralCodeRow</c>).</summary>
public sealed record ReferralCodeRow(string CodeId, string? Code, string State = "live", ulong RefereeCount = 0, ulong? BonusOwedNanoUsd = 0)
{
    public bool IsLive => State == "live";

    internal static ReferralCodeRow? From(JsonElement e)
    {
        if (DaemonCalls.Text(e, "code_id") is not { } id || DaemonCalls.Text(e, "state") is not { } state)
        {
            return null;
        }

        ulong count = e.TryGetProperty("referee_count", out JsonElement c) && c.TryGetUInt64(out ulong n) ? n : 0;
        return new ReferralCodeRow(id, DaemonCalls.Text(e, "code"), state, count, ReferralWire.NanoUsd(e, "bonus_owed_nano_usd"));
    }
}

/// <summary>The codes and totals (<c>ReferralCodesListing</c>).</summary>
public sealed record ReferralCodesListing(
    IReadOnlyList<ReferralCodeRow> Codes,
    int MaxLiveCodes = 5,
    bool CanCreate = true,
    ulong PeopleJoined = 0,
    ulong? BonusOwedNanoUsd = 0,
    ulong? BonusSentNanoUsd = 0)
{
    internal static ReferralCodesListing? From(JsonElement e)
    {
        if (!e.TryGetProperty("codes", out JsonElement codes) || codes.ValueKind != JsonValueKind.Array
            || !e.TryGetProperty("max_live_codes", out JsonElement max) || !max.TryGetInt32(out int maxLive))
        {
            return null;
        }

        var rows = new List<ReferralCodeRow>();
        foreach (JsonElement row in codes.EnumerateArray())
        {
            if (ReferralCodeRow.From(row) is not { } parsed)
            {
                return null;
            }

            rows.Add(parsed);
        }

        // A daemon that does not say is read as "no": a Create the commons
        // refuses is worse than one not offered.
        bool canCreate = e.TryGetProperty("can_create", out JsonElement cc) && cc.ValueKind == JsonValueKind.True;
        ulong joined = e.TryGetProperty("people_joined", out JsonElement pj) && pj.TryGetUInt64(out ulong p)
            ? p
            : rows.Aggregate(0UL, (sum, r) => unchecked(sum + r.RefereeCount));
        return new ReferralCodesListing(rows, maxLive, canCreate, joined,
            ReferralWire.NanoUsd(e, "bonus_owed_nano_usd"), ReferralWire.NanoUsd(e, "bonus_sent_nano_usd"));
    }
}

/// <summary>Whether a code was applied to this account, and until when one may be (<c>ReferralStatus</c>).</summary>
public sealed record ReferralStatus
{
    public ReferralStatus(bool applied, string? applyUntil = null, bool canApply = false)
    {
        Applied = applied;
        ApplyUntil = applyUntil;
        // Never offered on an applied account, whatever was sent.
        CanApply = !applied && canApply;
    }

    public bool Applied { get; }
    public string? ApplyUntil { get; }
    public bool CanApply { get; }

    internal static ReferralStatus? From(JsonElement e) =>
        e.TryGetProperty("applied", out JsonElement a) && a.ValueKind is JsonValueKind.True or JsonValueKind.False
            ? new ReferralStatus(
                a.ValueKind == JsonValueKind.True,
                DaemonCalls.Text(e, "apply_until"),
                e.TryGetProperty("can_apply", out JsonElement c) && c.ValueKind == JsonValueKind.True)
            : null;
}

internal static class ReferralWire
{
    /// <summary>A nano-USD amount as digits text or a number; null otherwise (<c>nanoUSD</c>).</summary>
    public static ulong? NanoUsd(JsonElement e, string key)
    {
        if (!e.TryGetProperty(key, out JsonElement v))
        {
            return null;
        }

        if (v.ValueKind == JsonValueKind.String)
        {
            string text = v.GetString()!;
            return text.Length > 0 && text.All(char.IsAsciiDigit) && ulong.TryParse(text, out ulong parsed) ? parsed : null;
        }

        return v.ValueKind == JsonValueKind.Number && v.TryGetUInt64(out ulong n) ? n : null;
    }
}

public enum ReferralRefusal
{
    SessionEnded,
    NotApplied,
    InvalidCode,
    Unavailable,
    LimitReached,
    Failed,
}

public static class ReferralRefusals
{
    public static ReferralRefusal From(string label) => label switch
    {
        "account-session-required" => ReferralRefusal.SessionEnded,
        "referral_not_applied" => ReferralRefusal.NotApplied,
        "referral-code-invalid" => ReferralRefusal.InvalidCode,
        "referral_unavailable" => ReferralRefusal.Unavailable,
        "referral_code_limit_reached" => ReferralRefusal.LimitReached,
        _ => ReferralRefusal.Failed,
    };
}

/// <summary>What the Referrals section shows (<c>ReferralSurface</c>). Every sentence is the core's.</summary>
public sealed class ReferralSurface
{
    public ReferralSurface(ReferralCopy copy)
    {
        Copy = copy;
    }

    public ReferralCopy Copy { get; }

    public string ShownCode(ReferralCodeRow row) => row.Code ?? Copy["code_hidden"];

    public string? StateLabel(ReferralCodeRow row) => row.State switch
    {
        "live" => null,
        "suspended" => Copy["state_suspended"],
        "voided" => Copy["state_voided"],
        _ => Copy["state_disabled"],
    };

    public string JoinedLine(ulong count) =>
        ReferralCopy.Render(count == 1 ? Copy["joined_one"] : Copy["joined_many"], new Dictionary<string, string> { ["n"] = count.ToString(System.Globalization.CultureInfo.InvariantCulture) });

    public static bool CanCopy(ReferralCodeRow row) => row.Code is not null;

    public static bool CanDisable(ReferralCodeRow row) => row.IsLive;

    public string CopyAccessibilityName(ReferralCodeRow row) =>
        ReferralCopy.Render(Copy["copy_accessibility_name"], new Dictionary<string, string> { ["code"] = ShownCode(row) });

    public string DisableAccessibilityName(ReferralCodeRow row) =>
        ReferralCopy.Render(Copy["disable_accessibility_name"], new Dictionary<string, string> { ["code"] = ShownCode(row) });

    public static bool ShowsDisableScope(ReferralCodesListing listing) => listing.Codes.Any(c => c.IsLive);

    public string? LimitLine(ReferralCodesListing listing) =>
        listing.CanCreate
            ? null
            : ReferralCopy.Render(Copy["limit_reached"], new Dictionary<string, string> { ["n"] = listing.MaxLiveCodes.ToString(System.Globalization.CultureInfo.InvariantCulture) });

    public string? CreditLine(ReferralCodesListing listing, Func<ulong, string> money)
    {
        if (listing.BonusOwedNanoUsd is not { } owed)
        {
            return null;
        }

        string earned = money(owed);
        if (listing.BonusSentNanoUsd is not { } sent || sent == 0)
        {
            return earned;
        }

        return $"{earned} · {ReferralCopy.Render(Copy["credit_sent"], new Dictionary<string, string> { ["amount"] = money(sent) })}";
    }

    public string EnterScope(string date) =>
        ReferralCopy.Render(Copy["enter_scope"], new Dictionary<string, string> { ["date"] = date });

    public static bool ShowsEnterField(ReferralStatus? status) => status?.CanApply == true;

    public string RefusalLine(ReferralRefusal refusal, int? maxLiveCodes = null) => refusal switch
    {
        ReferralRefusal.SessionEnded => Copy["session_ended"],
        ReferralRefusal.NotApplied => Copy["not_applied"],
        ReferralRefusal.InvalidCode => Copy["invalid_code"],
        ReferralRefusal.Unavailable => Copy["unavailable"],
        ReferralRefusal.LimitReached => maxLiveCodes is { } n
            ? ReferralCopy.Render(Copy["limit_reached"], new Dictionary<string, string> { ["n"] = n.ToString(System.Globalization.CultureInfo.InvariantCulture) })
            : Copy["failed"],
        _ => Copy["failed"],
    };

    /// <summary>The invalid-code line for typed text that is not a code; null while empty or valid.</summary>
    public string? FieldProblem(string typed, Func<string, string?> display)
    {
        string trimmed = typed.Trim();
        if (trimmed.Length == 0)
        {
            return null;
        }

        return display(trimmed) is null ? Copy["invalid_code"] : null;
    }
}

/// <summary>The referral IPC calls (<c>ReferralClient</c>).</summary>
public sealed class ReferralClient
{
    private readonly Func<string, string, Task<DaemonResponse>> _call;

    public ReferralClient(Func<string, string, Task<DaemonResponse>> call)
    {
        _call = call ?? throw new ArgumentNullException(nameof(call));
    }

    public async Task<ReferralCodesListing> ListAsync() =>
        ReferralCodesListing.From(await DaemonCalls.ResultAsync(_call, "referral_codes_list", new()).ConfigureAwait(true))
        ?? throw new DaemonRefusedException(DaemonCalls.Unparseable);

    public async Task CreateAsync()
    {
        JsonElement r = await DaemonCalls.ResultAsync(_call, "referral_codes_create", new()).ConfigureAwait(true);
        if (DaemonCalls.Text(r, "code_id") is null || DaemonCalls.Text(r, "code") is null)
        {
            throw new DaemonRefusedException(DaemonCalls.Unparseable);
        }
    }

    public Task DisableAsync(string codeId) =>
        DaemonCalls.ResultAsync(_call, "referral_codes_disable", new() { ["code_id"] = codeId });

    /// <summary><paramref name="code"/> as typed; the daemon reads it and sends only its canonical form.</summary>
    public Task ApplyAsync(string code) =>
        DaemonCalls.ResultAsync(_call, "referral_apply", new() { ["code"] = code });

    public async Task<ReferralStatus> StatusAsync() =>
        ReferralStatus.From(await DaemonCalls.ResultAsync(_call, "referral_status", new()).ConfigureAwait(true))
        ?? throw new DaemonRefusedException(DaemonCalls.Unparseable);
}

/// <summary>
/// The Referrals section's state (<c>ReferralModel</c>, ReferralModel.swift):
/// the codes and the status read together, an ended session said with
/// nothing stale beside it, an earlier answer kept over a failed re-read, and
/// every action followed by a re-read. Drive it from the UI thread.
/// </summary>
public sealed class ReferralFlow
{
    private readonly ReferralClient _client;
    private readonly Func<string, string?> _display;
    private ulong _generation;

    public ReferralFlow(ReferralClient client, ReferralSurface? surface, Func<string, string?>? display = null)
    {
        _client = client;
        Surface = surface;
        _display = display ?? ReferralCopy.Display;
    }

    public event Action? Changed;

    public ReferralSurface? Surface { get; }
    public ReferralCodesListing? Listing { get; private set; }
    public ReferralStatus? Status { get; private set; }
    public bool Answered { get; private set; }
    public bool Failed { get; private set; }
    public bool SessionEnded { get; private set; }
    public bool Busy { get; private set; }
    public ReferralRefusal? Refusal { get; private set; }
    public string? CopiedCodeId { get; private set; }
    public string Draft { get; set; } = string.Empty;

    public string? FieldProblem => Surface?.FieldProblem(Draft, _display);

    public bool CanApply => !Busy && FieldProblem is null && Draft.Trim().Length > 0;

    public async Task RefreshAsync()
    {
        ulong current = ++_generation;
        if (Surface is null)
        {
            Listing = null;
            Status = null;
            Changed?.Invoke();
            return;
        }

        try
        {
            ReferralCodesListing listing = await _client.ListAsync().ConfigureAwait(true);
            ReferralStatus status = await _client.StatusAsync().ConfigureAwait(true);
            if (current != _generation)
            {
                return;
            }

            Listing = listing;
            Status = status;
            Answered = true;
            Failed = false;
            SessionEnded = false;
        }
        catch (DaemonRefusedException e)
        {
            if (current != _generation)
            {
                return;
            }

            if (ReferralRefusals.From(e.Label) == ReferralRefusal.SessionEnded)
            {
                // Nothing stale stays beside Sign in again.
                Listing = null;
                Status = null;
                SessionEnded = true;
                Answered = true;
                Failed = false;
            }
            else
            {
                // An earlier answer stays: it is still what the daemon said.
                Failed = !Answered;
            }
        }

        Changed?.Invoke();
    }

    public Task CreateAsync() => ActAsync(_client.CreateAsync);

    public Task DisableAsync(ReferralCodeRow row) => ActAsync(() => _client.DisableAsync(row.CodeId));

    public async Task ApplyAsync()
    {
        if (!CanApply)
        {
            return;
        }

        string typed = Draft;
        if (await ActAsync(() => _client.ApplyAsync(typed)).ConfigureAwait(true))
        {
            Draft = string.Empty;
            Changed?.Invoke();
        }
    }

    /// <summary>Marks a row's code copied; the shell clears it after two seconds.</summary>
    public void Copied(ReferralCodeRow row)
    {
        CopiedCodeId = row.CodeId;
        Changed?.Invoke();
    }

    public void ClearCopied(string codeId)
    {
        if (CopiedCodeId == codeId)
        {
            CopiedCodeId = null;
            Changed?.Invoke();
        }
    }

    private async Task<bool> ActAsync(Func<Task> work)
    {
        if (Busy)
        {
            return false;
        }

        Busy = true;
        Changed?.Invoke();
        bool succeeded;
        try
        {
            await work().ConfigureAwait(true);
            Refusal = null;
            succeeded = true;
        }
        catch (DaemonRefusedException e)
        {
            Refusal = ReferralRefusals.From(e.Label);
            succeeded = false;
        }

        Busy = false;
        Changed?.Invoke();
        await RefreshAsync().ConfigureAwait(true);
        return succeeded;
    }
}

/// <summary>Referral credit as money (<c>ReferralMoney.format</c>, DaemonClient+Referral.swift:16-22).</summary>
public static class ReferralMoney
{
    /// <summary>The scale the referral amounts are sent in: nano-USD.</summary>
    public const byte NanoScale = 9;

    /// <summary>The core's money figure, or a dash when it gives none.</summary>
    public static string Format(ulong nanoUsd)
    {
        string amount = NearAiBalanceSurface.Amount((long)Math.Min(nanoUsd, long.MaxValue), NanoScale);
        return amount.Length > 0 ? amount : "—";
    }
}
