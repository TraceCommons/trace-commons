namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// When the first-run stage's backdrop must be drawn again. The backdrop (the
/// scene's two gradients and its blurred breathing glow, FirstRunStage.swift
/// :109-149) samples time on its own 30-a-second timeline, while the stage
/// is drawn at 60 a second on Uses and Traces and at the display's rate while
/// something moves. The painter draws it into a bitmap once per phase and
/// copies that bitmap on the frames in between, so the pixels are the ones a
/// fresh drawing would make. Windows only: on a machine with no GPU each
/// backdrop is a window-sized gradient fill and blur on the CPU.
/// </summary>
public sealed class FirstRunBackdropCache
{
    private (double Width, double Height, double Scale, double Dpi, double Phase)? _drawn;

    /// <summary>The backdrop's phase at <paramref name="now"/>: its timeline's time, sampled 30 a second.</summary>
    public static double Phase(double now, bool reduceMotion) =>
        SceneMotion.BackdropPhase(FirstRunStageLayout.AtRate(now, FirstRunMotion.Thirty), reduceMotion);

    /// <summary>
    /// True when the backdrop for this canvas size, fit scale, DPI and phase
    /// is not the one last drawn; the caller then draws it, and this
    /// remembers it as drawn.
    /// </summary>
    public bool Redraw(double width, double height, double scale, double dpi, double phase)
    {
        var key = (width, height, scale, dpi, phase);
        if (_drawn == key)
        {
            return false;
        }

        _drawn = key;
        return true;
    }

    /// <summary>The bitmap was let go (a new device): the next frame draws.</summary>
    public void Forget() => _drawn = null;
}
