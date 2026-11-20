namespace TraceCommons.Interop.Shell;

/// <summary>
/// What a Settings section draws in place of its content (<c>MonitorGate</c>,
/// MonitorNavigation.swift:287-312): nothing yet while the core has not said
/// whether onboarding is required, the core's down title when the daemon
/// refused to start, the onboarding notice while it is required, and the
/// content otherwise. A section available before onboarding (R-43) is
/// never gated; Payouts draws nothing at all unless open.
/// </summary>
public enum SettingsGate
{
    /// <summary>The core has not said whether onboarding is required.</summary>
    Awaiting,
    /// <summary>Onboarding is required: the notice whose button opens first run.</summary>
    SignedOut,
    /// <summary>The daemon refused to start: the core's down title, for anyone, onboarded or not.</summary>
    Down,
    /// <summary>Nothing in the way.</summary>
    Open,
}

public static class SettingsGates
{
    /// <summary>The window's gate (<c>MonitorGate.of</c>, less the refused daemon, which the Monitor says).</summary>
    public static SettingsGate Of(bool onboardingKnown, bool requiresOnboarding) =>
        !onboardingKnown ? SettingsGate.Awaiting : requiresOnboarding ? SettingsGate.SignedOut : SettingsGate.Open;

    /// <summary>
    /// The window's gate over the status reads' (<c>MonitorGate.of</c>): a
    /// daemon that refused to start is down whatever the reads said, for
    /// anyone, onboarded or not. A failed read over a running daemon is not
    /// a refusal and stays signed out (fail closed), as on the Mac.
    /// </summary>
    public static SettingsGate Window(bool startRefused, SettingsGate read) =>
        startRefused ? SettingsGate.Down : read;

    /// <summary>One section's gate (<c>forSettings</c>).</summary>
    public static SettingsGate For(SettingsSection section, SettingsGate window) =>
        SettingsSections.AvailableBeforeOnboarding(section) ? SettingsGate.Open : window;
}

/// <summary>
/// Reads the window's gate, newest read wins. Each read takes a ticket; a
/// reply that lands after a newer read began is dropped, so an older
/// "set up" answer can never reopen sections a newer "needs onboarding"
/// answer closed. A failed read keeps the last answered status, as the
/// Mac's model keeps its <c>status</c>; with no answer yet, the Mac's
/// placeholder status requires onboarding, so a first failed read closes
/// the gated sections (fail closed; MonitorNavigation.swift:275-283).
/// </summary>
public sealed class SettingsGateReader
{
    private readonly System.Func<System.Threading.Tasks.Task<bool?>> _requiresOnboarding;
    private long _ticket;
    private bool _answered;

    /// <param name="requiresOnboarding">
    /// One status read: true while onboarding is required, false once set
    /// up, null when the read failed.
    /// </param>
    public SettingsGateReader(System.Func<System.Threading.Tasks.Task<bool?>> requiresOnboarding)
    {
        _requiresOnboarding = requiresOnboarding;
    }

    /// <summary>The gate the last applied read gave; awaiting until one answers.</summary>
    public SettingsGate Gate { get; private set; } = SettingsGate.Awaiting;

    /// <summary>
    /// Reads once. True when this read changed <see cref="Gate"/>; false when
    /// it failed, was superseded by a newer read, or changed nothing.
    /// </summary>
    public async System.Threading.Tasks.Task<bool> ReadAsync()
    {
        long ticket = ++_ticket;
        bool? requires;
        try
        {
            requires = await _requiresOnboarding().ConfigureAwait(true);
        }
        catch (System.Exception)
        {
            requires = null;
        }

        if (ticket != _ticket)
        {
            return false;
        }

        bool value;
        if (requires is { } answered)
        {
            value = answered;
            _answered = true;
        }
        else if (!_answered)
        {
            value = true;
        }
        else
        {
            return false;
        }

        SettingsGate gate = SettingsGates.Of(onboardingKnown: true, requiresOnboarding: value);
        if (gate == Gate)
        {
            return false;
        }

        Gate = gate;
        return true;
    }
}
