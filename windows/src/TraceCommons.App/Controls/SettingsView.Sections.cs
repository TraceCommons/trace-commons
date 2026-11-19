using System.Collections.Generic;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.Controls.Settings;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls;

/// <summary>
/// The Settings window's sections (W16-D1 (a); <c>SettingsModal</c>,
/// SettingsModal.swift): the section list and the rule over each section
/// in the one body, both named from the core (<see cref="SettingsSections"/>),
/// and the new sections' bodies: devices, payouts, General and compute.
/// </summary>
public sealed partial class SettingsView
{
    /// <summary>
    /// The app's compute controller, opened once and kept while the app runs,
    /// as the Mac's <c>ComputeModel</c> is the app's and not the window's.
    /// </summary>
    private static readonly ComputeModel AppCompute = new();

    /// <summary>
    /// Sections whose bodies this window draws. The rest of the Mac's
    /// nineteen have no Windows body yet; their rows and rules are left out
    /// rather than drawn over nothing.
    /// </summary>
    private static readonly HashSet<SettingsSection> Drawn = new()
    {
        SettingsSection.Account, SettingsSection.Payouts, SettingsSection.Referrals, SettingsSection.Watching, SettingsSection.Tools,
        SettingsSection.Projects, SettingsSection.Consent, SettingsSection.PublicProfile, SettingsSection.Changes,
        SettingsSection.PrivateAI, SettingsSection.General, SettingsSection.Startup, SettingsSection.Witness,
        SettingsSection.Compute,
    };

    private readonly Dictionary<SettingsSection, TextBlock> _rules = new();
    private readonly Dictionary<SettingsSection, StackPanel> _bodies = new();
    private readonly Dictionary<SettingsSection, ContentControl> _notices = new();
    private PayoutLinkSection? _payouts;
    private DaemonHost? _sectionsHost;

    /// <summary>The window's onboarding gate; awaiting until the status is read.</summary>
    private SettingsGate _gate = SettingsGate.Awaiting;

    private void BuildSections(DaemonHost host)
    {
        SettingsCards.Styles = Resources;
        SettingsTitleText.Text = MonitorWords.SettingsTitle;
        StartAtLoginSwitch.Header = MonitorShellCopy.Shell("start_at_login");
        AutomationProperties.SetName(SectionList, MonitorShellCopy.Top("settings_sections"));

        _rules[SettingsSection.Account] = RuleAccount;
        _rules[SettingsSection.Payouts] = RulePayouts;
        _rules[SettingsSection.Referrals] = RuleReferrals;
        _rules[SettingsSection.Watching] = RuleWatching;
        _rules[SettingsSection.Tools] = RuleTools;
        _rules[SettingsSection.Insights] = RuleInsights;
        _rules[SettingsSection.Projects] = RuleProjects;
        _rules[SettingsSection.Consent] = RuleConsent;
        _rules[SettingsSection.PublicProfile] = RulePublicProfile;
        _rules[SettingsSection.Changes] = RuleChanges;
        _rules[SettingsSection.PrivateAI] = RulePrivateAI;
        _rules[SettingsSection.SavedAccounts] = RuleSavedAccounts;
        _rules[SettingsSection.General] = RuleGeneral;
        _rules[SettingsSection.Startup] = RuleStartup;
        _rules[SettingsSection.Updates] = RuleUpdates;
        _rules[SettingsSection.Witness] = RuleWitness;
        _rules[SettingsSection.Compute] = RuleCompute;

        _bodies[SettingsSection.Account] = BodyAccount;
        _bodies[SettingsSection.Payouts] = BodyPayouts;
        _bodies[SettingsSection.Referrals] = BodyReferrals;
        _bodies[SettingsSection.Watching] = BodyWatching;
        _bodies[SettingsSection.Tools] = BodyTools;
        _bodies[SettingsSection.Insights] = BodyInsights;
        _bodies[SettingsSection.Projects] = BodyProjects;
        _bodies[SettingsSection.Consent] = BodyConsent;
        _bodies[SettingsSection.PublicProfile] = BodyPublicProfile;
        _bodies[SettingsSection.Changes] = BodyChanges;
        _bodies[SettingsSection.PrivateAI] = BodyPrivateAI;
        _bodies[SettingsSection.SavedAccounts] = BodySavedAccounts;
        _bodies[SettingsSection.General] = BodyGeneral;
        _bodies[SettingsSection.Startup] = BodyStartup;
        _bodies[SettingsSection.Updates] = BodyUpdates;
        _bodies[SettingsSection.Witness] = BodyWitness;
        _bodies[SettingsSection.Compute] = BodyCompute;

        // The notice a gated section draws in its body's place, just under
        // its rule (SettingsModal.swift:186-215).
        foreach ((SettingsSection section, TextBlock rule) in _rules)
        {
            var notice = new ContentControl { HorizontalContentAlignment = HorizontalAlignment.Stretch, IsTabStop = false };
            SectionBody.Children.Insert(SectionBody.Children.IndexOf(rule) + 1, notice);
            _notices[section] = notice;
        }

        _sectionsHost = host;
        Loaded += OnSectionsLoaded;
        Unloaded += (_, _) => host.StatusChanged -= OnGateStatusChanged;

        DevicesHost.Content = new AccountDevicesSection(host);
        _payouts = new PayoutLinkSection(host);
        _payouts.VisibilityChanged += _ => DrawList();
        PayoutsHost.Content = _payouts;
        ReferralsHost.Content = new ReferralsSection(host);
        GeneralHost.Content = new GeneralSection();
        ComputeHost.Content = new ComputeSection(host, AppCompute);

        DrawList();
    }

