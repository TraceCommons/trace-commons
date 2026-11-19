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
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.App.FirstRun;
using TraceCommons.Interop;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Shell;
using Windows.ApplicationModel.DataTransfer;
using Windows.UI.Text;
using Windows.UI.ViewManagement;

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
public sealed partial class FirstRunWindow : Window
{
    private const double DesignWidth = FirstRunStageLayout.CanvasWidth, DesignHeight = FirstRunStageLayout.CanvasHeight;

    private readonly DaemonHost _host;
    private readonly OnboardingState _onboarding;
    private readonly FirstRunCopy? _copy;
    private readonly PrivateInferenceCopy? _privateCopy = PrivateInferenceSurface.Copy();
    private readonly FirstRunRunner _runner;
    private readonly FirstRunStageModel _model = new();
    private readonly FirstRunStagePainter _painter;
    private readonly Stopwatch _clock = Stopwatch.StartNew();
    private readonly UISettings _ui = new();
    private readonly WindowClockGate _gate = new();
    private readonly List<(Action<double> Apply, Func<double, bool> Moving)> _motions = new();
    private readonly Dictionary<string, string> _names = new(StringComparer.Ordinal);
    private IReadOnlyList<SourceCandidate> _discovered = Array.Empty<SourceCandidate>();
    private List<ConsentOption> _options = new();
    private FetchedTerms? _terms;
    private string? _termsLabel;
    private bool _ticking;
    private double _lastFrame = double.NegativeInfinity;
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
        _host = host;
        _onboarding = onboarding;
        Title = MonitorWords.AppName;
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
        _runner = previewAt is FirstRunStep step
            ? new FirstRunRunner(new FirstRunState { Step = step }, new RefusingFirstRunDaemon())
            : new FirstRunRunner(new FirstRunState(), daemon);
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
        Activated += (_, _) => ReadMotionSetting();
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
        int width = (int)Math.Round(DesignWidth * scale) + extraWidth;
        int height = (int)Math.Round(DesignHeight * scale) + extraHeight;
        if (width != AppWindow.Size.Width || height != AppWindow.Size.Height)
        {
            AppWindow.Resize(new Windows.Graphics.SizeInt32(width, height));
        }

