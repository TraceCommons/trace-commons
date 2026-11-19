using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;

namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// One screen of the animated first run (macos/Sources/TCShellCore/FirstRun/FirstRunState.swift:9-19).
/// The order is <see cref="FirstRunNavigation.Steps"/>.
/// </summary>
public enum FirstRunStep
{
    Welcome,
    Tools,
    Uses,
    Traces,
    PrivateAI,
    Terms,
    Join,
}

/// <summary>What the person answered on Join about an account (FirstRunState.swift:22-35).</summary>
public enum AccountAnswer
{
    None,
    WatchOnly,
    NearAI,
    Enrolled,
}

/// <summary>The sharing decision on Uses (FirstRunState.swift:38-41).</summary>
public enum SharingPath
{
    AskMe,
    Automatic,
}

/// <summary>A folder rule (<c>ProjectMode</c>), by its wire word.</summary>
public enum ProjectRule
{
    Ask,
    AutoUpload,
    Ignore,
}

/// <summary>One tool's answer: undecided, watch a folder, or off (<c>SourceChoice</c>).</summary>
public readonly record struct ToolChoice(ToolChoiceKind Kind, string Path)
{
    public static readonly ToolChoice Undecided = new(ToolChoiceKind.Undecided, "");
    public static readonly ToolChoice Off = new(ToolChoiceKind.Off, "");

    public static ToolChoice Watch(string path) => new(ToolChoiceKind.Watch, path);

    public bool IsAnswered => Kind != ToolChoiceKind.Undecided;

    /// <summary>The declaration the daemon takes, or null for undecided (<c>SourceChoice.declaration</c>).</summary>
    public SortedDictionary<string, string>? Declaration => Kind switch
    {
        ToolChoiceKind.Watch => new SortedDictionary<string, string>(StringComparer.Ordinal) { ["mode"] = "watch", ["path"] = Path },
        ToolChoiceKind.Off => new SortedDictionary<string, string>(StringComparer.Ordinal) { ["mode"] = "off" },
        _ => null,
    };
}

public enum ToolChoiceKind
{
    Undecided,
    Watch,
    Off,
}

/// <summary>A folder picked with "add your tool" on Tools (FirstRunState.swift:49-71).</summary>
public sealed record AddedFolder
{
    public AddedFolder(string? sourceKind, string path, bool? watched = null)
    {
        SourceKind = sourceKind;
        Path = path;
        Watched = watched ?? (sourceKind is null ? true : null);
    }

    /// <summary>The tool whose store this is, or null for a folder of exported traces.</summary>
    public string? SourceKind { get; init; }

    public bool IsTrajectory => SourceKind is null;

    public string Path { get; init; }

    /// <summary>Null until answered, true Watch, false Not used.</summary>
    public bool? Watched { get; init; }
}

/// <summary>
/// The session roots as one declaration (<c>SessionRoots</c>,
/// macos/Sources/TCShellCore/SessionRoots.swift): five tools and the
/// trajectory folder, Claude Code and Codex required.
/// </summary>
public sealed class FirstRunRoots
{
    public static readonly IReadOnlyDictionary<string, string> SettingsKeys = new Dictionary<string, string>
    {
        ["claude-code"] = "claude_source",
        ["codex"] = "codex_source",
        ["gemini-cli"] = "gemini_source",
        ["cline"] = "cline_source",
        ["opencode"] = "opencode_source",
    };

    public const string TrajectoryKey = "trajectory_source";

    private readonly Dictionary<string, ToolChoice> _tools = new(StringComparer.Ordinal);

    public ToolChoice this[string kind]
    {
        get => _tools.TryGetValue(kind, out ToolChoice c) ? c : ToolChoice.Undecided;
        set => _tools[kind] = value;
    }

    public ToolChoice Trajectory { get; set; } = ToolChoice.Undecided;

    public bool IsComplete => this["claude-code"].IsAnswered && this["codex"].IsAnswered;

