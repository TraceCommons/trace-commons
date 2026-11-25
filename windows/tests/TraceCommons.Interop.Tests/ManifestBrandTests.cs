using System;
using System.IO;
using System.Linq;
using System.Xml.Linq;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The MSIX manifest names the app with the core's name (W10, checklist J1):
/// <c>brand::APP_NAME</c>, read through <c>tc_app_name</c>, which is
/// "Near AI Gateway" (crates/trace-commons-contributor/src/brand.rs:28-35).
/// The manifest cannot read the core at install time, so this test is what
/// keeps the two together.
/// </summary>
public class ManifestBrandTests
{
    private static XDocument Manifest() =>
        XDocument.Load(Path.Combine(AppContext.BaseDirectory, "manifest-source", "Package.appxmanifest.txt"));

    [Fact]
    public void TheCoresAppNameIsNearAIGateway()
    {
        // brand.rs:30, the macro's one literal.
        Assert.Equal("Near AI Gateway", MonitorWords.AppName);
    }

    /// <summary>
    /// The four names Windows shows for the app: the package's
    /// <c>Properties/DisplayName</c>, the tile's <c>VisualElements</c>
    /// <c>DisplayName</c>, the startup task's, which Settings &gt; Apps &gt;
    /// Startup lists, and the invite protocol's <c>uap:DisplayName</c>, which
    /// is the app name alone (owner decision W10-D2 (a), 2026-11-19).
    /// </summary>
    [Fact]
    public void EveryAppNameTheManifestShowsIsTheCores()
    {
        XDocument manifest = Manifest();
        string package = manifest.Descendants().Single(e => e.Name.LocalName == "Properties")
            .Elements().Single(e => e.Name.LocalName == "DisplayName").Value;
        XElement app = manifest.Descendants().Single(e => e.Name.LocalName == "Application" && (string?)e.Attribute("Id") == "App");
        string tile = app.Elements().Single(e => e.Name.LocalName == "VisualElements")
            .Attribute("DisplayName")!.Value;
        string startup = app.Descendants().Single(e => e.Name.LocalName == "StartupTask")
            .Attribute("DisplayName")!.Value;

        Assert.Equal(MonitorWords.AppName, package);
        Assert.Equal(MonitorWords.AppName, tile);
        Assert.Equal(MonitorWords.AppName, startup);

        string invite = app.Descendants().Single(e => e.Name.LocalName == "Protocol")
            .Elements().Single(e => e.Name.LocalName == "DisplayName").Value;
        Assert.Equal(MonitorWords.AppName, invite);
    }

    /// <summary>
    /// The unpackaged build registers the same protocol in HKCU, and its
    /// display name is the app name alone too (W10-D2 (a)).
    /// </summary>
    [Fact]
    public void TheUnpackagedInviteProtocolIsNamedForTheApp()
    {
        Assert.Equal(MonitorWords.AppName, InviteScheme.DisplayName);
    }

    /// <summary>
    /// The exe's version resource: <c>FileDescription</c> (from
    /// <c>AssemblyTitle</c>) is what Settings &gt; Apps &gt;
    /// Startup shows for an unpackaged build, and <c>ProductName</c> (from
    /// <c>Product</c>) is beside it in the file's properties. Both are the
    /// core's name; without them the SDK writes the assembly name,
    /// "TraceCommons" (checklist J1).
    /// </summary>
    [Fact]
    public void TheExesVersionResourceNamesTheApp()
    {
        XDocument project = XDocument.Load(
            Path.Combine(AppContext.BaseDirectory, "manifest-source", "TraceCommons.App.csproj.txt"));
        string? Property(string name) =>
            project.Descendants().SingleOrDefault(e => e.Name.LocalName == name)?.Value;

        Assert.Equal(MonitorWords.AppName, Property("AssemblyTitle"));
        Assert.Equal(MonitorWords.AppName, Property("Product"));
    }

    /// <summary>
    /// The exe's <c>CompanyName</c>, which Settings &gt; Apps &gt; Startup
    /// shows as the publisher beside the name. Unset, the SDK writes the
    /// assembly name, and it showed "TraceCommons" under "Near AI Gateway"
    /// (tc-win-az11, 2026-11-21). It is the publisher the packaged build
    /// shows, the manifest's <c>PublisherDisplayName</c>.
    /// </summary>
    [Fact]
    public void TheExesPublisherIsThePackagesNotTheAssemblyName()
    {
        XDocument project = XDocument.Load(
            Path.Combine(AppContext.BaseDirectory, "manifest-source", "TraceCommons.App.csproj.txt"));
        string? company = project.Descendants().SingleOrDefault(e => e.Name.LocalName == "Company")?.Value;
        string? publisher = Manifest().Descendants().Single(e => e.Name.LocalName == "PublisherDisplayName").Value;

        Assert.False(string.IsNullOrWhiteSpace(publisher));
        Assert.Equal(publisher, company);
        Assert.NotEqual("TraceCommons", company);
    }
}
