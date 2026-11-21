using Microsoft.UI.Xaml;
using TraceCommons.App.Controls.Settings;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App;

/// <summary>
/// Settings > General's appearance on one window (W17-D1 (a)): the stored
/// choice when the window opens, and every later choice at once, as the
/// Mac sets <c>NSApplication.appearance</c> for every window of the app
/// (#2219 review, defect 4). First run keeps its dark stage.
/// </summary>
internal static class WindowAppearance
{
    public static void Follow(Window window, FrameworkElement root)
    {
        root.RequestedTheme = GeneralSection.Theme(AppearanceChoice.Shared.Current);
        void OnChanged(Appearance appearance) => root.RequestedTheme = GeneralSection.Theme(appearance);
        AppearanceChoice.Shared.Changed += OnChanged;
        window.Closed += (_, _) => AppearanceChoice.Shared.Changed -= OnChanged;

        // The main window draws its own title bar.
        if (window is not MainWindow)
        {
            WindowCaption.Follow(window, root);
        }
    }
}
