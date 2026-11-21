using System;
using System.Collections.Generic;
using System.Diagnostics;
using System.Linq;
using System.Runtime.InteropServices;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.Graphics.Canvas.UI.Xaml;
using Microsoft.UI;
using Microsoft.UI.Text;
using Microsoft.UI.Windowing;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Input;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.App.Controls;
using TraceCommons.App.FirstRun;
using TraceCommons.Interop;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Shell;
using Windows.ApplicationModel.DataTransfer;
using Windows.UI.Text;
using Windows.UI.ViewManagement;

using TraceCommons.App.Look;

namespace TraceCommons.App;

/// <summary>
/// The animated first run, as the Mac's (OnboardingCoordinatorView.swift
/// and Views/FirstRun): the stage drawn with Win2D behind the steps, each
/// step built from the core's words, and every motion the Mac's curve,
/// duration and frame rate from <see cref="FirstRunMotion"/>. Replaces the
/// legacy onboarding window.
/// </summary>
/// <remarks>
/// One clock: <c>CompositionTarget.Rendering</c>, subscribed only while the
/// window is shown, not minimised, animations are on and something moves
/// (<see cref="FirstRunStageModel.Interval"/>, the XAML motions in flight).
/// A frame draws only when due at the step's rate, and each layer samples
/// time at its own Mac timeline's rate. With animations off nothing ticks
/// and every state change still redraws the final frame.
/// </remarks>
public sealed partial class FirstRunWindow : Window, IFirstRunSheets
{
    private const double DesignWidth = FirstRunStageLayout.CanvasWidth, DesignHeight = FirstRunStageLayout.CanvasHeight;

    private readonly DaemonHost _host;
    private readonly OnboardingState _onboarding;
    private readonly FirstRunCopy? _copy;
    private readonly PrivateInferenceCopy? _privateCopy = PrivateInferenceSurface.Copy();
    private readonly FirstRunRunner _runner;
    private readonly FirstRunController _controller;
    private readonly FirstRunFocus _focus = new();
    private readonly FirstRunStageModel _model = new();
    private readonly FirstRunStagePainter _painter;
    private readonly Stopwatch _clock = Stopwatch.StartNew();
    private readonly UISettings _ui = new();
    private readonly WindowClockGate _gate = new();
    private readonly List<(Action<double> Apply, Func<double, bool> Moving)> _motions = new();
    private readonly Dictionary<string, string> _names = new(StringComparer.Ordinal);
    private IReadOnlyList<SourceCandidate> _discovered = Array.Empty<SourceCandidate>();

    /// <summary>
    /// How many times discovery has answered. Tools is built when the window
    /// opens, before the probe returns; counting it in the content key builds
    /// the step again with the tools found, signed in or not.
    /// </summary>
    private int _discoveries;
    private List<ConsentOption> _options = new();
    private string? _termsLabel;
    private bool _ticking;
    private double _lastFrame = double.NegativeInfinity;

    /// <summary>The interval the clock last drew at; the backdrop budget judges only fast ones.</summary>
    private double _drawInterval = FirstRunMotion.DisplayRate;

    /// <summary>Whether the window is the foreground one; an unfocused window may draw slower on purpose.</summary>
    private bool _foreground = true;

    /// <summary>The lighter backdrop on a machine that cannot keep up; made at the first draw, from the canvas's device.</summary>
    private StageBackdropBudget? _budget;
    private bool _reduceMotion;
    private FirstRunStep _shownStep = (FirstRunStep)(-1);
    private double? _scanEndedAt;
    private readonly OnboardingCopy? _onboardingCopy = OnboardingCopy.Load();
    private bool _closed;
    private bool _contentStale;
    private string? _shownKey;

    /// <summary>A folder that matched more than one kind, waiting for the person (ToolsScreen.swift <c>PendingFolder</c>).</summary>
    private (string Path, IReadOnlyList<string?> Kinds)? _pendingFolder;

    /// <summary>The last picked folder was refused: the tile's red line (ToolsScreen.swift:791-794).</summary>
    private bool _addRefused;

    /// <summary>Join's invite field while edited, and whether its last look-up was refused.</summary>
    private string? _inviteDraft;
    private bool _inviteRefused;

    public FirstRunWindow(DaemonHost host, OnboardingState onboarding)
        : this(host, onboarding, null)
    {
    }

    /// <summary>
    /// <paramref name="previewAt"/>: a Debug recorded-sample launch's
    /// screenshot preview (<see cref="FirstRunPreview"/>), opened at that step
    /// on a daemon that refuses every call. Null for every real first run.
    /// </summary>
    internal FirstRunWindow(DaemonHost host, OnboardingState onboarding, FirstRunStep? previewAt)
    {
        InitializeComponent();
        WindowClocks.Watch(this);
        _host = host;
        _onboarding = onboarding;
        Title = WindowTitles.FirstRun;
        _copy = FirstRunCopy.Load();
        foreach (string kind in FirstRunStageLayout.RingOrder)
        {
            _names[kind] = SessionRootsCopy.AgentName(kind);
        }

        _painter = new FirstRunStagePainter(_names);
        var daemon = new FirstRunDaemon(
            (method, json) => _host.CallAsync(method, json),
            async json =>
            {
                try
                {
                    await _host.StartAsync(json).ConfigureAwait(true);
                    DaemonStarted?.Invoke();
                    DispatcherQueue.TryEnqueue(async () => await RefreshOptionsAsync());
                    return true;
                }
                catch (TcException)
                {
                    return false;
                }
            },
            () => _host.IsRunning,
            MarkCompleteAsync,
            MarkWatchOnlyCompleteAsync,
            // What a finished first run must still say goes to the shell's
            // notices once this window has gone (UsesScreen.swift:244-250).
            notice => Finished?.Invoke(_copy is null ? null : FirstRunNotices.Finished(notice, Copy.Uses)),
            async uri => await Windows.System.Launcher.LaunchUriAsync(uri));
        daemon.ConsentScopesConfirmed += (before, confirmed) => ConsentScopesConfirmed?.Invoke(before, confirmed);
        _runner = previewAt is FirstRunStep step
            ? new FirstRunRunner(new FirstRunState { Step = step }, new RefusingFirstRunDaemon())
            : new FirstRunRunner(new FirstRunState(), daemon);
        string configDir = _host.ConfigDir;
        _controller = new FirstRunController(_runner, this, () => Task.Run(() => AutomaticGrantCopy.Load(configDir)), _copy?.Uses ?? new UsesWords());
        CurrentStep.GotFocus += (_, e) => _focus.Focused((e.OriginalSource as FrameworkElement) is { } f ? AutomationProperties.GetAutomationId(f) : null);
        _runner.Changed += () => DispatcherQueue.TryEnqueue(() =>
        {
            if (!_closed)
            {
                Render();
            }
        });
        _model.Changed += OnModelChanged;

        Stage.Draw += OnDraw;
        // Where the step's graphic sits once laid out, and again whenever the
        // step's words reflow: a measure queued at the step change ran before
        // layout and read 0, drawing Welcome's N at the window's top edge
        // (box tc-win-az3, 2026-11-19). SetGraphicTop ignores an unchanged top.
        Design.LayoutUpdated += (_, _) => MeasureGraphic();
        Root.SizeChanged += (_, _) => Fit();
        Root.Loaded += async (_, _) =>
        {
            SizeToCanvas();
            await LoadAsync();
        };
        VisibilityChanged += (_, e) => _gate.SetHiddenToTray(!e.Visible);
        Activated += (_, e) =>
        {
            _foreground = e.WindowActivationState != WindowActivationState.Deactivated;
            _budget?.Pause();
            ReadMotionSetting();
        };
        AppWindow.Changed += (sender, args) =>
        {
            if (args.DidPresenterChange || args.DidSizeChange)
            {
                _gate.SetMinimised(sender.Presenter is OverlappedPresenter { State: OverlappedPresenterState.Minimized });
            }
        };
        _gate.Changed += mayTick => { if (mayTick) { ReadMotionSetting(); Kick(); } else { Stop(); FinishInFlight(); } };
        Closed += (_, _) =>
        {
            // A sign-in still waiting on the browser ends with the window:
            // its poll would otherwise run on and draw into a closed window.
            _closed = true;
            _ = _runner.CancelSignIn();
            _gate.SetClosed();
            Stop();
            _painter.Dispose();
        };
        SettingWatcher.Attach(this, ReadMotionSetting);
        HideTitleBar();
        // The canvas is the client area, as the Mac's defaultSize is its content's.
        AppWindow.ResizeClient(new Windows.Graphics.SizeInt32((int)(DesignWidth * Scale()), (int)(DesignHeight * Scale())));
        _reduceMotion = !_ui.AnimationsEnabled;
        _model.SetReduceMotion(_reduceMotion);
        Render();
    }

    /// <summary>
    /// The Mac's hidden title bar (<c>.windowStyle(.hiddenTitleBar)</c>,
    /// TraceCommonsAppMain.swift:150): the stage fills the window and the
    /// system's caption buttons sit over it on a transparent ground, in the
    /// stage's ink. The strip at the top drags the window; the step thread
    /// in it takes the pointer (<see cref="UpdateDragRegions"/>).
    /// </summary>
    private void HideTitleBar()
    {
        ExtendsContentIntoTitleBar = true;
        SetTitleBar(DragStrip);
        AppWindowTitleBar bar = AppWindow.TitleBar;
        bar.ButtonBackgroundColor = Colors.Transparent;
        bar.ButtonInactiveBackgroundColor = Colors.Transparent;
        bar.ButtonForegroundColor = FirstRunStagePainter.Argb(FirstRunTokens.TextPrimary);
        bar.ButtonInactiveForegroundColor = FirstRunStagePainter.Argb(FirstRunTokens.TextTertiary);
        bar.ButtonHoverBackgroundColor = FirstRunStagePainter.Argb(FirstRunTokens.FirstRunControlHover);
        bar.ButtonHoverForegroundColor = FirstRunStagePainter.Argb(FirstRunTokens.TextPrimary);
        bar.ButtonPressedBackgroundColor = FirstRunStagePainter.Argb(FirstRunTokens.FirstRunControl);
        bar.ButtonPressedForegroundColor = FirstRunStagePainter.Argb(FirstRunTokens.TextPrimary);
        Thread.SizeChanged += (_, _) => UpdateDragRegions();
        Thread.LayoutUpdated += (_, _) => UpdateDragRegions();
    }

    /// <summary>The step thread's buttons take the pointer inside the drag strip.</summary>
    private void UpdateDragRegions()
    {
        if (Content?.XamlRoot is null)
        {
            return;
        }

        double scale = Scale();
        if (!TitleBarRegions.IsUsableScale(scale))
        {
            return;
        }

        var rects = new List<LayoutRect>();
        if (Thread.Visibility == Visibility.Visible && Thread.ActualWidth > 0)
        {
            Windows.Foundation.Rect bounds = Thread.TransformToVisual(null)
                .TransformBounds(new Windows.Foundation.Rect(0, 0, Thread.ActualWidth, Thread.ActualHeight));
            rects.Add(new LayoutRect(bounds.X, bounds.Y, bounds.Width, bounds.Height));
        }

        var regions = TitleBarRegions.Passthrough(rects, scale)
            .Select(r => new Windows.Graphics.RectInt32(r.X, r.Y, r.Width, r.Height))
            .ToArray();
        if (!regions.SequenceEqual(_passthrough))
        {
            _passthrough = regions;
            Microsoft.UI.Input.InputNonClientPointerSource.GetForWindowId(AppWindow.Id)
                .SetRegionRects(Microsoft.UI.Input.NonClientRegionKind.Passthrough, regions);
        }
    }

