#if DEBUG || TC_SAMPLE_TESTS
// Recorded sample data in place of a daemon. Debug builds only.
//
// The Windows half of the macOS app's SampleDaemonClient
// (macos/Sources/TCShellCore/DataContract/SampleDaemonClient.swift) and of the
// Linux shell's sample.rs (#2161): when TRACE_COMMONS_SAMPLE names a set, the
// window reads that set's recorded replies instead of reaching a daemon, so the
// shells can be drawn from the same data and their captures compared side by
// side (slice W1 of docs/superpowers/specs/2026-11-18-windows-mac-parity-design.md).
//
// The replies are the files the macOS app bundles,
// macos/Sources/TCShellCore/DataContract/RecordedSamples/<set>/<method>.json:
// real answers the real daemon sent against a throwaway store, recorded by
// crates/trace-commons-contributor/tests/k2_sample_recorder.rs, whose drift
// test keeps them matched to the daemon. A Debug build copies them beside the
// assembly (TraceCommons.Interop.csproj), so there is one source and no second
// recording to drift.
//
// What it never does: start or attach to a daemon, open the state directory,
// take the lock, enroll, upload or write anything. The app's own writes to
// this machine (the single-instance key, the URL scheme, run at login, the
// witness files) are refused in sample mode too; see Program.IsSampleLaunch. A method is answered only
// when its set, or the shared/ set the macOS app uses for the four writes whose
// reply never depends on the set (approve, keep, undo_keep, set_project_mode),
// holds a recording of it. Every other method is refused with one fixed label.
// A write answered from shared/ changes nothing: the sets are fixtures, not a
// simulator.
//
// The whole file is compiled out of a Release build of this assembly. The test
// project compiles its own copy under TC_SAMPLE_TESTS in Release, so CI, which
// tests in Release, still runs the sample tests; nothing shipped defines it.
using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.Json.Nodes;

namespace TraceCommons.Interop;

/// <summary>Which set a <c>TRACE_COMMONS_SAMPLE</c> value names.</summary>
/// <param name="Set">The set used.</param>
/// <param name="Unknown">
/// The value was set and non-empty but named no set; the set is then the
/// default, and the launch logs one line saying so.
/// </param>
public readonly record struct SampleChoice(string Set, bool Unknown);

/// <summary>A daemon stand-in answering <c>tc_call</c> from one recorded set.</summary>
public sealed class SampleDaemon
{
    /// <summary>The variable the macOS app reads (<c>MonitorWindowView.sampleClient()</c>).</summary>
    public const string EnvironmentVariable = "TRACE_COMMONS_SAMPLE";

    /// <summary>The set an empty or unrecognised name falls back to, as on macOS.</summary>
    public const string DefaultSet = "normalDay";

    /// <summary>The set whose every call fails, as a daemon that is not there does.</summary>
    public const string CoreDown = "coreDown";

    /// <summary>The directory beside the sets that holds the set-independent writes.</summary>
    public const string SharedDir = "shared";

    /// <summary>The answer to a method no recording covers. Fixed and content-free.</summary>
    public const string UnrecordedLabel = "sample-unrecorded";

    /// <summary>The label the Linux shell uses for a daemon that went away.</summary>
    public const string NotRunningLabel = "daemon-not-running";

    /// <summary>Every set the macOS app names (<c>SampleDaemonClient.SampleSet</c>), in its order.</summary>
    public static readonly IReadOnlyList<string> Sets = new[]
    {
        "empty",
        "normalDay",
        "busyQueue",
        CoreDown,
        "unknownCounts",
        "heldSessions",
        "armedFolder",
    };

    /// <summary>
    /// The variable naming a Windows-only extra beside the set. Only
    /// <see cref="SpendOrganizationsExtra"/> exists; it is never on by default,
    /// so the standard sets still draw what the Mac's draw.
    /// </summary>
    public const string ExtraVariable = "TRACE_COMMONS_SAMPLE_EXTRA";

    /// <summary>
    /// Two throwaway spending organizations and no choice made, so the Private
    /// AI prompt and the Settings picker draw (W19-D2). The Mac records no
    /// listing, because the recorder has no NEAR AI sign-in; this one is pinned
    /// to the daemon's own answer by
    /// <c>the_windows_spend_organization_sample_is_this_listing</c>
    /// (spend_org_tests.rs). There is no recorded switch: a pick is refused
    /// as unrecorded and the listing stays as it was.
    /// </summary>
    public const string SpendOrganizationsExtra = "spendOrganizations";

    /// <summary>The directory beside the assembly that holds the extras.</summary>
    public const string ExtrasDir = "SampleExtras";

    private readonly string _root;
    private readonly string? _extra;

    public SampleDaemon(string set, string root, string? extra = null)
    {
        Set = set ?? throw new ArgumentNullException(nameof(set));
        _root = root ?? throw new ArgumentNullException(nameof(root));
        _extra = extra;
    }

    /// <summary>The set this client answers from.</summary>
    public string Set { get; }

