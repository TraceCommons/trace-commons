using System;
using System.Collections.Generic;
using System.Globalization;
using System.Text.Json;

namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// The core's line for a failed commit, by the step that shows it
/// (FoldersScreen.swift:47-80, UsesScreen.swift:224-249,
/// JoinScreen.swift:177-201). Pure: the window shows what these return.
/// </summary>
public static class FirstRunNotices
{
    /// <summary>
    /// Tools' and the sign-in's line (<c>FoldersScreenLayout.notice</c>): a
    /// failed start reads <c>watcher_start_failed</c> and the step stays; a
    /// changed declaration refused reads <c>settings_failed</c>; a near.ai
    /// enrollment refusal is the core's line for its label, or the enroll
    /// refusal when it has none.
    /// </summary>
    /// <param name="watcherStartFailed">The onboarding copy's <c>watcher_start_failed</c>, or null.</param>
    /// <param name="enrollLine">The core's line for a near.ai enrollment label (<c>tc_near_ai_enroll_line</c>).</param>
    public static string? Tools(FirstRunFailure? failure, FirstRunCopy copy, string? watcherStartFailed, Func<string, string?> enrollLine) => failure switch
    {
        FirstRunFailure.StartFailed => Nonempty(watcherStartFailed),
        FirstRunFailure.SettingsFailed => copy.Folders.SettingsFailed,
        FirstRunFailure.EnrollFailed => copy.Folders.EnrollRefused,
        FirstRunFailure.LookupUnavailable => copy.Folders.LookupUnavailable,
        FirstRunFailure.SignInFailed => copy.Folders.SignInFailed,
        FirstRunFailure.NearAIEnrollFailed n => Nonempty(enrollLine(n.Label)) ?? copy.Folders.EnrollRefused,
        _ => null,
    };

    /// <summary>
    /// Start's line (<c>UsesScreenLayout.notice</c>): a refused grant reads
    /// <c>sharing_refused</c>; Tools' and the sign-in's failures, which never
    /// stop Start, read nothing here.
    /// </summary>
    /// <param name="privateWriteUnconfirmed">The Private AI copy's <c>write_unconfirmed</c>, or null.</param>
    public static string? Start(FirstRunFailure? failure, UsesWords uses, string? privateWriteUnconfirmed) => failure switch
    {
        FirstRunFailure.GrantRefused => uses.SharingRefused,
        FirstRunFailure.ScopesFailed => uses.ScopesFailed,
        FirstRunFailure.SharingFailed => Nonempty(uses.SharingSaveFailed) ?? uses.RulesFailed,
        FirstRunFailure.RulesFailed => uses.RulesFailed,
        FirstRunFailure.PrivateAIFailed => Nonempty(privateWriteUnconfirmed) ?? uses.PrivateAiFailed,
        FirstRunFailure.CompleteFailed => uses.CompleteFailed,
        _ => null,
    };

    /// <summary>
    /// Why near.ai is no longer chosen (<c>JoinScreenLayout.nearAINotice</c>):
    /// shown under its card only while no account is answered.
    /// </summary>
    public static string? JoinNearAI(FirstRunState state, FirstRunFailure? failure, FirstRunCopy copy, Func<string, string?> enrollLine)
    {
        if (state.Account != AccountAnswer.None)
        {
            return null;
        }

        return failure is FirstRunFailure.SignInFailed or FirstRunFailure.NearAIEnrollFailed
            ? Tools(failure, copy, null, enrollLine)
            : null;
    }

    /// <summary>
    /// The line under Join's cards (<c>JoinScreenLayout.notice</c>): Start's,
    /// then the sign-in's. One said under the near.ai card, or a dead invite
    /// said on the invite card, is not said twice.
    /// </summary>
    public static string? Join(FirstRunFailure? failure, FirstRunState state, FirstRunCopy copy, string? watcherStartFailed, string? privateWriteUnconfirmed, Func<string, string?> enrollLine)
    {
        if (Start(failure, copy.Uses, privateWriteUnconfirmed) is { } start)
        {
            return start;
        }

        if (JoinNearAI(state, failure, copy, enrollLine) is not null || failure is FirstRunFailure.InviteDead)
        {
            return null;
        }

        return Tools(failure, copy, watcherStartFailed, enrollLine);
    }

