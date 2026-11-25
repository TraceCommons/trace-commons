using System;
using System.Collections.Generic;
using System.Globalization;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop.FirstRun;

/// <summary>One block of the published terms (macos/Sources/TCShellCore/TermsDocument.swift:18-50).</summary>
public sealed record TermsBlock(string Kind, string? Text, IReadOnlyList<string>? Items, IReadOnlyList<string>? Header, IReadOnlyList<IReadOnlyList<string>>? Rows);

/// <summary>The published terms and the hash of the bytes served (<c>FetchedTerms</c>, TermsDocument.swift:58-77).</summary>
public sealed record FetchedTerms(string Version, string Effective, bool InEffect, string Title, IReadOnlyList<TermsBlock> Blocks, string ContentSha256)
{
    private static readonly HashSet<string> Kinds = new(StringComparer.Ordinal) { "heading", "paragraph", "list", "table", "callout" };

    /// <summary>Decode <c>tc_terms_document_json</c>'s reply; null for anything the Mac would refuse, an unknown block kind included.</summary>
    public static FetchedTerms? Decode(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            Wire? w = JsonSerializer.Deserialize<Wire>(json);
            if (w?.Document is not { } d || w.ContentSha256 is not { } sha || d.Version is null || d.Effective is null || d.Title is null || d.Blocks is null)
            {
                return null;
            }

            var blocks = new List<TermsBlock>();
            foreach (WireBlock b in d.Blocks)
            {
                if (b.Kind is null || !Kinds.Contains(b.Kind))
                {
                    return null;
                }

                bool ok = b.Kind switch
                {
                    "list" => b.Items is not null,
                    "table" => b.Header is not null && b.Rows is not null,
                    _ => b.Text is not null,
                };
                if (!ok)
                {
                    return null;
                }

                blocks.Add(new TermsBlock(b.Kind, b.Text, b.Items, b.Header, b.Rows?.Select(r => (IReadOnlyList<string>)r).ToList()));
            }

            return new FetchedTerms(d.Version, d.Effective, d.InEffect, d.Title, blocks, sha);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>Fetch the published terms (blocking: call off the UI thread). Null and the core's label on failure.</summary>
    public static (FetchedTerms? Terms, string? Label) Fetch()
    {
        IntPtr raw = NativeMethods.tc_terms_document_json(out IntPtr err);
        string? label = NativeMethods.TakeOwnedString(err);
        string? json = NativeMethods.TakeOwnedString(raw);
        return Resolve(json, label);
    }

    /// <summary>
    /// The terms and the failure label for what the core returned. A document
    /// that arrives but does not decode (a block kind this shell does not know,
    /// a missing field) is a failure with its own label, so the reader says so
    /// and offers Retry instead of saying "Loading" for good.
    /// </summary>
    internal static (FetchedTerms? Terms, string? Label) Resolve(string? json, string? label)
    {
        FetchedTerms? terms = Decode(json);
        if (json is null)
        {
            return (null, label ?? "terms-unavailable");
        }

        return terms is null ? (null, "terms-unreadable") : (terms, null);
    }

    private sealed class Wire
    {
        [JsonPropertyName("document")] public WireDocument? Document { get; set; }
        [JsonPropertyName("content_sha256")] public string? ContentSha256 { get; set; }
    }

    private sealed class WireDocument
    {
        [JsonPropertyName("version")] public string? Version { get; set; }
        [JsonPropertyName("effective")] public string? Effective { get; set; }
        [JsonPropertyName("in_effect")] public bool InEffect { get; set; }
        [JsonPropertyName("title")] public string? Title { get; set; }
        [JsonPropertyName("blocks")] public List<WireBlock>? Blocks { get; set; }
    }

    private sealed class WireBlock
    {
        [JsonPropertyName("kind")] public string? Kind { get; set; }
        [JsonPropertyName("text")] public string? Text { get; set; }
        [JsonPropertyName("items")] public List<string>? Items { get; set; }
        [JsonPropertyName("header")] public List<string>? Header { get; set; }
        [JsonPropertyName("rows")] public List<List<string>>? Rows { get; set; }
    }
}

/// <summary>The Traces line under the count, worded by the core (<c>tc_first_run_estimate_line</c>).</summary>
public static class FirstRunEstimate
{
    /// <summary>The request the export reads: <c>{"traces": n, "earns_credit": b}</c>, each optional.</summary>
    public static string Request(ulong? traces, bool earnsCredit)
    {
        var o = new SortedDictionary<string, object>(StringComparer.Ordinal) { ["earns_credit"] = earnsCredit };
        if (traces is ulong t)
        {
            o["traces"] = t;
        }

        return JsonSerializer.Serialize(o);
    }

