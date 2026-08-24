using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Xaml.Controls;

namespace TraceCommons.App.Controls;

/// <summary>
/// Serializes ContentDialog display. WinUI permits one dialog per XamlRoot
/// and throws on a second ShowAsync, which takes the process with it. Every
/// dialog in this app goes through here.
/// </summary>
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
        catch (System.Exception)
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
