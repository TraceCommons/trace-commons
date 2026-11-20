using System.Collections.Generic;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop.Shell;
using Windows.Foundation;

namespace TraceCommons.App;

/// <summary>
/// The controls the notice stack covers (#2275). The stack is drawn over the
/// top of the tab and pushes nothing down, as on the Mac, so a control under
/// the front card cannot be reached by the pointer. While it is covered it is
/// taken out of the Tab order and the UI Automation tree as well, and it comes
/// back as the card goes, shrinks, or the tab scrolls it out from under. On
/// Private AI that is the guide card's X under the offer notice's X: only the
/// notice's is reachable while both are drawn.
/// </summary>
public sealed partial class MainWindow
{
    /// <summary>Each control taken out, with the tab stop and UIA view it had.</summary>
    private readonly Dictionary<Control, (bool TabStop, AccessibilityView View)> _coveredByNotice = new();

    /// <summary>The scrolling regions under the card, watched so a scroll re-reads the cover.</summary>
    private readonly HashSet<ScrollViewer> _coverScrollers = new();

    private bool _coverQueued;
    private DispatcherQueueTimer? _coverLayoutTimer;

    /// <summary>
    /// Re-reads the cover once layout has settled. Coalesced: many calls in
    /// one turn read once.
    /// </summary>
    private void UpdateNoticeCover()
    {
        if (_coverLayoutTimer is null && TabContent is not null)
        {
            // A tab's content arriving or reflowing under a card that stays.
            // Layout passes come every frame while anything animates, so this
            // path reads at most four times a second, and only while a card
            // is up or something is still taken out.
            _coverLayoutTimer = DispatcherQueue.CreateTimer();
            _coverLayoutTimer.Interval = System.TimeSpan.FromMilliseconds(250);
            _coverLayoutTimer.IsRepeating = false;
            _coverLayoutTimer.Tick += (_, _) => UpdateNoticeCover();
            TabContent.LayoutUpdated += (_, _) =>
            {
                if (!_coverLayoutTimer.IsRunning
                    && (_coveredByNotice.Count > 0 || NoticeStackHost.Visibility == Visibility.Visible))
                {
                    _coverLayoutTimer.Start();
                }
            };
        }

        if (_coverQueued)
        {
            return;
        }

        _coverQueued = true;
        DispatcherQueue.TryEnqueue(DispatcherQueuePriority.Low, () =>
        {
            _coverQueued = false;
            ApplyNoticeCover();
        });
    }

    private void ApplyNoticeCover()
    {
        var covered = new List<Control>();
        if (NoticeStackHost.Visibility == Visibility.Visible
            && NoticeStackHost.IsHitTestVisible
            && NoticeFront.ActualWidth > 0
            && NoticeFront.ActualHeight > 0
            && Content?.XamlRoot is not null)
        {
            CoverRect card = Bounds(NoticeFront);
            var host = new Rect(card.X, card.Y, card.Width, card.Height);
            foreach (UIElement element in VisualTreeHelper.FindElementsInHostCoordinates(host, TabContent, includeAllElements: true))
            {
                if (Within(element, NoticeStackHost))
                {
                    continue;
                }

                if (element is ScrollViewer scroller && _coverScrollers.Add(scroller))
                {
                    scroller.ViewChanged += (_, _) => UpdateNoticeCover();
                }

                if (element is Control control
                    && control.Visibility == Visibility.Visible
                    && (control.IsTabStop || _coveredByNotice.ContainsKey(control))
                    && NoticeCover.Covers(card, Bounds(control)))
                {
                    covered.Add(control);
                }
            }
        }

        NoticeCoverChange<Control> change = NoticeCover.Change(_coveredByNotice.Keys, covered);
        foreach (Control control in change.Restore)
        {
            (bool tabStop, AccessibilityView view) = _coveredByNotice[control];
            _coveredByNotice.Remove(control);
            control.IsTabStop = tabStop;
            AutomationProperties.SetAccessibilityView(control, view);
        }

        foreach (Control control in change.Hide)
        {
            _coveredByNotice[control] = (control.IsTabStop, AutomationProperties.GetAccessibilityView(control));
            control.IsTabStop = false;
            AutomationProperties.SetAccessibilityView(control, AccessibilityView.Raw);
        }
    }

    /// <summary>An element's rectangle in window coordinates.</summary>
    private static CoverRect Bounds(FrameworkElement element)
    {
        Rect r = element.TransformToVisual(null).TransformBounds(new Rect(0, 0, element.ActualWidth, element.ActualHeight));
        return new CoverRect(r.X, r.Y, r.Width, r.Height);
    }

    private static bool Within(DependencyObject element, DependencyObject ancestor)
    {
        for (DependencyObject? at = element; at is not null; at = VisualTreeHelper.GetParent(at))
        {
            if (ReferenceEquals(at, ancestor))
            {
                return true;
            }
        }

        return false;
    }
}
