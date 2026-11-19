namespace TraceCommons.Interop.Shell;

/// <summary>
/// What a Settings section draws in place of its content (<c>MonitorGate</c>,
/// MonitorNavigation.swift:287-312): nothing yet while the core has not said
/// whether onboarding is required, the onboarding notice while it is, and
/// the content otherwise. A section available before onboarding (R-43) is
/// never gated; Payouts draws nothing at all unless open.
/// </summary>
public enum SettingsGate
{
    /// <summary>The core has not said whether onboarding is required.</summary>
    Awaiting,
    /// <summary>Onboarding is required: the notice whose button opens first run.</summary>
    SignedOut,
    /// <summary>Nothing in the way.</summary>
    Open,
}

public static class SettingsGates
{
    /// <summary>The window's gate (<c>MonitorGate.of</c>, less the refused daemon, which the Monitor says).</summary>
    public static SettingsGate Of(bool onboardingKnown, bool requiresOnboarding) =>
        !onboardingKnown ? SettingsGate.Awaiting : requiresOnboarding ? SettingsGate.SignedOut : SettingsGate.Open;

    /// <summary>One section's gate (<c>forSettings</c>).</summary>
    public static SettingsGate For(SettingsSection section, SettingsGate window) =>
        SettingsSections.AvailableBeforeOnboarding(section) ? SettingsGate.Open : window;
}
