using System;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

/// <summary>
/// What the shell says when something outside the daemon fails (owner,
/// 2026-11-22), written in the core (<c>managed/copy.rs</c>
/// <c>shell_failures</c>) and read across the ABI because they are needed when
/// no daemon is there to send a copy table: <c>clipboard_failed</c>, and
/// <c>daemon_start_failed</c> for a start that failed for a reason other than
/// the lock another copy holds.
/// </summary>
public sealed record ShellFailureWords
{
    [JsonPropertyName("clipboard_failed")] public string ClipboardFailed { get; init; } = string.Empty;
    [JsonPropertyName("daemon_start_failed")] public string DaemonStartFailed { get; init; } = string.Empty;
}

/// <summary>The words of <see cref="ShellFailureWords"/>, read once.</summary>
public static class ShellFailureCopy
{
    private static readonly Lazy<ShellFailureWords?> Words = new(Read);

    /// <summary>The clipboard line, or null when the core cannot be read (nothing is then invented).</summary>
    public static string? ClipboardFailed => Words.Value?.ClipboardFailed;

    /// <summary>The unexpected-start-failure line, or null when the core cannot be read.</summary>
    public static string? DaemonStartFailed => Words.Value?.DaemonStartFailed;

    private static ShellFailureWords? Read()
    {
        try
        {
            return Parse(NativeMethods.TakeOwnedString(NativeMethods.tc_shell_failure_copy()));
        }
        catch (Exception exception) when (exception is DllNotFoundException or EntryPointNotFoundException or BadImageFormatException)
        {
            // The very failure this exists for can mean the core library is not
            // there to be asked; the callers then fall back, never invent.
            return null;
        }
    }

    /// <summary>The payload half of the read, testable without the cdylib: both lines or nothing.</summary>
    internal static ShellFailureWords? Parse(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            ShellFailureWords? value = JsonSerializer.Deserialize<ShellFailureWords>(json);
            return value is null || string.IsNullOrWhiteSpace(value.ClipboardFailed) || string.IsNullOrWhiteSpace(value.DaemonStartFailed)
                ? null
                : value;
        }
        catch (JsonException)
        {
            return null;
        }
    }
}