    private Windows.Graphics.RectInt32[] _passthrough = Array.Empty<Windows.Graphics.RectInt32>();

    /// <summary>
    /// Once the content is laid out, the window is sized so the content, not
    /// the client area below a title bar, is the canvas: with the content in
    /// the title bar, a ResizeClient of 860 by 760 still left the content 792
    /// tall (box tc-win-az3, 2026-11-19).
    /// </summary>
    private void SizeToCanvas()
    {
        double scale = Scale();
        if (Root.ActualWidth <= 0 || Root.ActualHeight <= 0)
        {
            return;
        }

        // The frame round the content: the window less the content.
        int extraWidth = AppWindow.Size.Width - (int)Math.Round(Root.ActualWidth * scale);
        int extraHeight = AppWindow.Size.Height - (int)Math.Round(Root.ActualHeight * scale);
        Windows.Graphics.RectInt32 work = DisplayArea.GetFromWindowId(AppWindow.Id, DisplayAreaFallback.Nearest).WorkArea;
        var workArea = new LayoutRect(work.X, work.Y, work.Width, work.Height);
        var outer = new LayoutRect(AppWindow.Position.X, AppWindow.Position.Y, AppWindow.Size.Width, AppWindow.Size.Height);
        LayoutRect frame = FirstRunStageLayout.FirstWindowFrame(outer, scale, extraWidth, extraHeight, workArea);
        var target = new Windows.Graphics.RectInt32((int)frame.X, (int)frame.Y, (int)frame.Width, (int)frame.Height);
        if (target.X != AppWindow.Position.X || target.Y != AppWindow.Position.Y
            || target.Width != AppWindow.Size.Width || target.Height != AppWindow.Size.Height)
        {
            AppWindow.MoveAndResize(target);
        }

        ApplyMinimumSize(extraWidth, extraHeight, workArea);
    }

    /// <summary>
    /// The Mac's minimum window (<see cref="FirstRunStageLayout.MinimumWidth"/>),
    /// through WM_GETMINMAXINFO, held to the work area.
    /// </summary>
    private void ApplyMinimumSize(int extraWidth, int extraHeight, LayoutRect workArea)
    {
        (int width, int height) = FirstRunStageLayout.MinimumWindow(Scale(), extraWidth, extraHeight, workArea);
        WindowMinimumSize.Set(WinRT.Interop.WindowNative.GetWindowHandle(this), width, height);
    }

    /// <summary>The Tools step started the daemon with the person's declaration.</summary>
    public event Action? DaemonStarted;

    /// <summary>The first run's consent write was confirmed: the scopes before it and after (<see cref="FirstRunDaemon.ConsentScopesConfirmed"/>).</summary>
    public event Action<IReadOnlyList<string>, IReadOnlyList<string>>? ConsentScopesConfirmed;

    /// <summary>Set by the caller after the window closes.</summary>
    internal Task CloseCompletion { get; private set; } = Task.CompletedTask;

    private double Now => _clock.Elapsed.TotalSeconds;

    /// <summary>The table; only read once <see cref="Render"/> has seen it decode.</summary>
    private FirstRunCopy Copy => _copy!;

    private double Scale() => Win32Dpi.ForWindow(WinRT.Interop.WindowNative.GetWindowHandle(this)) / 96.0;

    /// <summary>
    /// A deep link's invite, kept as Join's "Look up" keeps one
    /// (<see cref="JoinInvite.Offer"/>): Join's invite card shows what would
    /// be joined, and the sign-in there looks it up and joins it. Nothing is
    /// sent before the person signs in (checklist L5).
    /// </summary>
    public void OfferInvite(string invite)
    {
        _runner.State = JoinInvite.Offer(invite, _runner.State, TraceCommons.Interop.Invite.IssuerHost);
        _contentStale = true;
        Render();
    }

    /// <summary>The first run finished; the line it must still say, or null.</summary>
    public event Action<string?>? Finished;

    /// <summary>Health's Connect target: past Welcome, as Get started.</summary>
    public void GetStarted()
    {
        _runner.ContinueFromWelcome();
        Render();
    }

    // ---- Loading -----------------------------------------------------------

    private async Task LoadAsync()
    {
        _discovered = await Task.Run(SourceDiscovery.ProbeThisMachine).ConfigureAwait(true);
        _discoveries++;
        var rows = _discovered.Select(c => (c.Source, c.Exists)).ToList();
        _runner.State.RecordDiscovery(rows);
        _model.SetRows(rows);
        _model.SetTracesTotal((ulong)_discovered.Where(c => c.Exists).Sum(c => (long)c.SessionCount));
        await RefreshOptionsAsync();
        // An enrollment the daemon already holds is this first run's account
        // (OnboardingNavigation.recordEnrolment), never joined again.
        DaemonResponse status = await _host.CallAsync(DaemonProtocol.Methods.Status).ConfigureAwait(true);
        if (status.ResultAs<DaemonStatus>() is { LoggedIn: true })
        {
            _runner.State = FirstRunNavigation.RecordEnrolment(_runner.State);
        }

        var fetched = await Task.Run(FetchedTerms.Fetch).ConfigureAwait(true);
        _controller.Terms = fetched.Terms;
        _termsLabel = fetched.Label;
        if (_runner.State.Step == FirstRunStep.Terms)
        {
            // Terms built while loading is built again with the document or
            // the failure line, as the Mac's TermsScreen follows its fetch:
            // it said "Loading" for good otherwise (box tc-win-az3, 2026-11-19).
            _contentStale = true;
        }

        Render();
    }


    /// <summary>
    /// <c>consent_options</c>, read again until the daemon answers: on a new
    /// computer the daemon starts on Tools, after the window opened, and
    /// Uses and Join wait on the required use.
    /// </summary>
    private async Task RefreshOptionsAsync()
    {
        if (_options.Count > 0 || !_host.IsRunning)
        {
            return;
        }

        DaemonResponse options = await _host.CallAsync(DaemonProtocol.Methods.ConsentOptions).ConfigureAwait(true);
        _options = options.ResultAs<ConsentOptionsPayload>()?.Scopes ?? new List<ConsentOption>();
        _controller.RequiredScope = _options.FirstOrDefault(o => o.AlwaysOn)?.Name;
        if (_options.Count > 0 && _runner.State.Step is FirstRunStep.Uses or FirstRunStep.Join)
        {
            _contentStale = true;
        }

        Render();
    }

    // ---- Markers ----------------------------------------------------------

    /// <summary>The enrolled marker, keyed by the tenant status names (AppModel.swift:5087-5092).</summary>
    private async Task<bool> MarkCompleteAsync()
    {
        DaemonResponse status = await _host.CallAsync(DaemonProtocol.Methods.Status).ConfigureAwait(true);
        string? tenant = status.ResultAs<DaemonStatus>()?.TenantId;
        if (status.IsError || string.IsNullOrEmpty(tenant))
        {
            return false;
        }

        _onboarding.MarkComplete(tenant);
        return _onboarding.IsComplete(tenant);
    }

    /// <summary>
    /// Watching only, which this first run does not offer: Join needs a
    /// NEAR AI account (JoinScreen.swift canForward). Refused, so a
    /// watch-only Start can never finish here.
    /// </summary>
    private Task<bool> MarkWatchOnlyCompleteAsync() => Task.FromResult(false);

    // ---- The clock --------------------------------------------------------

    private void ReadMotionSetting()
    {
        bool reduce = !_ui.AnimationsEnabled;
        if (reduce != _reduceMotion)
        {
            _reduceMotion = reduce;
            _model.SetReduceMotion(reduce);
            if (reduce)
            {
                Stop();
                FinishInFlight();
            }
            else
            {
                Kick();
            }
        }
    }

    private void OnModelChanged()
    {
        Stage.Invalidate();
        Kick();
    }

    private bool Moving(double now) =>
        _model.Interval(now) is not null || _motions.Any(m => m.Moving(now));

    /// <summary>Start the clock when something moves and the window can be seen.</summary>
    private void Kick()
    {
        if (_ticking || _reduceMotion || !_gate.MayTick || !Moving(Now))
        {
            return;
        }

        _ticking = true;
        Microsoft.UI.Xaml.Media.CompositionTarget.Rendering += OnFrame;
    }

    private void Stop()
    {
        if (_ticking)
        {
            _ticking = false;
            Microsoft.UI.Xaml.Media.CompositionTarget.Rendering -= OnFrame;
        }

        _budget?.Pause();
    }

    private readonly FocusFramePace<XamlRoot> _framePace = new(WindowClocks.Windows);

    private void OnFrame(object? sender, object e)
    {
        // The window's frame rule (FocusFrameRule): at most 15 a second while unfocused.
        if (_framePace.Skips(Content?.XamlRoot, GlassClock.Now))
        {
            return;
        }

        double now = Now;
        if (_reduceMotion || !_gate.MayTick)
        {
            Stop();
            return;
        }

        double? stage = _model.Interval(now);
        bool xaml = _motions.Any(m => m.Moving(now));
        if (stage is null && !xaml)
        {
            Stop();
            ApplyMotions(now);
            Stage.Invalidate();
            return;
        }

        double interval = xaml ? FirstRunMotion.DisplayRate : stage!.Value;
        if (!FirstRunMotion.FrameDue(now - _lastFrame, interval))
        {
            return;
        }

        _lastFrame = now;
        _drawInterval = interval;
        ApplyMotions(now);
        Stage.Invalidate();
    }

    /// <summary>Hiding the window or turning motion off finishes every move at its end state.</summary>
    private void FinishInFlight()
    {
        ApplyMotions(Now + 60);
        Stage.Invalidate();
    }

    private void ApplyMotions(double now)
    {
        foreach (var (apply, _) in _motions)
        {
            apply(_reduceMotion ? now + 1e6 : now);
        }

        if (!_scanEndedAt.HasValue && _model.ToolsScanned)
        {
            _scanEndedAt = now;
        }
    }

    private void OnDraw(CanvasControl sender, CanvasDrawEventArgs args)
    {
        long begun = LookFrames.Begin();
        double now = Now;
        Budget(sender, now);
        _painter.Draw(args.DrawingSession, sender, _model, _reduceMotion ? now + 1e6 : now, sender.ActualWidth, sender.ActualHeight,
            tracesCountText: TracesCount(now), tracesCaption: _copy?.Traces.OnThisComputer,
            privateAIPill: _privateCopy?.StatInferenceAccess is { Length: > 0 } pill ? pill : null,
            usesCaptions: _copy is null ? null : new[] { Copy.Uses.ThisComputer, UsesGateLabel(), Copy.Uses.TheCommons },
            backdrop: _budget.Level);
        LookFrames.End("stage", begun);
    }

    /// <summary>
    /// Counts a frame the clock drew toward the lighter backdrop
    /// (<see cref="StageBackdropBudget"/>): only while the clock runs and the
    /// window is the foreground one, since a still stage or an unfocused
    /// window draws slowly on purpose.
    /// </summary>
    [System.Diagnostics.CodeAnalysis.MemberNotNull(nameof(_budget))]
    private void Budget(CanvasControl sender, double now)
    {
        if (_budget is null)
        {
            var adapter = RenderAdapter.Read(sender.Device);
            _budget = new StageBackdropBudget(RenderAdapter.IsSoftware(adapter));
            string read = adapter.HasValue ? $"{adapter.Value.Vendor:x4}:{adapter.Value.Device:x4}:{adapter.Value.Flags}" : "unread";
            Look.LookTrace.Write($"stage-backdrop adapter={read} level={_budget.Level}");
        }

        if (!_ticking || !_foreground || _reduceMotion)
        {
            _budget.Pause();
        }
        else if (_budget.Drew(now, _drawInterval))
        {
            Look.LookTrace.Write($"stage-backdrop level={_budget.Level}");
        }
    }

