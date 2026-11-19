using System;
using System.Collections;
using System.Collections.Generic;
using System.Globalization;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// The whole first-run wording table, <c>tc_first_run_copy_json</c>
/// (crates/trace-commons-contributor/src/first_run_copy.rs:531-544), decoded
/// field by field as the Mac's <c>FirstRunCopy</c> does
/// (macos/Sources/TCShellCore/FirstRunCopy.swift). Every word the first run
/// shows comes from here; the shell writes none.
/// </summary>
public sealed record FirstRunCopy
{
    public TraceCommons.Interop.AgentConfigurationCopy AgentConfiguration { get; init; } = new();
    public FrameWords Frame { get; init; } = new();
    public WelcomeWords Welcome { get; init; } = new();
    public JoinWords Join { get; init; } = new();
    public FoldersWords Folders { get; init; } = new();
    public ToolsWords Tools { get; init; } = new();
    public RulesWords Rules { get; init; } = new();
    public UsesWords Uses { get; init; } = new();
    public TracesWords Traces { get; init; } = new();
    public PrivateAiWords PrivateAi { get; init; } = new();
    public TermsHoldWords TermsHold { get; init; } = new();

    private static readonly JsonSerializerOptions Options = new()
    {
        PropertyNamingPolicy = JsonNamingPolicy.SnakeCaseLower,
        UnmappedMemberHandling = JsonUnmappedMemberHandling.Disallow,
    };

    /// <summary>The table from the core, or null if it will not decode or has a blank word.</summary>
    public static FirstRunCopy? Load() => Decode(NativeMethods.TakeOwnedString(NativeMethods.tc_first_run_copy_json()));

