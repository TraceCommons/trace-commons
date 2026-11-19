using System.Collections.Generic;
using System.Linq;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Settings window's sections against the Mac's (W16-D1 (a)).
/// </summary>
/// <remarks>
/// Expected values were printed by a throwaway Swift probe that compiles
/// macos/Sources/TraceCommonsApp/Views/SettingsSections.swift:1-180 verbatim
/// at b8d4a6d70 (the <c>SettingsSection</c> enum, its groups, listing,
/// hairlines, onboarding gate and <c>navName</c>), with the four words it
/// reads from elsewhere stubbed as markers ("&lt;general&gt;", "&lt;updates&gt;",
/// "&lt;referrals&gt;", "&lt;saved&gt;", "&lt;insights&gt;") and
/// <c>MonitorSettingsNavCopy</c> filled with the core's values.
/// </remarks>
public class SettingsSectionsTests
{
    private static string Raw(SettingsSection s) => s switch
    {
        SettingsSection.WatchedFolders => "watchedFolders",
        SettingsSection.PublicProfile => "publicProfile",
        SettingsSection.PrivateAI => "privateAI",
        SettingsSection.SavedAccounts => "savedAccounts",
        _ => char.ToLowerInvariant(s.ToString()[0]) + s.ToString()[1..],
    };

    private static SettingsNavWords Marked() => new(
        new Dictionary<string, string>
        {
            ["account"] = "Account", ["payouts"] = "Payouts", ["startup"] = "Startup & notifications",
            ["watching"] = "Watching", ["uses"] = "Data uses", ["profile"] = "Public profile",
            ["folders"] = "Watched folders", ["tools"] = "Tools", ["private_ai"] = "Private AI",
            ["witness"] = "Redaction witness", ["projects"] = "Projects", ["log"] = "Change log",
            ["compute"] = "Compute",
        },
        Insights: "<insights>", Updates: "<updates>", Referrals: "<referrals>", SavedAccounts: "<saved>",
        General: "<general>");

    [Fact]
    public void EverySectionIsInTheMacsOrder()
    {
        // probe: ALL
        Assert.Equal(
            "account,payouts,referrals,watching,watchedFolders,tools,insights,projects,consent,publicProfile,changes,privateAI,savedAccounts,general,startup,notifications,updates,witness,compute",
            string.Join(",", SettingsSections.All.Select(Raw)));
    }

    [Fact]
    public void TheListNamesSeventeenSections()
    {
        // probe: LISTED
        Assert.Equal(
            "account,payouts,referrals,watching,tools,insights,projects,consent,publicProfile,changes,privateAI,savedAccounts,general,startup,updates,witness,compute",
            string.Join(",", SettingsSections.Listed.Select(Raw)));
    }

    // probe: one line per section: group, availableBeforeOnboarding,
    // startsGroup in the listed rows, navName with the marked words.
    [Theory]
    [InlineData(SettingsSection.Account, SettingsGroup.Account, true, false, "Account")]
    [InlineData(SettingsSection.Payouts, SettingsGroup.Account, false, false, "Payouts")]
    [InlineData(SettingsSection.Referrals, SettingsGroup.Account, false, false, "<referrals>")]
    [InlineData(SettingsSection.Watching, SettingsGroup.Capture, false, true, "Watching")]
    [InlineData(SettingsSection.WatchedFolders, SettingsGroup.Capture, false, false, null)]
    [InlineData(SettingsSection.Tools, SettingsGroup.Capture, false, false, "Tools")]
    [InlineData(SettingsSection.Insights, SettingsGroup.Capture, false, false, "<insights>")]
    [InlineData(SettingsSection.Projects, SettingsGroup.Sharing, false, true, "Projects")]
    [InlineData(SettingsSection.Consent, SettingsGroup.Sharing, false, false, "Data uses")]
    [InlineData(SettingsSection.PublicProfile, SettingsGroup.Sharing, false, false, "Public profile")]
    [InlineData(SettingsSection.Changes, SettingsGroup.Sharing, false, false, "Change log")]
    [InlineData(SettingsSection.PrivateAI, SettingsGroup.PrivateAI, true, true, "Private AI")]
    [InlineData(SettingsSection.SavedAccounts, SettingsGroup.PrivateAI, false, false, "<saved>")]
    [InlineData(SettingsSection.General, SettingsGroup.App, true, true, "<general>")]
    [InlineData(SettingsSection.Startup, SettingsGroup.App, true, false, "Startup & notifications")]
    [InlineData(SettingsSection.Notifications, SettingsGroup.App, true, false, null)]
    [InlineData(SettingsSection.Updates, SettingsGroup.App, true, false, "<updates>")]
    [InlineData(SettingsSection.Witness, SettingsGroup.Advanced, false, true, "Redaction witness")]
    [InlineData(SettingsSection.Compute, SettingsGroup.Advanced, true, false, "Compute")]
    public void EachSectionMatchesTheMac(
        SettingsSection section, SettingsGroup group, bool beforeOnboarding, bool startsGroup, string? name)
    {
        Assert.Equal(group, SettingsSections.Group(section));
        Assert.Equal(beforeOnboarding, SettingsSections.AvailableBeforeOnboarding(section));
        Assert.Equal(startsGroup, SettingsSections.StartsGroup(section, SettingsSections.Listed));
        Assert.Equal(name, SettingsSections.NavName(section, Marked()));
    }

