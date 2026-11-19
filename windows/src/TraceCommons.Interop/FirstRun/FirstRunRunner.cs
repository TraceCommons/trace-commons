using System;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop.FirstRun;

/// <summary>What <c>invite_lookup</c> answered (macos/Sources/TraceCommonsApp/FirstRunRunner.swift:8-12).</summary>
public abstract record FirstRunLookup
{
    public sealed record Found(string Json) : FirstRunLookup;
    public sealed record Refused(string Label) : FirstRunLookup;
    public sealed record Unavailable : FirstRunLookup;
}

/// <summary>What <c>grant_automatic</c> answered; Refused carries the daemon's label.</summary>
public readonly record struct GrantAnswer(bool Granted, string? Label);

/// <summary>What <c>near_ai_account_enroll</c> answered.</summary>
public readonly record struct NearAIEnrolment(bool Enrolled, string? Label);

/// <summary>How a browser sign-in ended (FirstRunPlan.swift:113-117).</summary>
public enum NearAILoginOutcome
{
    SignedIn,
    Cancelled,
    Failed,
}

/// <summary>The terms the person read and accepted: the version and hash read, and when.</summary>
public sealed record AcceptedTerms(string Version, string ContentSha256, DateTimeOffset AcceptedAt);

/// <summary>
/// The daemon calls a first run makes, one per <see cref="FirstRunCall"/>
/// (FirstRunRunner.swift:33-80). Each answers whether the daemon confirmed
/// it; nothing here carries a sentence.
/// </summary>
public interface IFirstRunDaemon
{
    Task<bool> StartDaemon(string settingsJson);
    Task<bool> SetSourceSettings(string settingsJson);
    Task<FirstRunLookup> LookupInvite(string invite);
    Task<bool> EnrollInvite(string invite);
    Task<bool> SignInNearAI();
    Task<NearAILoginOutcome> NearAILoginOutcome(CancellationToken cancel);
    Task<bool> CancelNearAILogin();
    Task<NearAIEnrolment> EnrollNearAI(string? referralCode);
    Task<bool> SetAskEachTime();
    Task<bool> SaveConsentScopes(System.Collections.Generic.IReadOnlyList<string> scopes);
    Task<bool> SetProjectMode(string projectId, ProjectRule mode);
    Task<bool> IncludePastSessions(string projectId, System.Collections.Generic.IReadOnlyList<string> sessionIds);
    Task<bool> SetPrivateAI(bool on);
    Task<GrantAnswer> GrantAutomatic(string? witness);
    Task<bool> MarkComplete();
    Task<bool> MarkWatchOnlyComplete();

    /// <summary><c>terms_accept</c>: true only once the daemon took it.</summary>
    Task<bool> RecordTermsAcceptance(AcceptedTerms terms);

    /// <summary>The first run is finished; <paramref name="notice"/> is a refused grant or a carried refusal, or null.</summary>
    void FirstRunFinished(FirstRunFailure? notice);
}

/// <summary>Why a commit stopped (FirstRunRunner.swift:84-117). Labels are the daemon's own.</summary>
public abstract record FirstRunFailure
{
    public sealed record StartFailed : FirstRunFailure;
    public sealed record SettingsFailed : FirstRunFailure;
    public sealed record InviteDead(string Label) : FirstRunFailure;
    public sealed record LookupUnavailable : FirstRunFailure;
    public sealed record EnrollFailed : FirstRunFailure;
    public sealed record SignInFailed : FirstRunFailure;
    public sealed record NearAIEnrollFailed(string Label) : FirstRunFailure;
    public sealed record ScopesFailed : FirstRunFailure;
    public sealed record SharingFailed : FirstRunFailure;
    public sealed record RulesFailed : FirstRunFailure;
    public sealed record PrivateAIFailed : FirstRunFailure;
    public sealed record GrantRefused(string Label) : FirstRunFailure;
    public sealed record CompleteFailed : FirstRunFailure;
}

/// <summary>
/// Executes <see cref="FirstRunPlan"/>'s calls in order and records what the
/// daemon now holds back into the state (FirstRunRunner.swift:125-434).
/// The terms are handed to the started daemon once per first run.
/// </summary>
public sealed class FirstRunRunner
{
    private readonly IFirstRunDaemon _daemon;
    private CancellationTokenSource? _login;
    private Task<bool>? _loginCancel;
    private FirstRunFailure? _carriedRefusal;

    public FirstRunRunner(FirstRunState state, IFirstRunDaemon daemon)
    {
        State = state;
        _daemon = daemon;
    }

    /// <summary>Raised after the state, a failure, or the committing flag changes.</summary>
    public event Action? Changed;

    public FirstRunState State { get; set; }

