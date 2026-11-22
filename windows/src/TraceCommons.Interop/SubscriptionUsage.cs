using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

// Current, account-wide Claude and Codex limits (the daemon's
// subscription_usage; DaemonData.SubscriptionUsage, SubscriptionUsage.swift).
// They do not belong to the historical week or the local Insights store. No
// shell infers an allowance from a missing percentage or from local traces.

/// <summary>One window a provider reports (<c>SubscriptionUsageWindow</c>).</summary>
public sealed record SubscriptionUsageWindow(string Lane, string? Label = null, int? UsedPermille = null, string? RemainingText = null,
    DateTimeOffset? ResetsAt = null, int? DurationMins = null);

/// <summary>One local login (<c>SubscriptionUsageLogin</c>); <see cref="Source"/> is "api" or "cli", null without a reading.</summary>
public sealed record SubscriptionUsageLogin(string Id, string Label, string State, bool DefaultLogin = false, string? Reason = null,
    DateTimeOffset? UpdatedAt = null, bool Stale = false, IReadOnlyList<SubscriptionUsageWindow>? Windows = null, int? BankedResets = null,
    bool? Refreshing = null, string? Source = null)
{
    public IReadOnlyList<SubscriptionUsageWindow> Windows { get; init; } = Windows ?? Array.Empty<SubscriptionUsageWindow>();
}

/// <summary>One provider, <c>codex</c> or <c>claude</c> (<c>SubscriptionUsageProvider</c>).</summary>
public sealed record SubscriptionUsageProvider(string Provider, bool Enabled, bool Refreshing = false, IReadOnlyList<SubscriptionUsageLogin>? Logins = null)
{
    public IReadOnlyList<SubscriptionUsageLogin> Logins { get; init; } = Logins ?? Array.Empty<SubscriptionUsageLogin>();
}

/// <summary>
/// <c>subscription_usage</c>'s reply. Decoded as the Mac decodes it: a
/// required field missing or mistyped refuses the whole reply, while an
/// unreadable measurement (a percentage, a reset, a length, a banked count, a
/// source) is unknown without discarding its neighbours.
/// </summary>
public sealed record SubscriptionUsage(IReadOnlyList<SubscriptionUsageProvider> Providers, string SchemaVersion = SubscriptionUsage.Schema, bool? Truncated = null)
{
    public const string Schema = "trace_commons.subscription_usage.v1";
    public const string Method = "subscription_usage";

    private sealed class Malformed : Exception
    {
    }

    private static T Fail<T>() => throw new Malformed();

    /// <summary>The reply, or null when it does not decode.</summary>
    public static SubscriptionUsage? Parse(JsonElement value)
    {
        try
        {
            JsonElement o = Obj(value);
            return new SubscriptionUsage(List(o, "providers", Provider), Str(o, "schema_version"), OptBoolStrict(o, "truncated"));
        }
        catch (Exception e) when (e is Malformed or OverflowException or InvalidOperationException)
        {
            return null;
        }
    }

    internal static SubscriptionUsageLogin? ParseLogin(JsonElement value)
    {
        try
        {
            return Login(value);
        }
        catch (Malformed)
        {
            return null;
        }
    }

    internal static SubscriptionUsageWindow? ParseWindow(JsonElement value)
    {
        try
        {
            return Window(value);
        }
        catch (Malformed)
        {
            return null;
        }
    }

    private static SubscriptionUsageProvider Provider(JsonElement value)
    {
        JsonElement o = Obj(value);
        return new SubscriptionUsageProvider(Str(o, "provider"), Bool(o, "enabled"), Bool(o, "refreshing"), List(o, "logins", Login));
    }

    private static SubscriptionUsageLogin Login(JsonElement value)
    {
        JsonElement o = Obj(value);
        int? banked = Lenient<int>(o, "banked_resets", TryInt);
        return new SubscriptionUsageLogin(
            Str(o, "id"), Str(o, "label"), Str(o, "state"), Bool(o, "default_login"), OptStrStrict(o, "reason"),
            Lenient<DateTimeOffset>(o, "updated_at", TryDate), Bool(o, "stale"), List(o, "windows", Window),
            banked is >= 0 ? banked : null, Lenient<bool>(o, "refreshing", TryBool), LenientStr(o, "source"));
    }

