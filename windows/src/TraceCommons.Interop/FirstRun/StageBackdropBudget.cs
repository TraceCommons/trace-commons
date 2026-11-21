namespace TraceCommons.Interop.FirstRun;

/// <summary>How much of the first-run backdrop is drawn.</summary>
public enum StageBackdrop
{
    /// <summary>The ground, the corner glow and the blurred breath.</summary>
    Full,

    /// <summary>Without the blurred breath.</summary>
    NoBreath,

    /// <summary>Without the breath and the two ground gradients: the scene's base colour alone.</summary>
    NoGround,
}

/// <summary>
/// A lighter first-run backdrop on a machine that cannot keep up. Windows
/// only, at the owner's request (2026-11-20, from the profiling in #2278);
/// the Mac has no such mode.
///
/// On a 4-core box with no GPU the stage drew 11-12 frames a second,
/// limited by how fast WARP fills pixels: 18 without the blurred breath,
/// more without the ground gradients too, 24 without the whole backdrop.
/// So: software rendering starts without the breath; a stage asking for 30
/// or more a second that draws under <see cref="SlowRate"/> in each of
/// <see cref="SustainedSeconds"/> whole seconds running drops one more layer,
/// the breath first, then the ground gradients. Nothing else is ever dropped.
///
/// It never steps back. A lighter look draws faster, so its own rate says
/// nothing about whether the full look would now keep up; stepping back on
/// it would flap between the two. The first run is one window, shown once.
/// </summary>
public sealed class StageBackdropBudget
{
    /// <summary>
    /// Under 20 a second the stage misses at least a third of the 30 a second
    /// it asks for at its slowest, and its motion reads as stepping. A GPU
    /// draws the asked rate; the measured software box drew 11-12.
    /// </summary>
    public const double SlowRate = 20;

    /// <summary>
    /// Three seconds, so the launch (first loads, JIT) and a step change's
    /// one-off work are not taken for a slow machine.
    /// </summary>
    public const int SustainedSeconds = 3;

    /// <summary>A gap longer than this between two frames is a stopped clock, not a slow one.</summary>
    public const double StopGap = 1;

    /// <summary>The DXGI adapter flag for a software adapter.</summary>
    public const uint SoftwareFlag = 2;

    private double? _secondStarted;
    private double _last = double.NegativeInfinity;
    private int _frames;
    private int _slowSeconds;

    /// <param name="software">Whether the stage draws with software rendering (<see cref="IsSoftwareAdapter"/>).</param>
    public StageBackdropBudget(bool software)
    {
        Level = software ? StageBackdrop.NoBreath : StageBackdrop.Full;
    }

    /// <summary>The backdrop to draw.</summary>
    public StageBackdrop Level { get; private set; }

    /// <summary>
    /// One frame drawn at <paramref name="now"/> (seconds) while the stage
    /// asked for <paramref name="askedInterval"/> between frames
    /// (<see cref="FirstRunMotion.DisplayRate"/> is the display's rate).
    /// True when <see cref="Level"/> changed.
    /// </summary>
    public bool Drew(double now, double askedInterval)
    {
        bool judged = askedInterval <= FirstRunMotion.Thirty + 1e-9;
        bool stopped = now - _last > StopGap;
        _last = now;
        if (!judged || stopped)
        {
            _slowSeconds = 0;
            _secondStarted = judged ? now : null;
            _frames = 0;
            return false;
        }

        if (_secondStarted is not double started)
        {
            _secondStarted = now;
            _frames = 0;
            return false;
        }

        _frames++;
        double elapsed = now - started;
        if (elapsed < 1)
        {
            return false;
        }

        _slowSeconds = _frames / elapsed < SlowRate ? _slowSeconds + 1 : 0;
        _secondStarted = now;
        _frames = 0;
        if (_slowSeconds < SustainedSeconds || Level == StageBackdrop.NoGround)
        {
            return false;
        }

        // One layer at a time: the next is judged on fresh seconds at the new look.
        Level++;
        _slowSeconds = 0;
        return true;
    }

    /// <summary>The clock stopped (nothing moves, the window hid, motion went off): the count starts again.</summary>
    public void Pause()
    {
        _secondStarted = null;
        _frames = 0;
        _slowSeconds = 0;
        _last = double.NegativeInfinity;
    }

    /// <summary>
    /// Software rendering: the Microsoft Basic Render Driver (vendor 0x1414,
    /// device 0x8C, which WARP also reports) or any adapter DXGI flags as
    /// software.
    /// </summary>
    public static bool IsSoftwareAdapter(uint vendorId, uint deviceId, uint flags) =>
        (flags & SoftwareFlag) != 0 || (vendorId == 0x1414 && deviceId == 0x8C);
}
