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

    public FirstRunWindow(DaemonHost host, OnboardingState onboarding)
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
            _ => { });
        _runner = new FirstRunRunner(new FirstRunState(), daemon);
        _runner.Changed += () => DispatcherQueue.TryEnqueue(Render);
        _model.Changed += OnModelChanged;

        Stage.Draw += OnDraw;
        Root.SizeChanged += (_, _) => Fit();
        Root.Loaded += async (_, _) => await LoadAsync();
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
            _gate.SetClosed();
            Stop();
            _painter.Dispose();
        };
        SettingWatcher.Attach(this, ReadMotionSetting);
        AppWindow.Resize(new Windows.Graphics.SizeInt32((int)(DesignWidth * Scale()), (int)(DesignHeight * Scale())));
        _reduceMotion = !_ui.AnimationsEnabled;
        _model.SetReduceMotion(_reduceMotion);
        Render();
    }

    /// <summary>Set by the caller after the window closes.</summary>
    internal Task CloseCompletion { get; private set; } = Task.CompletedTask;

    private double Now => _clock.Elapsed.TotalSeconds;

    private double Scale() => Win32Dpi.ForWindow(WinRT.Interop.WindowNative.GetWindowHandle(this)) / 96.0;

    /// <summary>A deep link's invite: held for Join; it fills and stops (no join without the person).</summary>
    public void OfferInvite(string invite)
    {
        _runner.State.Invite = invite;
    }

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
        DaemonResponse options = await _host.CallAsync(DaemonProtocol.Methods.ConsentOptions).ConfigureAwait(true);
        _options = options.ResultAs<ConsentOptionsPayload>()?.Scopes ?? new List<ConsentOption>();
        var fetched = await Task.Run(FetchedTerms.Fetch).ConfigureAwait(true);
        _terms = fetched.Terms;
        _termsLabel = fetched.Label;
        Render();
    }

    private string? RequiredScope => _options.FirstOrDefault(o => o.AlwaysOn)?.Name;

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
            usesCaptions: _copy is null ? null : new[] { _copy.Uses.ThisComputer, UsesGateLabel(), _copy.Uses.TheCommons });
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
            Outgoing.Content = first ? null : Current.Content;
            _motions.Clear();
            _model.SetStep(state.Step, now);
            if (state.Step == FirstRunStep.Tools && _model.ToolsScanned)
            {
                _scanEndedAt ??= now;
            }

            FirstRunStep? from = first ? null : _shownStep;
            _shownStep = state.Step;
            Current.Content = BuildStep(state.Step);
            if (Outgoing.Content is UIElement leaving)
            {
                _motions.Add((t => { double p = FirstRunMotion.Crossfade(t - now, false); leaving.Opacity = 1 - p; if (p >= 1) { Outgoing.Content = null; } }, t => t - now < FirstRunStageLayout.SpatialSettle));
            }

            if (Current.Content is UIElement arriving && !first)
            {
                _motions.Add((t => arriving.Opacity = FirstRunMotion.Crossfade(t - now, false), t => t - now < FirstRunStageLayout.SpatialSettle));
            }

            BuildThread(from, state.Step, now);
            ApplyMotions(_reduceMotion ? now + 1e6 : now);
            DispatcherQueue.TryEnqueue(MeasureGraphic);
        }

        Footer.Visibility = state.Step == FirstRunStep.Welcome ? Visibility.Collapsed : Visibility.Visible;
        BackButton.Content = _copy.Frame.Back;
        BackButton.Visibility = state.Step == FirstRunStep.Welcome ? Visibility.Collapsed : Visibility.Visible;
        BackButton.IsEnabled = !_runner.IsCommitting;
        ContinueButton.Content = state.Step == FirstRunStep.Join ? _copy.Uses.Start : _copy.Frame.ContinueButton;
        ContinueButton.IsEnabled = !_runner.IsCommitting && CanContinue(state);
        AutomationProperties.SetName(ContinueButton, (string)ContinueButton.Content);
        AutomationProperties.SetName(BackButton, _copy.Frame.Back);
        Stage.Invalidate();
        Kick();
    }

    private bool CanContinue(FirstRunState state) => state.Step switch
    {
        FirstRunStep.Terms => _runner.HasAcceptedTerms(true, _terms?.Version, _terms?.ContentSha256),
        // Automatic needs both sharing disclosures and the core's ready
        // answer before Start can grant it (SharingDisclosures.swift), which
        // this window does not show yet: Start stays off for it.
        FirstRunStep.Join => FirstRunNavigation.CanContinue(JoinStartState(state), RequiredScope)
            && !(state.SharingChosen && state.Sharing == SharingPath.Automatic),
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
                _runner.State = JoinStartState(state);
                await _runner.Commit(CommitPoint.Start);
                break;
            default:
                _runner.State = FirstRunNavigation.Next(state);
                break;
        }

        Render();
    }

    private void OnBack(object sender, RoutedEventArgs e)
    {
        _runner.State = FirstRunNavigation.Back(_runner.State);
        Render();
    }

    private void OnLink(object sender, RoutedEventArgs e)
    {
    }

    private UIElement BuildStep(FirstRunStep step) => step switch
    {
        FirstRunStep.Welcome => Welcome(),
        FirstRunStep.Tools => Framed(step, _copy!.Tools.TitleLight, _copy.Tools.TitleBold, ToolsList()),
        FirstRunStep.PrivateAI => Framed(step, _copy!.PrivateAi.TitleLight, _copy.PrivateAi.TitleBold, PrivateAICard()),
        FirstRunStep.Traces => Framed(step, _copy!.Traces.TitleLight, _copy.Traces.TitleBold, TracesEstimate()),
        FirstRunStep.Uses => Framed(step, _copy!.Uses.TitleLight, _copy.Uses.TitleBold, UsesCard()),
        FirstRunStep.Terms => Framed(step, _copy!.Welcome.Legal.TitleLight, _copy.Welcome.Legal.Terms, TermsReader()),
        _ => Framed(step, _copy!.Join.TitleLight, _copy.Join.TitleBold, JoinCard()),
    };

    private static SolidColorBrush Brush(uint argb) => new(FirstRunStagePainter.Argb(argb));

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
        return panel;
    }

    // ---- Welcome ------------------------------------------------------------

    /// <summary>WelcomeScreen (WelcomeScreen.swift:90-168): the N drawn, the words rising, Get started.</summary>
    private UIElement Welcome()
    {
        var panel = new StackPanel { VerticalAlignment = VerticalAlignment.Center, Margin = new Thickness(40, 0, 40, 0) };
        _graphic = new Border { Height = WelcomeMotion.Hub, Margin = new Thickness(0, 0, 0, WelcomeMotion.HubGap), IsHitTestVisible = false };
        panel.Children.Add(_graphic);
        var title = new StackPanel { Orientation = Orientation.Horizontal, HorizontalAlignment = HorizontalAlignment.Center, Margin = new Thickness(0, 0, 0, 28) };
        AutomationProperties.SetName(title, _copy!.Welcome.Title);
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level1);
        IReadOnlyList<string> words = WelcomeMotion.Words(_copy.Welcome.Title);
        for (int i = 0; i < words.Count; i++)
        {
            TextBlock word = Text(words[i], 26 * WelcomeMotion.HeroScale, FirstRunTokens.TextPrimary, FontWeights.SemiBold);
            word.TextWrapping = TextWrapping.NoWrap;
            AutomationProperties.SetAccessibilityView(word, Microsoft.UI.Xaml.Automation.Peers.AccessibilityView.Raw);
            Rise(word, WelcomeMotion.WordDelay(i), focusable: false);
            title.Children.Add(word);
        }

        panel.Children.Add(title);
        TextBlock body = Text(_copy.Welcome.Body, 20, FirstRunTokens.TextSecondary);
        body.Margin = new Thickness(0, 0, 0, 38);
        Rise(body, WelcomeMotion.BodyDelay, focusable: false);
        panel.Children.Add(body);
        var start = new Button { Content = _copy.Welcome.Start, Style = (Style)Application.Current.Resources["AccentButtonStyle"], HorizontalAlignment = HorizontalAlignment.Center };
        AutomationProperties.SetName(start, _copy.Welcome.Start);
        start.Click += (_, _) => GetStarted();
        Rise(start, WelcomeMotion.ButtonDelay, focusable: true);
        panel.Children.Add(start);
        AgentConfigurationCopy agent = _copy.AgentConfiguration;
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
    /// the marker check, nothing ticked, rising after the scan.
    /// </summary>
    private UIElement ToolsList()
    {
        var list = new StackPanel { Padding = new Thickness(28, 4, 28, 4) };
        var rows = FirstRunStageLayout.RingRows(_discovered.Where(c => c.Exists), c => c.Source);
        for (int i = 0; i < rows.Count; i++)
        {
            SourceCandidate row = rows[i];
            string name = _names.TryGetValue(row.Source, out string? n) ? n : row.Source;
            var line = new Grid { Height = rows.Count > 3 ? 36 : 44, ColumnSpacing = 16 };
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            line.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            TextBlock label = Text(name, 14, FirstRunTokens.TextPrimary, FontWeights.SemiBold);
            label.HorizontalAlignment = HorizontalAlignment.Left;
            label.VerticalAlignment = VerticalAlignment.Center;
            label.MinWidth = 150;
            line.Children.Add(label);
            string question = FirstRunCopy.Fill(_copy!.Folders.WatchQuestion, new Dictionary<string, string> { ["tool"] = name });
            MarkerCheckButton check = MarkerCheck(question, _runner.State.RowAnswer(row.Source).Kind == ToolChoiceKind.Watch, on =>
            {
                _runner.State.Answer(row.Source, on ? ToolChoice.Watch(row.Path) : ToolChoice.Off);
                _model.SetWatched(_runner.State.ToolAnswers.Where(p => p.Value.Kind == ToolChoiceKind.Watch).Select(p => p.Key), Now);
                Render();
            });
            Grid.SetColumn(check, 1);
            line.Children.Add(check);
            RowRise(line, i, check);
            list.Children.Add(line);
        }

        Border card = Card(list, 0, 4);
        double opened = Now;
        _motions.Add((t =>
        {
            double since = _scanEndedAt is double end ? t - end : double.NegativeInfinity;
            card.Opacity = since < 0 ? 0 : FirstRunMotion.CardOpacity(since, _reduceMotion);
            card.IsHitTestVisible = card.Opacity > 0.98;
        }, t => !_model.ToolsScanned || (_scanEndedAt is double end && t - end < FirstRunMotion.CardFade)));
        return card;
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
        figure.Children.Add(Text(_copy!.PrivateAi.OfTotal(found), 14, FirstRunTokens.TextSecondary));
        panel.Children.Add(figure);
        panel.Children.Add(Text(_copy.PrivateAi.OfferLead, 14, FirstRunTokens.TextSecondary));
        var more = new Expander { Header = _copy.PrivateAi.LearnMore, Content = Text(_copy.PrivateAi.OfferMore, 13, FirstRunTokens.TextSecondary) };
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
        bool earnsCredit = _runner.State.Scopes.Contains(_copy!.Uses.CreditScope);
        string? line = FirstRunEstimate.Line(found, earnsCredit);
        if (line is null)
        {
            return new Border();
        }

        var panel = new StackPanel { Spacing = 2 };
        panel.Children.Add(Text(line, 17, FirstRunTokens.TextPrimary, FontWeights.SemiBold));
        panel.Children.Add(Text(_copy.Traces.CreditDepends, 12, FirstRunTokens.TextSecondary));
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
        panel.Children.Add(Text(_copy!.Uses.Eyebrow, 12, FirstRunTokens.TextSecondary, FontWeights.SemiBold));
        if (_options.FirstOrDefault(o => o.AlwaysOn) is { } required && required.Title is { } requiredTitle)
        {
            var locked = new CheckBox { Content = FirstRunCopy.Fill(_copy.Uses.AlwaysOn, new Dictionary<string, string> { ["title"] = requiredTitle }), IsChecked = true, IsEnabled = false };
            panel.Children.Add(locked);
        }

        var optional = _options.Where(o => !o.AlwaysOn && o.GrantsDataUse && o.Title is not null).ToList();
        optional = optional.Where(o => o.Name == _copy.Uses.CreditScope).Concat(optional.Where(o => o.Name != _copy.Uses.CreditScope)).ToList();
        foreach (ConsentOption option in optional)
        {
            var box = new CheckBox { Content = option.Title, IsChecked = _runner.State.Scopes.Contains(option.Name) };
            ToolTipService.SetToolTip(box, option.Description);
            box.Checked += (_, _) => { _runner.State.Scopes.Add(option.Name); Render(); };
            box.Unchecked += (_, _) => { _runner.State.Scopes.Remove(option.Name); Render(); };
            panel.Children.Add(box);
        }

        panel.Children.Add(Text(_copy.Uses.Sharing, 12, FirstRunTokens.TextSecondary, FontWeights.SemiBold));
        panel.Children.Add(Segmented(_copy.Uses.Sharing, new[] { (_copy.Uses.AskEachTime, SharingPath.AskMe), (_copy.Uses.AutomaticScope, SharingPath.Automatic) }));
        return Card(panel, 24, 12);
    }

    /// <summary>FirstRunSegmented (FirstRunStage.swift:410-455): the chosen pill fills over 0.15 s.</summary>
    private UIElement Segmented(string label, (string Title, SharingPath Value)[] options)
    {
        var well = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 2, Padding = new Thickness(2), CornerRadius = new CornerRadius(13), Background = Brush(FirstRunTokens.WellFill), HorizontalAlignment = HorizontalAlignment.Left };
        AutomationProperties.SetName(well, label);
        var since = new Dictionary<SharingPath, double?>();
        foreach (var (title, value) in options)
        {
            bool chosen = _runner.State.SharingChosen && _runner.State.Sharing == value;
            var pill = new ToggleButton { Content = title, IsChecked = chosen, Height = 22, Padding = new Thickness(10, 0, 10, 0), CornerRadius = new CornerRadius(11), BorderThickness = new Thickness(0), FontSize = 12 };
            AutomationProperties.SetName(pill, title);
            since[value] = null;
            pill.Click += (_, _) =>
            {
                FirstRunState s = _runner.State;
                s.Sharing = value;
                s.SharingChosen = true;
                double now = Now;
                foreach (var key in since.Keys.ToList())
                {
                    since[key] = now;
                }

                _model.SetSharing(value, now);
                foreach (ToggleButton other in well.Children.OfType<ToggleButton>())
                {
                    other.IsChecked = ReferenceEquals(other, pill);
                }

                Render();
            };
            _motions.Add((t =>
            {
                bool on = _runner.State.SharingChosen && _runner.State.Sharing == value;
                double p = FirstRunMotion.PillChosen(on, since[value] is double s ? t - s : null, _reduceMotion);
                pill.Background = new SolidColorBrush(Lerp(0x00000000, FirstRunTokens.ToggleOn, p));
                pill.Foreground = new SolidColorBrush(Lerp(FirstRunTokens.TextPrimary, FirstRunTokens.TextOnAccent, p));
            }, t => since[value] is double s && t - s < FirstRunMotion.PillFill));
            well.Children.Add(pill);
        }

        return well;
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
        LegalWords legal = _copy!.Welcome.Legal;
        var panel = new StackPanel { Spacing = 8, MaxWidth = 600 };
        if (_terms is null)
        {
            panel.Children.Add(Text(_termsLabel is null ? legal.Loading : legal.Failed, 14, FirstRunTokens.TextSecondary));
            return Card(panel, 24, 12);
        }

        var body = new StackPanel { Spacing = 8 };
        body.Children.Add(Text(_terms.Title, 17, FirstRunTokens.TextPrimary, FontWeights.SemiBold));
        if (!_terms.InEffect)
        {
            body.Children.Add(Text(legal.NotInEffect, 13, FirstRunTokens.TextSecondary));
        }

        foreach (TermsBlock block in _terms.Blocks)
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
        accept.Checked += (_, _) => { _runner.AcceptedTerms = new AcceptedTerms(_terms.Version, _terms.ContentSha256, DateTimeOffset.UtcNow); Render(); };
        accept.Unchecked += (_, _) => { _runner.AcceptedTerms = null; Render(); };
        panel.Children.Add(scroll);
        panel.Children.Add(Text(legal.ScrollHint, 12, FirstRunTokens.TextSecondary));
        panel.Children.Add(accept);
        return Card(panel, 24, 12);
    }

    // ---- Join -------------------------------------------------------------

    /// <summary>
    /// Join (JoinScreen.swift:346-520): the NEAR AI sign-in, which Start
    /// needs; no wallet path in the first run (W7-D1 (a)).
    /// </summary>
    private UIElement JoinCard()
    {
        JoinWords join = _copy!.Join;
        FirstRunState state = _runner.State;
        var panel = new StackPanel { Spacing = 8, MaxWidth = 460 };
        panel.Children.Add(Text(join.NearAiEyebrow, 12, FirstRunTokens.TextSecondary, FontWeights.SemiBold));
        panel.Children.Add(Text(join.NearAiText, 14, FirstRunTokens.TextPrimary));
        if (state.SignedIn)
        {
            panel.Children.Add(Text(join.SignedIn, 14, FirstRunTokens.TextPrimary, FontWeights.SemiBold));
        }
        else
        {
            var signIn = new Button { Content = join.NearAiSignIn, HorizontalAlignment = HorizontalAlignment.Center, IsEnabled = !_runner.IsCommitting };
            AutomationProperties.SetName(signIn, join.NearAiSignIn);
            signIn.Click += async (_, _) =>
            {
                _runner.State.Account = AccountAnswer.NearAI;
                await _runner.Commit(CommitPoint.SignIn);
                if (_runner.State.SignedIn)
                {
                    _model.SignedIn(Now);
                }

                _shownStep = (FirstRunStep)(-1);
                Render();
            };
            panel.Children.Add(signIn);
        }

        if (FailureLine(_runner.Failure) is { } line)
        {
            panel.Children.Add(Text(line, 13, FirstRunTokens.TextSecondary));
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
        return card;
    }

    /// <summary>The core's line for a failed commit (FoldersCopy and UsesCopy failure words).</summary>
    private string? FailureLine(FirstRunFailure? failure) => failure switch
    {
        FirstRunFailure.SignInFailed => _copy!.Folders.SignInFailed,
        FirstRunFailure.LookupUnavailable => _copy!.Folders.LookupUnavailable,
        FirstRunFailure.EnrollFailed => _copy!.Folders.EnrollRefused,
        FirstRunFailure.SettingsFailed => _copy!.Folders.SettingsFailed,
        FirstRunFailure.ScopesFailed => _copy!.Uses.ScopesFailed,
        FirstRunFailure.RulesFailed => _copy!.Uses.RulesFailed,
        FirstRunFailure.PrivateAIFailed => _copy!.Uses.PrivateAiFailed,
        FirstRunFailure.CompleteFailed => _copy!.Uses.CompleteFailed,
        FirstRunFailure.SharingFailed => _copy!.Uses.SharingSaveFailed,
        _ => null,
    };

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
        IReadOnlyList<string> labels = _copy!.ThreadLabels;
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
                        _runner.State = FirstRunNavigation.BackTo(FirstRunNavigation.ThreadSteps[at], _runner.State);
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