    private static SubscriptionUsageWindow Window(JsonElement value)
    {
        JsonElement o = Obj(value);
        int? used = Lenient<int>(o, "used_permille", TryInt);
        int? duration = Lenient<int>(o, "duration_mins", TryInt);
        return new SubscriptionUsageWindow(
            Str(o, "lane"), OptStrStrict(o, "label"), used is >= 0 and <= 1000 ? used : null, LenientStr(o, "remaining_text"),
            Lenient<DateTimeOffset>(o, "resets_at", TryDate), duration is > 0 ? duration : null);
    }

    private static JsonElement Obj(JsonElement value) => value.ValueKind == JsonValueKind.Object ? value : Fail<JsonElement>();

    private static IReadOnlyList<T> List<T>(JsonElement o, string name, Func<JsonElement, T> read) =>
        o.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.Array
            ? value.EnumerateArray().Select(read).ToList()
            : Fail<IReadOnlyList<T>>();

    private static string Str(JsonElement o, string name) =>
        o.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.String ? value.GetString()! : Fail<string>();

    private static bool Bool(JsonElement o, string name) =>
        o.TryGetProperty(name, out JsonElement value) && value.ValueKind is JsonValueKind.True or JsonValueKind.False
            ? value.GetBoolean()
            : Fail<bool>();

    /// <summary>Absent or null is null; any other type refuses the reply (Swift's <c>decodeIfPresent</c>).</summary>
    private static string? OptStrStrict(JsonElement o, string name)
    {
        if (!o.TryGetProperty(name, out JsonElement value) || value.ValueKind == JsonValueKind.Null) return null;
        return value.ValueKind == JsonValueKind.String ? value.GetString() : Fail<string>();
    }

    private static bool? OptBoolStrict(JsonElement o, string name)
    {
        if (!o.TryGetProperty(name, out JsonElement value) || value.ValueKind == JsonValueKind.Null) return null;
        return value.ValueKind is JsonValueKind.True or JsonValueKind.False ? value.GetBoolean() : Fail<bool>();
    }

    /// <summary>Anything unreadable is null (Swift's <c>try? decodeIfPresent</c>).</summary>
    private static T? Lenient<T>(JsonElement o, string name, Func<JsonElement, T?> read) where T : struct =>
        o.TryGetProperty(name, out JsonElement value) ? read(value) : null;

    private static string? LenientStr(JsonElement o, string name) =>
        o.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.String ? value.GetString() : null;

    private static int? TryInt(JsonElement value) =>
        value.ValueKind == JsonValueKind.Number && value.TryGetInt32(out int n) ? n : null;

    private static bool? TryBool(JsonElement value) =>
        value.ValueKind is JsonValueKind.True or JsonValueKind.False ? value.GetBoolean() : null;

    private static DateTimeOffset? TryDate(JsonElement value) =>
        value.ValueKind == JsonValueKind.String
        && DateTimeOffset.TryParse(value.GetString(), CultureInfo.InvariantCulture, DateTimeStyles.RoundtripKind, out DateTimeOffset date)
            ? date
            : null;
}

/// <summary>A refused or failed read or write; <see cref="NotServed"/> for a daemon that predates the method.</summary>
public sealed class SubscriptionUsageException : Exception
{
    public SubscriptionUsageException(string message, bool notServed = false) : base(message) => NotServed = notServed;

    public bool NotServed { get; }
}

/// <summary>
/// The daemon calls (<c>LiveDaemonClient.subscriptionUsage</c> and
/// <c>setSubscriptionUsage</c>, LiveDaemonClient.swift:344-371): only Codex
/// and Claude, a bounded login, a Claude refresh only for one named login,
/// and a prompt only with a refresh.
/// </summary>
public static class SubscriptionUsageClient
{
    public static SubscriptionUsageModel.Reader Reader(Func<string, string, Task<DaemonResponse>> call) =>
        async (refresh, interactive, provider, login) =>
        {
            DaemonResponse reply = await call(SubscriptionUsage.Method, Parameters(refresh, interactive, provider, login)).ConfigureAwait(false);
            if (reply.Error is { } error)
            {
                throw new SubscriptionUsageException(error.Message, error.Code == "unknown_method");
            }

            return (reply.Result is { } result ? SubscriptionUsage.Parse(result) : null)
                ?? throw new SubscriptionUsageException("undecodable");
        };

    public static SubscriptionUsageModel.Writer Writer(Func<string, string, Task<DaemonResponse>> call) =>
        async (provider, on) =>
        {
            string key = provider switch
            {
                "codex" => "subscription_usage_codex_enabled",
                "claude" => "subscription_usage_claude_on",
                _ => throw new SubscriptionUsageException("subscription-provider-invalid"),
            };
            DaemonResponse reply = await call("set_settings", JsonSerializer.Serialize(new Dictionary<string, bool> { [key] = on })).ConfigureAwait(false);
            if (reply.Error is { } error)
            {
                throw new SubscriptionUsageException(error.Message, error.Code == "unknown_method");
            }
        };

