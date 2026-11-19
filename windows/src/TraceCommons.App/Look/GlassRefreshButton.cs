using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Look;

/// <summary>
/// The Mac's refresh pill (Containers.swift:71-196, GlassRefreshButton and
/// GlassRefreshPill): the word, then the refresh glyph, on the control
/// surface. A click turns the glyph one full turn in 0.8 s however quick the
/// answer; while <see cref="IsRefreshing"/> it keeps turning and the button
/// is disabled; 30 frames a second; still under reduce motion. A Button
/// re-templated (TcGlassRefreshButtonStyle), keeping its automation peer;
/// its accessible name is the caller's, from the core's words.
/// </summary>
public sealed class GlassRefreshButton : Button
{
    public static readonly DependencyProperty IsRefreshingProperty = DependencyProperty.Register(
        nameof(IsRefreshing), typeof(bool), typeof(GlassRefreshButton),
        new PropertyMetadata(false, (d, _) => ((GlassRefreshButton)d).OnRefreshingChanged()));

    private readonly GlassClock _clock;
    private readonly DispatcherQueueTimer _completing;
    private double? _clickedAt;
    private bool _completingClick;
    private RotateTransform? _turn;

    public GlassRefreshButton()
    {
        Style = (Style)Application.Current.Resources["TcGlassRefreshButtonStyle"];
        _clock = new GlassClock(this, GlassRefreshMotion.FrameInterval, Draw) { Paused = true };
        // The click's turn: only the acknowledgement waits, never the work
        // (Containers.swift:130-137). Stopped with the element.
        _completing = DispatcherQueue.GetForCurrentThread().CreateTimer();
        _completing.Interval = System.TimeSpan.FromSeconds(GlassTokens.Motion.Spin);
        _completing.IsRepeating = false;
        _completing.Tick += (_, _) =>
        {
            _completing.Stop();
            _completingClick = false;
            Sync();
        };
        Unloaded += (_, _) => _completing.Stop();
        Click += (_, _) =>
        {
            _clickedAt = GlassClock.Now;
            _completingClick = true;
            _completing.Stop();
            _completing.Start();
            Sync();
        };
    }

    public bool IsRefreshing
    {
        get => (bool)GetValue(IsRefreshingProperty);
        set => SetValue(IsRefreshingProperty, value);
    }

    protected override void OnApplyTemplate()
    {
        base.OnApplyTemplate();
        _turn = (GetTemplateChild("Glyph") as UIElement)?.RenderTransform as RotateTransform;
        Sync();
    }

    private void OnRefreshingChanged()
    {
        IsEnabled = !IsRefreshing;
        Sync();
    }

    private void Sync() =>
        _clock.Paused = !GlassRefreshMotion.Animates(IsRefreshing, _completingClick, reduceMotion: !GlassMotionSettings.AnimationsEnabled);

    private void Draw(double now)
    {
        if (_turn is null)
        {
            return;
        }

        double elapsed = _clickedAt is double at ? now - at : IsRefreshing ? now : 0;
        _turn.Angle = GlassRefreshMotion.Angle(elapsed, IsRefreshing, reduceMotion: !GlassMotionSettings.AnimationsEnabled);
    }
}