    /// <summary>The line, or null when the core draws none.</summary>
    public static string? Line(ulong? traces, bool earnsCredit) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_first_run_estimate_line(Request(traces, earnsCredit)));
}

/// <summary>
/// The first run's daemon calls over <c>tc_call</c>, each the method and
/// parameters the Mac sends (macos/Sources/TraceCommonsApp/AppModel.swift:4774-5160,
/// DaemonClient.swift, DaemonClient+FirstRun.swift, DaemonClient+NativeIdentity.swift).
/// The shell supplies the transport, the daemon start and the completion
/// markers it keeps on this computer.
/// </summary>
public sealed class FirstRunDaemon : IFirstRunDaemon
{
    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private readonly Func<string, Task<bool>> _start;
    private readonly Func<bool> _running;
    private readonly Func<Task<bool>> _markComplete;
    private readonly Func<Task<bool>> _markWatchOnly;
    private readonly Action<FirstRunFailure?> _finished;
    private readonly Func<Uri, Task<bool>> _openBrowser;

    /// <param name="call">Sends one method with its JSON parameters.</param>
    /// <param name="start">Starts the daemon with a settings declaration; true once it runs.</param>
    /// <param name="running">Whether a daemon runs now.</param>
    /// <param name="markComplete">Writes the enrolled completion marker, after reading status for its tenant.</param>
    /// <param name="markWatchOnly">Writes the watch-only marker, refused while the daemon is logged in.</param>
    /// <param name="finished">The first run finished; what it must still say.</param>
    /// <param name="openBrowser">Opens the sign-in page; true once it opened.</param>
    public FirstRunDaemon(
        Func<string, string, Task<DaemonResponse>> call, Func<string, Task<bool>> start, Func<bool> running,
        Func<Task<bool>> markComplete, Func<Task<bool>> markWatchOnly, Action<FirstRunFailure?> finished,
        Func<Uri, Task<bool>> openBrowser)
    {
        _openBrowser = openBrowser;
        _call = call;
        _start = start;
        _running = running;
        _markComplete = markComplete;
        _markWatchOnly = markWatchOnly;
        _finished = finished;
    }

    private static string Json(object o) => JsonSerializer.Serialize(o);

    private async Task<DaemonResponse?> Call(string method, object? args = null)
    {
        try
        {
            return await _call(method, args is null ? "{}" : Json(args)).ConfigureAwait(true);
        }
        catch (Exception e) when (e is TcException or InvalidOperationException or JsonException)
        {
            return null;
        }
    }

    private async Task<bool> Ok(string method, object? args = null) => await Call(method, args).ConfigureAwait(true) is { IsError: false };

    /// <summary>Already running: the declaration goes through <c>set_settings</c> (AppModel.swift:4774-4787).</summary>
    public async Task<bool> StartDaemon(string settingsJson) =>
        _running() ? await SetSourceSettings(settingsJson).ConfigureAwait(true) : await _start(settingsJson).ConfigureAwait(true);