    /// <summary>The request's parameters, or the refusal the Mac's client throws before sending anything.</summary>
    public static string Parameters(bool refresh, bool interactive, string? provider, string? login)
    {
        if (provider is not (null or "codex" or "claude"))
        {
            throw new SubscriptionUsageException("subscription-provider-invalid");
        }

        if (login is not null
            && (provider is null || login.Length == 0 || System.Text.Encoding.UTF8.GetByteCount(login) > 256 || login.Any(char.IsControl)))
        {
            throw new SubscriptionUsageException("subscription-login-invalid");
        }

        if (refresh && provider == "claude" && login is null)
        {
            throw new SubscriptionUsageException("subscription-login-invalid");
        }

        var parameters = new Dictionary<string, object> { ["refresh"] = refresh, ["interactive"] = refresh && interactive };
        if (provider is not null) parameters["provider"] = provider;
        if (login is not null) parameters["login"] = login;
        return JsonSerializer.Serialize(parameters);
    }
}

/// <summary>
/// The Mac's <c>SubscriptionUsageModel</c> (SubscriptionUsageModel.swift):
/// account limits kept apart from the week. The only credential-capable
/// operations are the injected daemon calls, and only an explicit Refresh is
/// interactive; opening Overview never prompts. Each segment between awaits
/// runs under one lock, as the Mac's runs on its main actor.
/// </summary>
public sealed class SubscriptionUsageModel
{
    public delegate Task<SubscriptionUsage> Reader(bool refresh, bool interactive, string? provider, string? login);

    public delegate Task Writer(string provider, bool on);

    private readonly object _gate = new();
    private readonly Reader? _reader;
    private readonly Writer? _writer;
    private readonly TimeSpan _activePollInterval;
    private readonly TimeSpan _idlePollInterval;
    private readonly TimeSpan _silentReadInterval;
    private readonly Func<TimeSpan> _clock;
    private long _generation;
    private object? _reading;
    private long? _readingGeneration;
    private bool _queuedRefresh;
    private bool _queuedInteractive;
    private string? _queuedProvider;
    private string? _queuedLogin;
    private string? _refreshingProvider;
    private string? _refreshingLogin;
    private object? _observation;
    private CancellationTokenSource? _observationSleep;
    private readonly Dictionary<string, TimeSpan> _silentReads = new(StringComparer.Ordinal);
    private bool _claudeAvailable;
    private bool? _codexEnabled;
    private bool? _claudeEnabled;
    private SubscriptionUsage? _shown;
    private string? _failureKey;
    private bool _unsupported;
    private readonly HashSet<string> _changingProviders = new(StringComparer.Ordinal);

    public SubscriptionUsageModel(Reader? reader = null, Writer? writer = null, TimeSpan? activePollInterval = null,
        TimeSpan? silentReadInterval = null, Func<TimeSpan>? clock = null, TimeSpan? idlePollInterval = null)
    {
        _reader = reader;
        _writer = writer;
        _activePollInterval = activePollInterval ?? TimeSpan.FromSeconds(2);
        _idlePollInterval = idlePollInterval ?? TimeSpan.FromSeconds(60);
        _silentReadInterval = silentReadInterval ?? TimeSpan.FromMinutes(20);
        if (clock is null)
        {
            var watch = Stopwatch.StartNew();
            clock = () => watch.Elapsed;
        }

        _clock = clock;
    }

    /// <summary>Something shown changed; raised on whatever thread changed it.</summary>
    public event Action? Changed;

    public bool? CodexEnabled { get { lock (_gate) return _codexEnabled; } }

    public bool? ClaudeEnabled { get { lock (_gate) return _claudeEnabled; } }

    public SubscriptionUsage? Shown { get { lock (_gate) return _shown; } }

    /// <summary>The core's copy key for a failed read or write, or null.</summary>
    public string? FailureKey { get { lock (_gate) return _failureKey; } }

    /// <summary>The daemon predates the method: nothing is shown, and that is not a failure.</summary>
    public bool Unsupported { get { lock (_gate) return _unsupported; } }

    public IReadOnlyCollection<string> ChangingProviders { get { lock (_gate) return _changingProviders.ToArray(); } }