    /// <summary>
    /// What a finished first run must still say once its window has gone
    /// (<c>UsesScreenLayout.finishedNotice</c>): Automatic was refused, so
    /// sharing is on Ask.
    /// </summary>
    public static string? Finished(FirstRunFailure? refusal, UsesWords uses) =>
        refusal is FirstRunFailure.GrantRefused ? Nonempty(uses.SharingRefused) : null;

    private static string? Nonempty(string? s) => string.IsNullOrEmpty(s) ? null : s;
}

/// <summary>What the invite card's status line reads (JoinScreen.swift <c>JoinInviteLine</c>).</summary>
public abstract record JoinInviteLine
{
    public sealed record Hidden : JoinInviteLine;
    public sealed record Host(string Name) : JoinInviteLine;
    public sealed record Joined(string Line) : JoinInviteLine;
    public sealed record Error(string Line) : JoinInviteLine;
}

/// <summary>What "Look up" did (JoinScreen.swift <c>JoinLookUpOutcome</c>).</summary>
public enum JoinLookUpOutcome
{
    Found,
    Refused,
    Withdrawn,
}

/// <summary>
/// Join's invite card (JoinScreen.swift:203-335): drawn only while an invite
/// is held, looked up and joined by the daemon at sign-in
/// (<see cref="CommitPoint.SignIn"/>), never on a guess.
/// </summary>
public static class JoinInvite
{
    /// <summary>The one credit-range unit the daemon accepts (<c>CREDIT_RANGE_UNIT_POINTS</c>).</summary>
    public const string PointsPerAcceptedTrace = "points_per_accepted_trace";

    /// <summary>An invite is held: one kept, or one already enrolled.</summary>
    public static bool HoldsInvite(FirstRunState state) =>
        state.Invite.Trim().Length > 0 || state.EnrolledInvite is not null;

    /// <summary>The card is drawn only while an invite is held (JoinScreen.swift:206-208).</summary>
    public static bool ShowsCard(FirstRunState state) => HoldsInvite(state);

    /// <summary>Once the daemon enrolled an invite the field is read-only.</summary>
    public static bool IsEditable(FirstRunState state) => state.EnrolledInvite is null;

    /// <summary>An invite this first run (or an earlier one) is known to have joined.</summary>
    public static bool Joined(FirstRunState state) => state.EnrolledInvite is { } e && e.Trim().Length > 0;

    /// <summary>The field shows unless an invite was joined.</summary>
    public static bool ShowsField(FirstRunState state) => IsEditable(state) || !Joined(state);

    /// <summary>"Look up" acts on a non-empty field, or an empty one while an invite is kept.</summary>
    public static bool CanLookUp(string draft, FirstRunState state) => draft.Trim().Length > 0 || state.Invite.Length > 0;

    /// <summary>
    /// "Look up" (JoinScreen.swift:241-266): the host is read locally; the
    /// invite is looked up and joined when the person signs in. Something
    /// that is not an invite is refused and not kept. An empty field
    /// withdraws the kept invite and the refusal that went with it.
    /// </summary>
    public static (FirstRunState State, JoinLookUpOutcome Outcome, FirstRunFailure? Failure) LookUp(
        string draft, FirstRunState state, FirstRunFailure? failure, Func<string, string?> host)
    {
        string invite = draft.Trim();
        FirstRunState looked = state.Clone();
        FirstRunFailure? deadCleared = failure is FirstRunFailure.InviteDead ? null : failure;
        if (invite.Length == 0)
        {
            looked.Invite = "";
            looked.IssuerHost = null;
            return (looked, JoinLookUpOutcome.Withdrawn, deadCleared);
        }

        if (host(invite) is not { } issuerHost)
        {
            if (failure is FirstRunFailure.InviteDead)
            {
                return (looked, JoinLookUpOutcome.Refused, failure);
            }

            looked.Invite = "";
            looked.IssuerHost = null;
            return (looked, JoinLookUpOutcome.Refused, failure);
        }

        looked.Invite = invite;
        looked.IssuerHost = issuerHost;
        if (looked.Account == AccountAnswer.WatchOnly)
        {
            looked.Account = AccountAnswer.None;
        }

        bool same = invite == state.Invite.Trim();
        return (looked, JoinLookUpOutcome.Found, same ? failure : deadCleared);
    }

