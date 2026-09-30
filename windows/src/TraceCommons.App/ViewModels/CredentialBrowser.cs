using System;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>One browser continuation shared by onboarding, keys and balances.</summary>
internal static class CredentialBrowser
{
    internal static async Task ContinueAsync(
        NearAiCredentialAttempt? attempt,
        Func<Uri, Task<bool>> open,
        Func<Task> cancel,
        Func<Task> readStatus,
        Func<Task> poll)
    {
        if (attempt is { } started)
        {
            bool opened = false;
            try
            {
                opened = Uri.TryCreate(started.BrowserUrl, UriKind.Absolute, out var uri)
                    && await open(uri);
            }
            catch
            {
                // Browser-launch errors can include the credential-bearing URL.
                System.Diagnostics.Trace.TraceWarning(nameof(CredentialBrowser));
            }
            if (!opened) await cancel();
        }

        // Start returns an attempt, not a refreshed credential status. Read
        // before the polling predicate or it exits on the previous idle state.
        await readStatus();
        await poll();
    }
}
