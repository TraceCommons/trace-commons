using System;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// No unnamed Tab stops on the Traces and Private AI tabs (checklist E2).
/// An ItemsControl or ContentControl is a tab stop unless told otherwise,
/// and the box's Tab walk (2026-11-19, a focus log naming each stop's type)
/// landed on these with no name: the certificate list on Traces, and the
/// managed sessions host and the harness list on Private AI.
/// </summary>
public class TabStopTests
{
    [Theory]
    [InlineData("TraceCommons.App/MainWindow.xaml", "ItemsControl", "ViewModel.CertificateHeld")]
    [InlineData("TraceCommons.App/Controls/PrivateInferenceView.xaml", "ContentControl", "x:Name=\"ManagedContent\"")]
    [InlineData("TraceCommons.App/Controls/PrivateInferenceView.xaml", "ItemsControl", "ViewModel.Harnesses")]
    public void ContainersTabPastThemselves(string file, string element, string marker)
    {
        string markup = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, "shell-source", file + ".txt"));
        Match tag = Regex.Match(markup, "<" + element + @"\b[^>]*" + Regex.Escape(marker) + "[^>]*>");
        Assert.True(tag.Success, $"no <{element}> with {marker} in {file}");
        Assert.Contains("IsTabStop=\"False\"", tag.Value, StringComparison.Ordinal);
    }
}
