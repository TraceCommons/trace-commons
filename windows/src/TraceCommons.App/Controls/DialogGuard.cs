using System;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Xaml.Controls;

namespace TraceCommons.App.Controls;

/// <summary>
/// Serializes ContentDialog display. WinUI permits one dialog per XamlRoot
/// and throws on a second ShowAsync, which takes the process with it.
///
/// Every ShowAsync in this app goes through here: MainWindow's four (quit,
/// withdraw, go-public, ignore-project) and OnboardingWindow's one. That last
/// one lives on a different XamlRoot and so could never have collided with
/// the others, but a guard that is only mostly used is a guard the next
/// person has to re-check before trusting, and #316's premise was already an
/// undercount once.
/// </summary>
/// <remarks>
/// One process-wide gate, held for as long as the dialog is on screen. That
/// makes a NESTED dialog -- one opened from inside another's handler, or
/// awaited from a running ShowAsync -- deadlock the whole app rather than
/// crash it, which is quieter and considerably harder to diagnose than what
/// it replaced. No call site nests today and none may start: a dialog that
/// needs a second dialog must close first and open the second from the
/// result. The gate is not per-XamlRoot for the same reason it exists at
/// all -- the cheap version that cannot be got subtly wrong is worth more
/// here than the precise one.
/// </remarks>
internal static class DialogGuard
{
    private static readonly SemaphoreSlim Gate = new(1, 1);

    /// <summary>
    /// Shows the dialog, waiting if another is already open. Returns
    /// <see cref="ContentDialogResult.None"/> if the dialog could not be
    /// shown at all — a caller must treat that as "the person did not
    /// confirm", never as consent.
    /// </summary>
    public static async Task<ContentDialogResult> ShowOnceAsync(ContentDialog dialog)
    {
        await Gate.WaitAsync().ConfigureAwait(true);
        try
        {
            return await dialog.ShowAsync();
        }
        catch (Exception)
        {
            // A dialog that cannot be shown must not be read as a yes.
            return ContentDialogResult.None;
        }
        finally
        {
            Gate.Release();
        }
    }
}
