namespace TraceCommons.App;

/// <summary>The About window (spec W24), kept by reference, never found by title.</summary>
public sealed partial class MainWindow
{
    private AboutWindow? _aboutWindow;

    private void OnTrayAboutRequested() => _host.Dispatcher.TryEnqueue(ShowAboutWindow);

    private void ShowAboutWindow()
    {
        if (_aboutWindow is null)
        {
            _aboutWindow = new AboutWindow();
            _aboutWindow.Closed += (_, _) => _aboutWindow = null;
        }

        _aboutWindow.ShowNearOwner(AppWindow);
    }
}
