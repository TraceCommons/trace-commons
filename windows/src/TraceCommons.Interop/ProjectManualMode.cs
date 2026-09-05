namespace TraceCommons.Interop;

/// <summary>
/// The existing Settings action may restore manual review or ignore a project.
/// It never grants automatic-upload consent, including for unknown modes.
/// </summary>
public static class ProjectManualMode
{
    public static string? Next(string? currentMode) => currentMode switch
    {
        "auto_upload" or "ignore" => "ask",
        "ask" => "ignore",
        _ => null,
    };
}
