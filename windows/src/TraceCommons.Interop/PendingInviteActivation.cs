namespace TraceCommons.Interop;

/// <summary>UI-thread state for one pending credential; never performs enrollment.</summary>
public sealed class PendingInviteActivation
{
    private string? _latest;
    public void Receive(string? invite)
    {
        if (invite is not null) _latest = invite;
    }

    public void Clear() => _latest = null;

    public InviteActivationDecision Take(bool ready, bool needsRoots, bool daemonAvailable, bool onboardingOpen)
    {
        if (!ready || _latest is null) return new(null, null);
        if (needsRoots) return new(null, "Choose trace folders before opening this invite. Only the latest invite is kept.");
        if (!daemonAvailable)
        {
            Clear();
            return new(null, "This window cannot reach the daemon. Open the running app or restart, then open your invite again.");
        }
        if (onboardingOpen) return new(null, "Finish or close the current onboarding before opening the latest invite.");
        var invite = _latest;
        Clear();
        return new(invite, null);
    }
}

public sealed record InviteActivationDecision(string? Invite, string? Notice);
