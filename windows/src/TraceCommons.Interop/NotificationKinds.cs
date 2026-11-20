using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop;

// W17-D2 (owner ruling 2026-11-19): the Mac's shown notification kinds as
// per-kind switches, its two one-time offers, and a notification for each
// reengage_due, every word from the core's nudge table or the daemon. The
// Mac's counterparts are NudgeSettings (NudgeCopy.swift), NudgeSurface
// (NudgeSurface.swift) and Notifier (Notifier.swift).

/// <summary>
/// What <c>get_settings</c> reported about notifications. A value the
/// daemon did not send is absent (null, or missing from
/// <see cref="Notify"/>), never off: no switch is drawn from a default.
/// </summary>
public sealed record NotificationSettingsState(
    bool? NotificationsEnabled,
    IReadOnlyDictionary<string, bool> Notify,
    bool VerdictsOfferPending,
    bool IdleOfferPending)
{
    /// <summary>The state from a <c>get_settings</c> result; null when it is not an object.</summary>
    public static NotificationSettingsState? Parse(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            return From(doc.RootElement);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>The state from a <c>get_settings</c> result element.</summary>
    public static NotificationSettingsState? From(JsonElement settings)
    {
        if (settings.ValueKind != JsonValueKind.Object)
        {
            return null;
        }

        var notify = new Dictionary<string, bool>(StringComparer.Ordinal);
        if (settings.TryGetProperty("notify", out JsonElement kinds) && kinds.ValueKind == JsonValueKind.Object)
        {
            foreach (JsonProperty kind in kinds.EnumerateObject())
            {
                if (Bool(kind.Value) is { } on)
                {
                    notify[kind.Name] = on;
                }
            }
        }

        return new NotificationSettingsState(
            settings.TryGetProperty("notifications_enabled", out JsonElement master) ? Bool(master) : null,
            notify,
            // Written only while true (settings.rs), so absent is "no offer".
            settings.TryGetProperty("verdicts_offer_pending", out JsonElement verdicts) && Bool(verdicts) == true,
            settings.TryGetProperty("idle_offer_pending", out JsonElement idle) && Bool(idle) == true);
    }

    private static bool? Bool(JsonElement value) => value.ValueKind switch
    {
        JsonValueKind.True => true,
        JsonValueKind.False => false,
        _ => null,
    };
}

/// <summary>One kind's switch: the core's label and help, the stored value, and whether it can be changed now.</summary>
public sealed record NotifySwitchRow(string Kind, string Label, string? Help, bool IsOn, bool Enabled);

/// <summary>The master switch: the core's label and hover tip, and the stored value.</summary>
public sealed record NotifyMasterRow(string Label, string? Tooltip, bool IsOn);

/// <summary>A one-time offer to turn a kind on, in the core's words.</summary>
public sealed record NotifyOffer(string Kind, string Text, string Accept, string Decline);

/// <summary>One daemon request: a method and its params.</summary>
public sealed record NudgeRequest(string Method, string ParamsJson);

/// <summary>Where an offer is drawn (<c>NudgeSettings.offers(_:copy:on:)</c>).</summary>
public enum OfferPlace
{
    /// <summary>The idle-sessions offer.</summary>
    Traces,

    /// <summary>The verdicts offer.</summary>
    History,
}

/// <summary>Where a notification's default action takes the window (<c>NudgeSurface.Destination</c>).</summary>
public enum NudgeDestination
{
    Traces,

    /// <summary>Traces, narrowed to the idle traces.</summary>
    TracesIdle,
    History,

    /// <summary>Private AI, where the NEAR AI balance is.</summary>
    PrivateAI,
}

/// <summary>What a click on a notification does: one request (none for some kinds), then a place to open.</summary>
public sealed record NudgeClick(NudgeRequest? Request, NudgeDestination Destination);

/// <summary>
/// The core's nudge table (<c>tc_nudge_copy_json</c>): every fixed nudge
/// word, key to text. A key the table lacks, or holds blank, is null, and
/// what it would have labelled is not drawn.
/// </summary>
public static class NudgeWords
{
    private static readonly Lazy<IReadOnlyDictionary<string, string>> Table = new(() =>
        Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_nudge_copy_json())));

    /// <summary>The word for <paramref name="key"/>; null when absent or blank.</summary>
    public static string? Word(string key) =>
        Table.Value.TryGetValue(key, out string? text) && !string.IsNullOrWhiteSpace(text) ? text : null;

    /// <summary>
    /// The digest switch's help for the schedule the daemon reported
    /// (<c>tc_nudge_digest_help_json</c>), or null when the core has no line.
    /// </summary>
    public static string? DigestHelp(string? settingsJson)
    {
        string? input = DigestHelpInput(settingsJson);
        if (input is null)
        {
            return null;
        }

        string? answer = NativeMethods.TakeOwnedString(NativeMethods.tc_nudge_digest_help_json(input));
        if (string.IsNullOrEmpty(answer))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(answer);
            return doc.RootElement.ValueKind == JsonValueKind.Object
                && doc.RootElement.TryGetProperty("digest_help", out JsonElement help)
                && help.ValueKind == JsonValueKind.String
                && !string.IsNullOrWhiteSpace(help.GetString())
                    ? help.GetString()
                    : null;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>
    /// What the core is asked for the digest's help: the schedule and the
    /// interval as the daemon sent them, and nothing else about the
    /// settings. Null when the settings carry neither.
    /// </summary>
    internal static string? DigestHelpInput(string? settingsJson)
    {
        if (string.IsNullOrEmpty(settingsJson))
        {
            return null;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(settingsJson);
            if (doc.RootElement.ValueKind != JsonValueKind.Object)
            {
                return null;
            }

            var input = new Dictionary<string, JsonElement>(StringComparer.Ordinal);
            foreach (string key in new[] { "digest_interval_secs", "digest_schedule" })
            {
                if (doc.RootElement.TryGetProperty(key, out JsonElement value))
                {
                    input[key] = value.Clone();
                }
            }

            return input.Count == 0 ? null : JsonSerializer.Serialize(input);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    internal static IReadOnlyDictionary<string, string> Decode(string? json)
    {
        var table = new Dictionary<string, string>(StringComparer.Ordinal);
        if (string.IsNullOrEmpty(json))
        {
            return table;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(json);
            if (doc.RootElement.ValueKind == JsonValueKind.Object)
            {
                foreach (JsonProperty entry in doc.RootElement.EnumerateObject())
                {
                    if (entry.Value.ValueKind == JsonValueKind.String)
                    {
                        table[entry.Name] = entry.Value.GetString() ?? string.Empty;
                    }
                }
            }
        }
        catch (JsonException)
        {
        }

        return table;
    }
}

/// <summary>
/// Settings > Notifications' per-kind switches and the one-time offers
/// (<c>NudgeSettings</c>, NudgeCopy.swift).
/// </summary>
public static class NotificationKinds
{
    /// <summary>
    /// The kinds drawn, in the Mac's order (<c>NudgeSettings.shownKinds</c>).
    /// <c>weekly_recap</c>, <c>insights_tip</c> and <c>settlement_estimate</c>
    /// are held and left out, as on the Mac.
    /// </summary>
    public static readonly IReadOnlyList<string> ShownKinds = new[] { "digest", "verdicts_landed", "idle_sessions", "near_ai_credit" };

    /// <summary>
    /// The switches: one per shown kind the daemon reported and the core
    /// worded, each disabled (its stored value still shown) while the master
    /// switch is off. <paramref name="digestHelp"/> is the core's finished
    /// line for the digest (<see cref="NudgeWords.DigestHelp"/>).
    /// </summary>
    public static IReadOnlyList<NotifySwitchRow> Rows(NotificationSettingsState? state, Func<string, string?> word, string? digestHelp)
    {
        var rows = new List<NotifySwitchRow>();
        if (state is null)
        {
            return rows;
        }

        bool master = state.NotificationsEnabled != false;
        foreach (string kind in ShownKinds)
        {
            (string label, string? help) = kind switch
            {
                "digest" => ("SETTING_DIGEST", digestHelp),
                "verdicts_landed" => ("SETTING_NOTIFY_VERDICTS", null),
                "idle_sessions" => ("SETTING_NOTIFY_IDLE", word("SETTING_NOTIFY_IDLE_HELP")),
                _ => ("SETTING_NOTIFY_NEAR_AI_CREDIT", (string?)null),
            };
            if (state.Notify.TryGetValue(kind, out bool on) && word(label) is { } text)
            {
                rows.Add(new NotifySwitchRow(kind, text, help, on, master));
            }
        }

        return rows;
    }

    /// <summary>
    /// The master switch above the kinds (<c>.notifications</c>), drawn only
    /// when the daemon reported it and the core worded it.
    /// </summary>
    public static NotifyMasterRow? Master(NotificationSettingsState? state, Func<string, string?> word) =>
        state?.NotificationsEnabled is { } on && word("SETTING_NOTIFY_MASTER") is { } label
            ? new NotifyMasterRow(label, word("SETTING_NOTIFY_MASTER_TOOLTIP"), on)
            : null;

    /// <summary>The master switch, written through <c>set_notifications_enabled</c>.</summary>
    public static NudgeRequest SetMaster(bool on) =>
        new("set_notifications_enabled", JsonSerializer.Serialize(new { on }));

    /// <summary>The line under the kinds: the caps and quiet hours, in the core's words; null without switches.</summary>
    public static string? Footnote(IReadOnlyList<NotifySwitchRow> rows, Func<string, string?> word) =>
        rows.Count == 0 ? null : word("SETTING_NOTIFY_BUDGET_HELP");

    /// <summary>The pending offers, verdicts first; none without the core's words.</summary>
    public static IReadOnlyList<NotifyOffer> Offers(NotificationSettingsState? state, Func<string, string?> word)
    {
        var offers = new List<NotifyOffer>();
        if (state is null || word("OFFER_TURN_ON") is not { } accept || word("OFFER_NO_THANKS") is not { } decline)
        {
            return offers;
        }

        if (state.VerdictsOfferPending && word("OFFER_NOTIFY_VERDICTS_EXISTING") is { } verdicts)
        {
            offers.Add(new NotifyOffer("verdicts_landed", verdicts, accept, decline));
        }

        if (state.IdleOfferPending && word("OFFER_NOTIFY_IDLE_EXISTING") is { } idle)
        {
            offers.Add(new NotifyOffer("idle_sessions", idle, accept, decline));
        }

        return offers;
    }

    /// <summary>The offer <paramref name="place"/> draws: verdicts on History, idle on Traces.</summary>
    public static IReadOnlyList<NotifyOffer> Offers(NotificationSettingsState? state, Func<string, string?> word, OfferPlace place)
    {
        string kind = place == OfferPlace.History ? "verdicts_landed" : "idle_sessions";
        return Offers(state, word).Where(o => o.Kind == kind).ToList();
    }

    /// <summary>One shown kind's switch, written through <c>set_notify_kind</c>.</summary>
    public static NudgeRequest SetKind(string kind, bool on)
    {
        if (!ShownKinds.Contains(kind, StringComparer.Ordinal))
        {
            throw new ArgumentOutOfRangeException(nameof(kind), kind, "not a shown notification kind");
        }

        return new NudgeRequest("set_notify_kind", JsonSerializer.Serialize(new { kind, on }));
    }

    /// <summary>Enable: writes the kind on, which also ends the offer.</summary>
    public static NudgeRequest Accept(NotifyOffer offer) => SetKind(offer.Kind, true);

    /// <summary>
    /// No thanks: clears the offer's marker and changes nothing else
    /// (<c>dismissNotifyOffer</c>), not <c>nudge_decline</c>, which is the
    /// re-engagement card's Not now.
    /// </summary>
    public static NudgeRequest Decline(NotifyOffer offer)
    {
        string marker = offer.Kind switch
        {
            "verdicts_landed" => "verdicts_offer_pending",
            "idle_sessions" => "idle_offer_pending",
            _ => throw new ArgumentOutOfRangeException(nameof(offer), offer.Kind, "no offer for this kind"),
        };
        return new NudgeRequest("set_settings", JsonSerializer.Serialize(new Dictionary<string, bool> { [marker] = false }));
    }
}

/// <summary>
/// A <c>reengage_due</c> notification (<c>NudgeSurface.notification</c>,
/// <c>Notifier.postReengage</c>): the daemon's title and body unchanged.
/// The daemon decides whether one is due: it emits a kind only while that
/// kind's switch and the master are on, under its own caps
/// (daemon/reengage.rs). A tray balloon has no buttons, so a click is the
/// notification's default action and ignoring it is Not now, which on the
/// Mac only dismisses.
/// </summary>
public sealed record ReengageNotice(string Kind, string Title, string Body, NudgeClick? Click)
{
    /// <summary>The kinds this build posts (<c>NudgeSurface.Kind</c>).</summary>
    private static readonly HashSet<string> Kinds = new(StringComparer.Ordinal)
    {
        "idle_sessions", "review_backlog", "verdicts_landed", "settlement_estimate", "near_ai_credit",
    };

    /// <summary>
    /// The notice for a <c>reengage_due</c> frame; null for another event, a
    /// kind this build does not know, or one without words.
    /// </summary>
    public static ReengageNotice? From(DaemonEvent evt)
    {
        if (evt.Event != DaemonProtocol.Events.ReengageDue
            || evt.Data is not { ValueKind: JsonValueKind.Object } data
            || Text(data, "kind") is not { } kind
            || !Kinds.Contains(kind)
            || Text(data, "title") is not { } title
            || Text(data, "body") is not { } body)
        {
            return null;
        }

        string? first = null;
        if (data.TryGetProperty("actions", out JsonElement actions) && actions.ValueKind == JsonValueKind.Array)
        {
            foreach (JsonElement action in actions.EnumerateArray())
            {
                if (action.ValueKind == JsonValueKind.Object
                    && Text(action, "id") is { } id
                    && id != "not_now"
                    && Text(action, "label") is not null)
                {
                    first = id;
                    break;
                }
            }
        }

        return new ReengageNotice(kind, title, body, first is null ? null : ClickFor(first, kind));
    }

    /// <summary>
    /// The opt-in events to declare: <c>reengage_due</c> only while a
    /// notification can be shown. The daemon stamps an announcement against
    /// its caps only while a renderer is declared, so a declaration nobody
    /// can draw would spend that budget on nothing (<c>Notifier.acceptedEvents</c>).
    /// </summary>
    public static IReadOnlyList<string> AcceptedEvents(bool canShow) =>
        canShow ? new[] { DaemonProtocol.Events.ReengageDue } : Array.Empty<string>();

    /// <summary>What a button means and does (<c>NudgeSurface.intent</c> and <c>effect</c>).</summary>
    private static NudgeClick? ClickFor(string actionId, string kind) => (actionId, kind) switch
    {
        ("review", "settlement_estimate") => new NudgeClick(null, NudgeDestination.Traces),
        ("review", "near_ai_credit") => new NudgeClick(null, NudgeDestination.PrivateAI),
        ("review", "idle_sessions") => new NudgeClick(Opened(kind), NudgeDestination.TracesIdle),
        ("review", "review_backlog") => new NudgeClick(Opened(kind), NudgeDestination.Traces),
        ("see_history", "verdicts_landed") => new NudgeClick(Opened(kind), NudgeDestination.History),
        _ => null,
    };

    private static NudgeRequest Opened(string kind) =>
        new("nudge_opened", JsonSerializer.Serialize(new { kind }));

    private static string? Text(JsonElement element, string key) =>
        element.TryGetProperty(key, out JsonElement value)
        && value.ValueKind == JsonValueKind.String
        && value.GetString() is { Length: > 0 } text
            ? text
            : null;
}

/// <summary>
/// Settings > Notifications' switches and the one-time offers, read and
/// written through the daemon (<c>NudgeSettingsStore</c>): a switch is drawn
/// only from a value the daemon reported, every taken write is followed by a
/// fresh read so a switch shows what the daemon stored, and a refused write
/// says so under what it was about.
/// </summary>
public sealed class NotificationKindsStore
{
    private readonly Func<string, string, System.Threading.Tasks.Task<DaemonResponse>> _call;
    private NotificationSettingsState? _state;
    private string? _settingsJson;

    public NotificationKindsStore(Func<string, string, System.Threading.Tasks.Task<DaemonResponse>> call)
    {
        _call = call ?? throw new ArgumentNullException(nameof(call));
    }

    /// <summary>The settings have answered at least once.</summary>
    public bool SettingsAnswered => _state is not null;

    /// <summary>The latest settings read failed.</summary>
    public bool ReadFailed { get; private set; }

    /// <summary>A write is out.</summary>
    public bool Busy { get; private set; }

    /// <summary>The refused write's line, until the next write.</summary>
    public string? Refusal { get; private set; }

    /// <summary>The offer whose answer was refused; null when the refused write was a switch.</summary>
    public string? RefusedOfferKind { get; private set; }

    /// <summary>The request-failed line the Mac shows. Settable so tests need not load the core.</summary>
    public Func<string> RefusalLine { get; init; } = () => MonitorShellCopy.Top("request_failed");

    /// <summary>The core's nudge words. Settable so tests need not load the core.</summary>
    public Func<string, string?> Word { get; init; } = NudgeWords.Word;

    /// <summary>The digest switch's help for the reported schedule. Settable for tests.</summary>
    public Func<string?, string?> DigestHelp { get; init; } = NudgeWords.DigestHelp;

    /// <summary>The switches (<see cref="NotificationKinds.Rows"/>).</summary>
    public IReadOnlyList<NotifySwitchRow> Rows =>
        NotificationKinds.Rows(_state, Word, _state is null ? null : DigestHelp(_settingsJson));

    /// <summary>The master switch, or null.</summary>
    public NotifyMasterRow? Master => NotificationKinds.Master(_state, Word);

    /// <summary>The line under the switches, or null.</summary>
    public string? Footnote => NotificationKinds.Footnote(Rows, Word);

    /// <summary>The offer <paramref name="place"/> draws.</summary>
    public IReadOnlyList<NotifyOffer> Offers(OfferPlace place) => NotificationKinds.Offers(_state, Word, place);

    /// <summary>Reads the settings. A failed read keeps nothing it cannot confirm: no switch is drawn from it.</summary>
    public async System.Threading.Tasks.Task ReadAsync()
    {
        DaemonResponse response = await _call(DaemonProtocol.Methods.GetSettings, "{}").ConfigureAwait(true);
        string? json = response.IsError ? null : response.Result?.GetRawText();
        NotificationSettingsState? state = NotificationSettingsState.Parse(json);
        ReadFailed = state is null;
        _state = state;
        _settingsJson = state is null ? null : json;
    }

    /// <summary>The master switch, then a fresh read.</summary>
    public System.Threading.Tasks.Task SetMasterAsync(bool on) =>
        WriteAsync(NotificationKinds.SetMaster(on), offerKind: null);

    /// <summary>One kind's switch, then a fresh read.</summary>
    public System.Threading.Tasks.Task SetAsync(string kind, bool on) =>
        WriteAsync(NotificationKinds.SetKind(kind, on), offerKind: null);

    /// <summary>An offer's Enable (the kind on) or No thanks (the marker cleared), then a fresh read.</summary>
    public System.Threading.Tasks.Task AnswerAsync(NotifyOffer offer, bool accept) =>
        WriteAsync(accept ? NotificationKinds.Accept(offer) : NotificationKinds.Decline(offer), offer.Kind);

    private async System.Threading.Tasks.Task WriteAsync(NudgeRequest request, string? offerKind)
    {
        if (Busy)
        {
            return;
        }

        Busy = true;
        Refusal = null;
        RefusedOfferKind = null;
        try
        {
            DaemonResponse response = await _call(request.Method, request.ParamsJson).ConfigureAwait(true);
            if (response.IsError)
            {
                Refusal = RefusalLine();
                RefusedOfferKind = offerKind;
                return;
            }

            await ReadAsync().ConfigureAwait(true);
        }
        finally
        {
            Busy = false;
        }
    }
}