    private string TracesCount(double now)
    {
        double counted = _model.TracesCounted ? 1 : TracesGraphic.Counted(now - _model.StepOpenedAt, _reduceMotion);
        return ((ulong)Math.Round(_model.TracesTotal * counted)).ToString("N0", System.Globalization.CultureInfo.CurrentCulture);
    }

    private string UsesGateLabel() => _copy?.Uses.YouReview ?? "";

    private void Fit()
    {
        double w = Root.ActualWidth, h = Root.ActualHeight;
        double k = FirstRunStageLayout.FitScale(w, h);
        Design.Width = k < 1 ? DesignWidth : w;
        Design.Height = k < 1 ? DesignHeight : h;
        DesignScale.ScaleX = DesignScale.ScaleY = k;
        Design.RenderTransformOrigin = new Windows.Foundation.Point(0.5, 0.5);
        MeasureGraphic();
    }

    // ---- Steps ------------------------------------------------------------

    private Border? _graphic;

    private void MeasureGraphic()
    {
        if (_graphic is { } g && g.XamlRoot is not null)
        {
            var at = g.TransformToVisual(Design).TransformPoint(new Windows.Foundation.Point(0, 0));
            _model.SetGraphicTop(_runner.State.Step, at.Y);
        }
    }

    /// <summary>Rebuild the current step when it changes, and its footer each time.</summary>
    private void Render()
    {
        if (_closed)
        {
            return;
        }

        if (_copy is null)
        {
            // A table that will not decode is refused, not shown (FirstRunCopy.swift:325-327).
            Footer.Visibility = Visibility.Collapsed;
            return;
        }

        FirstRunState state = _runner.State;
        if (_runner.Completed)
        {
            Close();
            return;
        }

        if (state.Step != _shownStep)
        {
            double now = Now;
            bool first = (int)_shownStep < 0;
            OutgoingStep.Content = first ? null : CurrentStep.Content;
            _motions.Clear();
            _model.SetStep(state.Step, now);
            if (state.Step == FirstRunStep.Tools && _model.ToolsScanned)
            {
                _scanEndedAt ??= now;
            }

            FirstRunStep? from = first ? null : _shownStep;
            _shownStep = state.Step;
            CurrentStep.Content = BuildStep(state.Step);
            if (OutgoingStep.Content is UIElement leaving)
            {
                _motions.Add((t => { double p = FirstRunMotion.Crossfade(t - now, false); leaving.Opacity = 1 - p; if (p >= 1) { OutgoingStep.Content = null; } }, t => t - now < FirstRunStageLayout.SpatialSettle));
            }

            if (CurrentStep.Content is UIElement arriving && !first)
            {
                _motions.Add((t => arriving.Opacity = FirstRunMotion.Crossfade(t - now, false), t => t - now < FirstRunStageLayout.SpatialSettle));
            }

            BuildThread(from, state.Step, now);
            if (state.Step is FirstRunStep.Uses or FirstRunStep.Join && _options.Count == 0)
            {
                DispatcherQueue.TryEnqueue(async () => await RefreshOptionsAsync());
            }

            _focus.StepChanged();
            if (state.Step is FirstRunStep.Uses or FirstRunStep.Join && !_controller.GrantCopyRead)
            {
                // Uses and Join read the automatic contribution copy as they
                // appear (UsesScreen.swift:665-669, JoinScreen.swift:410-414).
                DispatcherQueue.TryEnqueue(async () =>
                {
                    await _controller.ReadGrantCopy();
                    _contentStale = true;
                    Render();
                });
            }

            ApplyMotions(_reduceMotion ? now + 1e6 : now);
            DispatcherQueue.TryEnqueue(MeasureGraphic);
            _shownKey = ContentKey(state);
            _contentStale = false;
        }
        else if (_contentStale || ContentKey(state) != _shownKey)
        {
            // The same step with new facts (a failure's line, a commit
            // running or ended, the sign-in waiting, a loaded document):
            // its content is built again in place, and the stage and the
            // thread keep their motion.
            _shownKey = ContentKey(state);
            _contentStale = false;
            bool hadFocus = FocusWithin(CurrentStep);
            CurrentStep.Content = BuildStep(state.Step);
            if (hadFocus && CurrentStep.Content is FrameworkElement rebuilt)
            {
                // Focus goes back where it was, by its key, or to the
                // heading (checklist O2, copy guide rule 13).
                rebuilt.Loaded += RestoreFocusOnce;
            }

            ApplyMotions(_reduceMotion ? Now + 1e6 : Now);
            DispatcherQueue.TryEnqueue(MeasureGraphic);
        }

        Footer.Visibility = state.Step == FirstRunStep.Welcome ? Visibility.Collapsed : Visibility.Visible;
        BackButton.Content = Copy.Frame.Back;
        BackButton.Visibility = state.Step == FirstRunStep.Welcome ? Visibility.Collapsed : Visibility.Visible;
        BackButton.IsEnabled = _controller.BackEnabled;
        ContinueButton.Content = state.Step == FirstRunStep.Join ? Copy.Uses.Start : Copy.Frame.ContinueButton;
        ContinueButton.IsEnabled = _controller.CanContinue;
        AutomationProperties.SetName(ContinueButton, (string)ContinueButton.Content);
        AutomationProperties.SetName(BackButton, Copy.Frame.Back);
        Stage.Invalidate();
        Kick();
    }

    /// <summary>
    /// What the step's content shows that can change without a step change;
    /// a different key builds the content again.
    /// </summary>
    private string ContentKey(FirstRunState state) => string.Join("|",
        Notice(state) ?? "", _runner.IsCommitting, _runner.SignInWaiting, state.SignedIn, state.Account,
        state.Invite, state.EnrolledInvite ?? "-", _runner.Failure?.GetType().Name ?? "",
        _pendingFolder?.Path ?? "", _addRefused, _inviteRefused, state.AddedFolders.Count,
        state.SharingChosen, state.Sharing, _controller.GrantCopyRead, _discoveries);

    /// <summary>
    /// The failed request's line under the step's content
    /// (FirstRunFrame.swift:201-205), the step's own mapping of the runner's
    /// failure (FoldersScreen.swift:47-73, JoinScreen.swift:193-201).
    /// </summary>
    private string? Notice(FirstRunState state)
    {
        if (_copy is null)
        {
            return null;
        }

        FirstRunFailure? failure = _runner.Failure;
        string? started = _onboardingCopy?.WatcherStartFailed;
        return state.Step switch
        {
            FirstRunStep.Tools => FirstRunNotices.Tools(failure, Copy, started, NearAiEnrollSurface.Line),
            FirstRunStep.Join => FirstRunNotices.Join(failure, state, Copy, started, _privateCopy?.WriteUnconfirmed, NearAiEnrollSurface.Line),
            _ => null,
        };
    }

    private async void OnContinue(object sender, RoutedEventArgs e)
    {
        Task moving = _controller.Continue();
        Render();
        await moving;
        Render();
    }

    // ---- IFirstRunSheets: what Start asks of the window -------------------

    Task<bool> IFirstRunSheets.ShowDisclosures(SharingDisclosureFlow flow, AutomaticGrantCopy grant) => ShowDisclosuresAsync(flow, grant);

    async Task<bool> IFirstRunSheets.Connected()
    {
        DaemonResponse status = await _host.CallAsync(DaemonProtocol.Methods.Status).ConfigureAwait(true);
        return status.ResultAs<DaemonStatus>()?.LoggedIn == true;
    }

    Task<Flow1GrantRequest?> IFirstRunSheets.GrantRequest(string progressJson) => Task.Run(() => Flow1GrantRequest.Ask(progressJson));

    // ---- Focus across a rebuild ---------------------------------------------

    /// <summary>Whether keyboard focus is inside <paramref name="root"/>.</summary>
    private bool FocusWithin(DependencyObject root)
    {
        if (Content?.XamlRoot is not { } xamlRoot || FocusManager.GetFocusedElement(xamlRoot) is not DependencyObject focused)
        {
            return false;
        }

        for (DependencyObject? at = focused; at is not null; at = VisualTreeHelper.GetParent(at))
        {
            if (ReferenceEquals(at, root))
            {
                return true;
            }
        }

        return false;
    }

    private void RestoreFocusOnce(object sender, RoutedEventArgs e)
    {
        var rebuilt = (FrameworkElement)sender;
        rebuilt.Loaded -= RestoreFocusOnce;
        if (!ReferenceEquals(CurrentStep.Content, rebuilt))
        {
            return;
        }

        string target = _focus.Target(key => Keyed(rebuilt, key) is Control { IsEnabled: true, IsTabStop: true, Visibility: Visibility.Visible });
        if (Keyed(rebuilt, target) is Control control && target != FirstRunFocus.Heading)
        {
            control.Focus(FocusState.Keyboard);
        }
        else if (Keyed(rebuilt, FirstRunFocus.Heading) is Control heading)
        {
            // The heading takes focus only for this: it is no Tab stop.
            heading.IsTabStop = true;
            heading.LostFocus += Untab;
            heading.Focus(FocusState.Keyboard);
        }

        static void Untab(object s, RoutedEventArgs _)
        {
            var c = (Control)s;
            c.IsTabStop = false;
            c.LostFocus -= Untab;
        }
    }

    /// <summary>The element under <paramref name="root"/> with this automation id.</summary>
    private static DependencyObject? Keyed(DependencyObject root, string key)
    {
        if (root is FrameworkElement f && AutomationProperties.GetAutomationId(f) == key)
        {
            return root;
        }

        int n = VisualTreeHelper.GetChildrenCount(root);
        for (int i = 0; i < n; i++)
        {
            if (Keyed(VisualTreeHelper.GetChild(root, i), key) is { } found)
            {
                return found;
            }
        }

        return null;
    }

    /// <summary>Give a control its stable key: the automation id the rebuild finds it by.</summary>
    private static T Key<T>(T element, string key)
        where T : FrameworkElement
    {
        AutomationProperties.SetAutomationId(element, key);
        return element;
    }

    /// <summary>The two disclosure sheets, shared with the Traces toolbar's eye (<see cref="SharingDisclosureSheets"/>).</summary>
    private Task<bool> ShowDisclosuresAsync(SharingDisclosureFlow flow, AutomaticGrantCopy grant) =>
        SharingDisclosureSheets.ShowAsync(
            Root.XamlRoot, flow, grant, Copy, () => _host.CallAsync(DaemonProtocol.Methods.RouteDisclosure), () => _closed, ElementTheme.Dark);

    private async void OnBack(object sender, RoutedEventArgs e)
    {
        await _controller.Back();
        Render();
    }

    private void OnLink(object sender, RoutedEventArgs e)
    {
    }

