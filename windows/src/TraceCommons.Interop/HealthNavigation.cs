namespace TraceCommons.Interop;

/// <summary>Navigation only; no route grants consent or changes daemon settings.</summary>
public enum HealthNavigationTarget
{
    None,
    Connect,
    Waiting,
}

public static class HealthNavigation
{
    public static HealthNavigationTarget ForLabel(string? label) => label switch
    {
        "not-logged-in" => HealthNavigationTarget.Connect,
        "queue-full" => HealthNavigationTarget.Waiting,
        _ => HealthNavigationTarget.None,
    };
}