    /// <summary>Codex while on, Claude while on with a login found.</summary>
    public IReadOnlyList<SubscriptionUsageProvider> SupportedProviders { get { lock (_gate) return Supported(); } }

    public bool IsVisible { get { lock (_gate) return _reader is not null && !_unsupported && (_codexEnabled == true || _claudeAvailable); } }

    public bool IsRefreshing { get { lock (_gate) return Refreshing(); } }

    public bool CanChange => _writer is not null;

    private IReadOnlyList<SubscriptionUsageProvider> Supported() =>
        (_shown?.Providers ?? Array.Empty<SubscriptionUsageProvider>())
            .Where(p => (p.Provider == "codex" || p.Provider == "claude") && p.Enabled && (p.Provider == "codex" || p.Logins.Count > 0))
            .ToList();

    private bool Refreshing() =>
        _reading is not null || Supported().Any(p => p.Refreshing || p.Logins.Any(l => l.Refreshing == true));

    private void Raise() => Changed?.Invoke();

    /// <summary>
    /// Reads now and again while <paramref name="cancellation"/> is not
    /// cancelled: every 2 seconds while anything refreshes, else every
    /// minute. With <paramref name="readOnOpen"/>, logins with no current
    /// figures are read silently, once per open: the daemon never prompts or
    /// starts the native tool for these.
    /// </summary>
    public async Task ObserveAsync(bool readOnOpen, CancellationToken cancellation)
    {
        object token = new();
        lock (_gate)
        {
            _observationSleep?.Cancel();
            _observation = token;
            _generation++;
            ClearQueuedRefresh();
            _silentReads.Clear();
        }

        // A transport may ignore cancellation while it waits; end the view's
        // authority at once rather than when that reply arrives.
        using CancellationTokenRegistration ended = cancellation.Register(() => EndObservation(token));
        try
        {
            while (!cancellation.IsCancellationRequested && Observing(token))
            {
                await LoadAsync(cancellation: cancellation).ConfigureAwait(false);
                if (cancellation.IsCancellationRequested || !Observing(token)) return;
                bool stop;
                lock (_gate) stop = _unsupported || _reader is null;
                if (stop) return;
                if (readOnOpen)
                {
                    await ReadMissingAsync(token, cancellation).ConfigureAwait(false);
                    if (cancellation.IsCancellationRequested || !Observing(token)) return;
                }

                CancellationTokenSource sleeper;
                TimeSpan delay;
                lock (_gate)
                {
                    delay = Refreshing() ? _activePollInterval : _idlePollInterval;
                    sleeper = new CancellationTokenSource();
                    _observationSleep = sleeper;
                }

                try
                {
                    await Task.Delay(delay, sleeper.Token).ConfigureAwait(false);
                }
                catch (OperationCanceledException)
                {
                }

                lock (_gate)
                {
                    if (!ReferenceEquals(_observation, token)) return;
                    if (ReferenceEquals(_observationSleep, sleeper)) _observationSleep = null;
                }

                sleeper.Dispose();
            }
        }
        finally
        {
            EndObservation(token);
        }
    }

    private bool Observing(object token)
    {
        lock (_gate) return ReferenceEquals(_observation, token);
    }

    private async Task ReadMissingAsync(object token, CancellationToken cancellation)
    {
        List<(string Provider, string Login)> due;
        TimeSpan now = _clock();
        lock (_gate)
        {
            due = Supported().SelectMany(row => row.Logins
                .Where(login => !row.Refreshing && login.Refreshing != true
                    && (login.State != "observed" || login.Stale)
                    && (!_silentReads.TryGetValue(row.Provider + "/" + login.Id, out TimeSpan at) || now - at >= _silentReadInterval))
                .Select(login => (row.Provider, login.Id))).ToList();
        }

        foreach ((string provider, string login) in due)
        {
            lock (_gate)
            {
                if (cancellation.IsCancellationRequested || !ReferenceEquals(_observation, token) || _reading is not null
                    || _queuedRefresh || _refreshingProvider is not null || _changingProviders.Count > 0)
                {
                    return;
                }

                _silentReads[provider + "/" + login] = now;
            }

            await LoadAsync(refresh: true, interactive: false, provider: provider, login: login, cancellation: cancellation).ConfigureAwait(false);
        }
    }

    private void EndObservation(object token)
    {
        lock (_gate)
        {
            if (!ReferenceEquals(_observation, token)) return;
            _observation = null;
            _observationSleep?.Cancel();
            _observationSleep = null;
            _generation++;
            _reading = null;
            _readingGeneration = null;
            _refreshingProvider = null;
            _refreshingLogin = null;
            ClearQueuedRefresh();
        }

        Raise();
    }

