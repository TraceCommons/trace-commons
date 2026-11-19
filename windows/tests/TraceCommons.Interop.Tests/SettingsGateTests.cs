using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Settings' onboarding gate against the Mac's <c>MonitorGate</c>
/// (MonitorNavigation.swift:300-312) and <c>availableBeforeOnboarding</c>
/// (SettingsSections.swift:52-60): Payouts and Referrals, and every other
/// section first run asks about, are not drawn before onboarding.
/// </summary>
public class SettingsGateTests
{
    [Theory]
    [InlineData(false, false, SettingsGate.Awaiting)]
    [InlineData(false, true, SettingsGate.Awaiting)]
    [InlineData(true, true, SettingsGate.SignedOut)]
    [InlineData(true, false, SettingsGate.Open)]
    public void TheWindowWaitsThenGatesOnOnboarding(bool known, bool requires, SettingsGate want)
    {
        Assert.Equal(want, SettingsGates.Of(known, requires));
    }

    [Theory]
    [InlineData(SettingsSection.Payouts, SettingsGate.SignedOut)]
    [InlineData(SettingsSection.Referrals, SettingsGate.SignedOut)]
    [InlineData(SettingsSection.Consent, SettingsGate.SignedOut)]
    [InlineData(SettingsSection.Witness, SettingsGate.SignedOut)]
    [InlineData(SettingsSection.Account, SettingsGate.Open)]
    [InlineData(SettingsSection.General, SettingsGate.Open)]
    [InlineData(SettingsSection.Compute, SettingsGate.Open)]
    public void OnlyTheSectionsFirstRunDoesNotAskWaitForIt(SettingsSection section, SettingsGate want)
    {
        Assert.Equal(want, SettingsGates.For(section, SettingsGate.SignedOut));
        Assert.Equal(SettingsGate.Open, SettingsGates.For(section, SettingsGate.Open));
    }
}