    /// <summary>Accepted on the Terms step; kept through Back; handed over once.</summary>
    public AcceptedTerms? AcceptedTerms { get; set; }

    public bool TermsHandedOver { get; private set; }

    public FirstRunFailure? Failure { get; set; }

    public FirstRunLookup.Found? Lookup { get; private set; }

    public bool IsCommitting { get; private set; }

    public CommitPoint? Committing { get; private set; }

    public bool SignInWaiting { get; private set; }

    public string ReferralCode { get; set; } = "";

    public bool Completed { get; private set; }

    public FirstRunFailure? RefusedGrant { get; private set; }

    /// <summary>The Terms step is left only with the loaded document accepted (FirstRunRunner.swift:193-197).</summary>
    public bool HasAcceptedTerms(bool hasLegalCopy, string? loadedVersion, string? loadedSha256) =>
        hasLegalCopy && AcceptedTerms is { } a && loadedVersion is not null && loadedSha256 is not null
        && a.Version == loadedVersion && a.ContentSha256 == loadedSha256;

    public void ContinueFromWelcome()
    {
        if (State.Step == FirstRunStep.Welcome)
        {
            Set(FirstRunNavigation.Next(State));
        }
    }

    public void ContinueFromTerms(bool hasLegalCopy, string? loadedVersion, string? loadedSha256)
    {
        if (State.Step == FirstRunStep.Terms && HasAcceptedTerms(hasLegalCopy, loadedVersion, loadedSha256))
        {
            Set(FirstRunNavigation.Next(State));
        }
    }

    public static bool DeclaresRoots(CommitPoint point) => point != CommitPoint.Start;

    /// <summary>Run the calls for <paramref name="point"/> (FirstRunRunner.swift:224-251).</summary>
    public async Task Commit(CommitPoint point, FirstRunFailure? carrying = null)
    {
        if (IsCommitting)
        {
            return;
        }

        IsCommitting = true;
        Committing = point;
        Failure = null;
        RefusedGrant = null;
        _carriedRefusal = carrying;
        Changed?.Invoke();
        try
        {
            if (DeclaresRoots(point) && State.SessionRoots.SettingsJson() is null)
            {
                Failure = new FirstRunFailure.StartFailed();
                return;
            }

            if (State.DaemonStarted)
            {
                await HandOverTerms();
            }

            foreach (FirstRunCall call in FirstRunPlan.Calls(State, point))
            {
                if (!await Run(call))
                {
                    return;
                }
            }

            if (point == CommitPoint.LeaveTools)
            {
                State = FirstRunNavigation.Next(State);
            }
        }
        finally
        {
            _carriedRefusal = null;
            IsCommitting = false;
            Committing = null;
            Changed?.Invoke();
        }
    }

    /// <summary>Cancel on Join while the browser sign-in waits (FirstRunRunner.swift:385-393).</summary>
    public async Task CancelSignIn()
    {
        if (!SignInWaiting || _loginCancel is not null || _login is null)
        {
            return;
        }

        SignInWaiting = false;
        _loginCancel = _daemon.CancelNearAILogin();
        _login.Cancel();
        Changed?.Invoke();
        await _loginCancel;
    }

    private async Task HandOverTerms()
    {
        if (TermsHandedOver || AcceptedTerms is null)
        {
            return;
        }

        TermsHandedOver = true;
        // A refusal does not fail the first run: the daemon's own check and
        // the terms hold ask again when it matters.
        await _daemon.RecordTermsAcceptance(AcceptedTerms);
    }