    /// <summary>
    /// Draws the list and the rules: each drawn section's row in the Mac's
    /// order, a hairline where the group changes, Payouts only while it
    /// applies, and a disabled dash for a row whose words have not loaded.
    /// </summary>
    private void DrawList()
    {
        SettingsNavWords words = SettingsNavWordsLive.Value;
        // Payouts is listed only while it applies, and only once its gate is
        // open: before first run there is no account to pay (D2, R-43).
        bool payoutsShown = _payouts?.Shown == true
            && SettingsGates.For(SettingsSection.Payouts, _gate) == SettingsGate.Open;
        var rows = new List<SettingsSection>();
        foreach (SettingsSection section in SettingsSections.Rows(payoutsShown))
        {
            if (Drawn.Contains(section))
            {
                rows.Add(section);
            }
        }

        SectionList.Children.Clear();
        foreach (SettingsSection section in rows)
        {
            if (SettingsSections.StartsGroup(section, rows))
            {
                SectionList.Children.Add(SettingsCards.Hairline());
            }

            SettingsListRow row = SettingsSections.ListRow(section, words);
            var button = new Button
            {
                Content = new TextBlock { Text = row.Text, Style = (Style)Application.Current.Resources["TcCaptionTextStyle"] },
                IsEnabled = row.Enabled,
                HorizontalAlignment = HorizontalAlignment.Stretch,
                HorizontalContentAlignment = HorizontalAlignment.Left,
                Background = null,
                BorderThickness = new Thickness(0),
                Padding = new Thickness(8, 4, 8, 4),
            };
            AutomationProperties.SetName(button, row.Enabled ? row.Text : MonitorShellCopy.Top("unknown"));
            ToolTipService.SetToolTip(button, row.Enabled ? row.Text : null);
            SettingsSection target = section;
            button.Click += (_, _) => ScrollTo(target);
            SectionList.Children.Add(button);
        }

        foreach ((SettingsSection section, TextBlock rule) in _rules)
        {
            string? name = SettingsSections.NavName(section, words);
            bool shown = Drawn.Contains(section) && name is not null
                && (section != SettingsSection.Payouts || payoutsShown);
            rule.Text = name ?? string.Empty;
            rule.Visibility = shown ? Visibility.Visible : Visibility.Collapsed;
            AutomationProperties.SetHeadingLevel(rule, Microsoft.UI.Xaml.Automation.Peers.AutomationHeadingLevel.Level2);
            ApplyGate(section, shown);
        }
    }

    /// <summary>
    /// Reads whether onboarding is required, as the main window decides it
    /// (OnboardingState.IsSetupComplete over the daemon's status), then gates
    /// the sections first run asks about. A failed read requires onboarding,
    /// so a write surface stays closed. Sample data opens every section, as
    /// a sample launch skips first run.
    /// </summary>
    private async void OnSectionsLoaded(object sender, RoutedEventArgs e)
    {
        if (_sectionsHost is not { } host)
        {
            return;
        }

        // Read again whenever the daemon's status changes while Settings is
        // up, so a Set up again that unenrolls closes the gated sections.
        host.StatusChanged -= OnGateStatusChanged;
        host.StatusChanged += OnGateStatusChanged;
        await ReadGateAsync(host);
    }

    private async void OnGateStatusChanged()
    {
        if (_sectionsHost is { } host && IsLoaded)
        {
            await ReadGateAsync(host);
        }
    }

    private async System.Threading.Tasks.Task ReadGateAsync(DaemonHost host)
    {
        bool requires;
        if (Program.IsSampleLaunch)
        {
            requires = false;
        }
        else
        {
            DaemonResponse status = await host.CallAsync(DaemonProtocol.Methods.Status);
            requires = status.IsError || !OnboardingState.Default().IsSetupComplete(status.ResultAs<DaemonStatus>());
        }

        SettingsGate gate = SettingsGates.Of(onboardingKnown: true, requiresOnboarding: requires);
        if (gate != _gate)
        {
            _gate = gate;
            DrawList();
        }
    }

    /// <summary>
    /// A section's body while its gate is open; otherwise nothing, a spinner
    /// while the core has not said, or the onboarding notice whose button
    /// opens first run. Payouts draws nothing at all unless open (D2).
    /// </summary>
    private void ApplyGate(SettingsSection section, bool shown)
    {
        if (!_bodies.TryGetValue(section, out StackPanel? body) || !_notices.TryGetValue(section, out ContentControl? notice))
        {
            return;
        }

        SettingsGate gate = SettingsGates.For(section, _gate);
        body.Visibility = shown && gate == SettingsGate.Open ? Visibility.Visible : Visibility.Collapsed;
        if (!shown || gate == SettingsGate.Open || section == SettingsSection.Payouts)
        {
            notice.Content = null;
            return;
        }

        if (gate == SettingsGate.Awaiting)
        {
            notice.Content = new ProgressRing { IsActive = true, Width = 16, Height = 16, HorizontalAlignment = HorizontalAlignment.Left };
            return;
        }

        var stack = new StackPanel { Spacing = 8 };
        stack.Children.Add(SettingsCards.Text(MonitorShellCopy.Top("signed_out")));
        stack.Children.Add(SettingsCards.Button(
            MonitorShellCopy.Top("sign_in"),
            (_, _) => SetUpAgainRequested?.Invoke(this, System.EventArgs.Empty)));
        notice.Content = SettingsCards.Card(string.Empty, stack);
    }

    /// <summary>Straight to a section, its rule at the top (<c>scrollTo(_, anchor: .top)</c>, no animation).</summary>
    public void ScrollTo(SettingsSection section)
    {
        if (_rules.TryGetValue(section, out TextBlock? rule) && rule.Visibility == Visibility.Visible)
        {
            rule.StartBringIntoView(new BringIntoViewOptions { AnimationDesired = false, VerticalAlignmentRatio = 0 });
        }
    }
}
