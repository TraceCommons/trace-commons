using System;
using Microsoft.UI.Xaml;
using TraceCommons.App.Controls.Settings;
using TraceCommons.Interop;
using Windows.Storage.Pickers;

namespace TraceCommons.App.Controls;

public sealed partial class SettingsView
{
    public string ChooseRoutingFolderText => SourceChecks.ReadSettingsCopy()?.ChooseFolder ?? string.Empty;

    private async void OnChooseRoutingFolder(object sender, RoutedEventArgs e)
    {
        if (SettingsOwnerHandle == 0 || !Settings.RoutingControlsEnabled) return;
        var picker = new FolderPicker();
        picker.FileTypeFilter.Add("*");
        WinRT.Interop.InitializeWithWindow.Initialize(picker, SettingsOwnerHandle);
        if (await picker.PickSingleFolderAsync() is { } folder && IsLoaded && Settings.RoutingControlsEnabled)
        {
            Settings.RoutingTokenDir = folder.Path;
        }
    }

    private void DrawProfileReadNotice()
    {
        ProfileReadHost.Visibility = ViewModel.ShowsReadNotice ? Visibility.Visible : Visibility.Collapsed;
        ProfileReadHost.Content = ViewModel.ShowsReadNotice
            ? SettingsCards.ReadNotice(ViewModel.ReadFailed, async (_, _) => await ViewModel.LoadAsync())
            : null;
    }

    private void DrawConsentReadNotice()
    {
        ConsentReadHost.Visibility = Settings.ConsentReadAnswered ? Visibility.Collapsed : Visibility.Visible;
        ConsentReadHost.Content = Settings.ConsentReadAnswered
            ? null
            : SettingsCards.ReadNotice(Settings.ConsentReadFailed, async (_, _) => await Settings.ReloadConsentAsync());

        string? failure = Settings.ConsentSaveFailed ? SourceChecks.ReadSettingsCopy()?.ConsentSaveFailed : null;
        DrawFailure(ConsentSaveHost, failure);
        DrawFailure(AttributionSaveHost, failure);
    }

    private static void DrawFailure(Microsoft.UI.Xaml.Controls.ContentControl host, string? failure)
    {
        host.Visibility = string.IsNullOrEmpty(failure) ? Visibility.Collapsed : Visibility.Visible;
        host.Content = string.IsNullOrEmpty(failure) ? null : SettingsCards.Caption(failure);
    }
}
