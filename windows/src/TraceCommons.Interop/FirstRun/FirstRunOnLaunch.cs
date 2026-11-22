namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// Whether the shell's first activation also opens first run. The shell lands
/// on Insights and starts the daemon quietly; first run is otherwise reached
/// only when a contribution screen is opened, so a person who has never set
/// the app up saw an empty Insights ("0 traces") and no way in until they
/// clicked something that happened to need the daemon. A device that has
/// finished first run before keeps the quiet landing.
/// </summary>
public static class FirstRunOnLaunch
{
    /// <summary>True when first run has never been finished on this device.</summary>
    public static bool Wanted(bool finishedBefore) => !finishedBefore;
}