    /// <summary>The recordings a Debug build copies beside this assembly.</summary>
    public static string DefaultRoot() => Path.Combine(AppContext.BaseDirectory, "RecordedSamples");

    /// <summary>
    /// <c>MonitorWindowView.sampleChoice</c>, plus its caller's unset check:
    /// unset is no sample at all (null), empty is the default, a set's name is
    /// that set, and anything else is the default marked unknown.
    /// </summary>
    public static SampleChoice? Choose(string? value)
    {
        if (value is null)
        {
            return null;
        }

        if (value.Length == 0)
        {
            return new SampleChoice(DefaultSet, false);
        }

        string? set = Sets.FirstOrDefault(s => string.Equals(s, value, StringComparison.Ordinal));
        return set is null ? new SampleChoice(DefaultSet, true) : new SampleChoice(set, false);
    }

    /// <summary>
    /// The set <paramref name="value"/> names, or null when it is unset. An
    /// unrecognised name is logged once, as a fixed label and the set used,
    /// never the value itself.
    /// </summary>
    public static SampleDaemon? FromValue(string? value, string root, Action<string> log, string? extra = null)
    {
        ArgumentNullException.ThrowIfNull(log);
        SampleChoice? choice = Choose(value);
        if (choice is not { } chosen)
        {
            return null;
        }

        if (chosen.Unknown)
        {
            log($"tracecommons-sample:{EnvironmentVariable}-unrecognised,fallback={chosen.Set}");
        }

        return new SampleDaemon(chosen.Set, root, Extra(extra, root));
    }

    /// <summary>
    /// The extras directory <paramref name="value"/> names, matched exactly;
    /// null for unset or anything else.
    /// </summary>
    public static string? Extra(string? value, string root) =>
        string.Equals(value, SpendOrganizationsExtra, StringComparison.Ordinal)
            ? Path.Combine(Path.GetDirectoryName(Path.TrimEndingDirectorySeparator(root)) ?? root, ExtrasDir, SpendOrganizationsExtra)
            : null;

    /// <summary>
    /// Whether this launch reads recorded samples: <c>TRACE_COMMONS_SAMPLE</c>
    /// is set, to anything. The app asks this before a window or host exists
    /// (single-instancing, the URL scheme), and every answer agrees with
    /// <see cref="FromEnvironment"/>.
    /// </summary>
    public static bool IsRequested() => IsRequested(Environment.GetEnvironmentVariable(EnvironmentVariable));

    /// <summary><see cref="IsRequested()"/> for a given value.</summary>
    public static bool IsRequested(string? value) => Choose(value) is not null;

    /// <summary>This launch's sample, from the environment; null over the daemon.</summary>
    public static SampleDaemon? FromEnvironment() => FromValue(
        Environment.GetEnvironmentVariable(EnvironmentVariable),
        DefaultRoot(),
        line => System.Diagnostics.Debug.WriteLine(line),
        Environment.GetEnvironmentVariable(ExtraVariable));

    /// <summary>
    /// The state directory the host reports in sample mode, for the reads
    /// that take a directory rather than a handle. Under the temp directory
    /// and never created here, so those reads see an empty state instead of
    /// the contributor's own.
    /// </summary>
    public static string ThrowawayConfigDir() =>
        Path.Combine(Path.GetTempPath(), $"tracecommons-sample-{Environment.ProcessId}");

    /// <summary>
    /// One request, one response frame: the recorded result, or a fixed
    /// refusal. Never throws, as <c>TcDaemon.Call</c> never does.
    /// </summary>
    public string Call(string method, string paramsJson = "{}")
    {
        ArgumentNullException.ThrowIfNull(method);
        _ = paramsJson;

        if (Set == CoreDown)
        {
            return ErrorFrame(NotRunningLabel);
        }

        // A method name is a file name here: anything but the daemon's
        // snake-case spelling could walk out of the directory.
        if (method.Length == 0 || !method.All(c => c is (>= 'a' and <= 'z') or (>= '0' and <= '9') or '_'))
        {
            return ErrorFrame(UnrecordedLabel);
        }

        string file = method + ".json";
        var paths = new List<string> { Path.Combine(_root, Set, file), Path.Combine(_root, SharedDir, file) };
        if (_extra is not null) paths.Add(Path.Combine(_extra, file));
        foreach (string path in paths)
        {
            string text;
            try
            {
                text = File.ReadAllText(path);
            }
            catch (Exception e) when (e is FileNotFoundException or DirectoryNotFoundException)
            {
                continue;
            }

            try
            {
                return new JsonObject { ["result"] = JsonNode.Parse(text) }.ToJsonString();
            }
            catch (JsonException)
            {
                return ErrorFrame(UnrecordedLabel);
            }
        }

        return ErrorFrame(UnrecordedLabel);
    }

    private static string ErrorFrame(string label) =>
        new JsonObject
        {
            ["error"] = new JsonObject { ["code"] = "unavailable", ["message"] = label },
        }.ToJsonString();
}
#endif