    private UIElement BuildStep(FirstRunStep step) => step switch
    {
        FirstRunStep.Welcome => Welcome(),
        FirstRunStep.Tools => Framed(step, Copy.Tools.TitleLight, Copy.Tools.TitleBold, ToolsList()),
        FirstRunStep.PrivateAI => Framed(step, Copy.PrivateAi.TitleLight, Copy.PrivateAi.TitleBold, PrivateAICard()),
        FirstRunStep.Traces => Framed(step, Copy.Traces.TitleLight, Copy.Traces.TitleBold, TracesEstimate()),
        FirstRunStep.Uses => Framed(step, Copy.Uses.TitleLight, Copy.Uses.TitleBold, UsesCard()),
        FirstRunStep.Terms => Framed(step, Copy.Welcome.Legal.TitleLight, Copy.Welcome.Legal.Terms, TermsReader()),
        _ => Framed(step, Copy.Join.TitleLight, Copy.Join.TitleBold, JoinCard()),
    };

    private static SolidColorBrush Brush(uint argb) => new(FirstRunStagePainter.Argb(argb));

    /// <summary>A space's width at a font: "x x" measured less "xx".</summary>
    private static double SpaceWidth(double size, FontWeight weight)
    {
        double Width(string text)
        {
            var block = new TextBlock { Text = text, FontSize = size, FontWeight = weight, TextWrapping = TextWrapping.NoWrap };
            block.Measure(new Windows.Foundation.Size(double.PositiveInfinity, double.PositiveInfinity));
            return block.DesiredSize.Width;
        }

        return Math.Max(0, Width("x x") - Width("xx"));
    }

    private static TextBlock Text(string text, double size, uint argb, FontWeight? weight = null) => new()
    {
        Text = text,
        FontSize = size,
        Foreground = Brush(argb),
        FontWeight = weight ?? FontWeights.Normal,
        TextWrapping = TextWrapping.Wrap,
        TextAlignment = TextAlignment.Center,
        HorizontalAlignment = HorizontalAlignment.Center,
    };

    private static Border Card(UIElement child, double padH = 16, double padV = 8) => new()
    {
        Child = child,
        Padding = new Thickness(padH, padV, padH, padV),
        CornerRadius = new CornerRadius(16),
        Background = Brush(FirstRunTokens.FirstRunCard),
        HorizontalAlignment = HorizontalAlignment.Center,
    };