    public async Task<bool> SetSourceSettings(string settingsJson)
    {
        try
        {
            DaemonResponse r = await _call(DaemonProtocol.Methods.SetSettings, settingsJson).ConfigureAwait(true);
            return !r.IsError;
        }
        catch (Exception e) when (e is TcException or InvalidOperationException)
        {
            return false;
        }
    }

    /// <summary>Lookup failures that say nothing about the invite (AppModel.swift:4835-4837).</summary>
    public static readonly IReadOnlySet<string> TransientLookupLabels = new HashSet<string>(StringComparer.Ordinal)
    {
        "invite-lookup-unavailable", "unparseable-response", "missing-result",
    };

    public async Task<FirstRunLookup> LookupInvite(string invite)
    {
        DaemonResponse? r = await Call("invite_lookup", new { code = invite }).ConfigureAwait(true);
        if (r is null)
        {
            return new FirstRunLookup.Unavailable();
        }

        if (r.Error is { } error)
        {
            return error.Message.Length > 0 && !TransientLookupLabels.Contains(error.Message)
                ? new FirstRunLookup.Refused(error.Message)
                : new FirstRunLookup.Unavailable();
        }

        if (r.Result is not JsonElement result || result.ValueKind != JsonValueKind.Object)
        {
            return new FirstRunLookup.Unavailable();
        }

        bool valid = result.TryGetProperty("valid", out JsonElement v) && v.ValueKind == JsonValueKind.True;
        if (valid)
        {
            return new FirstRunLookup.Found(result.GetRawText());
        }

        string label = result.TryGetProperty("reason_label", out JsonElement l) && l.ValueKind == JsonValueKind.String ? l.GetString()! : "invite-invalid";
        return new FirstRunLookup.Refused(label);
    }

    /// <summary>
    /// <c>enroll</c> with no scopes key: the first run sends its scopes at
    /// Start (AppModel.swift:4839-4842, DaemonClient.swift:787-790).
    /// </summary>
    public Task<bool> EnrollInvite(string invite) => Ok(DaemonProtocol.Methods.Enroll, new { invite });

    public async Task<bool> SignInNearAI()
    {
        DaemonResponse? r = await Call("account_sign_in").ConfigureAwait(true);
        return r is { IsError: false, Result: JsonElement res } && res.ValueKind == JsonValueKind.Object
            && res.TryGetProperty("signed_in", out JsonElement s) && s.ValueKind == JsonValueKind.True;
    }

    /// <summary>
    /// The near.ai login (AppModel.swift:4863-4935): signed in at once when
    /// the daemon keeps a session; else <c>near_ai_credential_start</c>, the
    /// shell opens its <c>browser_url</c> (a page that does not open cancels
    /// the attempt), and the attempt is polled by its id until it ends,
    /// within <c>NearAILoginPoll.limit</c> (600 s, FirstRunPlan.swift:57-60),
    /// read by <c>NearAILoginPoll.verdict</c> (:63-73).
    /// </summary>
    public async Task<NearAILoginOutcome> NearAILoginOutcome(CancellationToken cancel)
    {
        _attemptId = null;
        return await OpenNearAILoginAsync(Call2, _openBrowser, id => _attemptId = id, fresh: false, cancel);
    }

