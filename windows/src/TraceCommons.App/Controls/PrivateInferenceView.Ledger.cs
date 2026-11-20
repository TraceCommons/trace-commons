using System;
using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;
using TraceCommons.App.Look;

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
    private readonly SpendOrganizationModel _spendPrompt;

    /// <summary>Opens Settings (the Private AI Settings card).</summary>
    public event EventHandler? OpenSettingsRequested;

    /// <summary>
    /// The spending prompt's action: Settings, at the organization picker
    /// (SpendOrganizationPrompt.swift, <c>onOpenSettings</c>).
    /// </summary>
    public event EventHandler? ChooseSpendOrganizationRequested;

    public InferenceLedgerModel Ledger => _ledger ??= CreateLedger();

    public PrivateAIGuide Guide => _guide ??= new PrivateAIGuide(
        PrivateAIGuide.ManagedWords(),
        PrivateAIGuide.ClosedPath(_host.ShellStateDirectory));

    /// <summary>The window's spending state; the prompt draws from it.</summary>
    public SpendOrganizationModel SpendPrompt => _spendPrompt;

    private async void OnSpendSwitchSettled()
    {
        try
        {
            await ViewModel.LoadBalanceAsync(fresh: true);
        }
        catch (Exception)
        {
            System.Diagnostics.Trace.TraceWarning(nameof(OnSpendSwitchSettled));
        }
    }

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

    private VisibleReads? _reads;

    /// <summary>
    /// Reads follow the daemon only while this tab is chosen and the window
    /// can be seen (<see cref="VisibleReads"/>, one-pass 13): hidden to the
    /// tray or minimised, nothing reads; shown again, everything is read
    /// afresh, as the Mac's <c>.task</c> restarts when the view reappears
    /// (InferenceViews.swift:48-56).
    /// </summary>
    private VisibleReads Reads
    {
        get
        {
            if (_reads is null)
            {
                _reads = new VisibleReads();
                _reads.SetWindowShown(WindowClocks.Running);
                _reads.Changed += OnReadsChanged;
                WindowClocks.Changed += _reads.SetWindowShown;
            }

            return _reads;
        }
    }

    /// <summary>
    /// The pane is only collapsed when another tab is chosen, so Loaded fires
    /// once; the window calls this and its pair as the tab shows and hides.
    /// </summary>
    public void ActivateMacSections() => Reads.SetTabShown(true);

    /// <summary>The tab left view: nothing reads while it is hidden (one-pass 13).</summary>
    public void DeactivateMacSections() => Reads.SetTabShown(false);

    private async void OnReadsChanged(bool reading)
    {
        _host.InferenceCallAdded -= OnLedgerWorthyChange;
        _host.StatusChanged -= OnLedgerWorthyChange;
        if (!reading) return;
        _host.InferenceCallAdded += OnLedgerWorthyChange;
        _host.StatusChanged += OnLedgerWorthyChange;
        try
        {
            await Ledger.LoadAsync();
            // Lazily, with the tab and never on the sign-in (InferenceViews.swift:54-56).
            await SpendPrompt.RefreshAsync();
        }
        catch (Exception)
        {
            System.Diagnostics.Trace.TraceWarning(nameof(OnReadsChanged));
        }
    }

    /// <summary>The window is closing: stop following and let the clocks go.</summary>
    private void DetachReads()
    {
        if (_reads is null) return;
        WindowClocks.Changed -= _reads.SetWindowShown;
        _reads.SetTabShown(false);
        _reads.Changed -= OnReadsChanged;
        _reads = null;
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

    private void OnSettingsLink(object sender, RoutedEventArgs e) => OpenSettingsRequested?.Invoke(this, EventArgs.Empty);

    private void OnSpendPromptAction(object sender, RoutedEventArgs e) => ChooseSpendOrganizationRequested?.Invoke(this, EventArgs.Empty);

    private async void OnSpendPromptDismiss(object sender, RoutedEventArgs e) => await SpendPrompt.DismissAsync();

    /// <summary>A proof tag's ink by its tone (<c>GlassTag.Tone</c>), from the theme's status inks.</summary>
    public static Brush ToneBrush(LedgerTone tone) => ThemeBrushes.Get(tone switch
    {
        LedgerTone.On => "TcStatusOnTextBrush",
        LedgerTone.Ask => "TcStatusAskTextBrush",
        LedgerTone.Failed => "TcCoralTextBrush",
        LedgerTone.Outside => "TcStatusOutsideTextBrush",
        _ => "TcInkSecondaryBrush",
    });

    /// <summary>The heading chip's one dot for <paramref name="status"/>.</summary>
    public static Visibility DotShown(DotStatus dot, DotStatus status) => Shown(dot == status);

    /// <summary>The chip's word in its status ink (<c>GlassChip</c>: <c>status.textColor</c>, else secondary).</summary>
    public static Brush DotInk(DotStatus dot) => ThemeBrushes.Get(dot switch
    {
        DotStatus.On => "TcStatusOnTextBrush",
        DotStatus.Ask => "TcStatusAskTextBrush",
        DotStatus.Outside => "TcStatusOutsideTextBrush",
        _ => "TcInkSecondaryBrush",
    });

    public static Visibility Shown(bool value) => value ? Visibility.Visible : Visibility.Collapsed;
}
