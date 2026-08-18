using System;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// Going public: the consent dialog from section 5.7 of the shared design
/// spec, assembled in code like <see cref="WithdrawDialog"/>.
///
/// Every word here comes from <see cref="PublicProfileCopy"/>, which mirrors
/// the Linux shell's <c>copy.rs</c>. Nothing in this file writes copy of its
/// own, and nothing in it may: this class chooses type and spacing, and the
/// interop layer chooses words.
/// </summary>
/// <remarks>
/// <para><b>The handle is inside the dialog, not behind it.</b> The thing
/// being consented to is this exact string becoming public, and a contributor
/// cannot meaningfully acknowledge "my handle becomes public" and then be
/// asked afterwards what the handle is.</para>
///
/// <para><b>The acknowledgement is a control, not decoration.</b> Nothing is
/// pre-checked and the primary stays disabled until the box is ticked AND
/// there is a handle to claim -- the same rule the footnote states in words
/// on the same screen. Going public is a consent action; the gate is the
/// consent.</para>
///
/// <para><b>A refusal keeps the dialog open.</b> The one thing a contributor
/// needs after "that handle is reserved" is the box they typed it into, so
/// the claim is made from inside the primary-button handler and a refusal
/// cancels the close rather than dismissing the screen and reporting the
/// problem somewhere else.</para>
/// </remarks>
public static class GoPublicDialog
{
    /// <summary>
    /// Offers the dialog and, if the contributor goes through with it, claims
    /// the handle.
    /// </summary>
    /// <returns>
    /// True when a handle was published. False when the dialog was closed
    /// without one, which is every other case -- including a refusal the
    /// contributor then gave up on.
    /// </returns>
    /// <remarks>
    /// Abandoning this dialog must leave the caller's control off. The
    /// control says whether a handle is on the roster, and closing without
    /// claiming has put none there; a control left on would be this window
    /// claiming a listing that does not exist.
    /// </remarks>
    public static async Task<bool> RunAsync(XamlRoot xamlRoot, PublicProfileViewModel viewModel)
    {
        ArgumentNullException.ThrowIfNull(xamlRoot);
        ArgumentNullException.ThrowIfNull(viewModel);

        var handle = new TextBox
        {
            Header = PublicProfileCopy.GoPublicHandleLabel,
            FontFamily = (FontFamily)Application.Current.Resources["TcMonoFontFamily"],
        };

        var bio = new TextBox
        {
            Header = PublicProfileCopy.GoPublicBioLabel,
            AcceptsReturn = true,
            TextWrapping = TextWrapping.Wrap,
            Height = 78,
        };

        var counter = new TextBlock
        {
            Text = PublicProfileCopy.BioCounter(string.Empty),
            HorizontalAlignment = HorizontalAlignment.Right,
            Style = (Style)Application.Current.Resources["TcCaptionSmallTextStyle"],
        };

        // The counter tracks the box rather than any value the dialog opened
        // with, and it refuses nothing: what happens at and above the limit is
        // the server's call, and this window does not pre-empt it.
        bio.TextChanged += (_, _) => counter.Text = PublicProfileCopy.BioCounter(bio.Text);

        // A refusal stays here, next to the field it is about. It starts
        // collapsed rather than blank so it takes no vertical space until
        // there is something to say.
        var refusal = new TextBlock
        {
            TextWrapping = TextWrapping.Wrap,
            Visibility = Visibility.Collapsed,
            Foreground = (Brush)Application.Current.Resources["TcCoralTextBrush"],
        };

        var acknowledgement = new CheckBox
        {
            Content = new TextBlock
            {
                Text = PublicProfileCopy.GoPublicAcknowledgement,
                TextWrapping = TextWrapping.Wrap,
            },
            IsChecked = false,
        };

        var dialog = new ContentDialog
        {
            XamlRoot = xamlRoot,
            Title = PublicProfileCopy.GoPublicTitle,
            PrimaryButtonText = PublicProfileCopy.GoPublicConfirm,
            CloseButtonText = PublicProfileCopy.NotNow,

            // Not going public is what Enter and Escape both do. This is a
            // consent action whose effect is outward-facing and immediate; it
            // does not get to be the thing a stray keypress commits.
            DefaultButton = ContentDialogButton.Close,
            IsPrimaryButtonEnabled = false,
        };

        void Regate() => dialog.IsPrimaryButtonEnabled =
            acknowledgement.IsChecked == true && handle.Text.Trim().Length > 0;

        acknowledgement.Checked += (_, _) => Regate();
        acknowledgement.Unchecked += (_, _) => Regate();
        handle.TextChanged += (_, _) => Regate();

        dialog.Content = Body(handle, bio, counter, refusal, acknowledgement);

        bool published = false;

        dialog.PrimaryButtonClick += async (_, args) =>
        {
            // Held open across the await, then cancelled unless the claim went
            // through. Without the deferral the dialog closes while the call
            // is still in flight and the contributor is told nothing.
            args.Cancel = true;
            ContentDialogButtonClickDeferral deferral = args.GetDeferral();

            try
            {
                string? label = await viewModel.ClaimAsync(handle.Text, bio.Text);
                if (label is null)
                {
                    published = true;
                    dialog.Hide();
                    return;
                }

                refusal.Text = PublicProfileCopy.FailureSentence(label);
                refusal.Visibility = Visibility.Visible;
            }
            finally
            {
                deferral.Complete();
            }
        };

        await dialog.ShowAsync();
        return published;
    }