    /// <summary>Opens and waits for the NEAR AI credential ceremony used by first run and Account.</summary>
    public static async Task<NearAILoginOutcome> OpenNearAILoginAsync(
        Func<string, string, Task<DaemonResponse?>> call,
        Func<Uri, Task<bool>> openBrowser,
        Action<string?> rememberAttempt,
        bool fresh,
        CancellationToken cancel)
    {
        if (!fresh && await Verdict(call, null, fresh).ConfigureAwait(true) == LoginPoll.SignedIn)
        {
            return FirstRun.NearAILoginOutcome.SignedIn;
        }

        DaemonResponse? started = await call(DaemonProtocol.Methods.NearAiCredentialStart, NearAiCredentialSurface.StartRequest).ConfigureAwait(true);
        if (started is not { IsError: false, Result: JsonElement res }
            || NearAiCredentialSurface.ParseStart(res.GetRawText()) is not { } attempt
            || !Uri.TryCreate(attempt.BrowserUrl, UriKind.Absolute, out Uri? url))
        {
            return FirstRun.NearAILoginOutcome.Failed;
        }

        bool opened;
        try
        {
            opened = !cancel.IsCancellationRequested && await openBrowser(url).ConfigureAwait(true);
        }
        catch (Exception e) when (e is InvalidOperationException or UnauthorizedAccessException or System.Runtime.InteropServices.COMException)
        {
            opened = false;
        }

        if (!opened)
        {
            // A page that did not open cancels the attempt; a cancel the
            // daemon refuses still ends this sign-in as failed, never throws
            // out of the commit.
            await call(DaemonProtocol.Methods.NearAiCredentialCancel, NearAiCredentialSurface.SerializeCancel(attempt.AttemptId)).ConfigureAwait(true);
            return FirstRun.NearAILoginOutcome.Failed;
        }

        rememberAttempt(attempt.AttemptId);

        DateTime until = DateTime.UtcNow.AddSeconds(600);
        while (DateTime.UtcNow < until)
        {
            try
            {
                await Task.Delay(1000, cancel).ConfigureAwait(true);
            }
            catch (TaskCanceledException)
            {
                return FirstRun.NearAILoginOutcome.Failed;
            }

            switch (await Verdict(call, attempt.AttemptId, fresh).ConfigureAwait(true))
            {
                case LoginPoll.SignedIn:
                    return FirstRun.NearAILoginOutcome.SignedIn;
                case LoginPoll.Cancelled:
                    return FirstRun.NearAILoginOutcome.Cancelled;
                case LoginPoll.Ended:
                    return FirstRun.NearAILoginOutcome.Failed;
            }
        }

        return FirstRun.NearAILoginOutcome.Failed;
    }

    private enum LoginPoll
    {
        SignedIn,
        Waiting,
        Cancelled,
        Ended,
    }

    /// <summary><c>NearAILoginPoll.verdict</c> (FirstRunPlan.swift:63-73), read through the shared parser.</summary>
    private static async Task<LoginPoll> Verdict(Func<string, string, Task<DaemonResponse?>> call, string? attemptId, bool fresh)
    {
        DaemonResponse? r = await call(DaemonProtocol.Methods.NearAiCredentialStatus, NearAiCredentialSurface.SerializeStatus(attemptId)).ConfigureAwait(true);
        if (r is not { IsError: false, Result: JsonElement res })
        {
            return LoginPoll.Waiting;
        }

        NearAiCredentialStatus status = NearAiCredentialSurface.ParseStatus(res.GetRawText());
        if (fresh ? status.AttemptStatus == "complete" : status.SessionState == NearAiEnrollSurface.CredentialStatePresent)
        {
            return LoginPoll.SignedIn;
        }

        return status.AttemptStatus switch
        {
            null => LoginPoll.Waiting,
            "waiting_for_browser" => LoginPoll.Waiting,
            "cancelled" => LoginPoll.Cancelled,
            _ => LoginPoll.Ended,
        };
    }

    private async Task<DaemonResponse?> Call2(string method, string json)
    {
        try
        {
            return await _call(method, json).ConfigureAwait(true);
        }
        catch (Exception e) when (e is TcException or InvalidOperationException)
        {
            return null;
        }
    }

    private string? _attemptId;

    /// <summary>
    /// Cancel names the attempt this sign-in started, as the Mac names
    /// <c>firstRunLoginAttemptID</c> (AppModel.swift); before one started,
    /// the daemon cancels whatever runs.
    /// </summary>
    public async Task<bool> CancelNearAILogin()
    {
        DaemonResponse? r = await Call2(DaemonProtocol.Methods.NearAiCredentialCancel, NearAiCredentialSurface.SerializeCancel(_attemptId)).ConfigureAwait(true);
        return r is { IsError: false };
    }

