using System;
using System.Globalization;

namespace TraceCommons.Interop;

/// <summary>
/// The roots screen's words.
///
/// In the interop assembly rather than a view model for the reason
/// <see cref="WithdrawCopy"/> gives: this is the consent prompt for watching a
/// developer's real work, so it is a safety property of the shell, and here it
/// is exercised by tests on a machine that cannot build WinUI at all.
///
/// One rule governs everything below: <b>never imply a source is already
/// selected</b>. Discovery describes what is on the machine; it does not
/// answer for the contributor. Copy that reads as a recommendation ("we found
/// your sessions, continue to start contributing") turns a screen that asks
/// into a screen that informs, which is the difference this whole slice
/// exists to make.
/// </summary>
public static class SessionRootsCopy
{
    /// <summary>The screen's heading.</summary>
    public const string Title = "Which session folders should Trace Commons watch?";

    /// <summary>
    /// The screen's body. Says plainly that nothing is watched until asked,
    /// because the contributor arrives here from a refusal and deserves to
    /// know the refusal was deliberate rather than a fault.
    /// </summary>
    public const string Body =
        "Nothing is watched until you say so. Choose a folder for each agent, "
        + "or say you do not use it.";

    /// <summary>The label on the "I don't use this agent" choice.</summary>
    public const string DoNotUse = "I don't use this agent";

    /// <summary>
    /// Shown on every row, whether or not discovery found anything.
    ///
    /// Typing a folder is a first-class way to answer this screen, not a
    /// fallback for when something failed. The conventional locations are
    /// right for most machines, but a contributor who keeps their work
    /// somewhere else -- a relocated profile, a second drive, a store shared
    /// between machines -- must not have to work out for themselves that the
    /// box is editable.
    /// </summary>
    public const string ManualHint =
        "If your sessions are somewhere else, type or paste that folder above.";

    /// <summary>
    /// Why Continue is disabled. Shown rather than left to a greyed-out
    /// button: an unanswered source is not "no", and the contributor cannot
    /// know that from a disabled control.
    /// </summary>
    public const string Incomplete = "Answer for both agents to continue.";

    /// <summary>The display name for a <c>source</c> value.</summary>
    public static string AgentName(string source) => source switch
    {
        SourceDiscovery.ClaudeCode => "Claude Code",
        SourceDiscovery.Codex => "Codex",
        SourceDiscovery.GeminiCli => "Gemini CLI",
        SourceDiscovery.Cline => "Cline",
        // The raw slug, never another agent's name. A row's label is the only
        // thing naming the store being consented to, so an unknown source
        // shows what it actually is rather than borrowing a name it is not.
        _ => source,
    };

    /// <summary>
    /// What discovery found, as one sentence the contributor can consent to.
    ///
    /// The four cases are kept distinct on purpose. Discovery not describing
    /// this source at all, a folder that is not there, a folder that is there
    /// and empty, and a folder holding a thousand sessions are four different
    /// things to be asked about, and collapsing them into "no sessions found"
    /// would hide the only one that matters.
    /// </summary>
    public static string Evidence(SourceCandidate candidate)
    {
        ArgumentNullException.ThrowIfNull(candidate);

        if (!candidate.Exists && string.IsNullOrEmpty(candidate.Path))
        {
            // Discovery returned nothing for this source. Distinct from "the
            // conventional folder is not there": we do not even have a guess
            // to show, so the sentence has to ask rather than report.
            //
            // BOTH conditions, not just the empty path. Exists is only true
            // when a directory was actually stat'd, which cannot have happened
            // without a location -- so an emptiness check that outranked it
            // would answer "no location for this agent" about a store holding
            // thousands of sessions, turning the one line that makes this a
            // consent prompt into a dead end.
            return "Trace Commons has no location for this agent. Type the folder its sessions are in.";
        }

        if (!candidate.Exists)
        {
            return "This folder is not on this machine.";
        }

        if (candidate.SessionCount == 0)
        {
            return "This folder is here, but holds no sessions yet.";
        }

        string sessions = candidate.SessionCount == 1
            ? "1 session"
            : string.Format(
                CultureInfo.CurrentCulture,
                "{0} sessions",
                candidate.SessionCount);

        return string.Format(
            CultureInfo.CurrentCulture,
            "{0}, most recent {1}.",
            sessions,
            HumanWhen(candidate.MostRecent));
    }

    /// <summary>
    /// The note shown when an environment variable moved a store, so an
    /// unfamiliar path reads as explained rather than as a mistake.
    /// </summary>
    public static string RelocatedNote(string source) => source switch
    {
        SourceDiscovery.ClaudeCode => "CLAUDE_CONFIG_DIR moved this folder.",
        SourceDiscovery.Codex => "CODEX_HOME moved this folder.",
        _ => "An environment variable moved this folder.",
    };

    /// <summary>
    /// "3 hours ago". Never an absolute timestamp: a contributor placing a
    /// session in their own day thinks in elapsed time.
    ///
    /// The bands match <c>human_when</c> in
    /// <c>crates/trace-commons-contributor-gtk/src/model.rs</c> so the two
    /// shells describe the same instant the same way. It differs in one
    /// place: a missing timestamp is not "just now" here, because this screen
    /// only asks for one when there are sessions to have produced it, and
    /// answering "just now" about a store with no sessions would be a lie.
    /// </summary>
    public static string HumanWhen(DateTimeOffset? then) =>
        HumanWhen(then, DateTimeOffset.UtcNow);

    /// <summary>
    /// As <see cref="HumanWhen(DateTimeOffset?)"/>, with <paramref name="now"/>
    /// injected so the bands are testable without waiting for a clock.
    /// </summary>
    public static string HumanWhen(DateTimeOffset? then, DateTimeOffset now)
    {
        if (then is null)
        {
            return "unknown";
        }

        long mins = Math.Max(0, (long)(now - then.Value).TotalMinutes);

        return mins switch
        {
            <= 1 => "just now",
            <= 59 => string.Format(CultureInfo.CurrentCulture, "{0} minutes ago", mins),
            <= 119 => "an hour ago",
            <= 1439 => string.Format(CultureInfo.CurrentCulture, "{0} hours ago", mins / 60),
            <= 2879 => "yesterday",
            _ => string.Format(CultureInfo.CurrentCulture, "{0} days ago", mins / 1440),
        };
    }
}