    /// <summary>
    /// Decode the table, or null if it will not parse, carries a field this
    /// build does not know, or any string in it is empty: a first run with a
    /// blank control is refused, not shown (FirstRunCopy.swift:325-327).
    /// </summary>
    public static FirstRunCopy? Decode(string? json)
    {
        if (string.IsNullOrEmpty(json))
        {
            return null;
        }

        try
        {
            FirstRunCopy? copy = JsonSerializer.Deserialize<FirstRunCopy>(json, Options);
            return copy is not null && NoBlank(copy) ? copy : null;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary><c>FirstRunCopy.fill</c> (FirstRunCopy.swift:319-323): each <c>{key}</c> replaced.</summary>
    public static string Fill(string template, IReadOnlyDictionary<string, string> values)
    {
        string text = template;
        foreach (var (key, value) in values)
        {
            text = text.Replace("{" + key + "}", value, StringComparison.Ordinal);
        }

        return text;
    }

    /// <summary>The thread's name for a step; Welcome has none (FirstRunFrame.swift:66-77).</summary>
    public string? ThreadLabel(FirstRunStep step) => step switch
    {
        FirstRunStep.Tools => Frame.StepTools,
        FirstRunStep.Uses => Frame.StepUses,
        FirstRunStep.Traces => Frame.StepTraces,
        FirstRunStep.PrivateAI => Frame.StepPrivateAi,
        FirstRunStep.Terms => Frame.StepTerms,
        FirstRunStep.Join => Frame.StepJoin,
        _ => null,
    };

    /// <summary>The thread's names in navigation order (FirstRunFrame.swift:80-82).</summary>
    public IReadOnlyList<string> ThreadLabels
    {
        get
        {
            var labels = new List<string>();
            foreach (FirstRunStep step in FirstRunNavigation.ThreadSteps)
            {
                if (ThreadLabel(step) is { } l)
                {
                    labels.Add(l);
                }
            }

            return labels;
        }
    }

    private static bool NoBlank(object node)
    {
        foreach (PropertyInfo p in node.GetType().GetProperties(BindingFlags.Public | BindingFlags.Instance))
        {
            if (p.GetIndexParameters().Length > 0 || p.Name == "EqualityContract")
            {
                continue;
            }

            object? value = p.GetValue(node);
            switch (value)
            {
                case string s when s.Length == 0:
                    return false;
                case string:
                    break;
                case IEnumerable list:
                    foreach (object? item in list)
                    {
                        if (item is not string { Length: > 0 })
                        {
                            return false;
                        }
                    }

                    break;
                case null:
                    return false;
                default:
                    if (value is not ValueType && value.GetType().Assembly == typeof(FirstRunCopy).Assembly && !NoBlank(value))
                    {
                        return false;
                    }

                    break;
            }
        }

        return true;
    }
}

public sealed record FrameWords
{
    public string QuickSetup { get; init; } = "";
    public string CustomSetup { get; init; } = "";
    public string StepJoin { get; init; } = "";
    public string StepFolders { get; init; } = "";
    public string StepTools { get; init; } = "";
    public string StepRules { get; init; } = "";
    public string StepUses { get; init; } = "";
    public string StepTraces { get; init; } = "";
    public string StepPrivateAi { get; init; } = "";
    public string StepTerms { get; init; } = "";
    public string CustomSetupInstead { get; init; } = "";
    public string ContinueButton { get; init; } = "";
    public string AnswerEveryTool { get; init; } = "";
    public string SessionCount { get; init; } = "";
    public string SessionCountOne { get; init; } = "";
    public string Undo { get; init; } = "";
    public string Choose { get; init; } = "";
    public string Back { get; init; } = "";
    public string Cancel { get; init; } = "";
    public string Close { get; init; } = "";

    /// <summary>A tool's trace count line (FirstRunCopy.swift:47-49).</summary>
    public string SessionCountLine(ulong count) =>
        count == 1 ? SessionCountOne : FirstRunCopy.Fill(SessionCount, new Dictionary<string, string> { ["count"] = count.ToString("N0", CultureInfo.CurrentCulture) });
}

public sealed record JoinWords
{
    public string TitleLight { get; init; } = "";
    public string TitleBold { get; init; } = "";
    public string Body { get; init; } = "";
    public string InviteEyebrow { get; init; } = "";
    public string InvitePlaceholder { get; init; } = "";
    public string LookUp { get; init; } = "";
    public string InviteJoined { get; init; } = "";
    public string InviteError { get; init; } = "";
    public string InviteDead { get; init; } = "";
    public string PayRangePoints { get; init; } = "";
    public string PayRangePointsOne { get; init; } = "";
    public string NearAiEyebrow { get; init; } = "";
    public string NearAiText { get; init; } = "";
    public string NearAiSignIn { get; init; } = "";
    public string NearAiChosen { get; init; } = "";
    public string Unknown { get; init; } = "";
    public string SignedIn { get; init; } = "";
    public string NoSharing { get; init; } = "";
    public string SkipNote { get; init; } = "";
    public string Skip { get; init; } = "";
}

public sealed record WelcomeWords
{
    public string Title { get; init; } = "";
    public string Body { get; init; } = "";
    public string Start { get; init; } = "";
    public LegalWords Legal { get; init; } = new();
}

public sealed record LegalWords
{
    public string TitleLight { get; init; } = "";
    public string Accept { get; init; } = "";
    public string Terms { get; init; } = "";
    public string Privacy { get; init; } = "";
    public string TermsUrl { get; init; } = "";
    public string PrivacyUrl { get; init; } = "";
    public string ScrollHint { get; init; } = "";
    public string Loading { get; init; } = "";
    public string Failed { get; init; } = "";
    public string Retry { get; init; } = "";
    public string NotInEffect { get; init; } = "";
    public string AcceptButton { get; init; } = "";
}

public sealed record TermsHoldWords
{
    public string NoticeTitle { get; init; } = "";
    public string NoticeBody { get; init; } = "";
    public string Review { get; init; } = "";
    public string MenuRow { get; init; } = "";
}

public sealed record TracesWords
{
    public string TitleLight { get; init; } = "";
    public string TitleBold { get; init; } = "";
    public string OnThisMac { get; init; } = "";
    public string OnThisComputer { get; init; } = "";
    public string FoundCredit { get; init; } = "";
    public string FoundCouldCredit { get; init; } = "";
    public string CreditDepends { get; init; } = "";
    public string NoCredit { get; init; } = "";
    public string BackToUses { get; init; } = "";
}

public sealed record FoldersWords
{
    public string TitleLight { get; init; } = "";
    public string TitleBold { get; init; } = "";
    public string Body { get; init; } = "";
    public string Loading { get; init; } = "";
    public string LoadingThisComputer { get; init; } = "";
    public string Watch { get; init; } = "";
    public string DontUse { get; init; } = "";
    public string WatchQuestion { get; init; } = "";
    public string ChooseFolder { get; init; } = "";
    public string ChooseToolFolder { get; init; } = "";
    public string ChooseToolFolderAccessibility { get; init; } = "";
    public string GetTool { get; init; } = "";
    public string DownloadTool { get; init; } = "";
    public string DiscoveryFailed { get; init; } = "";
    public string Retry { get; init; } = "";
    public string EnrollRefused { get; init; } = "";
    public string LookupUnavailable { get; init; } = "";
    public string SignInFailed { get; init; } = "";
    public string SettingsFailed { get; init; } = "";
}

public sealed record ToolsWords
{
    public string TitleLight { get; init; } = "";
    public string TitleBold { get; init; } = "";
    public string AddTool { get; init; } = "";
    public string AddToolCaption { get; init; } = "";
    public string AddToolRefused { get; init; } = "";
    public string AddedByYou { get; init; } = "";
    public string WhichKind { get; init; } = "";
    public string TrajectoryLabel { get; init; } = "";
    public string Neither { get; init; } = "";
    public string OneFolderPerTool { get; init; } = "";
}

public sealed record RulesWords
{
    public string TitleLight { get; init; } = "";
    public string TitleBold { get; init; } = "";
    public string Loading { get; init; } = "";
    public string Empty { get; init; } = "";
    public string ReposFound { get; init; } = "";
    public string RuleFor { get; init; } = "";
    public string PastSessions { get; init; } = "";
    public string SelectedSummary { get; init; } = "";
    public string FolderSelected { get; init; } = "";
    public string IncludeEvery { get; init; } = "";
    public string ShowAll { get; init; } = "";
    public string ShowFewer { get; init; } = "";
    public string NeverCount { get; init; } = "";
    public string NeverLabel { get; init; } = "";
    public string Unavailable { get; init; } = "";
    public string Retry { get; init; } = "";
    public string SessionsUnavailable { get; init; } = "";
    public string PastSessionsWatchOnly { get; init; } = "";
    public string NotListed { get; init; } = "";
    public IReadOnlyList<string> Weekdays { get; init; } = Array.Empty<string>();
    public IReadOnlyList<string> Months { get; init; } = Array.Empty<string>();
    public string SessionDate { get; init; } = "";
    public string DurationMinutes { get; init; } = "";
    public string DurationHours { get; init; } = "";
}

public sealed record UsesWords
{
    public string TitleLight { get; init; } = "";
    public string TitleBold { get; init; } = "";
    public string Eyebrow { get; init; } = "";
    public string Required { get; init; } = "";
    public string AllOptional { get; init; } = "";
    public string RewardOpportunities { get; init; } = "";
    public string AutomaticScope { get; init; } = "";
    public string AskEachTime { get; init; } = "";
    public string SettingsSharingNote { get; init; } = "";
    public string KeepAsksTitle { get; init; } = "";
    public string KeepAsksBody { get; init; } = "";
    public string KeepAsksKeep { get; init; } = "";
    public string KeepAsksAll { get; init; } = "";
    public string OptionalAllOn { get; init; } = "";
    public string OptionalAllOff { get; init; } = "";
    public string OptionalSomeOn { get; init; } = "";
    public string Sharing { get; init; } = "";
    public string SharingLoading { get; init; } = "";
    public string SharingUnavailable { get; init; } = "";
    public string MoreAbout { get; init; } = "";
    public string Start { get; init; } = "";
    public string SharingRefused { get; init; } = "";
    public string SharingSaveFailed { get; init; } = "";
    public string SettingsSharingRefused { get; init; } = "";
    public string SettingsSharingSaveFailed { get; init; } = "";
    public string ScopesFailed { get; init; } = "";
    public string RulesFailed { get; init; } = "";
    public string PrivateAiFailed { get; init; } = "";
    public string CompleteFailed { get; init; } = "";
    public string ThisMac { get; init; } = "";
    public string ThisComputer { get; init; } = "";
    public string YouReview { get; init; } = "";
    public string TheCommons { get; init; } = "";
    public string AutomaticAfterSignIn { get; init; } = "";
    public string AlwaysOn { get; init; } = "";
    public string CreditScope { get; init; } = "";
    public string CreditRequired { get; init; } = "";
}

public sealed record PrivateAiWords
{
    public string Loading { get; init; } = "";
    public string Unavailable { get; init; } = "";
    public string OfferLead { get; init; } = "";
    public string OfferMore { get; init; } = "";
    public string LearnMore { get; init; } = "";
    public string TitleLight { get; init; } = "";
    public string TitleBold { get; init; } = "";
    public string CanConnect { get; init; } = "";
    public string CanConnectOne { get; init; } = "";

    /// <summary>"of {total} tools can connect" (PrivateAIScreen.swift:23-25).</summary>
    public string OfTotal(int total) =>
        FirstRunCopy.Fill(total == 1 ? CanConnectOne : CanConnect, new Dictionary<string, string> { ["total"] = total.ToString("N0", CultureInfo.CurrentCulture) });
}