    /// <summary>
    /// FirstRunFrame (FirstRunFrame.swift:150-230): graphic, title, content,
    /// below the thread's 44, as a grid of <see cref="FirstRunFrameLayout.Rows"/>:
    /// the free height is split three ways, or taken by a card that scrolls.
    /// </summary>
    private UIElement Framed(FirstRunStep step, string light, string bold, UIElement content)
    {
        var grid = new Grid { Margin = new Thickness(0, FirstRunFrameLayout.ThreadBottom, 0, 0) };
        _graphic = new Border { Height = StageActors.GraphicHeight(step), IsHitTestVisible = false };
        var title = new TextBlock { FontSize = 26, TextAlignment = TextAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center, TextWrapping = TextWrapping.Wrap, Margin = new Thickness(40, 0, 40, FirstRunFrameLayout.TitleGap) };
        title.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = light, Foreground = Brush(FirstRunTokens.TextSecondary) });
        title.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = bold, FontWeight = FontWeights.Bold, Foreground = Brush(FirstRunTokens.TextPrimary) });
        AutomationProperties.SetAccessibilityView(title, AccessibilityView.Raw);
        // The heading, focusable only when a rebuild hands it focus.
        var heading = Key(new ContentControl { Content = title, IsTabStop = false, UseSystemFocusVisuals = true, HorizontalAlignment = HorizontalAlignment.Center, HorizontalContentAlignment = HorizontalAlignment.Center }, FirstRunFocus.Heading);
        AutomationProperties.SetHeadingLevel(heading, AutomationHeadingLevel.Level1);
        AutomationProperties.SetName(heading, light + bold);
        // UsesScreen.swift:647, JoinScreen.swift:494: the card scrolls in the
        // height it takes, its top under the title.
        UIElement card = FirstRunFrameLayout.CardScrolls(step)
            ? new ScrollViewer { Content = content, VerticalAlignment = VerticalAlignment.Top, VerticalScrollBarVisibility = ScrollBarVisibility.Auto }
            : content;
        UIElement? notice = Notice(_runner.State) is { } line ? Alert(line, 560) : null;
        IReadOnlyList<FirstRunFrameRow> rows = FirstRunFrameLayout.Rows(step);
        for (int i = 0; i < rows.Count; i++)
        {
            FirstRunFrameRow row = rows[i];
            grid.RowDefinitions.Add(new RowDefinition
            {
                Height = row.Fills ? new GridLength(1, GridUnitType.Star) : row.Slot == FirstRunFrameSlot.Gap ? new GridLength(row.MinHeight) : GridLength.Auto,
                MinHeight = row.MinHeight,
            });
            UIElement? cell = row.Slot switch
            {
                FirstRunFrameSlot.Graphic => _graphic,
                FirstRunFrameSlot.Title => heading,
                FirstRunFrameSlot.Card => card,
                FirstRunFrameSlot.Notice => notice,
                _ => null,
            };
            if (cell is not null)
            {
                Grid.SetRow((FrameworkElement)cell, i);
                grid.Children.Add(cell);
            }
        }

        return grid;
    }

    /// <summary>
    /// GlassAlert (StatCard.swift:124-138): the failed request's red line,
    /// unboxed, caption size, under the step's content (FirstRunFrame.swift:201-205).
    /// </summary>
    private static TextBlock Alert(string line, double maxWidth)
    {
        TextBlock alert = Text(line, 12, FirstRunTokens.StatusOutsideText);
        alert.MaxWidth = maxWidth;
        alert.Margin = new Thickness(0, 12, 0, 0);
        alert.TextAlignment = TextAlignment.Left;
        AutomationProperties.SetLiveSetting(alert, AutomationLiveSetting.Assertive);
        return alert;
    }

    // ---- Welcome ------------------------------------------------------------

    /// <summary>WelcomeScreen (WelcomeScreen.swift:90-168): the N drawn, the words rising, Get started.</summary>
    private UIElement Welcome()
    {
        var panel = new StackPanel { VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(40, 0, 40, 0) };
        _graphic = new Border { Height = WelcomeMotion.Hub, Margin = new Thickness(0, 0, 0, WelcomeMotion.HubGap), IsHitTestVisible = false };
        panel.Children.Add(_graphic);
        var title = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 0, 0, 28) };
        AutomationProperties.SetName(title, Copy.Welcome.Title);
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level1);
        IReadOnlyList<string> words = WelcomeMotion.Words(Copy.Welcome.Title);
        title.Spacing = SpaceWidth(26 * WelcomeMotion.HeroScale, FontWeights.SemiBold);
        for (int i = 0; i < words.Count; i++)
        {
            TextBlock word = Text(WelcomeMotion.Shown(words[i]), 26 * WelcomeMotion.HeroScale, FirstRunTokens.TextPrimary, FontWeights.SemiBold);
            word.TextWrapping = TextWrapping.NoWrap;
            AutomationProperties.SetAccessibilityView(word, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
            Rise(word, WelcomeMotion.WordDelay(i), focusable: false);
            title.Children.Add(word);
        }

        panel.Children.Add(title);
        TextBlock body = Text(Copy.Welcome.Body, 20, FirstRunTokens.TextSecondary);
        body.Margin = new Thickness(0, 0, 0, 38);
        Rise(body, WelcomeMotion.BodyDelay, focusable: false);
        panel.Children.Add(body);
        var start = Key(new Button { Content = Copy.Welcome.Start, Style = (Style)Application.Current.Resources["AccentButtonStyle"], HorizontalAlignment = HorizontalAlignment.Center }, "get-started");
        AutomationProperties.SetName(start, Copy.Welcome.Start);
        start.Click += (_, _) => GetStarted();
        Rise(start, WelcomeMotion.ButtonDelay, focusable: true);
        panel.Children.Add(start);
        AgentConfigurationCopy agent = Copy.AgentConfiguration;
        if (agent.Action.Length > 0)
        {
            var guide = new Button { Content = agent.Action, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 28, 0, 0) };
            AutomationProperties.SetName(guide, agent.Action);
            guide.Click += async (_, _) => await ShowAgentPromptAsync(agent);
            Rise(guide, WelcomeMotion.ButtonDelay, focusable: true);
            panel.Children.Add(guide);
        }

        return panel;
    }

    /// <summary>
    /// A line rising 18 points into place (<c>Rise</c>, WelcomeScreen.swift:171-178);
    /// not focusable or clickable until it is there (checklist E2).
    /// </summary>
    private void Rise(FrameworkElement element, double delay, bool focusable)
    {
        var lift = new TranslateTransform();
        element.RenderTransform = lift;
        double opened = Now;
        void Apply(double t)
        {
            var (opacity, y) = WelcomeMotion.Rise(WelcomeMotion.Risen(t - opened, delay, _reduceMotion));
            element.Opacity = opacity;
            lift.Y = y;
            bool there = opacity >= 0.98;
            element.IsHitTestVisible = there;
            if (focusable && element is Control c)
            {
                c.IsTabStop = there;
            }
        }

        _motions.Add((Apply, t => t - opened < delay + WelcomeMotion.RiseDuration));
        Apply(_reduceMotion ? opened + 1e6 : opened);
    }

    private async Task ShowAgentPromptAsync(AgentConfigurationCopy agent)
    {
        var text = new StackPanel { Spacing = 12 };
        text.Children.Add(new TextBlock { Text = agent.Body, TextWrapping = TextWrapping.Wrap });
        text.Children.Add(new TextBlock { Text = agent.Prerequisite, TextWrapping = TextWrapping.Wrap });
        text.Children.Add(new TextBox { Text = agent.Prompt, IsReadOnly = true, TextWrapping = TextWrapping.Wrap, AcceptsReturn = true, MaxHeight = 280 });
        var dialog = new ContentDialog
        {
            Title = agent.Title,
            Content = new ScrollViewer { Content = text },
            PrimaryButtonText = agent.Copy,
            CloseButtonText = agent.Close,
            XamlRoot = Root.XamlRoot,
            RequestedTheme = ElementTheme.Dark,
        };
        if (await dialog.ShowAsync() == ContentDialogResult.Primary)
        {
            var package = new DataPackage();
            package.SetText(agent.Prompt);
            Clipboard.SetContent(package);
        }
    }

    // ---- Tools ------------------------------------------------------------

    /// <summary>
    /// ToolChecks (ToolsScreen.swift:589-700): one row per found tool with
    /// the marker check, nothing ticked, rising after the scan; then the
    /// folders added with the tile, each a row of its own; then the add
    /// tile, which is how a computer with none of the tools is set up.
    /// </summary>
    private UIElement ToolsList()
    {
        FirstRunState state = _runner.State;
        bool enabled = !_runner.IsCommitting;
        var list = new StackPanel { Padding = new Thickness(28, 4, 28, 4) };
        var rows = FirstRunStageLayout.RingRows(_discovered.Where(c => c.Exists), c => c.Source);
        for (int i = 0; i < rows.Count; i++)
        {
            SourceCandidate row = rows[i];
            string name = ToolName(row.Source);
            var line = new Grid { Height = rows.Count > 3 ? 36 : 44, ColumnSpacing = 16 };
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            TextBlock label = Text(name, 14, FirstRunTokens.TextPrimary, FontWeights.SemiBold);
            label.HorizontalAlignment = HorizontalAlignment.Left;
            label.VerticalAlignment = VerticalAlignment.Center;
            label.MinWidth = 150;
            line.Children.Add(label);
            string question = FirstRunCopy.Fill(Copy.Folders.WatchQuestion, new Dictionary<string, string> { ["tool"] = name });
            MarkerCheckButton check = Key(MarkerCheck(question, state.RowAnswer(row.Source).Kind == ToolChoiceKind.Watch, on =>
            {
                _controller.AnswerTool(row.Source, row.Path, on);
                WatchedChanged();
            }), "tool:" + row.Source);
            check.IsEnabled = enabled;
            Grid.SetColumn(check, 1);
            line.Children.Add(check);
            RowRise(line, i, check);
            list.Children.Add(line);
        }

        foreach (AddedFolder folder in ToolsAdding.AddedRows(state))
        {
            string name = ToolsAdding.FolderName(folder.Path);
            AddedFolder f = folder;
            list.Children.Add(FolderRow(name, folder.Path, Copy.Tools.AddedByYou, KeyedCheck("folder:" + folder.Path,
                FirstRunCopy.Fill(Copy.Folders.WatchQuestion, new Dictionary<string, string> { ["tool"] = name }), folder.Watched == true,
                on =>
                {
                    _runner.State.AnswerAdded(f.Path, on);
                    WatchedChanged();
                }, enabled)));
            if (ToolsAdding.Conflict(folder, state, Copy.Tools, ToolName) is { } conflict)
            {
                TextBlock note = Text(conflict, 12, FirstRunTokens.TextSecondary);
                note.HorizontalAlignment = HorizontalAlignment.Left;
                note.TextAlignment = TextAlignment.Left;
                note.MaxWidth = 420;
                list.Children.Add(note);
            }
        }

        if (ToolsAdding.TrajectoryFolder(state) is { } trajectory)
        {
            string name = ToolsAdding.Name(null, Copy.Tools, ToolName);
            list.Children.Add(FolderRow(name, trajectory.Path, Copy.Tools.AddedByYou, KeyedCheck("trajectory",
                FirstRunCopy.Fill(Copy.Folders.WatchQuestion, new Dictionary<string, string> { ["tool"] = name }), true,
                on =>
                {
                    ToolsAdding.SelectTrajectory(on, _runner.State);
                    _contentStale = true;
                    WatchedChanged();
                }, enabled)));
        }

        if (_pendingFolder is { } pending)
        {
            var picker = Key(new ComboBox { PlaceholderText = Copy.Frame.Choose, IsEnabled = enabled, VerticalAlignment = VerticalAlignment.Center }, "pick-kind");
            string question = ToolsAdding.Question(pending.Path, Copy.Tools);
            AutomationProperties.SetName(picker, question);
            ToolTipService.SetToolTip(picker, question);
            foreach (string? kind in pending.Kinds)
            {
                picker.Items.Add(ToolsAdding.Name(kind, Copy.Tools, ToolName));
            }

            picker.Items.Add(Copy.Tools.Neither);
            picker.SelectionChanged += (_, _) =>
            {
                int index = picker.SelectedIndex;
                if (index < 0)
                {
                    return;
                }

                int? kind = index < pending.Kinds.Count ? index : null;
                if (ToolsAdding.Answer(kind, pending.Path, pending.Kinds, _runner.State))
                {
                    _pendingFolder = null;
                    WatchedChanged();
                }
            };
            list.Children.Add(FolderRow(ToolsAdding.FolderName(pending.Path), pending.Path, null, picker));
        }

        var column = new StackPanel { Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
        Border card = Card(list, 0, 4);
        card.Visibility = list.Children.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
        column.Children.Add(card);
        FrameworkElement tile = AddToolTile(enabled);
        column.Children.Add(tile);
        // The tile rises after the found rows, as the next row (ToolsScreen.swift:604).
        RowRise(tile, rows.Count, (Control)((StackPanel)tile).Children[0]);
        double opened = Now;
        _motions.Add((t =>
        {
            double since = _scanEndedAt is double end ? t - end : double.NegativeInfinity;
            column.Opacity = since < 0 ? 0 : FirstRunMotion.CardOpacity(since, _reduceMotion);
            column.IsHitTestVisible = column.Opacity > 0.98;
        }, t => !_model.ToolsScanned || (_scanEndedAt is double end && t - end < FirstRunMotion.CardFade)));
        return column;
    }

    private string ToolName(string kind) => _names.TryGetValue(kind, out string? n) ? n : kind;

    /// <summary>The stage's watched tools follow the state, and Continue with them.</summary>
    private void WatchedChanged()
    {
        _model.SetWatched(FirstRunStageLayout.RingOrder.Where(k => _runner.State.SessionRoots[k].Kind == ToolChoiceKind.Watch), Now);
        Render();
    }

    private MarkerCheckButton KeyedCheck(string key, string name, bool on, Action<bool> changed, bool enabled) =>
        Key(MarkerCheckFor(name, on, changed, enabled), key);

    private MarkerCheckButton MarkerCheckFor(string name, bool on, Action<bool> changed, bool enabled)
    {
        MarkerCheckButton check = MarkerCheck(name, on, changed);
        check.IsEnabled = enabled;
        return check;
    }

    /// <summary>
    /// A folder's row (ToolsScreen.swift:701-727): its name, "Added by you"
    /// or its path, the full path as the row's tooltip, and its control.
    /// </summary>
    private static Grid FolderRow(string name, string path, string? meta, Control trailing)
    {
        var line = new Grid { MinHeight = 44, ColumnSpacing = 16 };
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var words = new StackPanel { VerticalAlignment = VerticalAlignment.Center, MinWidth = 150, MaxWidth = 280 };
        TextBlock title = Text(name, 14, FirstRunTokens.TextPrimary, FontWeights.SemiBold);
        title.HorizontalAlignment = HorizontalAlignment.Left;
        title.TextWrapping = TextWrapping.NoWrap;
        title.TextTrimming = TextTrimming.CharacterEllipsis;
        words.Children.Add(title);
        TextBlock sub = Text(meta ?? path, 12, FirstRunTokens.TextSecondary);
        sub.HorizontalAlignment = HorizontalAlignment.Left;
        sub.TextWrapping = TextWrapping.NoWrap;
        sub.TextTrimming = TextTrimming.CharacterEllipsis;
        words.Children.Add(sub);
        ToolTipService.SetToolTip(words, path);
        line.Children.Add(words);
        trailing.VerticalAlignment = VerticalAlignment.Center;
        Grid.SetColumn(trailing, 1);
        line.Children.Add(trailing);
        return line;
    }

    /// <summary>
    /// The add row (AddToolTile, ToolsScreen.swift:757-810): a dashed
    /// rounded tile that picks a folder, which the core describes off the UI
    /// thread; a refused folder shows the core's red line under it.
    /// </summary>
    private FrameworkElement AddToolTile(bool enabled)
    {
        // The words are inset, not the tile: the dashed outline, the hover
        // fill and the focus ring are one rounded shape, as the Mac's tile.
        var label = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(32, 0, 32, 0) };
        var plus = new FontIcon { Glyph = "\uE710", FontSize = 11, FontFamily = new FontFamily(FirstRunStagePainter.IconFamily), Foreground = Brush(FirstRunTokens.TextSecondary) };
        AutomationProperties.SetAccessibilityView(plus, AccessibilityView.Raw);
        label.Children.Add(plus);
        label.Children.Add(Text(Copy.Tools.AddTool, 13, FirstRunTokens.TextSecondary));
        var outline = new Rectangle
        {
            RadiusX = 12,
            RadiusY = 12,
            Stroke = Brush(FirstRunTokens.FirstRunRule),
            StrokeThickness = 1,
            StrokeDashArray = new DoubleCollection { 4, 3 },
            IsHitTestVisible = false,
        };
        var face = new Grid { Height = 36 };
        face.Children.Add(outline);
        face.Children.Add(label);
        var button = Key(new Button
        {
            Content = face,
            Padding = new Thickness(0),
            CornerRadius = new CornerRadius(12),
            Background = new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(0),
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            IsEnabled = enabled,
        }, "add-tool");
        // A plain button, as the Mac's (.buttonStyle(.plain)): no fill on
        // hover or press; hovering shows the caption's tooltip (.help).
        foreach (string state in new[] { "PointerOver", "Pressed", "Disabled" })
        {
            button.Resources["ButtonBackground" + state] = new SolidColorBrush(Colors.Transparent);
            button.Resources["ButtonBorderBrush" + state] = new SolidColorBrush(Colors.Transparent);
        }

        AutomationProperties.SetName(button, Copy.Tools.AddTool);
        AutomationProperties.SetHelpText(button, Copy.Tools.AddToolCaption);
        ToolTipService.SetToolTip(button, Copy.Tools.AddToolCaption);
        button.Click += async (_, _) => await PickToolFolderAsync();
        var tile = new StackPanel { Spacing = 6, MinWidth = 320 };
        tile.Children.Add(button);
        if (_addRefused)
        {
            tile.Children.Add(Alert(Copy.Tools.AddToolRefused, 420));
        }

        return tile;
    }

    /// <summary>
    /// Pick a folder and describe it (AddToolModel.describe): its answer
    /// lands only while Tools is up and nothing is committing
    /// (<see cref="ToolsAdding.AcceptsFolder"/>).
    /// </summary>
    private async Task PickToolFolderAsync()
    {
        var picker = new Windows.Storage.Pickers.FolderPicker();
        picker.FileTypeFilter.Add("*");
        WinRT.Interop.InitializeWithWindow.Initialize(picker, WinRT.Interop.WindowNative.GetWindowHandle(this));
        Windows.Storage.StorageFolder? folder = await picker.PickSingleFolderAsync();
        if (folder is null || _closed)
        {
            return;
        }

        string path = folder.Path;
        string? json = await Task.Run(() => ToolsAdding.Describe(path)).ConfigureAwait(true);
        if (_closed || !ToolsAdding.AcceptsFolder(_runner.State.Step, _runner.IsCommitting))
        {
            return;
        }

        AddToolOutcome outcome = ToolsAdding.Outcome(path, json);
        _addRefused = !ToolsAdding.Apply(outcome, _runner.State);
        _pendingFolder = outcome is AddToolOutcome.Ask ask ? (ask.Path, ask.Kinds) : null;
        WatchedChanged();
    }

    /// <summary>RowRise (ToolsScreen.swift:740-751).</summary>
    private void RowRise(FrameworkElement row, int index, Control focus)
    {
        var lift = new TranslateTransform();
        row.RenderTransform = lift;
        void Apply(double t)
        {
            double since = _scanEndedAt is double end ? t - end : double.NegativeInfinity;
            var (y, o) = since < 0 ? (FirstRunMotion.RowLift, 0.0) : FirstRunMotion.RowRise(index, since, _reduceMotion);
            lift.Y = y;
            row.Opacity = o;
            focus.IsTabStop = o >= 0.98;
            row.IsHitTestVisible = o >= 0.98;
        }

        _motions.Add((Apply, t => !_model.ToolsScanned || (_scanEndedAt is double end && t - end < FirstRunMotion.RowStagger * (index + 1) + FirstRunStageLayout.SpringSettle(FirstRunMotion.RowRiseDuration))));
        Apply(Now);
    }

    /// <summary>The marker check (FirstRunMarkerCheckStyle, FirstRunStage.swift:341-405) as a toggle.</summary>
    private MarkerCheckButton MarkerCheck(string name, bool on, Action<bool> changed)
    {
        var check = new MarkerCheckButton(on);
        AutomationProperties.SetName(check, name);
        double? since = null;
        check.Checked += (_, _) => { since = Now; changed(true); };
        check.Unchecked += (_, _) => { since = Now; changed(false); };
        _motions.Add((t => check.Draw(MarkerCheckTrim(check.IsChecked == true, since, t)), t => since is double s && MarkerCheckMoving(check.IsChecked == true, t - s)));
        return check;
    }

    private double MarkerCheckTrim(bool on, double? since, double t) => Interop.FirstRun.MarkerCheck.Trim(on, since is double s ? t - s : null, _reduceMotion);

    private bool MarkerCheckMoving(bool on, double since) => Interop.FirstRun.MarkerCheck.Moving(on, since, _reduceMotion);

    // ---- Private AI --------------------------------------------------------

    /// <summary>The card (PrivateAIScreen.swift:136-190): the count ticking up, the line, Learn more.</summary>
    private UIElement PrivateAICard()
    {
        IReadOnlyList<string> tools = _model.Connectable;
        var panel = new StackPanel { Spacing = 4, MaxWidth = 520 };
        var figure = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        TextBlock count = Text("0", 26, FirstRunTokens.TextPrimary, FontWeights.SemiBold);
        figure.Children.Add(count);
        int found = _discovered.Count(c => c.Exists);
        figure.Children.Add(Text(Copy.PrivateAi.OfTotal(found), 14, FirstRunTokens.TextSecondary));
        panel.Children.Add(figure);
        panel.Children.Add(Text(Copy.PrivateAi.OfferLead, 14, FirstRunTokens.TextSecondary));
        var more = Key(new Expander { Header = Copy.PrivateAi.LearnMore, Content = Text(Copy.PrivateAi.OfferMore, 13, FirstRunTokens.TextSecondary) }, "learn-more");
        panel.Children.Add(more);
        double opened = _model.PrivateCounted ? double.NegativeInfinity : Now;
        _motions.Add((t => count.Text = PrivateAIDiagram.Counted(tools.Count, t - opened, _reduceMotion).ToString(System.Globalization.CultureInfo.CurrentCulture),
            t => PrivateAIDiagram.Counted(tools.Count, t - opened, false) < tools.Count));
        return Card(panel, 24, 12);
    }

    // ---- Traces -------------------------------------------------------------

    /// <summary>The estimate (TracesScreen.swift:262-287): the core's line, rising once the count is done.</summary>
    private UIElement TracesEstimate()
    {
        ulong? found = (ulong)_discovered.Where(c => c.Exists).Sum(c => (long)c.SessionCount);
        // TracesScreenLayout.showsNoCredit (TracesScreen.swift:87-89): the
        // line earns credit only with the credit use ticked on Uses.
        bool earnsCredit = _runner.State.Scopes.Contains(Copy.Uses.CreditScope);
        string? line = FirstRunEstimate.Line(found, earnsCredit);
        if (line is null)
        {
            return new Border();
        }

        var panel = new StackPanel { Spacing = 2 };
        panel.Children.Add(Text(line, 17, FirstRunTokens.TextPrimary, FontWeights.SemiBold));
        panel.Children.Add(Text(Copy.Traces.CreditDepends, 12, FirstRunTokens.TextSecondary));
        Border card = Card(panel, 24, 12);
        var lift = new TranslateTransform();
        card.RenderTransform = lift;
        // Up once the count is done, or at once when it already played.
        double done = _model.TracesCounted ? double.NegativeInfinity : Now + TracesGraphic.CountDelay + TracesGraphic.CountDuration;
        void Apply(double t)
        {
            double since = t - done;
            var (y, o) = since < 0 ? (FirstRunMotion.EstimateLift, 0.0) : FirstRunMotion.Estimate(since, _reduceMotion);
            lift.Y = y;
            card.Opacity = o;
        }

        _motions.Add((Apply, t => t - done < FirstRunStageLayout.SpringSettle(FirstRunMotion.EstimateRise)));
        return card;
    }

    // ---- Uses -----------------------------------------------------------------

    /// <summary>
    /// Uses (UsesScreen.swift:602-790): the required use ticked and locked,
    /// the optional uses unticked, and the sharing choice with neither
    /// picked (owner, 2026-10-27).
    /// </summary>
    private UIElement UsesCard()
    {
        var panel = new StackPanel { Spacing = 6, MaxWidth = 560 };
        panel.Children.Add(Text(Copy.Uses.Eyebrow, 12, FirstRunTokens.TextSecondary, FontWeights.SemiBold));
        if (_options.FirstOrDefault(o => o.AlwaysOn) is { } required && required.Title is { } requiredTitle)
        {
            var locked = new CheckBox { Content = FirstRunCopy.Fill(Copy.Uses.AlwaysOn, new Dictionary<string, string> { ["title"] = requiredTitle }), IsChecked = true, IsEnabled = false };
            panel.Children.Add(locked);
        }

        var optional = _options.Where(o => !o.AlwaysOn && o.GrantsDataUse && o.Title is not null).ToList();
        optional = optional.Where(o => o.Name == Copy.Uses.CreditScope).Concat(optional.Where(o => o.Name != Copy.Uses.CreditScope)).ToList();
        foreach (ConsentOption option in optional)
        {
            var box = Key(new CheckBox { Content = option.Title, IsChecked = _runner.State.Scopes.Contains(option.Name) }, "scope:" + option.Name);
            ToolTipService.SetToolTip(box, option.Description);
            box.Checked += (_, _) => { _controller.SetScope(option.Name, true); Render(); };
            box.Unchecked += (_, _) => { _controller.SetScope(option.Name, false); Render(); };
            panel.Children.Add(box);
        }

        // The Sharing eyebrow, its info button with the chosen path's words
        // (UsesScreen.swift:699-704), and the fallback line when the copy
        // would not decode (UsesScreen.swift:705-709).
        var eyebrow = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, HorizontalAlignment = HorizontalAlignment.Center };
        eyebrow.Children.Add(Text(Copy.Uses.Sharing, 12, FirstRunTokens.TextSecondary, FontWeights.SemiBold));
        if (_controller.SharingDetail is { } detail)
        {
            eyebrow.Children.Add(InfoButton(FirstRunCopy.Fill(Copy.Uses.MoreAbout, new Dictionary<string, string> { ["title"] = Copy.Uses.Sharing }), detail, "sharing-info"));
        }

        panel.Children.Add(eyebrow);
        if (_controller.SharingUnavailable)
        {
            panel.Children.Add(Text(Copy.Uses.SharingUnavailable, 12, FirstRunTokens.TextSecondary));
        }

        // The Mac's titles and lines (UsesSharing): Automatic by the core's
        // mode name, its scope and sign-in lines under the choices.
        IReadOnlyList<UsesSharing.Option> sharing = UsesSharing.Options(_runner.State, Copy.Uses, WatchCopy.Armed);
        panel.Children.Add(SharingRadios(Copy.Uses.Sharing, sharing));
        return Card(panel, 24, 12);
    }

    /// <summary>
    /// GlassRadioGroup (Forms.swift:140-175, UsesScreen.swift:710-717): one
    /// radio per sharing path, its title and, under it, that option's own
    /// lines; nothing chosen for the person (owner, 2026-10-27).
    /// </summary>
    private UIElement SharingRadios(string label, IReadOnlyList<UsesSharing.Option> options)
    {
        var group = new StackPanel { Spacing = 8, HorizontalAlignment = HorizontalAlignment.Left };
        AutomationProperties.SetName(group, label);
        string groupName = "sharing-" + Guid.NewGuid().ToString("N");
        foreach (UsesSharing.Option option in options)
        {
            var words = new StackPanel { Spacing = 1 };
            TextBlock title = Text(option.Title, 13, FirstRunTokens.TextPrimary);
            title.HorizontalAlignment = HorizontalAlignment.Left;
            title.TextAlignment = TextAlignment.Left;
            words.Children.Add(title);
            if (option.Lines.Count > 0)
            {
                TextBlock sub = Text(string.Join("\n", option.Lines), 12, FirstRunTokens.TextSecondary);
                sub.HorizontalAlignment = HorizontalAlignment.Left;
                sub.TextAlignment = TextAlignment.Left;
                sub.MaxWidth = 460;
                words.Children.Add(sub);
            }

            var radio = Key(new RadioButton
            {
                Content = words,
                GroupName = groupName,
                IsChecked = _runner.State.SharingChosen && _runner.State.Sharing == option.Path,
            }, "sharing:" + option.Path);
            AutomationProperties.SetName(radio, option.Title);
            if (option.Lines.Count > 0)
            {
                AutomationProperties.SetHelpText(radio, string.Join(" ", option.Lines));
            }

            SharingPath path = option.Path;
            radio.Checked += (_, _) =>
            {
                if (_controller.ChooseSharing(path))
                {
                    _model.SetSharing(path, Now);
                    Render();
                }
            };
            group.Children.Add(radio);
        }

        return group;
    }

    /// <summary>
    /// GlassInfoButton (InfoButton.swift:17-60): an "i" whose words show on
    /// hover and stay while pinned by a press; named "More about" the title,
    /// its words its help text.
    /// </summary>
    private static Button InfoButton(string name, string text, string key)
    {
        var glyph = new FontIcon { Glyph = "\uE946", FontSize = 12, FontFamily = new FontFamily(FirstRunStagePainter.IconFamily), Foreground = Brush(FirstRunTokens.TextTertiary) };
        var words = new TextBlock { Text = text, TextWrapping = TextWrapping.Wrap, MaxWidth = 260 };
        var button = Key(new Button
        {
            Content = glyph,
            Padding = new Thickness(2),
            MinWidth = 0,
            MinHeight = 0,
            Background = new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(0),
            VerticalAlignment = VerticalAlignment.Center,
            Flyout = new Flyout { Content = words, Placement = FlyoutPlacementMode.Bottom },
        }, key);
        ToolTipService.SetToolTip(button, text);
        AutomationProperties.SetName(button, name);
        AutomationProperties.SetHelpText(button, text);
        return button;
    }

    private static Windows.UI.Color Lerp(uint a, uint b, double p)
    {
        byte M(int shift) => (byte)Math.Round(((a >> shift) & 0xFF) * (1 - p) + ((b >> shift) & 0xFF) * p);
        return Windows.UI.Color.FromArgb(M(24), M(16), M(8), M(0));
    }

    // ---- Terms ----------------------------------------------------------------

    /// <summary>
    /// TermsReader (TermsReader.swift:93-240): a 420-wide column whose
    /// 260-high region holds the document, or the loading or failed state
    /// with Retry, so the step keeps its place as the document arrives; then
    /// the not-in-effect line, the checkbox, which unlocks once the end has
    /// been in view, the scroll hint while it is locked, and the Terms and
    /// Privacy links (<see cref="TermsReaderLayout"/>).
    /// </summary>
    private UIElement TermsReader()
    {
        LegalWords legal = Copy.Welcome.Legal;
        FetchedTerms? terms = _controller.Terms;
        TermsRegion region = TermsReaderLayout.Region(terms is not null, _termsLabel);
        bool unlocked = _runner.AcceptedTerms is not null;
        var column = new StackPanel { Spacing = TermsReaderLayout.Spacing, MaxWidth = TermsReaderLayout.Width, HorizontalAlignment = HorizontalAlignment.Stretch };
        var box = new Border
        {
            Height = TermsReaderLayout.Viewport,
            CornerRadius = new CornerRadius(GlassTokens.Radius.Control),
            Background = Brush(0x0AFFFFFF),
            BorderBrush = Brush(FirstRunTokens.FirstRunRule),
            BorderThickness = new Thickness(1),
        };
        CheckBox? accept = null;
        TextBlock? hint = null;
        ScrollViewer? scroll = null;
        switch (region)
        {
            case TermsRegion.Loading:
            {
                var loading = new StackPanel { Spacing = 8, VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
                loading.Children.Add(new ProgressRing { IsActive = true, Width = 16, Height = 16 });
                loading.Children.Add(Text(legal.Loading, 12, FirstRunTokens.TextSecondary));
                box.Child = loading;
                break;
            }

            case TermsRegion.Failed:
            {
                var failed = new StackPanel { Spacing = 8, Padding = new Thickness(TermsReaderLayout.DocumentPadding), VerticalAlignment = VerticalAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center };
                failed.Children.Add(Text(legal.Failed, 14, FirstRunTokens.TextSecondary));
                var retry = new Button { Content = legal.Retry, HorizontalAlignment = HorizontalAlignment.Center };
                retry.Click += async (_, _) => await RetryTermsAsync();
                failed.Children.Add(retry);
                box.Child = failed;
                break;
            }

            default:
            {
                var body = new StackPanel { Spacing = TermsReaderLayout.BlockSpacing, Padding = new Thickness(TermsReaderLayout.DocumentPadding) };
                var heading = new TextBlock { Text = terms!.Title, FontSize = 17, FontWeight = FontWeights.SemiBold, Foreground = Brush(FirstRunTokens.TextPrimary), TextWrapping = TextWrapping.Wrap };
                AutomationProperties.SetHeadingLevel(heading, AutomationHeadingLevel.Level2);
                body.Children.Add(heading);
                foreach (TermsBlock block in terms.Blocks)
                {
                    switch (block.Kind)
                    {
                        case "heading":
                            body.Children.Add(new TextBlock { Text = block.Text, FontWeight = FontWeights.SemiBold, Foreground = Brush(FirstRunTokens.TextPrimary), TextWrapping = TextWrapping.Wrap });
                            break;
                        case "list":
                            foreach (string item in block.Items!)
                            {
                                body.Children.Add(new TextBlock { Text = "\u2022 " + item, Foreground = Brush(FirstRunTokens.TextSecondary), TextWrapping = TextWrapping.Wrap });
                            }

                            break;
                        case "table":
                            body.Children.Add(new TextBlock { Text = string.Join(" | ", block.Header!), FontWeight = FontWeights.SemiBold, Foreground = Brush(FirstRunTokens.TextPrimary), TextWrapping = TextWrapping.Wrap });
                            foreach (var row in block.Rows!)
                            {
                                body.Children.Add(new TextBlock { Text = string.Join(" | ", row), Foreground = Brush(FirstRunTokens.TextSecondary), TextWrapping = TextWrapping.Wrap });
                            }

                            break;
                        default:
                            body.Children.Add(new TextBlock { Text = block.Text, Foreground = Brush(FirstRunTokens.TextSecondary), TextWrapping = TextWrapping.Wrap });
                            break;
                    }
                }

                scroll = new ScrollViewer { Content = body };
                box.Child = scroll;
                break;
            }
        }

        foreach (TermsReaderPart part in TermsReaderLayout.Parts(region, terms?.InEffect ?? true, unlocked))
        {
            switch (part)
            {
                case TermsReaderPart.Region:
                    column.Children.Add(box);
                    break;
                case TermsReaderPart.NotInEffect:
                    column.Children.Add(Text(legal.NotInEffect, 12, FirstRunTokens.TextSecondary));
                    break;
                case TermsReaderPart.Accept:
                    accept = Key(new CheckBox { Content = legal.Accept, IsEnabled = unlocked, IsChecked = _runner.AcceptedTerms is not null, HorizontalAlignment = HorizontalAlignment.Center, Foreground = Brush(FirstRunTokens.TextSecondary) }, "accept");
                    AutomationProperties.SetName(accept, legal.Accept);
                    if (!unlocked)
                    {
                        AutomationProperties.SetHelpText(accept, legal.ScrollHint);
                    }

                    accept.Checked += (_, _) => { _controller.AcceptTerms(true, DateTimeOffset.UtcNow); Render(); };
                    accept.Unchecked += (_, _) => { _controller.AcceptTerms(false, DateTimeOffset.UtcNow); Render(); };
                    column.Children.Add(accept);
                    break;
                case TermsReaderPart.ScrollHint:
                    hint = Text(legal.ScrollHint, 12, FirstRunTokens.TextSecondary);
                    hint.Opacity = 0.75;
                    hint.Margin = new Thickness(0, TermsReaderLayout.HintSpacing - TermsReaderLayout.Spacing, 0, 0);
                    AutomationProperties.SetAccessibilityView(hint, AccessibilityView.Raw);
                    column.Children.Add(hint);
                    break;
                case TermsReaderPart.Links:
                    var links = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
                    foreach ((string label, string url) in new[] { (legal.Terms, legal.TermsUrl), (legal.Privacy, legal.PrivacyUrl) })
                    {
                        if (Uri.TryCreate(url, UriKind.Absolute, out Uri? target))
                        {
                            links.Children.Add(new HyperlinkButton { Content = label, NavigateUri = target, FontSize = 12, Padding = new Thickness(0) });
                        }
                    }

                    column.Children.Add(links);
                    break;
            }
        }

        // Unlocked once the end has been in view, and at once for a document that fits.
        if (scroll is not null && accept is not null && !unlocked)
        {
            void Unlock()
            {
                // Only once laid out: before layout every extent reads 0,
                // which would unlock a document nobody has seen.
                if (scroll.ViewportHeight > 0 && scroll.ExtentHeight > 0
                    && scroll.VerticalOffset >= scroll.ScrollableHeight - 4)
                {
                    accept.IsEnabled = true;
                    AutomationProperties.SetHelpText(accept, string.Empty);
                    if (hint is not null)
                    {
                        hint.Visibility = Visibility.Collapsed;
                    }
                }
            }

            scroll.ViewChanged += (_, _) => Unlock();
            scroll.SizeChanged += (_, _) => Unlock();
        }

        return column;
    }

    /// <summary>The failed region's Retry: fetch the published terms again (TermsReader.swift:155).</summary>
    private async Task RetryTermsAsync()
    {
        _controller.Terms = null;
        _termsLabel = null;
        _contentStale = true;
        Render();
        var fetched = await Task.Run(FetchedTerms.Fetch).ConfigureAwait(true);
        _controller.Terms = fetched.Terms;
        _termsLabel = fetched.Label;
        if (_runner.State.Step == FirstRunStep.Terms)
        {
            _contentStale = true;
        }

        Render();
    }

    // ---- Join -------------------------------------------------------------

    /// <summary>
    /// Join (JoinScreen.swift:346-520): the invite card while an invite is
    /// held, then the NEAR AI sign-in, which Start needs; no wallet path in
    /// the first run (W7-D1 (a)). Cancel sits beside the sign-in while the
    /// browser sign-in waits.
    /// </summary>
    private UIElement JoinCard()
    {
        JoinWords join = Copy.Join;
        FirstRunState state = _runner.State;
        var column = new StackPanel { Spacing = 12, MaxWidth = 460 };
        if (JoinInvite.ShowsCard(state))
        {
            column.Children.Add(InviteCard());
        }

        var panel = new StackPanel { Spacing = 8 };
        panel.Children.Add(Text(join.NearAiEyebrow, 12, FirstRunTokens.TextSecondary, FontWeights.SemiBold));
        panel.Children.Add(Text(join.NearAiText, 14, FirstRunTokens.TextPrimary));
        if (state.SignedIn)
        {
            panel.Children.Add(Text(join.SignedIn, 14, FirstRunTokens.TextPrimary, FontWeights.SemiBold));
        }
        else
        {
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
            if (_runner.SignInWaiting)
            {
                var cancel = Key(new Button { Content = Copy.Frame.Cancel }, "cancel-sign-in");
                AutomationProperties.SetName(cancel, Copy.Frame.Cancel);
                cancel.Click += async (_, _) =>
                {
                    await _controller.CancelSignIn();
                    Render();
                };
                actions.Children.Add(cancel);
            }

            var signIn = Key(new Button { Content = join.NearAiSignIn, IsEnabled = !_runner.IsCommitting }, "sign-in");
            AutomationProperties.SetName(signIn, join.NearAiSignIn);
            signIn.Click += async (_, _) =>
            {
                await _controller.SignIn();
                if (_runner.State.SignedIn)
                {
                    _model.SignedIn(Now);
                }

                _contentStale = true;
                Render();
            };
            actions.Children.Add(signIn);
            panel.Children.Add(actions);
        }

        Border card = Card(panel, 24, 16);
        card.BorderThickness = new Thickness(FirstRunMotion.JoinBorderWidth);
        double? since = null;
        bool chosen = state.Account == AccountAnswer.NearAI;
        if (chosen)
        {
            since = Now;
        }

        _motions.Add((t => card.BorderBrush = new SolidColorBrush(Lerp(0x00000000, FirstRunTokens.ToggleOn, FirstRunMotion.JoinChosen(chosen, since is double s ? t - s : null, _reduceMotion))),
            t => since is double s && t - s < FirstRunMotion.JoinBorder));
        column.Children.Add(card);
        // Why near.ai is no longer chosen, under its card (JoinScreen.swift:506-508).
        if (FirstRunNotices.JoinNearAI(state, _runner.Failure, Copy, NearAiEnrollSurface.Line) is { } notice)
        {
            column.Children.Add(Alert(notice, 460));
        }

        return column;
    }

    /// <summary>
    /// The invite card (JoinScreen.swift:567-612): the invite field and Look
    /// up, read-only once an invite is enrolled, and its status line: the
    /// host it would join, what was joined, or why not.
    /// </summary>
    private UIElement InviteCard()
    {
        JoinWords join = Copy.Join;
        FirstRunState state = _runner.State;
        var panel = new StackPanel { Spacing = 6 };
        if (JoinInvite.ShowsField(state))
        {
            bool editable = JoinInvite.IsEditable(state) && !_runner.IsCommitting;
            var row = new Grid { ColumnSpacing = 8 };
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            var field = Key(new TextBox { Header = join.InviteEyebrow, PlaceholderText = join.InvitePlaceholder, Text = _inviteDraft ?? state.Invite, IsEnabled = editable }, "invite");
            AutomationProperties.SetName(field, join.InviteEyebrow);
            var lookUp = Key(new Button { Content = join.LookUp, VerticalAlignment = VerticalAlignment.Bottom, IsEnabled = editable && JoinInvite.CanLookUp(field.Text, state) }, "look-up");
            AutomationProperties.SetName(lookUp, join.LookUp);
            field.TextChanged += (_, _) =>
            {
                _inviteDraft = field.Text;
                _inviteRefused = false;
                lookUp.IsEnabled = editable && JoinInvite.CanLookUp(field.Text, _runner.State);
            };
            void LookUp()
            {
                if (!JoinInvite.IsEditable(_runner.State))
                {
                    return;
                }

                var looked = JoinInvite.LookUp(_inviteDraft ?? _runner.State.Invite, _runner.State, _runner.Failure, TraceCommons.Interop.Invite.IssuerHost);
                _runner.State = looked.State;
                _runner.Failure = looked.Failure;
                _inviteRefused = looked.Outcome == JoinLookUpOutcome.Refused;
                if (!_inviteRefused)
                {
                    _inviteDraft = null;
                }

                _contentStale = true;
                Render();
            }

            lookUp.Click += (_, _) => LookUp();
            field.KeyDown += (_, e) =>
            {
                if (e.Key == Windows.System.VirtualKey.Enter)
                {
                    LookUp();
                }
            };
            row.Children.Add(field);
            Grid.SetColumn(lookUp, 1);
            row.Children.Add(lookUp);
            panel.Children.Add(row);
        }
        else
        {
            TextBlock eyebrow = Text(join.InviteEyebrow, 14, FirstRunTokens.TextPrimary, FontWeights.SemiBold);
            eyebrow.HorizontalAlignment = HorizontalAlignment.Left;
            panel.Children.Add(eyebrow);
        }

        switch (JoinInvite.Line(state, _runner.Lookup?.Json, _runner.Failure, join, _inviteRefused))
        {
            case JoinInviteLine.Host host:
                panel.Children.Add(StatusLine(host.Name, FirstRunTokens.TextSecondary));
                break;
            case JoinInviteLine.Joined joined:
                panel.Children.Add(StatusLine(joined.Line, FirstRunTokens.ToggleOn));
                break;
            case JoinInviteLine.Error error:
                panel.Children.Add(Alert(error.Line, 420));
                break;
        }

        return Card(panel, 18, 14);
    }

    private static TextBlock StatusLine(string line, uint argb)
    {
        TextBlock t = Text(line, 12, argb);
        t.HorizontalAlignment = HorizontalAlignment.Left;
        t.TextAlignment = TextAlignment.Left;
        return t;
    }

    // ---- The thread -------------------------------------------------------------

    /// <summary>
    /// FirstRunStepThread (FirstRunStage.swift:160-226): the step titles on
    /// one thread, a passed step a button back to it; it comes in with the
    /// first step after Welcome, lifting from -10 on the spatial spring.
    /// </summary>
    private void BuildThread(FirstRunStep? from, FirstRunStep step, double now)
    {
        Thread.Children.Clear();
        int? current = FirstRunStageLayout.ThreadIndex(step);
        int? before = from is FirstRunStep f ? FirstRunStageLayout.ThreadIndex(f) : null;
        bool shown = current is not null, wasShown = before is not null;
        IReadOnlyList<string> labels = Copy.ThreadLabels;
        if (shown || wasShown)
        {
            for (int i = 0; i < labels.Count; i++)
            {
                Rectangle? fill = null;
                if (i > 0)
                {
                    var segment = new Grid { Width = FirstRunStepThread.SegmentWidth, Height = 3, VerticalAlignment = VerticalAlignment.Center };
                    segment.Children.Add(new Line { X1 = 0, Y1 = 1.5, X2 = FirstRunStepThread.SegmentWidth, Y2 = 1.5, StrokeThickness = 2, Stroke = Brush(FirstRunTokens.FirstRunRule), StrokeDashArray = new DoubleCollection { 1, 2 } });
                    fill = new Rectangle { Height = 3, RadiusX = 1.5, RadiusY = 1.5, HorizontalAlignment = HorizontalAlignment.Left, Fill = Brush(FirstRunTokens.FirstRunGlow) };
                    segment.Children.Add(fill);
                    AutomationProperties.SetAccessibilityView(segment, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
                    Thread.Children.Add(segment);
                    int index = i;
                    bool filled = current is int c && index <= c, was = before is int b && index <= b;
                    Rectangle r = fill;
                    _motions.Add((t => r.Width = FirstRunMotion.SegmentWidth(filled, was, t - now, _reduceMotion), t => t - now < FirstRunMotion.SegmentFill));
                }

                bool isCurrent = current == i, passed = current is int cur && i < cur;
                var title = new TextBlock { Text = labels[i], FontSize = 13, Margin = new Thickness(10, 0, 10, 0), VerticalAlignment = VerticalAlignment.Center };
                bool wasCurrent = before == i;
                int at = i;
                _motions.Add((t =>
                {
                    double lit = FirstRunMotion.TitleLit(isCurrent, wasCurrent, t - now, _reduceMotion);
                    uint rest = passed ? FirstRunTokens.TextSecondary : FirstRunTokens.TextTertiary;
                    title.Foreground = new SolidColorBrush(Lerp(rest, FirstRunTokens.TextPrimary, lit));
                    title.FontWeight = lit >= 0.5 ? FontWeights.SemiBold : FontWeights.Medium;
                }, t => t - now < FirstRunMotion.TitleGlowDelay + FirstRunMotion.TitleGlow));
                if (passed)
                {
                    var back = new Button { Content = title, Padding = new Thickness(0), Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0), IsEnabled = !_runner.IsCommitting };
                    AutomationProperties.SetName(back, labels[i]);
                    back.Click += (_, _) =>
                    {
                        _controller.BackTo(FirstRunNavigation.ThreadSteps[at]);
                        Render();
                    };
                    Thread.Children.Add(back);
                }
                else
                {
                    AutomationProperties.SetName(title, labels[i]);
                    Thread.Children.Add(title);
                }

            }
        }

        if (shown != wasShown)
        {
            _motions.Add((t =>
            {
                var (y, o) = FirstRunMotion.Thread(shown, t - now, _reduceMotion);
                ThreadLift.Y = y;
                Thread.Opacity = o;
            }, t => t - now < FirstRunStageLayout.SpatialSettle));
        }
        else
        {
            ThreadLift.Y = 0;
            Thread.Opacity = shown ? 1 : 0;
        }
    }
}

