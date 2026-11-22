using System;
using System.Runtime.InteropServices;
using System.Threading;
using Windows.ApplicationModel.DataTransfer;

namespace TraceCommons.App;

/// <summary>
/// Puts text on the clipboard without letting a failure escape. The clipboard
/// is a shared resource another process can hold open, and
/// <c>Clipboard.SetContent</c> then throws a COMException (CLIPBRD_E_CANT_OPEN)
/// out of a click handler, where an escaped exception ends the process. A short
/// retry covers a momentary hold; a hold that outlasts it is a false return,
/// which the caller treats as "not copied" (no "Copied" confirmation).
/// </summary>
internal static class ClipboardCopy
{
    private const int Attempts = 3;
    private const int PauseMilliseconds = 40;

    /// <summary>True when <paramref name="text"/> is on the clipboard.</summary>
    internal static bool TryCopy(string text, bool asCopyOperation = false)
    {
        for (int attempt = 1; attempt <= Attempts; attempt++)
        {
            try
            {
                var package = new DataPackage();
                if (asCopyOperation)
                {
                    package.RequestedOperation = DataPackageOperation.Copy;
                }

                package.SetText(text);
                Clipboard.SetContent(package);
                return true;
            }
            catch (COMException exception)
            {
                if (attempt == Attempts)
                {
                    CrashGuard.Log.Record("clipboard", exception);
                    return false;
                }

                Thread.Sleep(PauseMilliseconds);
            }
        }

        return false;
    }
}
