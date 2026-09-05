using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class HealthNavigationTests
{
    [Theory]
    [InlineData("not-logged-in", HealthNavigationTarget.Connect)]
    [InlineData("queue-full", HealthNavigationTarget.Waiting)]
    [InlineData("near-ai-notice-not-acknowledged", HealthNavigationTarget.ExistingOnboarding)]
    public void RecoveryActionsReachTheirIntendedExistingSurface(string label, HealthNavigationTarget target)
    {
        Assert.NotNull(HealthCopy.ForLabel(label)!.ActionLabel);
        Assert.Equal(target, HealthNavigation.ForLabel(label));
    }

    [Theory]
    [InlineData(null)]
    [InlineData("")]
    [InlineData("unknown-health")]
    [InlineData("ingest-unreachable")]
    [InlineData("privacy-filter-canary-failed")]
    public void UnknownAndAutomaticRecoveryConditionsCannotOpenEnrollment(string? label)
    {
        Assert.Equal(HealthNavigationTarget.None, HealthNavigation.ForLabel(label));
    }
}