    private void ClearQueuedRefresh()
    {
        _queuedRefresh = false;
        _queuedInteractive = false;
        _queuedProvider = null;
        _queuedLogin = null;
    }

    /// <summary>
    /// One read. A refresh asked for while a current read is in flight waits
    /// for it, keeping its exact scope; a read a setting change invalidated
    /// is superseded without waiting for its reply.
    /// </summary>
    public async Task LoadAsync(bool refresh = false, bool interactive = false, string? provider = null, string? login = null,
        CancellationToken cancellation = default)
    {
        Reader? reader = _reader;
        object request = new();
        long token;
        lock (_gate)
        {
            if (reader is null || _changingProviders.Count > 0 || cancellation.IsCancellationRequested) return;
            if (_reading is not null && _readingGeneration == _generation)
            {
                if (refresh)
                {
                    // Keep the exact scope; coalescing must never fan out.
                    _queuedProvider = provider;
                    _queuedLogin = login;
                    _queuedInteractive = interactive;
                }

                _queuedRefresh = _queuedRefresh || refresh;
                return;
            }

            token = _generation;
            _reading = request;
            _readingGeneration = token;
            _refreshingProvider = refresh ? provider ?? "codex" : null;
            _refreshingLogin = refresh ? login : null;
        }

        Raise();
        try
        {
            SubscriptionUsage value;
            try
            {
                value = await reader(refresh, refresh && interactive, provider, login).ConfigureAwait(false);
            }
            catch (Exception error)
            {
                lock (_gate)
                {
                    if (_generation != token || cancellation.IsCancellationRequested) return;
                    _shown = null;
                    _unsupported = error is SubscriptionUsageException { NotServed: true };
                    _failureKey = _unsupported ? null : "analytics_subscriptions_unavailable";
                }

                return;
            }

            lock (_gate)
            {
                if (_generation != token || cancellation.IsCancellationRequested) return;
                if (value.SchemaVersion != SubscriptionUsage.Schema)
                {
                    _shown = null;
                    _failureKey = "analytics_subscriptions_unavailable";
                    return;
                }

                _shown = value;
                Reconcile(value);
                _unsupported = false;
                _failureKey = null;
            }
        }
        finally
        {
            bool pending = false, prompt = false;
            string? selected = null, selectedLogin = null;
            lock (_gate)
            {
                if (ReferenceEquals(_reading, request))
                {
                    _reading = null;
                    _readingGeneration = null;
                    _refreshingProvider = null;
                    _refreshingLogin = null;
                    _observationSleep?.Cancel();
                    pending = _queuedRefresh && _generation == token && !cancellation.IsCancellationRequested;
                    prompt = _queuedInteractive;
                    selected = _queuedProvider;
                    selectedLogin = _queuedLogin;
                    ClearQueuedRefresh();
                }
            }

            Raise();
            if (pending)
            {
                _ = Task.Run(async () =>
                {
                    lock (_gate)
                    {
                        if (_generation != token) return;
                    }

                    await LoadAsync(refresh: true, interactive: prompt, provider: selected, login: selectedLogin).ConfigureAwait(false);
                });
            }
        }
    }

    /// <summary>
    /// Refresh, the only interactive read. Codex by default; Claude only for
    /// the exact login its card names.
    /// </summary>
    public Task Refresh(string provider = "codex", string? login = null)
    {
        lock (_gate)
        {
            if (!CanRefreshLocked(provider, login)) return Task.CompletedTask;
        }

        return RefreshAsync(provider, login);
    }

    private async Task RefreshAsync(string provider, string? login)
    {
        // The read claims its slot before the observer wakes, so the
        // observer's own read joins it rather than racing it (on the Mac
        // both run on the main actor, where they cannot interleave).
        Task read = LoadAsync(refresh: true, interactive: true, provider: provider, login: login);
        lock (_gate) _observationSleep?.Cancel();
        await read.ConfigureAwait(false);
        // Wake polling while the daemon waits on a credential decision.
        lock (_gate) _observationSleep?.Cancel();
    }

    public bool CanRefresh(string provider, string? login)
    {
        lock (_gate) return CanRefreshLocked(provider, login);
    }

