using System;
using System.Collections.Generic;
using System.Linq;

namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// What a folder picked with "add your tool" turned out to be
/// (macos/Sources/TraceCommonsApp/Views/FirstRun/ToolsScreen.swift:8-19).
/// </summary>
public abstract record AddToolOutcome
{
    /// <summary>One kind matched; the folder is added as that kind.</summary>
    public sealed record Added(AddedFolder Folder) : AddToolOutcome;

    /// <summary>More than one kind matched: the person picks which. <c>Kinds</c> holds only those this build can offer; null is a trajectory export.</summary>
    public sealed record Ask(string Path, IReadOnlyList<string?> Kinds) : AddToolOutcome
    {
        public bool Equals(Ask? other) => other is not null && Path == other.Path && Kinds.SequenceEqual(other.Kinds);

        public override int GetHashCode() => Path.GetHashCode(StringComparison.Ordinal);
    }

    /// <summary>Nothing this build can declare matched, or the core could not describe the folder.</summary>
    public sealed record Refused : AddToolOutcome;
}

/// <summary>
/// The Tools step's add-tool decisions, apart from the window so they can be
/// tested (ToolsScreenLayout, ToolsScreen.swift:20-160). Every write goes
/// through <see cref="FirstRunState.Add"/> or <see cref="FirstRunState.AnswerAdded"/>.
/// </summary>
public static class ToolsAdding
{
    /// <summary>The slug <c>tc_describe_folder</c> uses for a trajectory export (FolderMatch.swift:26).</summary>
    public const string TrajectorySlug = "trajectory";

    /// <summary>
    /// <c>tc_describe_folder</c>'s answer for <paramref name="path"/> (blocking:
    /// call off the UI thread). Null for a caught panic or a path the core
    /// will not read.
    /// </summary>
    public static string? Describe(string path) =>
        NativeMethods.TakeOwnedString(NativeMethods.tc_describe_folder(path));

    /// <summary>
    /// The answer as an outcome (ToolsScreen.swift:30-52). A null or
    /// undecodable answer is a refusal, so a folder the core could not read
    /// is never added on a guess; every row is kept, so a dropped trajectory
    /// row never turns a question into a certainty.
    /// </summary>
    public static AddToolOutcome Outcome(string path, string? json)
    {
        if (json is null)
        {
            return new AddToolOutcome.Refused();
        }

        IReadOnlyList<SourceCandidate> matches = SourceDiscovery.Parse(json);
        var offerable = new List<string?>();
        bool anyOfferable = false;
        foreach (SourceCandidate m in matches)
        {
            if (FirstRunRoots.SettingsKeys.ContainsKey(m.Source))
            {
                offerable.Add(m.Source);
                anyOfferable = true;
            }
            else if (m.Source == TrajectorySlug)
            {
                offerable.Add(null);
                anyOfferable = true;
            }
        }

        if (matches.Count == 1 && anyOfferable)
        {
            return new AddToolOutcome.Added(new AddedFolder(offerable[0], matches[0].Path));
        }

        // More than one match is a question even when only one can be
        // offered: the core said the folder is not certainly that kind.
        if (matches.Count > 1 && anyOfferable)
        {
            return new AddToolOutcome.Ask(path, offerable);
        }

        return new AddToolOutcome.Refused();
    }

    /// <summary>
    /// Apply an outcome (ToolsScreen.swift:57-67): an added folder is a row of
    /// its own, unanswered; a question writes nothing until it is answered.
    /// False for a refusal.
    /// </summary>
    public static bool Apply(AddToolOutcome outcome, FirstRunState state)
    {
        switch (outcome)
        {
            case AddToolOutcome.Added added:
                state.Add(added.Folder);
                return true;
            case AddToolOutcome.Ask:
                return true;
            default:
                return false;
        }
    }

    /// <summary>
    /// Answer an ambiguous folder (ToolsScreen.swift:136-150): a kind, by its
    /// index in the question's kinds, adds the row; null is the core's
    /// Neither and adds nothing. True when the question is closed; an index
    /// never offered keeps it open and writes nothing.
    /// </summary>
    public static bool Answer(int? index, string path, IReadOnlyList<string?> kinds, FirstRunState state)
    {
        if (index is not int i)
        {
            return true;
        }

        if (i < 0 || i >= kinds.Count)
        {
            return false;
        }

        state.Add(new AddedFolder(kinds[i], path));
        return true;
    }

    /// <summary>The folders added for a tool, each a row of its own (ToolsScreen.swift:78-80).</summary>
    public static IReadOnlyList<AddedFolder> AddedRows(FirstRunState state) =>
        state.AddedFolders.Where(f => !f.IsTrajectory).ToList();

    /// <summary>The added folder of exported traces, which has no tool row (ToolsScreen.swift:270-272).</summary>
    public static AddedFolder? TrajectoryFolder(FirstRunState state) =>
        state.AddedFolders.FirstOrDefault(f => f.IsTrajectory);

    /// <summary>
    /// The core's line beside an added folder that reads Watch while its tool
    /// is watched in another row too, naming the tool; null otherwise
    /// (ToolsScreen.swift:94-98).
    /// </summary>
    public static string? Conflict(AddedFolder folder, FirstRunState state, ToolsWords copy, Func<string, string> displayName)
    {
        if (folder.Watched != true || folder.SourceKind is not { } kind || !state.WatchedTwice.Contains(kind))
        {
            return null;
        }

        return FirstRunCopy.Fill(copy.OneFolderPerTool, new Dictionary<string, string> { ["tool"] = displayName(kind) });
    }

    /// <summary>An option's name: a tool's display name, or the core's label for exported traces (ToolsScreen.swift:160-165).</summary>
    public static string Name(string? kind, ToolsWords copy, Func<string, string> displayName) =>
        kind is null ? copy.TrajectoryLabel : displayName(kind);

    /// <summary>The core's question for an ambiguous folder, naming it by its last component (ToolsScreen.swift:167-176).</summary>
    public static string Question(string path, ToolsWords copy) =>
        FirstRunCopy.Fill(copy.WhichKind, new Dictionary<string, string> { ["folder"] = FolderName(path) });

    /// <summary>A picked folder's name: its last path component.</summary>
    public static string FolderName(string path)
    {
        // Both separators, whatever the host: the tests run off Windows.
        string trimmed = path.TrimEnd('\\', '/');
        int cut = trimmed.LastIndexOfAny(new[] { '\\', '/' });
        string name = cut < 0 ? trimmed : trimmed[(cut + 1)..];
        return name.Length == 0 ? trimmed : name;
    }

    /// <summary>
    /// Whether a described folder may still change the state
    /// (ToolsScreen.swift:242-246): a picker answers later, and by then a
    /// commit may hold the state, or the window may have moved on.
    /// </summary>
    public static bool AcceptsFolder(FirstRunStep step, bool isCommitting) => step == FirstRunStep.Tools && !isCommitting;

    /// <summary>The trajectory row's check: unticking withdraws the folder (ToolsScreen.swift:182-184).</summary>
    public static void SelectTrajectory(bool watched, FirstRunState state)
    {
        if (!watched)
        {
            state.WithdrawTrajectory();
        }
    }
}
