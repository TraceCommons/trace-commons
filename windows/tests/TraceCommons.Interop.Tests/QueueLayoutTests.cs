using System;
using System.IO;
using System.Linq;
using System.Xml.Linq;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Traces pane's queue grid. On the test box the certificate section's
/// heading and empty sentence were drawn over the first folder row: both
/// sat in the same grid row as the folder list, and both are shown at once.
/// </summary>
public class QueueLayoutTests
{
    private static readonly XNamespace Xaml = "http://schemas.microsoft.com/winfx/2006/xaml/presentation";
    private static readonly XNamespace X = "http://schemas.microsoft.com/winfx/2006/xaml";

    private static XDocument MainWindow() =>
        XDocument.Load(Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App", "MainWindow.xaml.txt"));

    private static string Row(XElement e) => (string?)e.Attribute("Grid.Row") ?? "0";

    [Fact]
    public void TheCertificateSectionHasARowOfItsOwnBesideTheFolderList()
    {
        XDocument doc = MainWindow();
        XElement list = doc.Descendants(Xaml + "ListView").Single(e => (string?)e.Attribute(X + "Name") == "QueueListView");
        XElement section = doc.Descendants(Xaml + "StackPanel").Single(e =>
            e.Elements(Xaml + "TextBlock").Any(t => ((string?)t.Attribute("Text") ?? "").Contains("ViewModel.CertificateHeldTitle", StringComparison.Ordinal)));
        Assert.Same(list.Parent, section.Parent);
        Assert.NotEqual(Row(list), Row(section));
        // Its row holds nothing else, so nothing else draws over it.
        Assert.Single(section.Parent!.Elements(), e => Row(e) == Row(section));
    }
}