    private bool CanRefreshLocked(string provider, string? login)
    {
        IReadOnlyList<SubscriptionUsageProvider> supported = Supported();
        if (_changingProviders.Count > 0 || supported.Any(p => p.Refreshing || p.Logins.Any(l => l.Refreshing == true))
            || _refreshingProvider is not null || _queuedRefresh
            || supported.FirstOrDefault(p => p.Provider == provider) is not { } row)
        {
            return false;
        }

        if (provider == "claude") return login is not null && row.Logins.Any(l => l.Id == login);
        return provider == "codex" && (login is null || row.Logins.Any(l => l.Id == login));
    }

    /// <summary>Whether this login's Refresh is spinning.</summary>
    public bool IsRefreshingLogin(string provider, string? login)
    {
        lock (_gate)
        {
            if (_refreshingProvider == provider && _refreshingLogin == login) return true;
            if (_queuedRefresh && _queuedProvider == provider && _queuedLogin == login) return true;
            if (Supported().FirstOrDefault(p => p.Provider == provider) is not { } row) return false;
            if (login is null) return row.Refreshing;
            if (row.Logins.FirstOrDefault(l => l.Id == login) is not { } entry) return false;
            return entry.Refreshing ?? row.Refreshing;
        }
    }

    private void Reconcile(SubscriptionUsage value)
    {
        _codexEnabled = value.Providers.FirstOrDefault(p => p.Provider == "codex")?.Enabled;
        SubscriptionUsageProvider? claude = value.Providers.FirstOrDefault(p => p.Provider == "claude");
        _claudeEnabled = claude?.Enabled;
        _claudeAvailable = claude is { Enabled: true, Logins.Count: > 0 };
    }

    /// <summary>
    /// A provider's switch. Off hides its figures at once and invalidates any
    /// read from before the choice; the daemon stays authoritative when the
    /// write returns. Saving never starts a credential read.
    /// </summary>
    public async Task SetEnabledAsync(string provider, bool on, CancellationToken cancellation = default)
    {
        Writer? writer = _writer;
        object? startedObservation;
        lock (_gate)
        {
            if (writer is null || provider is not ("codex" or "claude") || _changingProviders.Count > 0) return;
            startedObservation = _observation;
            _generation++;
            ClearQueuedRefresh();
            _changingProviders.Add(provider);
            if (!on && provider == "codex") _codexEnabled = false;
            if (!on && provider == "claude")
            {
                _claudeEnabled = false;
                _claudeAvailable = false;
            }

            if (!on && _shown is { } shown)
            {
                _shown = shown with
                {
                    Providers = shown.Providers.Select(row => row.Provider == provider ? new SubscriptionUsageProvider(provider, false) : row).ToList(),
                };
            }
        }

        Raise();
        bool still(object? started) => !cancellation.IsCancellationRequested && (started is null || ReferenceEquals(_observation, started));
        try
        {
            try
            {
                await writer(provider, on).ConfigureAwait(false);
            }
            catch (Exception)
            {
                long token;
                lock (_gate)
                {
                    if (!still(startedObservation)) return;
                    _changingProviders.Remove(provider);
                    _generation++;
                    _failureKey = "analytics_subscriptions_change_failed";
                    token = _generation;
                }

                Raise();
                // Read the saved state after a refused write, without prompting.
                if (_reader is { } reader)
                {
                    try
                    {
                        SubscriptionUsage actual = await reader(false, false, null, null).ConfigureAwait(false);
                        lock (_gate)
                        {
                            if (_generation == token)
                            {
                                _shown = actual;
                                Reconcile(actual);
                            }
                        }
                    }
                    catch (Exception)
                    {
                    }
                }

                lock (_gate) _observationSleep?.Cancel();
                return;
            }

            lock (_gate)
            {
                if (!still(startedObservation)) return;
                _changingProviders.Remove(provider);
                _generation++;
            }

            // Consent only permits a later explicit Refresh: reconcile the
            // saved permission without launching any credential operation.
            await LoadAsync(cancellation: cancellation).ConfigureAwait(false);
            lock (_gate) _observationSleep?.Cancel();
        }
        finally
        {
            lock (_gate) _changingProviders.Remove(provider);
            Raise();
        }
    }
}

/// <summary>
/// A login's words for its card (<c>InsightsSubscriptionWords.login</c>,
/// InsightsSubscriptionsCard.swift:28-113): its name, why it has no figures,
/// and whether the figures shown are an earlier reading.
/// </summary>
public static class InsightsSubscriptionWords
{
    public sealed record Login(string Label, string? State, string? Freshness);