    /// <summary>
    /// The dialog body: what is published, what never is, the two fields, a
    /// refusal slot, the acknowledgement and the footnote.
    /// </summary>
    /// <remarks>
    /// The two columns sit in one bordered frame split by a single hairline,
    /// because what is published and what never is are the same object seen
    /// from both sides rather than two separate claims.
    /// </remarks>
    private static FrameworkElement Body(
        TextBox handle,
        TextBox bio,
        TextBlock counter,
        TextBlock refusal,
        CheckBox acknowledgement)
    {
        var panel = new StackPanel { Spacing = 14, MinWidth = 460 };

        panel.Children.Add(new TextBlock
        {
            Text = PublicProfileCopy.GoPublicHeadline,
            TextWrapping = TextWrapping.Wrap,
            Style = (Style)Application.Current.Resources["TcTitleSectionTextStyle"],
        });

        var columns = new Grid
        {
            Padding = new Thickness(14),
            CornerRadius = new CornerRadius(8),
            Background = (Brush)Application.Current.Resources["TcSurfaceInsetBrush"],
            BorderBrush = (Brush)Application.Current.Resources["TcHairlineBrush"],
            BorderThickness = new Thickness(1),
        };
        columns.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        columns.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        columns.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });

        StackPanel published = Column(
            PublicProfileCopy.PublishedHeading,
            PublicProfileCopy.PublishedBody);
        published.Margin = new Thickness(0, 0, 14, 0);
        Grid.SetColumn(published, 0);
        columns.Children.Add(published);

        var hairline = new Rectangle
        {
            Width = 1,
            Fill = (Brush)Application.Current.Resources["TcHairlineDividerBrush"],
        };
        Grid.SetColumn(hairline, 1);
        columns.Children.Add(hairline);

        StackPanel never = Column(PublicProfileCopy.NeverHeading, PublicProfileCopy.NeverBody);
        never.Margin = new Thickness(14, 0, 0, 0);
        Grid.SetColumn(never, 2);
        columns.Children.Add(never);

        panel.Children.Add(columns);
        panel.Children.Add(handle);

        var bioGroup = new StackPanel { Spacing = 4 };
        bioGroup.Children.Add(bio);
        bioGroup.Children.Add(counter);
        panel.Children.Add(bioGroup);

        panel.Children.Add(refusal);
        panel.Children.Add(acknowledgement);

        panel.Children.Add(new TextBlock
        {
            Text = PublicProfileCopy.GoPublicFootnote,
            TextWrapping = TextWrapping.Wrap,
            Style = (Style)Application.Current.Resources["TcCaptionTextStyle"],
        });

        return new ScrollViewer
        {
            Content = panel,
            VerticalScrollBarVisibility = ScrollBarVisibility.Auto,
            MaxHeight = 520,
        };
    }

    private static StackPanel Column(string heading, string body)
    {
        var column = new StackPanel { Spacing = 6 };

        column.Children.Add(new TextBlock
        {
            Text = heading,
            TextWrapping = TextWrapping.Wrap,
            Style = (Style)Application.Current.Resources["TcEyebrowTextStyle"],
        });

        column.Children.Add(new TextBlock
        {
            Text = body,
            TextWrapping = TextWrapping.Wrap,
            Style = (Style)Application.Current.Resources["TcBodyDenseTextStyle"],
        });

        return column;
    }
}