    /// <summary>The <c>settings_json</c> for the daemon start, sorted keys, or null while Claude Code or Codex is unanswered.</summary>
    public string? SettingsJson()
    {
        if (!IsComplete)
        {
            return null;
        }

        var o = new SortedDictionary<string, SortedDictionary<string, string>>(StringComparer.Ordinal);
        foreach (var (kind, key) in SettingsKeys)
        {
            if (this[kind].Declaration is { } d)
            {
                o[key] = d;
            }
        }

        if (Trajectory.Declaration is { } t)
        {
            o[TrajectoryKey] = t;
        }

        return JsonSerializer.Serialize(o);
    }
}

/// <summary>
/// Every answer the person gives, and how far the daemon calls have got
/// (macos/Sources/TCShellCore/FirstRun/FirstRunState.swift:78-310). Pure.
/// </summary>
public sealed class FirstRunState
{
    private SharingPath _sharing = SharingPath.AskMe;

    public FirstRunStep Step { get; set; } = FirstRunStep.Welcome;
    public string Invite { get; set; } = "";
    public string? IssuerHost { get; set; }
    public AccountAnswer Account { get; set; } = AccountAnswer.None;
    public Dictionary<string, ToolChoice> ToolAnswers { get; set; } = new(StringComparer.Ordinal);
    public List<AddedFolder> AddedFolders { get; set; } = new();
    public SortedDictionary<string, ProjectRule> Rules { get; set; } = new(StringComparer.Ordinal);
    public SortedDictionary<string, SortedSet<string>> PastSelections { get; set; } = new(StringComparer.Ordinal);
    public SortedSet<string> Scopes { get; set; } = new(StringComparer.Ordinal);

    /// <summary>Writing it, to any value, clears <see cref="GrantReady"/> (FirstRunState.swift:103-105).</summary>
    public SharingPath Sharing
    {
        get => _sharing;
        set
        {
            _sharing = value;
            GrantReady = false;
        }
    }

    /// <summary>The person picked a Sharing choice; neither is chosen for them (owner, 2026-10-27).</summary>
    public bool SharingChosen { get; set; }
    public bool PrivateAI { get; set; }
    public string? WitnessSigningAddress { get; set; }
    public bool GrantReady { get; set; }
    public bool DaemonStarted { get; set; }
    public string? StartedSettingsJson { get; set; }
    public string? EnrolledInvite { get; set; }
    public bool SignedIn { get; set; }
    public bool NearAIEnrolled { get; set; }
    public HashSet<string> NotFound { get; set; } = new(StringComparer.Ordinal);
    public bool SignedOutOfEnrolment { get; set; }

    /// <summary>A deep copy: the Swift state is a value.</summary>
    public FirstRunState Clone()
    {
        var c = (FirstRunState)MemberwiseClone();
        c.ToolAnswers = new Dictionary<string, ToolChoice>(ToolAnswers, StringComparer.Ordinal);
        c.AddedFolders = new List<AddedFolder>(AddedFolders);
        c.Rules = new SortedDictionary<string, ProjectRule>(Rules, StringComparer.Ordinal);
        c.PastSelections = new SortedDictionary<string, SortedSet<string>>(StringComparer.Ordinal);
        foreach (var (k, v) in PastSelections)
        {
            c.PastSelections[k] = new SortedSet<string>(v, StringComparer.Ordinal);
        }

        c.Scopes = new SortedSet<string>(Scopes, StringComparer.Ordinal);
        c.NotFound = new HashSet<string>(NotFound, StringComparer.Ordinal);
        return c;
    }

    /// <summary>
    /// Record what discovery found (FirstRunState.swift:186-191): a tool not
    /// on this computer is not asked; a found tool nobody answered reads Not
    /// used, since its check starts unticked.
    /// </summary>
    public void RecordDiscovery(IEnumerable<(string Source, bool Exists)> candidates)
    {
        var list = candidates.ToList();
        NotFound = new HashSet<string>(list.Where(c => !c.Exists).Select(c => c.Source), StringComparer.Ordinal);
        foreach (var (source, exists) in list)
        {
            if (exists && !RowAnswer(source).IsAnswered)
            {
                ToolAnswers[source] = ToolChoice.Off;
            }
        }
    }

