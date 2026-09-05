namespace TraceCommons.Interop;

/// <summary>Navigation only; no route grants consent or changes daemon settings.</summary>
public enum HealthNavigationTarget
{
    None,
    Connect,
    Waiting,
    ExistingOnboarding,
}

public static class HealthNavigation
{
    public static HealthNavigationTarget ForLabel(string? label) => label switch
    {
        "not-logged-in" => HealthNavigationTarget.Connect,
        "queue-full" => HealthNavigationTarget.Waiting,
        // Keep the established disclosure flow until its shared consent
        // contract is revised. Navigation must not acknowledge it implicitly.
        "near-ai-notice-not-acknowledged" => HealthNavigationTarget.ExistingOnboarding,
        _ => HealthNavigationTarget.None,
    };
}