    public static Login Of(SubscriptionUsageLogin login, string provider, Func<string, string?> copy)
    {
        string Text(string key) => copy(key) ?? string.Empty;
        string Fill(string key, string name) => InsightsAnalyticsWords.Fill(Text(key), ("harness", name));
        string name = Text("analytics_subscriptions_provider_" + provider);
        bool claude = provider == "claude";
        string? reason = login.Reason;
        string? state = reason switch
        {
            "captured_on_refresh" when claude => null,
            "credentials_expired" or "auth_expired" when claude => Text("analytics_subscriptions_claude_expired"),
            "auth_missing" when claude => Text("analytics_subscriptions_claude_auth_missing"),
            "scope_missing" => Text("analytics_subscriptions_claude_scope"),
            "keychain_needs_permission" when claude => Text("analytics_subscriptions_claude_keychain_denied"),
            "credential_read_timeout" => Text("analytics_subscriptions_credential_pending"),
            "auth_changed" when claude => Text("analytics_subscriptions_claude_auth_changed"),
            "request_timeout" or "rate_limited" => Text("analytics_subscriptions_" + reason),
            "credentials_missing" or "credentials_expired" or "sign_in_required" or "unsupported_login" or "authentication_failed"
                or "auth_missing" or "auth_expired" or "auth_changed" => Fill("analytics_subscriptions_sign_in", name),
            "tools_missing" or "native_cli_unavailable" => Fill("analytics_subscriptions_native_tool", name),
            "native_tool_running" => Text("analytics_subscriptions_native_tool_running"),
            "base_url_refused" or "invalid_response" or "no_logins" or "native_storage_unsupported" => Fill("analytics_subscriptions_" + reason, name),
            "not_checked" or "not_observed" => Text("analytics_subscriptions_not_checked"),
            "keychain_unavailable" or "keychain_locked" or "keychain_permission_required" or "keychain_needs_permission" =>
                Fill("analytics_subscriptions_keychain", name),
            _ => login.State != "observed" || reason is not null ? Text("analytics_subscriptions_unavailable") : null,
        };

        // A refused refresh may keep an earlier reading: its age and the
        // current failure both matter, and neither replaces the other.
        string? freshness = login.Stale && reason != "captured_on_refresh"
            ? Text(state is null ? "analytics_subscriptions_stale" : "analytics_subscriptions_cached")
            : null;
        return new Login(login.DefaultLogin ? Text("analytics_subscriptions_default_login") : login.Label, state, freshness);
    }
}

/// <summary>
/// One card per Codex or Claude login (<c>InsightsLimitsRows</c>,
/// InsightsUsageCards.swift:347-487), pure so the pace and the figures are
/// tested without a view. Unknown figures are the core's dash, never a zero.
/// </summary>
public static class InsightsLimitsRows
{
    /// <summary>
    /// A card's rows. A window the provider does not report has no rows
    /// (null); a login with no measured window shows both as the dash.
    /// </summary>
    public sealed record Row(string Id, string Provider, string LoginId, string LoginLabel, string Account, string? Session,
        string? SessionReset, string? Weekly, string? WeeklyReset, string Pace, bool PaceWarns, string? Banked, string? Freshness,
        bool Measured, string? Headline);

    /// <summary>Lasts until the reset, or runs out at <see cref="RunsOut"/>.</summary>
    public sealed record PaceReading(DateTimeOffset? RunsOut)
    {
        public bool OnPace => RunsOut is null;
    }