    public bool WatchesAnything
    {
        get
        {
            FirstRunRoots roots = SessionRoots;
            return FirstRunRoots.SettingsKeys.Keys.Any(k => roots[k].Kind == ToolChoiceKind.Watch)
                || roots.Trajectory.Kind == ToolChoiceKind.Watch;
        }
    }

    public FirstRunState IncludingRequiredScope(string? name)
    {
        if (name is null || Scopes.Contains(name))
        {
            return this;
        }

        FirstRunState c = Clone();
        c.Scopes.Add(name);
        return c;
    }

    /// <summary>Whether the daemon holds an enrollment for this first run (FirstRunState.swift:221-228).</summary>
    public bool HoldsEnrolment
    {
        get
        {
            if (SignedOutOfEnrolment)
            {
                return false;
            }

            return Account switch
            {
                AccountAnswer.Enrolled => true,
                AccountAnswer.NearAI => EnrolledInvite is not null || NearAIEnrolled,
                _ => false,
            };
        }
    }

    /// <summary>Whether the daemon may hold an enrollment (FirstRunState.swift:237-243).</summary>
    public bool DaemonHoldsEnrolment =>
        SignedOutOfEnrolment || EnrolledInvite is not null || NearAIEnrolled || Account == AccountAnswer.Enrolled;

    public void Answer(string kind, ToolChoice choice) => ToolAnswers[kind] = choice;

    /// <summary>Add a folder as a row of its own (FirstRunState.swift:254-257).</summary>
    public void Add(AddedFolder folder)
    {
        AddedFolders.RemoveAll(f => f.Path == folder.Path || (folder.IsTrajectory && f.IsTrajectory));
        AddedFolders.Add(folder);
    }

    public ToolChoice RowAnswer(string kind) => ToolAnswers.TryGetValue(kind, out ToolChoice c) ? c : ToolChoice.Undecided;

    public void AnswerAdded(string path, bool? watched)
    {
        int i = AddedFolders.FindIndex(f => f.Path == path);
        if (i >= 0)
        {
            AddedFolders[i] = AddedFolders[i] with { Watched = watched };
        }
    }

    /// <summary>Tools two rows both watch (FirstRunState.swift:274-283).</summary>
    public IReadOnlySet<string> WatchedTwice
    {
        get
        {
            var counts = new Dictionary<string, int>(StringComparer.Ordinal);
            foreach (var (kind, choice) in ToolAnswers)
            {
                if (choice.Kind == ToolChoiceKind.Watch)
                {
                    counts[kind] = counts.GetValueOrDefault(kind) + 1;
                }
            }

            foreach (AddedFolder f in AddedFolders)
            {
                if (f.Watched == true && f.SourceKind is { } k)
                {
                    counts[k] = counts.GetValueOrDefault(k) + 1;
                }
            }

            return counts.Where(p => p.Value > 1).Select(p => p.Key).ToHashSet(StringComparer.Ordinal);
        }
    }

    public bool EveryAddedFolderAnswered => AddedFolders.All(f => f.Watched is not null);

    public void WithdrawTrajectory() => AddedFolders.RemoveAll(f => f.IsTrajectory);

    /// <summary>The answers and folders as one declaration (FirstRunState.swift:310-327).</summary>
    public FirstRunRoots SessionRoots
    {
        get
        {
            var roots = new FirstRunRoots();
            foreach (var (kind, choice) in ToolAnswers)
            {
                roots[kind] = choice;
            }

            foreach (string kind in new[] { "claude-code", "codex" })
            {
                if (NotFound.Contains(kind) && !roots[kind].IsAnswered)
                {
                    roots[kind] = ToolChoice.Off;
                }
            }

            foreach (AddedFolder f in AddedFolders)
            {
                if (f.Watched != true)
                {
                    continue;
                }

                if (f.SourceKind is { } k)
                {
                    roots[k] = ToolChoice.Watch(f.Path);
                }
                else
                {
                    roots.Trajectory = ToolChoice.Watch(f.Path);
                }
            }

            return roots;
        }
    }
}

