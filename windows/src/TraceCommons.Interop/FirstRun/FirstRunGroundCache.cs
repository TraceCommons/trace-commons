namespace TraceCommons.Interop.FirstRun;

/// <summary>
/// When the first-run stage's ground (the backdrop's two gradients,
/// FirstRunStage.swift:109-128) must be drawn again into its bitmap: only
/// for a new canvas size or DPI, since it never moves. The painter copies
/// the bitmap each frame and draws the breathing glow over it, so the
/// pixels are the ones drawing the gradients would make. Windows only: on a
/// machine with no GPU each gradient is a window-sized fill on the CPU.
/// </summary>
public sealed class FirstRunGroundCache
{
    private (double Width, double Height, double Dpi)? _drawn;

    /// <summary>True when the ground in the bitmap is not this size and DPI; it is then counted as drawn.</summary>
    public bool Redraw(double width, double height, double dpi)
    {
        var key = (width, height, dpi);
        if (_drawn == key)
        {
            return false;
        }

        _drawn = key;
        return true;
    }

    /// <summary>The bitmap was let go (a new device): the next frame draws it.</summary>
    public void Forget() => _drawn = null;
}
