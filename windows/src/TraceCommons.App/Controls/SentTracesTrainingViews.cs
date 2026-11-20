using System;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// The three places the Mac offers Train coding models on traces already
/// sent (macos/Sources/TraceCommonsApp/Views/SentTracesTrainingViews.swift):
/// the one-time sheet, the quiet line under Home's Insights card, and the
/// section in a sent trace's inspector. Every word is the core's
/// (<see cref="SentTracesTrainingCopy"/>); nothing here sends a grant except
/// the two Allow handlers, which call the model.
/// </summary>
public static class SentTracesTrainingViews
{
    private static bool _sheetOpen;

    /// <summary>
    /// Shows the model's sheet in <paramref name="xamlRoot"/> while it is up
    /// for <paramref name="site"/> (<c>SentTracesTrainingSheetLayer</c>,
    /// :68-88). Not now and Allow carry equal weight: the same style, neither
    /// the default, neither on Enter. Escape and a dialog that cannot be shown
    /// are Not now: nothing is sent.
    /// </summary>
    public static async Task ShowSheetAsync(XamlRoot xamlRoot, SentTracesTrainingModel model, SentTracesTrainingSite site)
    {
        if (_sheetOpen || model.Copy is not { } copy || model.Sheet is not { } sheet || sheet.Site != site)
        {
            return;
        }

        _sheetOpen = true;
        try
        {
            var body = new StackPanel { Spacing = 12 };
            body.Children.Add(Text(copy.Body(sheet.Count), "TcBodyTextStyle", "TcInkPrimaryBrush"));
            body.Children.Add(Text(copy.Scope, "TcCaptionTextStyle", "TcInkSecondaryBrush"));
            TextBlock failed = Text(copy.Failed, "TcBodyTextStyle", "TcStatusOutsideTextBrush");
            failed.Visibility = Visibility.Collapsed;
            AutomationProperties.SetLiveSetting(failed, AutomationLiveSetting.Assertive);
            body.Children.Add(failed);

            var actions = SentTracesTrainingModel.Actions(copy, sheet.Count, busy: false);
            var dialog = new ContentDialog
            {
                XamlRoot = xamlRoot,
                Title = copy.Title,
                Content = body,
                PrimaryButtonText = actions[0].Title,
                SecondaryButtonText = actions[1].Title,
                DefaultButton = ContentDialogButton.None,
                PrimaryButtonStyle = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
                SecondaryButtonStyle = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
            };
            dialog.Opened += (_, _) => Name(dialog, actions[0].AccessibilityName, actions[1].AccessibilityName);

            // Allow: the grant runs with the sheet up; an answer closes it, a
            // refusal keeps it up and says so, a hiding refusal closes it.
            dialog.SecondaryButtonClick += async (_, args) =>
            {
                args.Cancel = true;
                ContentDialogButtonClickDeferral deferral = args.GetDeferral();
                dialog.IsPrimaryButtonEnabled = dialog.IsSecondaryButtonEnabled = false;
                try
                {
                    await model.AllowAllAsync();
                }
                finally
                {
                    deferral.Complete();
                }

                dialog.IsPrimaryButtonEnabled = dialog.IsSecondaryButtonEnabled = !model.Busy;
                failed.Visibility = model.SheetFailed ? Visibility.Visible : Visibility.Collapsed;
                if (model.Sheet is null)
                {
                    dialog.Hide();
                }
            };

            // While a grant runs neither choice can be taken, Escape included.
            dialog.Closing += (_, args) =>
            {
                if (model.Busy)
                {
                    args.Cancel = true;
                }
            };

            await DialogGuard.ShowOnceAsync(dialog);
        }
        finally
        {
            _sheetOpen = false;

            // Whatever closed it that was not an answer is Not now.
            model.NotNow();
        }
    }

    /// <summary>Names the dialog's two buttons with the core's accessibility names.</summary>
    private static void Name(ContentDialog dialog, string notNow, string allow)
    {
        foreach (Button button in Descendants<Button>(dialog))
        {
            if (Equals(button.Content, dialog.PrimaryButtonText))
            {
                AutomationProperties.SetName(button, notNow);
            }
            else if (Equals(button.Content, dialog.SecondaryButtonText))
            {
                AutomationProperties.SetName(button, allow);
            }
        }
    }