/// <summary>The steps and the rules for moving between them (macos/Sources/TCShellCore/FirstRun/FirstRunNavigation.swift).</summary>
public static class FirstRunNavigation
{
    /// <summary>One order for everyone (FirstRunNavigation.swift:15).</summary>
    public static readonly IReadOnlyList<FirstRunStep> Steps = new[]
    {
        FirstRunStep.Welcome, FirstRunStep.Tools, FirstRunStep.PrivateAI, FirstRunStep.Traces, FirstRunStep.Uses, FirstRunStep.Terms, FirstRunStep.Join,
    };

    /// <summary>The steps the thread names: every one after Welcome (FirstRunNavigation.swift:18).</summary>
    public static readonly List<FirstRunStep> ThreadSteps = Steps.Skip(1).ToList();

    public static FirstRunState Next(FirstRunState state) => Move(state, 1);

    public static FirstRunState Back(FirstRunState state)
    {
        FirstRunState moved = Move(state, -1);
        moved.GrantReady = false;
        return moved;
    }

    /// <summary>Back to a step already passed, picked on the thread (FirstRunNavigation.swift:37-44).</summary>
    public static FirstRunState BackTo(FirstRunStep step, FirstRunState state)
    {
        int target = IndexOf(step), current = IndexOf(state.Step);
        if (target < 0 || current < 0 || target >= current)
        {
            return state;
        }

        FirstRunState moved = state.Clone();
        moved.Step = step;
        moved.GrantReady = false;
        return moved;
    }

    public static FirstRunState ReturnToJoinAfterDeadInvite(FirstRunState state)
    {
        FirstRunState r = state.Clone();
        r.Step = FirstRunStep.Join;
        return r;
    }

    public static FirstRunState ReturnToJoinAfterNearAIFailure(FirstRunState state)
    {
        FirstRunState r = ReturnToJoinAfterDeadInvite(state);
        if (r.Account == AccountAnswer.NearAI)
        {
            r.Account = AccountAnswer.None;
        }

        return r;
    }

    /// <summary>
    /// An enrollment the daemon reported after this first run began
    /// (OnboardingNavigation.swift:36-48): the account is held, so nothing
    /// is joined, signed in or created again. One the person signed out of
    /// is not theirs again.
    /// </summary>
    public static FirstRunState RecordEnrolment(FirstRunState state)
    {
        if (state.EnrolledInvite is not null || state.SignedOutOfEnrolment)
        {
            return state;
        }

        FirstRunState recorded = state.Clone();
        recorded.EnrolledInvite = "";
        recorded.Invite = "";
        recorded.IssuerHost = null;
        if (recorded.Account is AccountAnswer.None or AccountAnswer.WatchOnly)
        {
            recorded.Account = AccountAnswer.Enrolled;
        }

        return recorded;
    }

    /// <summary>Whether the current step's Continue (or Start, on Join) is enabled (FirstRunNavigation.swift:99-117).</summary>
    public static bool CanContinue(FirstRunState state, string? requiredScope) => state.Step switch
    {
        FirstRunStep.Welcome or FirstRunStep.Traces or FirstRunStep.PrivateAI => true,
        // The terms screen gates itself on the runner's acceptance.
        FirstRunStep.Terms => true,
        FirstRunStep.Tools => state.WatchesAnything && state.WatchedTwice.Count == 0 && state.SessionRoots.SettingsJson() is not null,
        FirstRunStep.Uses => requiredScope is not null,
        FirstRunStep.Join => requiredScope is not null
            && ((state.Account == AccountAnswer.WatchOnly && !state.DaemonHoldsEnrolment) || state.HoldsEnrolment),
        _ => false,
    };

    public static bool CanChooseAutomatic(FirstRunState state) => state.HoldsEnrolment;

    /// <summary>Ask, then Automatic; neither chosen for the person (FirstRunNavigation.swift:135-137).</summary>
    public static IReadOnlyList<SharingPath> SharingPaths(FirstRunState state) => new[] { SharingPath.AskMe, SharingPath.Automatic };

    private static int IndexOf(FirstRunStep step)
    {
        for (int i = 0; i < Steps.Count; i++)
        {
            if (Steps[i] == step)
            {
                return i;
            }
        }

        return -1;
    }

