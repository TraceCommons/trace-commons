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
    /// The three names Windows shows for the app: the package's
    /// <c>Properties/DisplayName</c>, the tile's <c>VisualElements</c>
    /// <c>DisplayName</c>, and the startup task's, which Settings &gt; Apps &gt;
    /// Startup lists. The invite protocol's <c>uap:DisplayName</c> is not the
    /// app's name and waits on owner decision W10-D2.
    /// </summary>
    [Fact]
    public void EveryAppNameTheManifestShowsIsTheCores()
    {
        XDocument manifest = Manifest();
        string package = manifest.Descendants().Single(e => e.Name.LocalName == "Properties")
            .Elements().Single(e => e.Name.LocalName == "DisplayName").Value;
        string tile = manifest.Descendants().Single(e => e.Name.LocalName == "VisualElements")
            .Attribute("DisplayName")!.Value;
        string startup = manifest.Descendants().Single(e => e.Name.LocalName == "StartupTask")
            .Attribute("DisplayName")!.Value;

        Assert.Equal(MonitorWords.AppName, package);
        Assert.Equal(MonitorWords.AppName, tile);
        Assert.Equal(MonitorWords.AppName, startup);
    }
}