/// <summary>
/// The marker check as a toggle (keeps the toggle's automation peer): a
/// 22-point box inset 2 in 26 points, and the oversized stroke drawn on by
/// length with a dash offset as it ticks (FirstRunStage.swift:347-405).
/// </summary>
internal sealed class MarkerCheckButton : ToggleButton
{
    private readonly Microsoft.UI.Xaml.Shapes.Path _stroke;
    private readonly Border _box;
    private readonly double _length;

    public MarkerCheckButton(bool on)
    {
        IsChecked = on;
        Width = Height = MarkerCheck.Frame;
        Padding = new Thickness(0);
        Background = new SolidColorBrush(Colors.Transparent);
        BorderThickness = new Thickness(0);
        var canvas = new Canvas { Width = MarkerCheck.Frame, Height = MarkerCheck.Frame };
        _box = new Border { Width = MarkerCheck.Box, Height = MarkerCheck.Box, CornerRadius = new CornerRadius(MarkerCheck.BoxRadius), BorderThickness = new Thickness(MarkerCheck.Border) };
        Canvas.SetLeft(_box, MarkerCheck.BoxInset);
        Canvas.SetTop(_box, MarkerCheck.BoxInset);
        canvas.Children.Add(_box);
        var figure = new PathFigure { StartPoint = new Windows.Foundation.Point(MarkerCheck.Start.X, MarkerCheck.Start.Y) };
        foreach (var (c1, c2, to) in MarkerCheck.Curves)
        {
            figure.Segments.Add(new BezierSegment { Point1 = new(c1.X, c1.Y), Point2 = new(c2.X, c2.Y), Point3 = new(to.X, to.Y) });
        }

        var geometry = new PathGeometry();
        geometry.Figures.Add(figure);
        IReadOnlyList<StagePoint> line = MarkerCheck.Polyline();
        for (int i = 1; i < line.Count; i++)
        {
            _length += Math.Sqrt(Math.Pow(line[i].X - line[i - 1].X, 2) + Math.Pow(line[i].Y - line[i - 1].Y, 2));
        }

        _stroke = new Microsoft.UI.Xaml.Shapes.Path
        {
            Data = geometry,
            Stroke = new SolidColorBrush(FirstRunStagePainter.Argb(FirstRunTokens.ToggleOn)),
            StrokeThickness = MarkerCheck.StrokeWidth,
            StrokeStartLineCap = PenLineCap.Round,
            StrokeEndLineCap = PenLineCap.Round,
            StrokeLineJoin = PenLineJoin.Round,
            IsHitTestVisible = false,
        };
        Canvas.SetLeft(_stroke, MarkerCheck.StrokeX);
        Canvas.SetTop(_stroke, MarkerCheck.StrokeY);
        canvas.Children.Add(_stroke);
        Content = canvas;
        Draw(on ? 1 : 0);
        IsEnabledChanged += (_, _) => Opacity = IsEnabled ? 1 : MarkerCheck.Disabled;
    }

