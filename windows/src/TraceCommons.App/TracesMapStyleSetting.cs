using System;
using System.IO;
using TraceCommons.Interop.Map;

namespace TraceCommons.App;

/// <summary>
/// The Traces map's style, a shell preference as on the Mac
/// (TracesMapStyle.swift: <c>traces_map_rocket</c>, the rocket by default
/// until it is turned off). Only the drawing changes, never what is sent.
/// Kept as one file beside the daemon's state directory; never written in a
/// sample launch, which writes nothing to this machine.
/// </summary>
internal static class TracesMapStyleSetting
{
    /// <summary>The Mac's key, used as the file's name.</summary>
    private const string Key = "traces_map_rocket";

    /// <summary>Raised with the new style when the switch changes.</summary>
    public static event Action<TracesMapStyle>? Changed;

    private static string PathOf() => System.IO.Path.Combine(DaemonHost.DefaultConfigDir(), "shell", Key);

    public static TracesMapStyle Load()
    {
        try
        {
            string path = PathOf();
            return File.Exists(path) && File.ReadAllText(path).Trim() == "false" ? TracesMapStyle.Standard : TracesMapStyle.Launch;
        }
        catch (Exception e) when (e is IOException or UnauthorizedAccessException)
        {
            return TracesMapStyle.Launch;
        }
    }

    public static void Save(TracesMapStyle style)
    {
        if (!Program.IsSampleLaunch)
        {
            try
            {
                string path = PathOf();
                Directory.CreateDirectory(System.IO.Path.GetDirectoryName(path)!);
                File.WriteAllText(path, style == TracesMapStyle.Launch ? "true" : "false");
            }
            catch (Exception e) when (e is IOException or UnauthorizedAccessException)
            {
                // The drawing still changes for this run.
            }
        }

        Changed?.Invoke(style);
    }
}