    private static FirstRunState Move(FirstRunState state, int offset)
    {
        int i = IndexOf(state.Step);
        if (i < 0 || i + offset < 0 || i + offset >= Steps.Count)
        {
            return state.Clone();
        }

        FirstRunState moved = state.Clone();
        moved.Step = Steps[i + offset];
        return moved;
    }
}

/// <summary>Where the first run commits answers to the daemon (FirstRunPlan.swift:122-138).</summary>
public enum CommitPoint
{
    DeclareRoots,
    LeaveTools,
    SignIn,
    Start,
}

/// <summary>One daemon call the first run makes (FirstRunPlan.swift:5-34).</summary>
public abstract record FirstRunCall
{
    public sealed record StartDaemon(string SettingsJson) : FirstRunCall;
    public sealed record SetSourceSettings(string SettingsJson) : FirstRunCall;
    public sealed record LookupInvite(string Invite) : FirstRunCall;
    public sealed record Enroll(string Invite) : FirstRunCall;
    public sealed record SignInNearAI : FirstRunCall;
    public sealed record NearAILogin : FirstRunCall;
    public sealed record EnrollNearAI : FirstRunCall;
    public sealed record SetAskEachTime : FirstRunCall;
    public sealed record SetConsentScopes(IReadOnlyList<string> Scopes) : FirstRunCall
    {
        public bool Equals(SetConsentScopes? other) => other is not null && Scopes.SequenceEqual(other.Scopes);

        public override int GetHashCode() => Scopes.Count;
    }

    public sealed record SetProjectMode(string ProjectId, ProjectRule Mode) : FirstRunCall;
    public sealed record IncludePastSessions(string ProjectId, IReadOnlyList<string> Sessions) : FirstRunCall
    {
        public bool Equals(IncludePastSessions? other) => other is not null && ProjectId == other.ProjectId && Sessions.SequenceEqual(other.Sessions);

        public override int GetHashCode() => ProjectId.GetHashCode(StringComparison.Ordinal);
    }

    public sealed record SetPrivateAI(bool On) : FirstRunCall;
    public sealed record GrantAutomatic(string? Witness) : FirstRunCall;
    public sealed record MarkComplete : FirstRunCall;
    public sealed record MarkWatchOnlyComplete : FirstRunCall;
}

/// <summary>State to ordered daemon calls; pure and recomputed at every commit (FirstRunPlan.swift:142-270).</summary>
public static class FirstRunPlan
{
    public static IReadOnlyList<FirstRunCall> Calls(FirstRunState state, CommitPoint commit) => commit switch
    {
        CommitPoint.DeclareRoots or CommitPoint.LeaveTools => Roots(state),
        CommitPoint.SignIn => SignIn(state),
        _ => Start(state),
    };

    private static List<FirstRunCall> Roots(FirstRunState state)
    {
        string? json = state.SessionRoots.SettingsJson();
        if (json is null)
        {
            return new List<FirstRunCall>();
        }

        if (!state.DaemonStarted)
        {
            return new List<FirstRunCall> { new FirstRunCall.StartDaemon(json) };
        }

        return ChangedDeclarations(state.StartedSettingsJson, json) is { } changed
            ? new List<FirstRunCall> { new FirstRunCall.SetSourceSettings(changed) }
            : new List<FirstRunCall>();
    }

    private static List<FirstRunCall> SignIn(FirstRunState state)
    {
        if (state.SessionRoots.SettingsJson() is null)
        {
            return new List<FirstRunCall>();
        }

        List<FirstRunCall> calls = Roots(state);
        string invite = state.Invite.Trim();
        bool joinsInvite = state.Account != AccountAnswer.WatchOnly && invite.Length > 0;
        if (joinsInvite && state.EnrolledInvite != invite)
        {
            calls.Add(new FirstRunCall.LookupInvite(invite));
            calls.Add(new FirstRunCall.Enroll(invite));
        }

        if (state.Account == AccountAnswer.NearAI)
        {
            if (joinsInvite || state.EnrolledInvite is not null)
            {
                if (!state.SignedIn)
                {
                    calls.Add(new FirstRunCall.SignInNearAI());
                }
            }
            else if (!state.NearAIEnrolled)
            {
                calls.Add(new FirstRunCall.NearAILogin());
                calls.Add(new FirstRunCall.EnrollNearAI());
            }
        }

        return calls;
    }