    public static IReadOnlyList<Row> Rows(IReadOnlyList<SubscriptionUsageProvider> providers, DateTimeOffset now, Func<string, string?> copy)
    {
        string Text(string key) => copy(key) ?? string.Empty;
        string unknown = Text("analytics_unavailable");
        var rows = new List<Row>();
        foreach (SubscriptionUsageProvider provider in providers)
        {
            string name = Text("analytics_subscriptions_provider_" + provider.Provider);
            foreach (SubscriptionUsageLogin login in provider.Logins)
            {
                InsightsSubscriptionWords.Login words = InsightsSubscriptionWords.Of(login, provider.Provider, copy);
                string account = provider.Logins.Count > 1 ? name + " · " + words.Label : name;
                bool measured = login.State == "observed" && login.Windows.Count > 0;
                List<SubscriptionUsageWindow> live = measured
                    ? login.Windows.Where(w => w.UsedPermille is not null && (w.ResetsAt is not { } r || r > now)).ToList()
                    : new List<SubscriptionUsageWindow>();
                SubscriptionUsageWindow? session = live.FirstOrDefault(IsSession);
                SubscriptionUsageWindow? weekly = live.FirstOrDefault(IsWeekly);
                bool Reports(Func<SubscriptionUsageWindow, bool> lane) => !measured || login.Windows.Any(lane);
                string? Resets(SubscriptionUsageWindow? window) =>
                    !measured ? null : window?.ResetsAt is { } at ? Duration(now, at) : unknown;
                PaceReading? paced = (weekly ?? session) is { } paceWindow ? Pace(paceWindow, now) : null;
                string paceWords = paced switch
                {
                    null => unknown,
                    { RunsOut: { } at } => InsightsAnalyticsWords.Fill(Text("analytics_limits_runs_out"), ("time", Duration(now, at))),
                    _ => Text("analytics_limits_on_pace"),
                };
                string windowWord = Text(weekly is not null ? "analytics_subscriptions_weekly" : "analytics_subscriptions_session");
                string? headline = paced switch
                {
                    null => null,
                    { RunsOut: { } at } => InsightsAnalyticsWords.Fill(Text("analytics_limits_headline_runs_out"),
                        ("window", windowWord), ("time", Duration(now, at))),
                    _ => Text("analytics_limits_headline_on_pace"),
                };
                bool hasSession = Reports(IsSession), hasWeekly = Reports(IsWeekly);
                rows.Add(new Row(
                    provider.Provider + ":" + login.Id, provider.Provider, login.Id, words.Label, account,
                    hasSession ? (session is not null ? Left(session) : unknown) : null,
                    hasSession ? Resets(session) : null,
                    hasWeekly ? (weekly is not null ? Left(weekly) : unknown) : null,
                    hasWeekly ? Resets(weekly) : null,
                    paceWords, paced is { RunsOut: not null },
                    login.BankedResets?.ToString(CultureInfo.InvariantCulture), words.Freshness, measured, headline));
            }
        }

        return rows;
    }

    public static bool IsWeekly(SubscriptionUsageWindow window) =>
        window.Lane != "model" && (window.DurationMins == 10_080 || (window.DurationMins is null && window.Lane == "weekly"));

    public static bool IsSession(SubscriptionUsageWindow window) =>
        window.Lane != "model" && !IsWeekly(window)
        && (window.Lane == "session" || window.DurationMins is < 10_080);

    /// <summary>What is left, as a whole percentage of the window.</summary>
    public static string Left(SubscriptionUsageWindow window)
    {
        if (window.UsedPermille is not { } used) return string.Empty;
        int remaining = Math.Max(0, Math.Min(1_000, 1_000 - used));
        return ((int)Math.Round(remaining / 10.0, MidpointRounding.AwayFromZero)).ToString(CultureInfo.InvariantCulture) + "%";
    }

    /// <summary>
    /// At the rate used so far in this window, whether it lasts until it
    /// resets, or when it runs out. Null when the window's length or reset is
    /// not reported, or under 5% of it has passed (too early to say).
    /// </summary>
    public static PaceReading? Pace(SubscriptionUsageWindow window, DateTimeOffset now)
    {
        if (window.UsedPermille is not { } used || window.ResetsAt is not { } resets || window.DurationMins is not { } minutes
            || minutes <= 0 || resets <= now)
        {
            return null;
        }

        double length = minutes * 60.0;
        double elapsed = length - (resets - now).TotalSeconds;
        if (elapsed < length * 0.05) return null;
        double fraction = used / 1_000.0;
        if (fraction >= 1) return new PaceReading(now);
        if (fraction <= 0) return new PaceReading((DateTimeOffset?)null);
        DateTimeOffset runsOut = now.AddSeconds((1 - fraction) * elapsed / fraction);
        return runsOut < resets ? new PaceReading(runsOut) : new PaceReading((DateTimeOffset?)null);
    }

    /// <summary>
    /// "4d 21h", "3h 5m", "12m": the two largest units from the largest one
    /// present, zeros dropped, never under a minute (the Mac's abbreviated
    /// <c>DateComponentsFormatter</c> with two units).
    /// </summary>
    public static string Duration(DateTimeOffset now, DateTimeOffset date)
    {
        long minutes = (long)(Math.Max(60, (date - now).TotalSeconds) / 60);
        (long Value, string Unit)[] units = { (minutes / 1_440, "d"), (minutes / 60 % 24, "h"), (minutes % 60, "m") };
        int first = Array.FindIndex(units, u => u.Value > 0);
        return string.Join(" ", units.Skip(first).Take(2).Where(u => u.Value > 0)
            .Select(u => u.Value.ToString(CultureInfo.InvariantCulture) + u.Unit));
    }
}
