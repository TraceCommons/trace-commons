using System;
using System.Threading.Tasks;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// The withdrawal confirmation, built from
/// <see cref="WithdrawCopy.Confirmation"/> rather than from markup.
///
/// It is assembled in code because its shape is not fixed: an
/// <c>accepted</c> trace is shown TWO canonical bodies, one of them weighted,
/// and every other status is shown one. A XAML template would have to
/// hard-code a body count, which is exactly the assumption the contract warns
/// against -- "do not build a flow that assumes the tier is known before the
/// request".
///
/// Every word here comes from <see cref="WithdrawCopy"/>, which transcribes
/// the "Canonical confirmation copy" table in
/// <c>docs/contributor-daemon-ipc-v1_1.md</c>. Nothing in this file writes
/// copy of its own, and nothing in it may: this class chooses type and
/// spacing, and the interop layer chooses words.
/// </summary>
public static class WithdrawDialog
{
    /// <summary>
    /// Asks, and answers whether the contributor confirmed.
    /// </summary>
    /// <remarks>
    /// The stage comes from the record's LOCAL status, because that is all
    /// this machine has. The server computes <c>distribution_reach</c> during
    /// the withdrawal from live export membership, so what is shown here is
    /// the honest range of what the act might achieve rather than a single
    /// confident sentence this window cannot support.
    /// </remarks>
    /// <summary>
    /// Whether this dialog is already on screen.
    /// </summary>
    /// <remarks>
    /// Static because the dialog is: one window, one XamlRoot, one slot.
    /// See the comment at the ShowAsync call for why an unguarded second
    /// open is fatal rather than merely wrong.
    /// </remarks>
    private static bool _open;

    public static async Task<bool> ConfirmAsync(XamlRoot xamlRoot, WithdrawStage stage)
    {
        ArgumentNullException.ThrowIfNull(xamlRoot);

        WithdrawConfirmation confirmation = WithdrawCopy.Confirmation(stage);

        var panel = new StackPanel { Spacing = 10 };

        // Said first, and in this shell's own voice: two canonical bodies with
        // no explanation of why there are two would read as indecision rather
        // than as a fact about where the decision is made.
        if (confirmation.Ambiguity is string ambiguity)
        {
            panel.Children.Add(Paragraph(ambiguity, weighted: false));
        }

        for (int index = 0; index < confirmation.Bodies.Count; index++)
        {
            panel.Children.Add(Paragraph(
                confirmation.Bodies[index],
                weighted: confirmation.Gravest == index));
        }

        // Rule 3, on every tier: credit already recorded stays. Nothing in
        // this dialog implies withdrawal reverses it.
        panel.Children.Add(new TextBlock
        {
            Text = confirmation.Credit,
            TextWrapping = TextWrapping.Wrap,
        });

        var dialog = new ContentDialog
        {
            XamlRoot = xamlRoot,
            Title = confirmation.Question,
            Content = panel,
            PrimaryButtonText = confirmation.ConfirmLabel,
            CloseButtonText = WithdrawCopy.Cancel,

            // Keeping it, not withdrawing, is what Enter and Escape both do.
            // This is a deletion whose full reach the contributor has just
            // been told cannot be known in advance; it does not get to be the
            // thing a stray keypress commits.
            DefaultButton = ContentDialogButton.Close,
        };

        // WinUI permits ONE ContentDialog per XamlRoot, and every caller of
        // this method is an `async void` handler -- an `async Task` in the
        // middle contains nothing when the frame above it cannot observe the
        // task, so a throw here leaves the async void boundary unhandled and
        // takes the process with it. The window's caption button and the tray
        // menu both stay live behind app-modal content, so a second dialog is
        // one click away rather than a race.
        //
        // Refusing is the safe direction: `false` is "not confirmed", so a
        // withdrawal the contributor was never actually asked about cannot
        // happen. The dialog already on screen is the explanation for why
        // nothing opened, which is the platform's own modal convention rather
        // than a missing message.
        if (_open)
        {
            return false;
        }

        _open = true;
        try
        {
            return await dialog.ShowAsync() == ContentDialogResult.Primary;
        }
        catch (Exception)
        {
            return false;
        }
        finally
        {
            _open = false;
        }
    }

    /// <summary>
    /// One canonical body.
    /// </summary>
    /// <remarks>
    /// The weighted one is the body carrying the cannot-be-recalled clause.
    /// It is set in semibold on an inset panel so that a contributor
    /// skim-reading two similar-looking paragraphs lands on the one that
    /// describes the outcome they cannot undo. Weight and background both,
    /// because weight alone does not survive a high-contrast theme.
    /// </remarks>
    private static FrameworkElement Paragraph(string text, bool weighted)
    {
        var block = new TextBlock
        {
            Text = text,
            TextWrapping = TextWrapping.Wrap,
        };

        if (!weighted)
        {
            return block;
        }

        block.FontWeight = FontWeights.SemiBold;

        return new Border
        {
            Child = block,
            Padding = new Thickness(12),
            CornerRadius = new CornerRadius(8),
            Background = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TcSurfaceInsetBrush"],
            BorderBrush = (Microsoft.UI.Xaml.Media.Brush)Application.Current.Resources["TcGoldAttentionBorderBrush"],
            BorderThickness = new Thickness(1),
        };
    }
}
