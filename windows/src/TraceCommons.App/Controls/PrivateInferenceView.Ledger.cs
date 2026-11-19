using System;
using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// The Mac tab body's sections on Windows (spec W19-D1 (a), InferenceTabView,
/// InferenceViews.swift:58-89): the guide card, the spending-organization
/// prompt, and the ledger, in the Mac's order around the managed cards.
/// </summary>
public sealed partial class PrivateInferenceView
{
    private InferenceLedgerModel? _ledger;
    private PrivateAIGuide? _guide;
    private SpendOrganizationPromptModel? _spendPrompt;

    /// <summary>Opens Settings, where the switch, the tools and the organization picker are.</summary>
    public event EventHandler? OpenSettingsRequested;

    public InferenceLedgerModel Ledger => _ledger ??= CreateLedger();

    public PrivateAIGuide Guide => _guide ??= new PrivateAIGuide(
        PrivateAIGuide.ManagedWords(),
        PrivateAIGuide.ClosedPath(_host.ShellStateDirectory));

    public SpendOrganizationPromptModel SpendPrompt => _spendPrompt ??=
        new SpendOrganizationPromptModel((m, p) => _host.CallAsync(m, p), () => _host.ConnectionGeneration);

    private InferenceLedgerModel CreateLedger()
    {
        RoutingCopy? routing = RoutingSurface.Copy();
        var insights = LocalInsights.CoreCopy();
        return new InferenceLedgerModel(
            (method, parameters) => _host.CallAsync(method, parameters),
            MonitorShellCopy.Top,
            key => insights.TryGetValue(key, out string? word) ? word : string.Empty,
            id => id switch
            {
                SourceDiscovery.ClaudeCode => routing?.ToolClaude,
                SourceDiscovery.Codex => routing?.ToolCodex,
                SourceDiscovery.GeminiCli => routing?.ToolGemini,
                SourceDiscovery.Cline => routing?.ToolCline,
                _ => null,
            },
            TimeZoneInfo.Local,
            CultureInfo.CurrentCulture);
    }

    /// <summary>Hooks the ledger's reads to the page's life: on show, on a new call, on a status change.</summary>
    private void WireMacSections()
    {
        Loaded += async (_, _) =>
        {
            await Ledger.LoadAsync();
            // Lazily, with the tab and never on the sign-in (InferenceViews.swift:54-56).
            await SpendPrompt.RefreshAsync();
        };
        // Nothing reads while the page is off screen (one-pass 13).
        Unloaded += (_, _) =>
        {
            _host.InferenceCallAdded -= OnLedgerWorthyChange;
            _host.StatusChanged -= OnLedgerWorthyChange;
        };
        Loaded += (_, _) =>
        {
            _host.InferenceCallAdded -= OnLedgerWorthyChange;
            _host.StatusChanged -= OnLedgerWorthyChange;
            _host.InferenceCallAdded += OnLedgerWorthyChange;
            _host.StatusChanged += OnLedgerWorthyChange;
        };
    }

    private async void OnLedgerWorthyChange()
    {
        try
        {
            await Ledger.LoadAsync();
        }
        catch (Exception)
        {
            System.Diagnostics.Trace.TraceWarning(nameof(OnLedgerWorthyChange));
        }
    }

    private void OnCloseGuide(object sender, RoutedEventArgs e)
    {
        Guide.Close();
        GuideCard.Visibility = Visibility.Collapsed;
    }

    private void OnSpendPromptAction(object sender, RoutedEventArgs e) => OpenSettingsRequested?.Invoke(this, EventArgs.Empty);

    private async void OnSpendPromptDismiss(object sender, RoutedEventArgs e) => await SpendPrompt.DismissAsync();

    /// <summary>A proof tag's ink by its tone (<c>GlassTag.Tone</c>), from the theme's status inks.</summary>
    public static Brush ToneBrush(LedgerTone tone) => (Brush)Application.Current.Resources[tone switch
    {
        LedgerTone.On => "TcStatusOnTextBrush",
        LedgerTone.Ask => "TcStatusAskTextBrush",
        LedgerTone.Failed => "TcCoralTextBrush",
        LedgerTone.Outside => "TcStatusOutsideTextBrush",
        _ => "TcInkSecondaryBrush",
    }];

    public static Visibility Shown(bool value) => value ? Visibility.Visible : Visibility.Collapsed;
}