    /// <summary>
    /// The account a grant change belongs to, settled before it is sent
    /// (<c>settleAutomaticGrantOwner</c>, AppModel.swift:4701-4704): the
    /// status's <c>account_scope</c>, or null when none is settled.
    /// </summary>
    private async Task<string?> AccountScope()
    {
        DaemonResponse? r = await Call(DaemonProtocol.Methods.Status).ConfigureAwait(true);
        return r is { IsError: false } ? r.ResultAs<DaemonStatus>()?.AccountScope : null;
    }

    /// <summary>
    /// A grant change under the Mac's guards (AppModel.swift:4994-5000,
    /// :5062-5070): refused unless an account is settled first, and its
    /// answer refused if the account changed while it ran.
    /// </summary>
    private async Task<DaemonResponse?> GuardedGrantCall(string method, object args)
    {
        if (await AccountScope().ConfigureAwait(true) is not { } owner)
        {
            return null;
        }

        DaemonResponse? r = await Call(method, args).ConfigureAwait(true);
        return await AccountScope().ConfigureAwait(true) == owner ? r : null;
    }

    public async Task<NearAIEnrolment> EnrollNearAI(string? referralCode)
    {
        DaemonResponse? r = referralCode is null
            ? await Call("near_ai_account_enroll").ConfigureAwait(true)
            : await Call("near_ai_account_enroll", new { referral_code = referralCode }).ConfigureAwait(true);
        if (r is { Error: { } e })
        {
            return new NearAIEnrolment(false, e.Message.Length == 0 ? "near_ai_enroll_unavailable" : e.Message);
        }

        bool enrolled = r is { Result: JsonElement res } && res.ValueKind == JsonValueKind.Object
            && res.TryGetProperty("enrolled", out JsonElement en) && en.ValueKind == JsonValueKind.True;
        return enrolled ? new NearAIEnrolment(true, null) : new NearAIEnrolment(false, "near_ai_enroll_unavailable");
    }

    /// <summary><c>withdraw_automatic_grant</c> returning folders to Ask (DaemonClient.swift:363-367).</summary>
    public async Task<bool> SetAskEachTime() =>
        await GuardedGrantCall("withdraw_automatic_grant", new { return_folders_to_ask = true }).ConfigureAwait(true) is { IsError: false };

    /// <summary>
    /// A consent write the daemon confirmed: the scopes before it (the
    /// status read just before) and the scopes its reply confirms. The main
    /// window offers Train coding models on traces already sent when the
    /// write turned it on (<c>saveConsentScopes</c>, AppModel.swift:5049-5058).
    /// Nothing is reported for a refusal, a reply without scopes, or a status
    /// that could not be read: nothing is offered on a guess.
    /// </summary>
    public event Action<IReadOnlyList<string>, IReadOnlyList<string>>? ConsentScopesConfirmed;

    public async Task<bool> SaveConsentScopes(IReadOnlyList<string> scopes)
    {
        DaemonResponse? status = await Call(DaemonProtocol.Methods.Status).ConfigureAwait(true);
        IReadOnlyList<string>? before = status is { IsError: false } ? status.ResultAs<DaemonStatus>()?.ConsentScopes : null;
        DaemonResponse? r = await Call(DaemonProtocol.Methods.SetConsentScopes, new { scopes }).ConfigureAwait(true);
        if (r is not { IsError: false })
        {
            return false;
        }

        if (before is not null && SentTracesTrainingState.ConfirmedScopes(r) is { } confirmed)
        {
            ConsentScopesConfirmed?.Invoke(before, confirmed);
        }

        return true;
    }

    public static string ModeWord(ProjectRule mode) => mode switch
    {
        ProjectRule.AutoUpload => "auto_upload",
        ProjectRule.Ignore => "ignore",
        _ => "ask",
    };