    /// <summary>The stroke drawn to <paramref name="trim"/> of its length, and the box for the state.</summary>
    public void Draw(double trim)
    {
        // Dashes are in stroke widths: one dash as long as the path, then a
        // gap as long, offset to show the first trim of it.
        double w = MarkerCheck.StrokeWidth;
        _stroke.StrokeDashArray = new DoubleCollection { _length / w, _length / w + 1 };
        _stroke.StrokeDashOffset = _length / w * (1 - trim);
        _stroke.Opacity = trim <= 0 ? 0 : 1;
        bool on = IsChecked == true;
        _box.Background = new SolidColorBrush(on ? FirstRunStagePainter.Argb(FirstRunTokens.TintAccent) : Colors.Transparent);
        _box.BorderBrush = new SolidColorBrush(FirstRunStagePainter.Argb(on ? FirstRunTokens.FirstRunGlow : FirstRunTokens.TextTertiary));
    }
}

/// <summary>
/// Reads the motion setting again when Windows says a setting changed
/// (WM_SETTINGCHANGE), so turning animations off in Settings takes effect
/// at once, with no timer polling for it.
/// </summary>
internal static class SettingWatcher
{
    private const uint WmSettingChange = 0x001A;
    private static readonly Dictionary<IntPtr, Action> Handlers = new();
    private static readonly SubclassProc Proc = WndProc;