    private static System.Collections.Generic.IEnumerable<T> Descendants<T>(DependencyObject root)
        where T : DependencyObject
    {
        int count = VisualTreeHelper.GetChildrenCount(root);
        for (int i = 0; i < count; i++)
        {
            DependencyObject child = VisualTreeHelper.GetChild(root, i);
            if (child is T match)
            {
                yield return match;
            }

            foreach (T nested in Descendants<T>(child))
            {
                yield return nested;
            }
        }

    }

    /// <summary>
    /// The quiet line under Home's Insights card while sent traces lack the
    /// use, with Review (<c>SentTracesTrainingLine</c>, :95-127). Null while
    /// nothing is to be drawn.
    /// </summary>
    public static FrameworkElement? Line(SentTracesTrainingModel model, Action review)
    {
        if (model.Copy is not { } copy || model.State.Count is not ulong count)
        {
            return null;
        }

        var grid = new Grid { ColumnSpacing = 12 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        TextBlock line = Text(copy.Line(count), "TcCaptionTextStyle", "TcInkSecondaryBrush");
        line.VerticalAlignment = VerticalAlignment.Center;
        grid.Children.Add(line);
        var button = new HyperlinkButton { Content = copy.ReviewButton, VerticalAlignment = VerticalAlignment.Center };
        AutomationProperties.SetName(button, copy.ReviewAccessibilityName);
        button.Click += (_, _) => review();
        Grid.SetColumn(button, 1);
        grid.Children.Add(button);
        return new Border
        {
            Background = (Brush)Application.Current.Resources["TcSurfaceInsetBrush"],
            BorderBrush = (Brush)Application.Current.Resources["TcHairlineBrush"],
            BorderThickness = new Thickness(1),
            CornerRadius = (CornerRadius)Application.Current.Resources["TcCardCornerRadius"],
            Padding = new Thickness(14, 10, 14, 10),
            Child = grid,
        };
    }

    /// <summary>
    /// In a sent trace's inspector: the line and Allow while the trace lacks
    /// the use, Credit pending once allowed until its row shows credit, and
    /// the failed line beside Allow after a refused grant
    /// (<c>SentTraceTrainingSection</c>, :133-164). Null while nothing is drawn.
    /// </summary>
    public static FrameworkElement? TraceSection(SentTracesTrainingModel model, string submissionId, double? creditFinal, Action changed)
    {
        if (model.Copy is not { } copy)
        {
            return null;
        }

        SentTraceTrainingMark? mark = model.State.Mark(submissionId, SentTracesTrainingState.Credited(creditFinal));
        var panel = new StackPanel { Spacing = 8 };
        if (mark == SentTraceTrainingMark.Pending)
        {
            panel.Children.Add(new Border
            {
                BorderBrush = (Brush)Application.Current.Resources["TcInkSecondaryBrush"],
                BorderThickness = new Thickness(1),
                CornerRadius = new CornerRadius(10),
                Padding = new Thickness(8, 2, 8, 2),
                HorizontalAlignment = HorizontalAlignment.Left,
                Child = Text(copy.Pending, "TcCaptionSmallTextStyle", "TcInkSecondaryBrush"),
            });
            return panel;
        }

        if (!model.State.Offers(submissionId))
        {
            return null;
        }

        panel.Children.Add(Text(copy.TraceLine, "TcCaptionTextStyle", "TcInkSecondaryBrush"));
        if (mark == SentTraceTrainingMark.Failed)
        {
            panel.Children.Add(Text(copy.Failed, "TcBodyTextStyle", "TcStatusOutsideTextBrush"));
        }

        var allow = new Button
        {
            Content = copy.TraceButton,
            Style = (Style)Application.Current.Resources["TcSecondaryButtonStyle"],
            MinHeight = 44,
            IsEnabled = !model.Busy,
        };
        AutomationProperties.SetName(allow, copy.TraceAccessibilityName);
        allow.Click += async (_, _) =>
        {
            allow.IsEnabled = false;
            await model.AllowAsync(submissionId);
            changed();
        };
        panel.Children.Add(allow);
        return panel;
    }

    private static TextBlock Text(string text, string style, string brush) => new()
    {
        Text = text,
        Style = (Style)Application.Current.Resources[style],
        Foreground = (Brush)Application.Current.Resources[brush],
        TextWrapping = TextWrapping.Wrap,
    };
}
