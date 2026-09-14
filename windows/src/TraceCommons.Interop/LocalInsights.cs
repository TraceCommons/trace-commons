using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

public interface ILocalInsights
{
    Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken);
}

/// <summary>Handle-free bounded calls. Cancellation stops waiting, not an already-started mutation.</summary>
public sealed class LocalInsights : ILocalInsights
{
    private static readonly HashSet<string> PublicInsightsErrors = new(StringComparer.Ordinal) {
        "insights_episode_invalid", "insights_episode_member_limit", "insights_episode_duplicate_member",
        "insights_episode_not_found", "insights_episode_missing_members", "insights_episode_revision_conflict",
        "insights_episode_limit_exceeded", "insights_episode_revision_overflow", "insights_response_too_large",
        "insights_card_invalid_selection", "insights_card_snapshot_limit", "insights_card_snapshot_not_found",
        "insights_card_episode_limit", "insights_card_episode_not_found"
    };
    private static readonly SemaphoreSlim Calls = new(1, 1);
    private readonly string? _storeDirectory;
    public LocalInsights(string? storeDirectory = null) => _storeDirectory = storeDirectory;

    public Task<JsonElement> CallAsync(object operation, CancellationToken cancellationToken)
    {
        byte[] bytes = JsonSerializer.SerializeToUtf8Bytes(new { store_dir = _storeDirectory, operation });
        if (bytes.Length > 65536) throw new InvalidOperationException("insights-request-too-large");
        return Task.Run(async () =>
        {
            await Calls.WaitAsync(cancellationToken).ConfigureAwait(false);
            try { cancellationToken.ThrowIfCancellationRequested(); return Call(bytes); }
            finally { Calls.Release(); }
        }, cancellationToken).WaitAsync(cancellationToken);
    }

    private static JsonElement Call(byte[] bytes)
    {
        IntPtr response = NativeMethods.tc_insights_call(bytes, (UIntPtr)bytes.Length, out IntPtr error);
        try
        {
            if (error != IntPtr.Zero || response == IntPtr.Zero)
            {
                string? code = error == IntPtr.Zero ? null : NativeMethods.BorrowedString(error);
                if (code != null && PublicInsightsErrors.Contains(code)) throw new InsightsServiceException(code);
                throw new InvalidOperationException("insights-operation-failed");
            }
            string json = NativeMethods.BorrowedString(response)
                ?? throw new InvalidOperationException("insights-response-invalid");
            using var document = JsonDocument.Parse(json);
            return document.RootElement.Clone();
        }
        finally
        {
            if (response != IntPtr.Zero) NativeMethods.tc_string_free(response);
            if (error != IntPtr.Zero) NativeMethods.tc_string_free(error);
        }
    }
}