    /// <summary>
    /// A deep link's invite, offered to this first run: kept as "Look up"
    /// keeps a pasted one, so Join's card shows what would be joined and the
    /// sign-in joins it. Nothing is kept once an invite is enrolled, and
    /// something that is not an invite is never kept.
    /// </summary>
    public static FirstRunState Offer(string invite, FirstRunState state, Func<string, string?> host)
    {
        if (!IsEditable(state))
        {
            return state;
        }

        var (looked, outcome, _) = LookUp(invite, state, null, host);
        return outcome == JoinLookUpOutcome.Found ? looked : state;
    }

    /// <summary>The card's status line (JoinScreen.swift:298-321).</summary>
    public static JoinInviteLine Line(FirstRunState state, string? lookupJson, FirstRunFailure? failure, JoinWords copy, bool refused)
    {
        if (Joined(state))
        {
            return new JoinInviteLine.Joined(FirstRunCopy.Fill(copy.InviteJoined, new Dictionary<string, string>
            {
                ["host"] = state.IssuerHost ?? copy.Unknown,
                ["pay_range"] = PayRange(lookupJson, copy),
            }));
        }

        if (refused)
        {
            return new JoinInviteLine.Error(copy.InviteError);
        }

        if (failure is FirstRunFailure.InviteDead)
        {
            return new JoinInviteLine.Error(copy.InviteDead);
        }

        if (state.Account == AccountAnswer.WatchOnly)
        {
            return new JoinInviteLine.Hidden();
        }

        return state.IssuerHost is { } h && state.Invite.Length > 0 ? new JoinInviteLine.Host(h) : new JoinInviteLine.Hidden();
    }

    /// <summary>
    /// The invite's credit range in the core's words, or its <c>unknown</c>
    /// for no range or a unit this build cannot word (JoinScreen.swift:324-331).
    /// </summary>
    public static string PayRange(string? lookupJson, JoinWords copy)
    {
        if (lookupJson is null)
        {
            return copy.Unknown;
        }

        try
        {
            using JsonDocument doc = JsonDocument.Parse(lookupJson);
            if (!doc.RootElement.TryGetProperty("credit_range", out JsonElement range) || range.ValueKind != JsonValueKind.Object
                || !range.TryGetProperty("unit", out JsonElement unit) || unit.ValueKind != JsonValueKind.String || unit.GetString() != PointsPerAcceptedTrace
                || !range.TryGetProperty("min", out JsonElement min) || !min.TryGetInt64(out long lo)
                || !range.TryGetProperty("max", out JsonElement max) || !max.TryGetInt64(out long hi))
            {
                return copy.Unknown;
            }

            if (lo == hi)
            {
                return FirstRunCopy.Fill(copy.PayRangePointsOne, new Dictionary<string, string> { ["min"] = lo.ToString(CultureInfo.InvariantCulture) });
            }

            return FirstRunCopy.Fill(copy.PayRangePoints, new Dictionary<string, string>
            {
                ["min"] = lo.ToString(CultureInfo.InvariantCulture),
                ["max"] = hi.ToString(CultureInfo.InvariantCulture),
            });
        }
        catch (JsonException)
        {
            return copy.Unknown;
        }
    }
}
