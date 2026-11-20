using TraceCommons.App.Controls.Settings;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// Settings > General's appearance on the Monitor window (W17-D1 (a)): the
/// stored choice at launch, and every later choice at once, as the Mac sets
/// <c>NSApplication.appearance</c>. A sample launch keeps choices for the
/// run only.
/// </summary>
public sealed partial class MainWindow
{
    private void FollowAppearance()
    {
        AppearanceChoice.Shared.ReadOnly = Program.IsSampleLaunch;
        WindowAppearance.Follow(this, WindowRoot);
    }
}
