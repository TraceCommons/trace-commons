using System;
using System.Threading.Tasks;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls;

/// <summary>
/// The Automatic path's two sheets, one at a time (SharingDisclosureSheet,
/// SharingDisclosures.swift:104-205): the scrub lines, then the route
/// disclosure. The first run's Start and the Traces toolbar's eye show the
/// same sheets, as the Mac's setup and its sharing choice do. Continue is
/// never the default button, so a stray Enter passes no disclosure; a
/// disclosure that cannot be read keeps Continue off. False when the
/// person cancelled.
/// </summary>
internal static class SharingDisclosureSheets
{
    public static async Task<bool> ShowAsync(
        XamlRoot root, SharingDisclosureFlow flow, AutomaticGrantCopy grant, FirstRunCopy copy,
        Func<Task<DaemonResponse>> routeDisclosure, Func<bool> closed, ElementTheme? theme = null,
        Func<ContentDialog, Task<ContentDialogResult>>? show = null)
    {
        show ??= dialog => dialog.ShowAsync().AsTask();
        var scrub = new StackPanel { Spacing = 10 };
        foreach (string line in grant.Lines)
        {
            scrub.Children.Add(new TextBlock { Text = line, TextWrapping = TextWrapping.Wrap });
        }

        if (await Sheet(root, copy, WatchCopy.Armed, scrub, true, theme, show) != ContentDialogResult.Primary || closed())
        {
            return false;
        }

        flow.AcknowledgeScrub();
        DaemonResponse facts = await routeDisclosure().ConfigureAwait(true);
        RouteDisclosure? disclosure = facts.IsError || facts.Result is null ? null : RouteDisclosureSurface.ForFacts(facts.Result.Value.GetRawText());
        RouteDisclosureUnreadable? unreadable = disclosure is null ? RouteDisclosureSurface.Unreadable() : null;
        var body = new StackPanel { Spacing = 8 };
        foreach (DisclosureRow row in RouteDisclosureSurface.PanelRows(disclosure, unreadable))
        {
            body.Children.Add(new TextBlock
            {
                Text = row.Value.Length > 0 ? row.Text + " " + row.Value : row.Text,
                TextWrapping = TextWrapping.Wrap,
                FontWeight = row.IsHeading ? FontWeights.SemiBold : FontWeights.Normal,
            });
        }

        string title = RouteDisclosureSurface.PanelTitle(disclosure, unreadable);
        if (await Sheet(root, copy, title.Length > 0 ? title : WatchCopy.Armed, body, disclosure is not null, theme, show) != ContentDialogResult.Primary || closed())
        {
            return false;
        }

        flow.AcknowledgeWitness(disclosure?.Facts.Witness?.SigningAddress);
        return flow.Step == DisclosureStep.Done;
    }

    private static async Task<ContentDialogResult> Sheet(XamlRoot root, FirstRunCopy copy, string title, UIElement body, bool canContinue, ElementTheme? theme,
        Func<ContentDialog, Task<ContentDialogResult>> show)
    {
        var dialog = new ContentDialog
        {
            Title = title,
            Content = new ScrollViewer { Content = body, MaxHeight = 420 },
            PrimaryButtonText = copy.Frame.ContinueButton,
            CloseButtonText = copy.Frame.Cancel,
            DefaultButton = ContentDialogButton.None,
            IsPrimaryButtonEnabled = canContinue,
            XamlRoot = root,
        };
        if (theme is ElementTheme requested)
        {
            dialog.RequestedTheme = requested;
        }

        return await show(dialog);
    }
}