        ApplyMinimumSize(extraWidth, extraHeight);
    }

    /// <summary>The Mac's minimum window (<see cref="FirstRunStageLayout.MinimumWidth"/>), through WM_GETMINMAXINFO.</summary>
    private void ApplyMinimumSize(int extraWidth, int extraHeight)
    {
        double scale = Scale();
        WindowMinimumSize.Set(
            WinRT.Interop.WindowNative.GetWindowHandle(this),
            (int)Math.Ceiling(FirstRunStageLayout.MinimumWidth * scale) + extraWidth,
            (int)Math.Ceiling(FirstRunStageLayout.MinimumHeight * scale) + extraHeight);
    }

    /// <summary>The Tools step started the daemon with the person's declaration.</summary>
    public event Action? DaemonStarted;

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
        _terms = fetched.Terms;
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

    private string? RequiredScope => _options.FirstOrDefault(o => o.AlwaysOn)?.Name;

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
    }

    private void OnFrame(object? sender, object e)
    {
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
        double now = Now;
        _painter.Draw(args.DrawingSession, sender, _model, _reduceMotion ? now + 1e6 : now, sender.ActualWidth, sender.ActualHeight,
            tracesCountText: TracesCount(now), tracesCaption: _copy?.Traces.OnThisComputer,
            privateAIPill: _privateCopy?.StatInferenceAccess is { Length: > 0 } pill ? pill : null,
            usesCaptions: _copy is null ? null : new[] { Copy.Uses.ThisComputer, UsesGateLabel(), Copy.Uses.TheCommons });
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
            CurrentStep.Content = BuildStep(state.Step);
            ApplyMotions(_reduceMotion ? Now + 1e6 : Now);
            DispatcherQueue.TryEnqueue(MeasureGraphic);
        }

        Footer.Visibility = state.Step == FirstRunStep.Welcome ? Visibility.Collapsed : Visibility.Visible;
        BackButton.Content = Copy.Frame.Back;
        BackButton.Visibility = state.Step == FirstRunStep.Welcome ? Visibility.Collapsed : Visibility.Visible;
        BackButton.IsEnabled = !_runner.IsCommitting || BackWhileCommitting(state);
        ContinueButton.Content = state.Step == FirstRunStep.Join ? Copy.Uses.Start : Copy.Frame.ContinueButton;
        ContinueButton.IsEnabled = !_runner.IsCommitting && CanContinue(state);
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
        _pendingFolder?.Path ?? "", _addRefused, _inviteRefused, state.AddedFolders.Count);

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

    /// <summary>
    /// Back while the browser sign-in waits takes the person back, ending
    /// that sign-in as Cancel does (JoinScreenLayout.backWhileCommitting);
    /// any other commit withdraws Back.
    /// </summary>
    private bool BackWhileCommitting(FirstRunState state) =>
        state.Step == FirstRunStep.Join && _runner.Committing == CommitPoint.SignIn && _runner.SignInWaiting;

    private bool CanContinue(FirstRunState state) => state.Step switch
    {
        FirstRunStep.Terms => _runner.HasAcceptedTerms(true, _terms?.Version, _terms?.ContentSha256),
        // Automatic goes through both sharing disclosures and the core's
        // answer when Start is pressed (StartAsync), as on the Mac.
        FirstRunStep.Join => FirstRunNavigation.CanContinue(JoinStartState(state), RequiredScope) && !_starting,
        _ => FirstRunNavigation.CanContinue(state, RequiredScope),
    };

    /// <summary>JoinScreenLayout.startState: the required use included, an account held read as enrolled.</summary>
    private FirstRunState JoinStartState(FirstRunState state)
    {
        FirstRunState ready = state.Clone();
        bool canForward = (ready.Account == AccountAnswer.NearAI || (ready.NearAIEnrolled && !ready.SignedOutOfEnrolment))
            && (ready.Account != AccountAnswer.NearAI || ready.SignedIn);
        if (canForward && ready.Account != AccountAnswer.NearAI)
        {
            ready.Account = AccountAnswer.Enrolled;
        }

        return ready.IncludingRequiredScope(RequiredScope);
    }

    private async void OnContinue(object sender, RoutedEventArgs e)
    {
        FirstRunState state = _runner.State;
        switch (state.Step)
        {
            case FirstRunStep.Tools:
                await _runner.Commit(CommitPoint.LeaveTools);
                break;
            case FirstRunStep.Terms:
                _runner.ContinueFromTerms(true, _terms?.Version, _terms?.ContentSha256);
                break;
            case FirstRunStep.Join:
                await StartAsync();
                break;
            default:
                _runner.State = FirstRunNavigation.Next(state);
                break;
        }

        Render();
    }

    private bool _starting;

    /// <summary>A refusal decided before a Start that then failed, said once one succeeds (UsesStart, UsesScreen.swift:275-309).</summary>
    private FirstRunFailure? _pendingRefusal;

    private AutomaticGrantCopy? _grantCopy;
    private bool _grantCopyRead;

    /// <summary>
    /// Start sharing (JoinScreen.swift:712-760): the required use included;
    /// on Automatic, the scrub and witness disclosures first, then the
    /// core's answer decides the state Start commits (Automatic with the
    /// grant ready, or Ask with the refusal said). Cancel on either sheet
    /// grants nothing and starts nothing.
    /// </summary>
    private async Task StartAsync()
    {
        _starting = true;
        Render();
        try
        {
            _runner.State = JoinStartState(_runner.State);
            FirstRunFailure? refused = null;
            bool decidedNow = false;
            if (SharingDisclosureFlow.IsNeeded(_runner.State))
            {
                if (!_grantCopyRead)
                {
                    string dir = _host.ConfigDir;
                    _grantCopy = await Task.Run(() => AutomaticGrantCopy.Load(dir)).ConfigureAwait(true);
                    _grantCopyRead = true;
                }

                var flow = new SharingDisclosureFlow();
                Flow1GrantRequest? request = null;
                if (_grantCopy is { } grant)
                {
                    if (!await ShowDisclosuresAsync(flow, grant))
                    {
                        return;
                    }

                    DaemonResponse status = await _host.CallAsync(DaemonProtocol.Methods.Status).ConfigureAwait(true);
                    bool connected = status.ResultAs<DaemonStatus>()?.LoggedIn == true;
                    string progress = flow.ProgressJson(connected, _runner.State.Scopes);
                    request = await Task.Run(() => Flow1GrantRequest.Ask(progress)).ConfigureAwait(true);
                }

                (FirstRunState next, refused) = SharingDisclosureFlow.Resolve(_runner.State, request);
                _runner.State = next;
                decidedNow = true;
            }

            FirstRunFailure? carried = decidedNow ? refused : _pendingRefusal;
            await _runner.Commit(CommitPoint.Start, carried);
            if (_runner.Completed)
            {
                _pendingRefusal = null;
                return;
            }

            // A Start that failed keeps the refusal for the retry; the
            // failure it stopped on is what the step says now.
            _pendingRefusal = _runner.Failure is FirstRunFailure.CompleteFailed ? _runner.RefusedGrant ?? carried : carried;
        }
        finally
        {
            _starting = false;
            Render();
        }
    }

    /// <summary>
    /// The two sheets, one at a time (SharingDisclosureSheet,
    /// SharingDisclosures.swift:104-205): the scrub lines, then the route
    /// disclosure. Continue is never the default button, so a stray Enter
    /// passes no disclosure; a disclosure that cannot be read keeps Continue
    /// off. False when the person cancelled.
    /// </summary>
    private async Task<bool> ShowDisclosuresAsync(SharingDisclosureFlow flow, AutomaticGrantCopy grant)
    {
        var scrub = new StackPanel { Spacing = 10 };
        foreach (string line in grant.Lines)
        {
            scrub.Children.Add(new TextBlock { Text = line, TextWrapping = TextWrapping.Wrap });
        }

        if (await Sheet(WatchCopy.Armed, scrub, true) != ContentDialogResult.Primary || _closed)
        {
            return false;
        }

        flow.AcknowledgeScrub();
        DaemonResponse facts = await _host.CallAsync(DaemonProtocol.Methods.RouteDisclosure).ConfigureAwait(true);
        RouteDisclosure? disclosure = facts.IsError || facts.Result is null ? null : RouteDisclosureSurface.ForFacts(facts.Result.Value.GetRawText());
        RouteDisclosureUnreadable? unreadable = disclosure is null ? RouteDisclosureSurface.Unreadable() : null;
        var body = new StackPanel { Spacing = 8 };
        foreach (DisclosureRow row in RouteDisclosureSurface.PanelRows(disclosure, unreadable))
        {
            body.Children.Add(new TextBlock
            {
                Text = row.Value.Length > 0 ? row.Text + " " + row.Value : row.Text,
                TextWrapping = TextWrapping.Wrap,
                FontWeight = row.IsHeading ? FontWeights.SemiBold : FontWeights.Normal,
            });
        }

        string title = RouteDisclosureSurface.PanelTitle(disclosure, unreadable);
        if (await Sheet(title.Length > 0 ? title : WatchCopy.Armed, body, disclosure is not null) != ContentDialogResult.Primary || _closed)
        {
            return false;
        }

        flow.AcknowledgeWitness(disclosure?.Facts.Witness?.SigningAddress);
        return flow.Step == DisclosureStep.Done;
    }

    private async Task<ContentDialogResult> Sheet(string title, UIElement body, bool canContinue)
    {
        var dialog = new ContentDialog
        {
            Title = title,
            Content = new ScrollViewer { Content = body, MaxHeight = 420 },
            PrimaryButtonText = Copy.Frame.ContinueButton,
            CloseButtonText = Copy.Frame.Cancel,
            DefaultButton = ContentDialogButton.None,
            IsPrimaryButtonEnabled = canContinue,
            XamlRoot = Root.XamlRoot,
            RequestedTheme = ElementTheme.Dark,
        };
        return await dialog.ShowAsync();
    }

    private async void OnBack(object sender, RoutedEventArgs e)
    {
        if (_runner.IsCommitting)
        {
            if (BackWhileCommitting(_runner.State))
            {
                await _runner.BackFromSignIn();
                Render();
            }

            return;
        }

        _runner.State = FirstRunNavigation.Back(_runner.State);
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

    /// <summary>FirstRunFrame (FirstRunFrame.swift:150-230): graphic, title, content, over the thread's 44.</summary>
    private UIElement Framed(FirstRunStep step, string light, string bold, UIElement content)
    {
        var panel = new StackPanel { Margin = new Thickness(0, 44, 0, 0), VerticalAlignment = VerticalAlignment.Center };
        _graphic = new Border { Height = StageActors.GraphicHeight(step), Margin = new Thickness(0, 24, 0, 24), IsHitTestVisible = false };
        panel.Children.Add(_graphic);
        var title = new TextBlock { FontSize = 26, TextAlignment = TextAlignment.Center, HorizontalAlignment = HorizontalAlignment.Center, TextWrapping = TextWrapping.Wrap, Margin = new Thickness(40, 0, 40, 20) };
        title.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = light, Foreground = Brush(FirstRunTokens.TextSecondary) });
        title.Inlines.Add(new Microsoft.UI.Xaml.Documents.Run { Text = bold, FontWeight = FontWeights.Bold, Foreground = Brush(FirstRunTokens.TextPrimary) });
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level1);
        AutomationProperties.SetName(title, light + bold);
        panel.Children.Add(title);
        panel.Children.Add(content);
        if (Notice(_runner.State) is { } notice)
        {
            panel.Children.Add(Alert(notice, 560));
        }

        return panel;
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
        var start = new Button { Content = Copy.Welcome.Start, Style = (Style)Application.Current.Resources["AccentButtonStyle"], HorizontalAlignment = HorizontalAlignment.Center };
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
            MarkerCheckButton check = MarkerCheck(question, state.RowAnswer(row.Source).Kind == ToolChoiceKind.Watch, on =>
            {
                _runner.State.Answer(row.Source, on ? ToolChoice.Watch(row.Path) : ToolChoice.Off);
                WatchedChanged();
            });
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
            list.Children.Add(FolderRow(name, folder.Path, Copy.Tools.AddedByYou, MarkerCheckFor(
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
            list.Children.Add(FolderRow(name, trajectory.Path, Copy.Tools.AddedByYou, MarkerCheckFor(
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
            var picker = new ComboBox { PlaceholderText = Copy.Frame.Choose, IsEnabled = enabled, VerticalAlignment = VerticalAlignment.Center };
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
        var label = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8, HorizontalAlignment = HorizontalAlignment.Center };
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
        var face = new Grid { Height = 36, Padding = new Thickness(32, 0, 32, 0) };
        face.Children.Add(outline);
        face.Children.Add(label);
        var button = new Button
        {
            Content = face,
            Padding = new Thickness(0),
            Background = new SolidColorBrush(Colors.Transparent),
            BorderThickness = new Thickness(0),
            HorizontalAlignment = HorizontalAlignment.Stretch,
            HorizontalContentAlignment = HorizontalAlignment.Stretch,
            IsEnabled = enabled,
        };
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
        var more = new Expander { Header = Copy.PrivateAi.LearnMore, Content = Text(Copy.PrivateAi.OfferMore, 13, FirstRunTokens.TextSecondary) };
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
            var box = new CheckBox { Content = option.Title, IsChecked = _runner.State.Scopes.Contains(option.Name) };
            ToolTipService.SetToolTip(box, option.Description);
            box.Checked += (_, _) => { _runner.State.Scopes.Add(option.Name); Render(); };
            box.Unchecked += (_, _) => { _runner.State.Scopes.Remove(option.Name); Render(); };
            panel.Children.Add(box);
        }

        panel.Children.Add(Text(Copy.Uses.Sharing, 12, FirstRunTokens.TextSecondary, FontWeights.SemiBold));
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

            var radio = new RadioButton
            {
                Content = words,
                GroupName = groupName,
                IsChecked = _runner.State.SharingChosen && _runner.State.Sharing == option.Path,
            };
            AutomationProperties.SetName(radio, option.Title);
            if (option.Lines.Count > 0)
            {
                AutomationProperties.SetHelpText(radio, string.Join(" ", option.Lines));
            }

            SharingPath path = option.Path;
            radio.Checked += (_, _) =>
            {
                FirstRunState s = _runner.State;
                if (s.SharingChosen && s.Sharing == path)
                {
                    return;
                }

                s.Sharing = path;
                s.SharingChosen = true;
                _model.SetSharing(path, Now);
                Render();
            };
            group.Children.Add(radio);
        }

        return group;
    }

    /// <summary>
    /// A glow under <paramref name="shape"/>: a composition drop shadow in
    /// firstRunGlow, masked by the element's own alpha, blurred to
    /// <paramref name="radius"/>, placed in <paramref name="host"/> behind it.
    /// The returned setter takes its opacity (0 to 1).
    /// </summary>
    private static Func<double, bool> Glow(Panel host, FrameworkElement shape, double radius)
    {
        var compositor = Microsoft.UI.Xaml.Hosting.ElementCompositionPreview.GetElementVisual(host).Compositor;
        var shadow = compositor.CreateDropShadow();
        shadow.BlurRadius = (float)radius;
        shadow.Color = FirstRunStagePainter.Argb(FirstRunTokens.FirstRunGlow);
        shadow.Offset = System.Numerics.Vector3.Zero;
        shadow.Opacity = 0;
        var sprite = compositor.CreateSpriteVisual();
        sprite.Shadow = shadow;
        var layer = new Canvas { IsHitTestVisible = false };
        Microsoft.UI.Xaml.Automation.AutomationProperties.SetAccessibilityView(layer, AccessibilityView.Raw);
        host.Children.Add(layer);
        Microsoft.UI.Xaml.Hosting.ElementCompositionPreview.SetElementChildVisual(layer, sprite);
        void Fit()
        {
            sprite.Size = new System.Numerics.Vector2((float)shape.ActualWidth, (float)shape.ActualHeight);
            shadow.Mask = shape switch
            {
                TextBlock t => t.GetAlphaMask(),
                Shape sh => sh.GetAlphaMask(),
                _ => null,
            };
        }

        shape.SizeChanged += (_, _) => Fit();
        shape.Loaded += (_, _) => Fit();
        return opacity =>
        {
            shadow.Opacity = (float)Math.Clamp(opacity, 0, 1);
            return true;
        };
    }

    private static Windows.UI.Color Lerp(uint a, uint b, double p)
    {
        byte M(int shift) => (byte)Math.Round(((a >> shift) & 0xFF) * (1 - p) + ((b >> shift) & 0xFF) * p);
        return Windows.UI.Color.FromArgb(M(24), M(16), M(8), M(0));
    }

    // ---- Terms ----------------------------------------------------------------

    /// <summary>TermsReader: the published terms, then Accept once read to the end.</summary>
    private UIElement TermsReader()
    {
        LegalWords legal = Copy.Welcome.Legal;
        var panel = new StackPanel { Spacing = 8, MaxWidth = 600 };
        if (_terms is null)
        {
            panel.Children.Add(Text(_termsLabel is null ? legal.Loading : legal.Failed, 14, FirstRunTokens.TextSecondary));
            return Card(panel, 24, 12);
        }

        FetchedTerms terms = _terms;
        var body = new StackPanel { Spacing = 8 };
        body.Children.Add(Text(terms.Title, 17, FirstRunTokens.TextPrimary, FontWeights.SemiBold));
        if (!terms.InEffect)
        {
            body.Children.Add(Text(legal.NotInEffect, 13, FirstRunTokens.TextSecondary));
        }

        foreach (TermsBlock block in terms.Blocks)
        {
            switch (block.Kind)
            {
                case "heading":
                    body.Children.Add(new TextBlock { Text = block.Text, FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap });
                    break;
                case "list":
                    foreach (string item in block.Items!)
                    {
                        body.Children.Add(new TextBlock { Text = "• " + item, TextWrapping = TextWrapping.Wrap });
                    }

                    break;
                case "table":
                    body.Children.Add(new TextBlock { Text = string.Join(" | ", block.Header!), FontWeight = FontWeights.SemiBold, TextWrapping = TextWrapping.Wrap });
                    foreach (var row in block.Rows!)
                    {
                        body.Children.Add(new TextBlock { Text = string.Join(" | ", row), TextWrapping = TextWrapping.Wrap });
                    }

                    break;
                default:
                    body.Children.Add(new TextBlock { Text = block.Text, TextWrapping = TextWrapping.Wrap });
                    break;
            }
        }

        var scroll = new ScrollViewer { Content = body, Height = 260 };
        var accept = new CheckBox { Content = legal.Accept, IsEnabled = _runner.AcceptedTerms is not null, IsChecked = _runner.AcceptedTerms is not null };
        AutomationProperties.SetName(accept, legal.Accept);
        scroll.ViewChanged += (_, _) =>
        {
            if (scroll.VerticalOffset >= scroll.ScrollableHeight - 4)
            {
                accept.IsEnabled = true;
            }
        };
        accept.Checked += (_, _) => { _runner.AcceptedTerms = new AcceptedTerms(terms.Version, terms.ContentSha256, DateTimeOffset.UtcNow); Render(); };
        accept.Unchecked += (_, _) => { _runner.AcceptedTerms = null; Render(); };
        panel.Children.Add(scroll);
        panel.Children.Add(Text(legal.ScrollHint, 12, FirstRunTokens.TextSecondary));
        panel.Children.Add(accept);
        return Card(panel, 24, 12);
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
                var cancel = new Button { Content = Copy.Frame.Cancel };
                AutomationProperties.SetName(cancel, Copy.Frame.Cancel);
                cancel.Click += async (_, _) =>
                {
                    await _runner.CancelSignIn();
                    Render();
                };
                actions.Children.Add(cancel);
            }

            var signIn = new Button { Content = join.NearAiSignIn, IsEnabled = !_runner.IsCommitting };
            AutomationProperties.SetName(signIn, join.NearAiSignIn);
            signIn.Click += async (_, _) =>
            {
                _runner.State.Account = AccountAnswer.NearAI;
                await _runner.Commit(CommitPoint.SignIn);
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
            var field = new TextBox { Header = join.InviteEyebrow, PlaceholderText = join.InvitePlaceholder, Text = _inviteDraft ?? state.Invite, IsEnabled = editable };
            AutomationProperties.SetName(field, join.InviteEyebrow);
            var lookUp = new Button { Content = join.LookUp, VerticalAlignment = VerticalAlignment.Bottom, IsEnabled = editable && JoinInvite.CanLookUp(field.Text, state) };
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
                var title = new TextBlock { Text = labels[i], FontSize = 13, VerticalAlignment = VerticalAlignment.Center };
                // The lit title's glow, firstRunGlow at 0.35, radius 7
                // (FirstRunStage.swift:210-211), drawn under the words from
                // their own shape and lit with them.
                var cell = new Grid { Margin = new Thickness(FirstRunStepThread.TitlePadding, 0, FirstRunStepThread.TitlePadding, 0), VerticalAlignment = VerticalAlignment.Center };
                Func<double, bool> setGlow = Glow(cell, title, FirstRunStepThread.TitleGlow);
                cell.Children.Add(title);
                bool wasCurrent = before == i;
                int at = i;
                _motions.Add((t =>
                {
                    double lit = FirstRunMotion.TitleLit(isCurrent, wasCurrent, t - now, _reduceMotion);
                    uint rest = passed ? FirstRunTokens.TextSecondary : FirstRunTokens.TextTertiary;
                    title.Foreground = new SolidColorBrush(Lerp(rest, FirstRunTokens.TextPrimary, lit));
                    title.FontWeight = lit >= 0.5 ? FontWeights.SemiBold : FontWeights.Medium;
                    setGlow(FirstRunStepThread.GlowAlpha * lit);
                }, t => t - now < FirstRunMotion.TitleGlowDelay + FirstRunMotion.TitleGlow));
                if (passed)
                {
                    var back = new Button { Content = cell, Padding = new Thickness(0), Background = new SolidColorBrush(Colors.Transparent), BorderThickness = new Thickness(0), IsEnabled = !_runner.IsCommitting };
                    AutomationProperties.SetName(back, labels[i]);
                    back.Click += (_, _) =>
                    {
                        _runner.State = FirstRunNavigation.BackTo(FirstRunNavigation.ThreadSteps[at], _runner.State);
                        Render();
                    };
                    Thread.Children.Add(back);
                }
                else
                {
                    AutomationProperties.SetName(title, labels[i]);
                    Thread.Children.Add(cell);
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
