using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using Microsoft.UI.Xaml.Shapes;
using TraceCommons.Interop.Shell;
using Windows.Foundation;

namespace TraceCommons.App.Look;

/// <summary>
/// The Mac's spinner (Feedback.swift:15-63, GlassSpinner): a 12 across ring
/// in ink at 0.18 with a quarter arc in the secondary ink turning once every
/// 0.8 s, at most 60 frames a second, still under reduce motion. A
/// ProgressRing re-templated (TcGlassSpinnerStyle), so it keeps the stock
/// progress automation peer; name it with AutomationProperties.Name from
/// the core's words, or leave it unnamed beside words that already say what
/// is happening.
/// </summary>
public sealed class GlassSpinner : ProgressRing
{
    private readonly GlassClock _clock;
    private Path? _arc;
    private RotateTransform? _turn;

    public GlassSpinner()
    {
        Style = (Style)Application.Current.Resources["TcGlassSpinnerStyle"];
        Width = Height = GlassTokens.Size.Spinner;
        IsActive = true;
        _clock = new GlassClock(this, GlassSpinnerMotion.FrameInterval, Draw);
        RegisterPropertyChangedCallback(IsActiveProperty, (_, _) => _clock.Paused = !IsActive);
        SizeChanged += (_, _) => Shape();
    }

    protected override void OnApplyTemplate()
    {
        base.OnApplyTemplate();
        _arc = GetTemplateChild("Arc") as Path;
        _turn = _arc?.RenderTransform as RotateTransform;
        Shape();
        _clock.Update();
    }

    /// <summary>
    /// The quarter arc from the ring's 3 o'clock clockwise to 6 o'clock
    /// (SwiftUI's Circle starts at angle 0 and runs clockwise on screen;
    /// trim(0, 0.25)), inset by half the 1.5 stroke as the Mac pads it.
    /// </summary>
    private void Shape()
    {
        if (_arc is null || ActualWidth <= 0)
        {
            return;
        }

        double stroke = GlassTokens.Size.SpinnerStroke;
        double r = (ActualWidth - stroke) / 2, c = ActualWidth / 2;
        var figure = new PathFigure { StartPoint = new Point(c + r, c), IsClosed = false, IsFilled = false };
        figure.Segments.Add(new ArcSegment { Point = new Point(c, c + r), Size = new Size(r, r), SweepDirection = SweepDirection.Clockwise });
        var geometry = new PathGeometry();
        geometry.Figures.Add(figure);
        _arc.Data = geometry;
        if (_turn is not null)
        {
            _turn.CenterX = c;
            _turn.CenterY = c;
        }
    }

    private void Draw(double now)
    {
        if (_turn is not null)
        {
            _turn.Angle = GlassSpinnerMotion.Angle(now, reduceMotion: !GlassMotionSettings.AnimationsEnabled);
        }
    }
}
