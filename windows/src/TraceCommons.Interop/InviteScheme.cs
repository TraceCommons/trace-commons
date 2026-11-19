namespace TraceCommons.Interop;

/// <summary>
/// The <c>tracecommons</c> invite protocol as an unpackaged build registers
/// it under <c>HKCU\Software\Classes</c> (UrlSchemeRegistration.cs). The
/// packaged build declares it in the manifest's <c>uap:Protocol</c>.
/// </summary>
public static class InviteScheme
{
    /// <summary>The scheme; an internal id, kept (owner decision W10-D1 (a)).</summary>
    public const string Scheme = "tracecommons";

    /// <summary>
    /// The protocol's display name: the app name alone, from the core
    /// (<c>tc_app_name</c>; owner decision W10-D2 (a), 2026-11-19). The
    /// manifest's <c>uap:DisplayName</c> says the same, pinned by
    /// ManifestBrandTests.
    /// </summary>
    public static string DisplayName => Shell.MonitorWords.AppName;
}