    private delegate IntPtr SubclassProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr id, UIntPtr data);

    [DllImport("comctl32.dll")]
    private static extern bool SetWindowSubclass(IntPtr hwnd, SubclassProc proc, UIntPtr id, UIntPtr data);

    [DllImport("comctl32.dll")]
    private static extern bool RemoveWindowSubclass(IntPtr hwnd, SubclassProc proc, UIntPtr id);

    [DllImport("comctl32.dll")]
    private static extern IntPtr DefSubclassProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam);

    public static void Attach(Window window, Action changed)
    {
        IntPtr hwnd = WinRT.Interop.WindowNative.GetWindowHandle(window);
        Handlers[hwnd] = changed;
        SetWindowSubclass(hwnd, Proc, (UIntPtr)7, UIntPtr.Zero);
        window.Closed += (_, _) =>
        {
            RemoveWindowSubclass(hwnd, Proc, (UIntPtr)7);
            Handlers.Remove(hwnd);
        };
    }

    private static IntPtr WndProc(IntPtr hwnd, uint msg, IntPtr wParam, IntPtr lParam, UIntPtr id, UIntPtr data)
    {
        if (msg == WmSettingChange && Handlers.TryGetValue(hwnd, out Action? changed))
        {
            changed();
        }

        return DefSubclassProc(hwnd, msg, wParam, lParam);
    }
}

/// <summary>The window's DPI, for sizing it in device-independent units.</summary>
internal static class Win32Dpi
{
    [DllImport("user32.dll")]
    private static extern uint GetDpiForWindow(IntPtr hwnd);

    public static double ForWindow(IntPtr hwnd)
    {
        uint dpi = GetDpiForWindow(hwnd);
        return dpi == 0 ? 96 : dpi;
    }
}