    private async Task<bool> Run(FirstRunCall call)
    {
        switch (call)
        {
            case FirstRunCall.StartDaemon s:
                if (!await _daemon.StartDaemon(s.SettingsJson))
                {
                    return Fail(new FirstRunFailure.StartFailed());
                }

                State.DaemonStarted = true;
                State.StartedSettingsJson = s.SettingsJson;
                await HandOverTerms();
                break;
            case FirstRunCall.SetSourceSettings c:
                if (!await _daemon.SetSourceSettings(c.SettingsJson))
                {
                    return Fail(new FirstRunFailure.SettingsFailed());
                }

                State.StartedSettingsJson = State.SessionRoots.SettingsJson();
                break;
            case FirstRunCall.LookupInvite l:
                switch (await _daemon.LookupInvite(l.Invite))
                {
                    case FirstRunLookup.Found found:
                        Lookup = found;
                        break;
                    case FirstRunLookup.Refused refused:
                        Lookup = null;
                        State = FirstRunNavigation.ReturnToJoinAfterDeadInvite(State);
                        return Fail(new FirstRunFailure.InviteDead(refused.Label));
                    default:
                        Lookup = null;
                        return Fail(new FirstRunFailure.LookupUnavailable());
                }

                break;
            case FirstRunCall.Enroll e:
                if (!await _daemon.EnrollInvite(e.Invite))
                {
                    return Fail(new FirstRunFailure.EnrollFailed());
                }

                State.EnrolledInvite = e.Invite;
                State.SignedOutOfEnrolment = false;
                break;
            case FirstRunCall.SignInNearAI:
                if (!await _daemon.SignInNearAI())
                {
                    State = FirstRunNavigation.ReturnToJoinAfterNearAIFailure(State);
                    return Fail(new FirstRunFailure.SignInFailed());
                }

                State.SignedIn = true;
                break;
            case FirstRunCall.NearAILogin:
                {
                    _login = new CancellationTokenSource();
                    SignInWaiting = true;
                    Changed?.Invoke();
                    NearAILoginOutcome outcome = await _daemon.NearAILoginOutcome(_login.Token);
                    SignInWaiting = false;
                    _login = null;
                    if (_loginCancel is { } cancel)
                    {
                        _loginCancel = null;
                        if (!await cancel)
                        {
                            Failure = new FirstRunFailure.SignInFailed();
                        }

                        return false;
                    }

                    if (outcome == FirstRun.NearAILoginOutcome.Cancelled)
                    {
                        return false;
                    }

                    if (outcome != FirstRun.NearAILoginOutcome.SignedIn)
                    {
                        State = FirstRunNavigation.ReturnToJoinAfterNearAIFailure(State);
                        return Fail(new FirstRunFailure.SignInFailed());
                    }

                    break;
                }

            case FirstRunCall.EnrollNearAI:
                {
                    string trimmed = ReferralCode.Trim();
                    NearAIEnrolment answer = await _daemon.EnrollNearAI(trimmed.Length == 0 ? null : trimmed);
                    if (!answer.Enrolled)
                    {
                        State = FirstRunNavigation.ReturnToJoinAfterNearAIFailure(State);
                        return Fail(new FirstRunFailure.NearAIEnrollFailed(answer.Label ?? ""));
                    }

                    State.NearAIEnrolled = true;
                    State.SignedIn = true;
                    State.SignedOutOfEnrolment = false;
                    break;
                }

            case FirstRunCall.SetAskEachTime:
                if (!await _daemon.SetAskEachTime())
                {
                    return Fail(new FirstRunFailure.SharingFailed());
                }

                break;
            case FirstRunCall.SetConsentScopes s:
                if (!await _daemon.SaveConsentScopes(s.Scopes))
                {
                    return Fail(new FirstRunFailure.ScopesFailed());
                }

                break;
            case FirstRunCall.SetProjectMode m:
                if (!await _daemon.SetProjectMode(m.ProjectId, m.Mode))
                {
                    return Fail(new FirstRunFailure.RulesFailed());
                }

                break;
            case FirstRunCall.IncludePastSessions p:
                if (!await _daemon.IncludePastSessions(p.ProjectId, p.Sessions))
                {
                    return Fail(new FirstRunFailure.RulesFailed());
                }

                break;
            case FirstRunCall.SetPrivateAI p:
                if (!await _daemon.SetPrivateAI(p.On))
                {
                    return Fail(new FirstRunFailure.PrivateAIFailed());
                }

                break;
            case FirstRunCall.GrantAutomatic g:
                {
                    // The one call that does not stop the run: a refused
                    // grant finishes on Ask, and says so.
                    GrantAnswer answer = await _daemon.GrantAutomatic(g.Witness);
                    if (!answer.Granted)
                    {
                        if (!await _daemon.SetAskEachTime())
                        {
                            return Fail(new FirstRunFailure.SharingFailed());
                        }

                        State.Sharing = SharingPath.AskMe;
                        Failure = new FirstRunFailure.GrantRefused(answer.Label ?? "");
                        RefusedGrant = Failure;
                    }

                    break;
                }

            case FirstRunCall.MarkComplete:
                if (!await _daemon.MarkComplete())
                {
                    return Fail(new FirstRunFailure.CompleteFailed());
                }

                Finish();
                break;
            case FirstRunCall.MarkWatchOnlyComplete:
                if (!await _daemon.MarkWatchOnlyComplete())
                {
                    return Fail(new FirstRunFailure.CompleteFailed());
                }

                Finish();
                break;
        }

        return true;
    }

    private void Finish()
    {
        Completed = true;
        _daemon.FirstRunFinished(RefusedGrant ?? _carriedRefusal);
    }

    private bool Fail(FirstRunFailure reason)
    {
        Failure = reason;
        return false;
    }

    private void Set(FirstRunState state)
    {
        State = state;
        Changed?.Invoke();
    }
}