    public Task<bool> SetProjectMode(string projectId, ProjectRule mode) =>
        Ok(DaemonProtocol.Methods.SetProjectMode, new { project_id = projectId, mode = ModeWord(mode) });

    public Task<bool> IncludePastSessions(string projectId, IReadOnlyList<string> sessionIds) =>
        Ok("include_past_sessions", new { project_id = projectId, session_ids = sessionIds });

    /// <summary>
    /// Private AI through <c>set_settings</c>; confirmed only when the echo
    /// says so (DaemonClient.swift:546-552). Never sent by this first run
    /// unless the state turns it on, which the step does not (#1685).
    /// </summary>
    public async Task<bool> SetPrivateAI(bool on)
    {
        DaemonResponse? r = await Call(DaemonProtocol.Methods.SetSettings, new { private_inference = on }).ConfigureAwait(true);
        return r is { IsError: false, Result: JsonElement res } && res.ValueKind == JsonValueKind.Object
            && res.TryGetProperty("private_inference", out JsonElement p) && (p.ValueKind == JsonValueKind.True) == on
            && p.ValueKind is JsonValueKind.True or JsonValueKind.False;
    }

    /// <summary>
    /// <c>grant_automatic_including_existing</c>, the witness sent as null
    /// rather than left out (DaemonClient+FirstRun.swift:48-60). A grant
    /// that does not include existing traces is refused.
    /// </summary>
    public Task<GrantAnswer> GrantAutomatic(string? witness) => GrantAutomatic(witness, keepFolderAsks: true);

    /// <summary>
    /// The grant from the Traces toolbar (as from the Mac's Settings sharing
    /// choice, AppModel.swift:5108-5113): <paramref name="keepFolderAsks"/>
    /// false makes every folder but a Never one Automatic.
    /// </summary>

    public async Task<GrantAnswer> GrantAutomatic(string? witness, bool keepFolderAsks)
    {
        DaemonResponse? r = await GuardedGrantCall("grant_automatic_including_existing", new Dictionary<string, object?>
        {
            ["confirmed"] = true,
            ["witness_signing_address"] = witness,
            ["include_existing"] = true,
            ["keep_folder_asks"] = keepFolderAsks,
        }).ConfigureAwait(true);
        if (r is null)
        {
            return new GrantAnswer(false, "automatic-grant-unavailable");
        }

        if (r is { Error: { } e })
        {
            return new GrantAnswer(false, e.Message.Length == 0 ? "automatic-grant-unavailable" : e.Message);
        }

        if (r is { Result: JsonElement res } && res.ValueKind == JsonValueKind.Object
            && res.TryGetProperty("granted", out JsonElement g) && g.ValueKind == JsonValueKind.True)
        {
            bool existing = res.TryGetProperty("include_existing", out JsonElement ie) && ie.ValueKind == JsonValueKind.True;
            return existing ? new GrantAnswer(true, null) : new GrantAnswer(false, "automatic-existing-traces-unsupported");
        }

        return new GrantAnswer(false, "automatic-grant-unavailable");
    }

    public Task<bool> MarkComplete() => _markComplete();

    public Task<bool> MarkWatchOnlyComplete() => _markWatchOnly();

    /// <summary><c>terms_accept</c> with the version, hash and UTC time read (TermsDocument.swift:118-127).</summary>
    public Task<bool> RecordTermsAcceptance(AcceptedTerms terms) => Ok("terms_accept", TermsParams(terms));

    public static Dictionary<string, string> TermsParams(AcceptedTerms terms) => new(StringComparer.Ordinal)
    {
        ["version"] = terms.Version,
        ["content_sha256"] = terms.ContentSha256,
        ["accepted_at"] = terms.AcceptedAt.ToUniversalTime().ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture),
    };

    public void FirstRunFinished(FirstRunFailure? notice) => _finished(notice);
}
