using System;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// Opens the first run at one step for screenshots, in a Debug recorded-sample
/// launch only (the app reads it under <c>#if DEBUG</c> and only when
/// <c>TRACE_COMMONS_SAMPLE</c> is set). The counterpart of the Mac's
/// <c>FirstRunRunner(state: FirstRunState(step:), daemon: RecordingFirstRunDaemon())</c>
/// render tests: every step can be seen without a daemon, an account, the
/// terms or this machine's onboarding state.
/// </summary>
public static class FirstRunPreview
{
    /// <summary>The step to open at: a <see cref="FirstRunStep"/> name, any case.</summary>
    public const string EnvironmentVariable = "TRACE_COMMONS_FIRST_RUN_STEP";

    /// <summary>
    /// The named step, or null for anything else (unset, blank, a number, an
    /// unknown name): a typo opens no first run rather than a guessed one.
    /// </summary>
    public static FirstRunStep? StepFrom(string? value)
    {
        if (string.IsNullOrWhiteSpace(value))
        {
            return null;
        }

        foreach (FirstRunStep step in Enum.GetValues<FirstRunStep>())
        {
            if (string.Equals(step.ToString(), value.Trim(), StringComparison.OrdinalIgnoreCase))
            {
                return step;
            }
        }

        return null;
    }
}

/// <summary>
/// A first-run daemon that refuses every call and records its name, so a
/// preview can never start a daemon, enroll, accept terms, grant sharing,
/// open a browser or mark this machine's first run done.
/// </summary>
public sealed class RefusingFirstRunDaemon : IFirstRunDaemon
{
    private readonly List<string> _calls = new();

    /// <summary>Every call made, in order.</summary>
    public IReadOnlyList<string> Calls => _calls;

    private Task<bool> No(string name)
    {
        _calls.Add(name);
        return Task.FromResult(false);
    }

    public Task<bool> StartDaemon(string settingsJson) => No(nameof(StartDaemon));
    public Task<bool> SetSourceSettings(string settingsJson) => No(nameof(SetSourceSettings));

    public Task<FirstRunLookup> LookupInvite(string invite)
    {
        _calls.Add(nameof(LookupInvite));
        return Task.FromResult<FirstRunLookup>(new FirstRunLookup.Unavailable());
    }

    public Task<bool> EnrollInvite(string invite) => No(nameof(EnrollInvite));
    public Task<bool> SignInNearAI() => No(nameof(SignInNearAI));

    public Task<NearAILoginOutcome> NearAILoginOutcome(CancellationToken cancel)
    {
        _calls.Add(nameof(NearAILoginOutcome));
        return Task.FromResult(FirstRun.NearAILoginOutcome.Failed);
    }

    public Task<bool> CancelNearAILogin() => No(nameof(CancelNearAILogin));

    public Task<NearAIEnrolment> EnrollNearAI(string? referralCode)
    {
        _calls.Add(nameof(EnrollNearAI));
        return Task.FromResult(new NearAIEnrolment(false, null));
    }

    public Task<bool> SetAskEachTime() => No(nameof(SetAskEachTime));
    public Task<bool> SaveConsentScopes(IReadOnlyList<string> scopes) => No(nameof(SaveConsentScopes));
    public Task<bool> SetProjectMode(string projectId, ProjectRule mode) => No(nameof(SetProjectMode));
    public Task<bool> IncludePastSessions(string projectId, IReadOnlyList<string> sessionIds) => No(nameof(IncludePastSessions));
    public Task<bool> SetPrivateAI(bool on) => No(nameof(SetPrivateAI));

    public Task<GrantAnswer> GrantAutomatic(string? witness)
    {
        _calls.Add(nameof(GrantAutomatic));
        return Task.FromResult(new GrantAnswer(false, null));
    }

    public Task<bool> MarkComplete() => No(nameof(MarkComplete));
    public Task<bool> MarkWatchOnlyComplete() => No(nameof(MarkWatchOnlyComplete));
    public Task<bool> RecordTermsAcceptance(AcceptedTerms terms) => No(nameof(RecordTermsAcceptance));

    public void FirstRunFinished(FirstRunFailure? notice) => _calls.Add(nameof(FirstRunFinished));
}