    /// <summary>The declarations that changed since the start; a withdrawn key is sent as off (FirstRunPlan.swift:180-198).</summary>
    public static string? ChangedDeclarations(string? started, string current)
    {
        Dictionary<string, Dictionary<string, string>>? now = Declarations(current);
        if (now is null)
        {
            return current;
        }

        Dictionary<string, Dictionary<string, string>> before = (started is null ? null : Declarations(started)) ?? new();
        var changed = new SortedDictionary<string, SortedDictionary<string, string>>(StringComparer.Ordinal);
        foreach (var (key, d) in now)
        {
            if (!before.TryGetValue(key, out var b) || !Same(b, d))
            {
                changed[key] = new SortedDictionary<string, string>(d, StringComparer.Ordinal);
            }
        }

        foreach (string key in before.Keys)
        {
            if (!now.ContainsKey(key))
            {
                changed[key] = new SortedDictionary<string, string>(StringComparer.Ordinal) { ["mode"] = "off" };
            }
        }

        return changed.Count == 0 ? null : JsonSerializer.Serialize(changed);
    }

    private static bool Same(Dictionary<string, string> a, Dictionary<string, string> b) =>
        a.Count == b.Count && a.All(p => b.TryGetValue(p.Key, out string? v) && v == p.Value);

    private static Dictionary<string, Dictionary<string, string>>? Declarations(string json)
    {
        try
        {
            return JsonSerializer.Deserialize<Dictionary<string, Dictionary<string, string>>>(json);
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>Start sharing on Join (FirstRunPlan.swift:224-269).</summary>
    private static List<FirstRunCall> Start(FirstRunState state)
    {
        bool watchOnly = state.Account == AccountAnswer.WatchOnly;
        var calls = new List<FirstRunCall>();
        if (watchOnly && state.DaemonHoldsEnrolment)
        {
            return calls;
        }

        if (!watchOnly && !state.HoldsEnrolment)
        {
            return calls;
        }

        if (!watchOnly)
        {
            if (state.Sharing == SharingPath.AskMe)
            {
                calls.Add(new FirstRunCall.SetAskEachTime());
            }

            calls.Add(new FirstRunCall.SetConsentScopes(state.Scopes.ToList()));
        }

        bool grants = state.Sharing == SharingPath.Automatic && state.GrantReady && FirstRunNavigation.CanChooseAutomatic(state);
        bool AfterGrant(string id) => grants && state.Rules.TryGetValue(id, out ProjectRule r) && r == ProjectRule.Ask;
        var later = new List<FirstRunCall>();
        foreach (var (id, mode) in state.Rules)
        {
            if (mode != ProjectRule.AutoUpload || FirstRunNavigation.CanChooseAutomatic(state))
            {
                var call = new FirstRunCall.SetProjectMode(id, mode);
                (AfterGrant(id) ? later : calls).Add(call);
            }
        }

        var laterIncludes = new List<FirstRunCall>();
        foreach (var (id, sessions) in state.PastSelections)
        {
            if ((state.Rules.TryGetValue(id, out ProjectRule r) && r == ProjectRule.Ignore) || sessions.Count == 0)
            {
                continue;
            }

            var call = new FirstRunCall.IncludePastSessions(id, sessions.ToList());
            (AfterGrant(id) ? laterIncludes : calls).Add(call);
        }

        if (state.PrivateAI)
        {
            calls.Add(new FirstRunCall.SetPrivateAI(true));
        }

        if (grants)
        {
            calls.Add(new FirstRunCall.GrantAutomatic(state.WitnessSigningAddress));
        }

        calls.AddRange(later);
        calls.AddRange(laterIncludes);
        calls.Add(watchOnly ? new FirstRunCall.MarkWatchOnlyComplete() : new FirstRunCall.MarkComplete());
        return calls;
    }
}
