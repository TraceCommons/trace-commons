using TraceCommons.Interop;
using Xunit;

public sealed class OnboardingCopyTests
{
    [Fact]
    public void SharedWelcomeAndDoneRespectConfiguredSourcesAndCadence()
    {
        var copy = Assert.IsType<OnboardingCopy>(OnboardingCopy.Load());
        Assert.Contains("according to your source settings", copy.WelcomeBody);
        Assert.Contains("sessions to review", copy.DoneBody);
        Assert.DoesNotContain("4 hours", copy.DoneBody);
        Assert.Contains("never submit", copy.NotificationPurpose);
        Assert.NotEqual(copy.NotificationAllowed, copy.NotificationUnknown);
    }
}