    [Fact]
    public void ARowWhoseWordsHaveNotLoadedIsADisabledDash()
    {
        // probe: row(nil) is "—|false" for every section but General, and
        // ListRow.row(title: "") is "— false".
        Assert.Equal(new SettingsListRow("—", false), SettingsSections.ListRow(SettingsSection.Account, null));
        Assert.Equal(new SettingsListRow("—", false), SettingsListRow.Row(""));
        Assert.Equal(new SettingsListRow("Account", true), SettingsSections.ListRow(SettingsSection.Account, Marked()));
    }

    [Fact]
    public void PayoutsIsListedOnlyWhileSomethingInItApplies()
    {
        // SettingsModal.swift:38-41: rows(payoutsShown:).
        Assert.DoesNotContain(SettingsSection.Payouts, SettingsSections.Rows(payoutsShown: false));
        Assert.Contains(SettingsSection.Payouts, SettingsSections.Rows(payoutsShown: true));
        Assert.Equal(16, SettingsSections.Rows(payoutsShown: false).Count);
        // Referrals then follows Account in the same group: no hairline.
        Assert.False(SettingsSections.StartsGroup(SettingsSection.Referrals, SettingsSections.Rows(false)));
    }

    [Fact]
    public void TheNavTableIsReadFromTheCoresMonitorWords()
    {
        // The live table (tc_monitor_screens_copy_json) names the Mac's words
        // (preview_copy.rs settings_nav).
        SettingsNavWords live = SettingsNavWordsLive.Value;
        Assert.Equal("Account", SettingsSections.NavName(SettingsSection.Account, live));
        Assert.Equal("Data uses", SettingsSections.NavName(SettingsSection.Consent, live));
        Assert.Equal("Compute", SettingsSections.NavName(SettingsSection.Compute, live));
        Assert.Equal("Updates", SettingsSections.NavName(SettingsSection.Updates, live));
        Assert.Equal("Saved accounts", SettingsSections.NavName(SettingsSection.SavedAccounts, live));
        Assert.Equal("General", SettingsSections.NavName(SettingsSection.General, live));
        // The referral title and Insights title are the core's own.
        Assert.False(string.IsNullOrEmpty(SettingsSections.NavName(SettingsSection.Referrals, live)));
        Assert.False(string.IsNullOrEmpty(SettingsSections.NavName(SettingsSection.Insights, live)));
        // Every listed row is enabled once the words have loaded.
        Assert.All(SettingsSections.Listed, s => Assert.True(SettingsSections.ListRow(s, live).Enabled, s.ToString()));
    }

    [Fact]
    public void TheGeneralWordsAreTheMacs()
    {
        // SettingsSections.swift:191-196.
        Assert.Equal(new SettingsGeneralCopy("General", "Appearance", "Light", "Dark", "System"), SettingsGeneralCopy.Live);
        Assert.Null(SettingsGeneralCopy.Decode("{\"general\":\"General\",\"appearance\":\"Appearance\",\"light\":\" \",\"dark\":\"Dark\",\"system\":\"System\"}"));
        Assert.Null(SettingsGeneralCopy.Decode("[]"));
    }

    [Fact]
    public void AFailedReadIsSaidInTheCoresWordsWithItsRetry()
    {
        // SettingsUnavailable.line (SettingsStateRow.swift:89-94): the
        // monitor table's request_failed; Retry from the Settings table.
        Assert.Equal(MonitorShellCopy.Top("request_failed"), SettingsReadWords.Failed);
        Assert.False(string.IsNullOrEmpty(SettingsReadWords.Failed));
        Assert.False(string.IsNullOrEmpty(SettingsReadWords.Retry));
    }
}
