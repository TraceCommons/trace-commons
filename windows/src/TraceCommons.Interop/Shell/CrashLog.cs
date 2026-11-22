using System;
using System.Collections.Generic;
using System.Globalization;
using System.IO;
using System.Text;
using System.Text.RegularExpressions;
using System.Threading;

namespace TraceCommons.Interop.Shell;

/// <summary>
/// What an exception that escapes the shell's UI thread leaves behind, so a
/// crash that ends the process with 0xc000027b (a stowed exception raised by
/// XAML) names the throwing line the next time. One file in the state
/// directory, beside the daemon's own <c>daemon-errors.log</c> and
/// <c>daemon-panics.log</c>: <c>shell-crashes.log</c>.
/// <para>
/// Payload-free, as the daemon's logs are: an entry is a time, a label for
/// where it was caught, the exception type and HResult, and the stack as
/// method names with file names and line numbers. Never the message (it can
/// quote a path or content), never argument values or directories (a path can
/// carry a user name). An inner exception is its type and HResult.
/// </para>
/// <para>
/// Each exception type is recorded once per run, so a fault in a draw loop
/// cannot flood the file, and the file is capped by rotating to
/// <c>.1</c>. Recording is never a second failure: it returns false rather
/// than throw.
/// </para>
/// </summary>
public sealed class CrashLog
{
    public const string FileName = "shell-crashes.log";

    /// <summary>Size at which the current file is rotated to <c>.1</c>.</summary>
    public const long CapBytes = 256 * 1024;

    private static readonly Regex Frame = new(
        @"^\s*at\s(?<method>.+?)\((?<args>.*?)\)(?:\sin\s(?<file>.+?):line\s(?<line>\d+))?\s*$",
        RegexOptions.CultureInvariant | RegexOptions.Compiled);

    private static readonly Regex Label = new("^[a-z0-9_-]{1,32}$", RegexOptions.CultureInvariant | RegexOptions.Compiled);

    private readonly string _directory;
    private readonly Func<DateTimeOffset> _clock;
    private readonly long _capBytes;
    private readonly HashSet<string> _seen = new(StringComparer.Ordinal);
    private readonly object _gate = new();

    public CrashLog(string directory, Func<DateTimeOffset>? clock = null, long capBytes = CapBytes)
    {
        _directory = directory;
        _clock = clock ?? (() => DateTimeOffset.UtcNow);
        _capBytes = capBytes;
    }

    public string Path => System.IO.Path.Combine(_directory, FileName);

    /// <summary>
    /// Appends <paramref name="exception"/> unless its type was recorded
    /// already in this run. True when it was written.
    /// </summary>
    public bool Record(string where, Exception exception)
    {
        if (exception is null)
        {
            return false;
        }

        try
        {
            lock (_gate)
            {
                if (string.IsNullOrEmpty(_directory) || !_seen.Add(exception.GetType().FullName ?? exception.GetType().Name))
                {
                    return false;
                }

                string entry = Format(where, exception, _clock());
                Directory.CreateDirectory(_directory);
                Rotate(Encoding.UTF8.GetByteCount(entry));
                File.AppendAllText(Path, entry, Encoding.UTF8);
                return true;
            }
        }
        catch (Exception)
        {
            // A log that cannot be written is not a reason to fail again.
            return false;
        }
    }

    private void Rotate(long incoming)
    {
        var info = new FileInfo(Path);
        if (!info.Exists || info.Length + incoming <= _capBytes)
        {
            return;
        }

        try
        {
            File.Move(Path, Path + ".1", overwrite: true);
        }
        catch (IOException)
        {
            File.Delete(Path);
        }
    }

    /// <summary>One entry, ending in a newline: see the type's remarks for what it holds.</summary>
    public static string Format(string where, Exception exception, DateTimeOffset now)
    {
        var text = new StringBuilder();
        text.Append(now.UtcDateTime.ToString("yyyy-MM-dd'T'HH:mm:ss'Z'", CultureInfo.InvariantCulture))
            .Append(" where=").Append(Label.IsMatch(where ?? string.Empty) ? where : "invalid")
            .Append(" type=").Append(TypeName(exception))
            .Append(" hresult=").AppendLine(HResult(exception));
        foreach (string raw in (exception.StackTrace ?? string.Empty).Split('\n'))
        {
            if (FrameLine(raw.TrimEnd('\r')) is string frame)
            {
                text.Append("  ").AppendLine(frame);
            }
        }

        Exception? inner = exception.InnerException;
        for (int depth = 0; inner is not null && depth < 8; depth++, inner = inner.InnerException)
        {
            text.Append("  inner type=").Append(TypeName(inner)).Append(" hresult=").AppendLine(HResult(inner));
        }

        return text.ToString();
    }

    /// <summary>
    /// A stack frame as its method and, when there is symbol information, its
    /// file name and line: <c>at Ns.Type.Method File.cs:line 12</c>. Argument
    /// lists and directories are dropped. Null for a line that is not a frame.
    /// </summary>
    public static string? FrameLine(string raw)
    {
        Match m = Frame.Match(raw ?? string.Empty);
        if (!m.Success)
        {
            return null;
        }

        string line = "at " + m.Groups["method"].Value;
        if (m.Groups["file"].Success)
        {
            string file = m.Groups["file"].Value;
            int cut = file.LastIndexOfAny(new[] { '\\', '/' });
            line += " " + (cut >= 0 ? file[(cut + 1)..] : file) + ":line " + m.Groups["line"].Value;
        }

        return line;
    }

    private static string TypeName(Exception e) => e.GetType().FullName ?? e.GetType().Name;

    private static string HResult(Exception e) => "0x" + ((uint)e.HResult).ToString("x8", CultureInfo.InvariantCulture);
}

/// <summary>
/// How many first-run windows are open (a count, since a preview opens one
/// too), and whether an exception that escaped belongs to first run: only then
/// is it survived. First run is the one flow whose window is built and rebuilt
/// by code that can throw before the person has anything to lose, and it owns
/// its own recovery (Welcome is still there to try again). Every other fault,
/// the main window's included, is logged and left to surface as before:
/// swallowing one could leave the app silently half-broken.
/// </summary>
public sealed class FirstRunWindowTracker
{
    /// <summary>Frames that mark an exception as first run's: its window and its flow.</summary>
    private static readonly string[] FirstRunFrames =
    {
        "TraceCommons.App.FirstRunWindow",
        "TraceCommons.Interop.FirstRun.",
    };

    private int _open;

    public bool Any => Volatile.Read(ref _open) > 0;

    public void Enter() => Interlocked.Increment(ref _open);

    public void Exit()
    {
        int seen;
        do
        {
            seen = Volatile.Read(ref _open);
            if (seen <= 0)
            {
                return;
            }
        }
        while (Interlocked.CompareExchange(ref _open, seen - 1, seen) != seen);
    }

    /// <summary>
    /// Whether <paramref name="exception"/> is survived: a first-run window is
    /// open and its stack (or an inner exception's) runs through the first-run
    /// window or flow.
    /// </summary>
    public bool Survives(Exception? exception)
    {
        if (!Any)
        {
            return false;
        }

        for (int depth = 0; exception is not null && depth < 8; depth++, exception = exception.InnerException)
        {
            string stack = exception.StackTrace ?? string.Empty;
            foreach (string frame in FirstRunFrames)
            {
                if (stack.Contains(frame, StringComparison.Ordinal))
                {
                    return true;
                }
            }
        }

        return false;
    }
}
