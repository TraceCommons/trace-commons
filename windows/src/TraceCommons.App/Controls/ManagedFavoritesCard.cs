using System;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

internal sealed class ManagedFavoritesCard : UserControl
{
    private readonly DaemonHost _host;
    private readonly StackPanel _content = new() { Spacing = 6 };
    private ManagedSnapshot? _snapshot;
    private ManagedLaunchCatalogue? _catalogue;
    private string[] _favorites = [];
    private bool _busy;
    private bool _failed;

    internal ManagedFavoritesCard(DaemonHost host)
    {
        _host = host;
        Content = _content;
        Visibility = Visibility.Collapsed;
        Loaded += async (_, _) => await LoadAsync();
    }

    private string T(string key) => _snapshot?.Text(key) ?? "";

    private async Task LoadAsync()
    {
        if (_busy) return;
        _busy = true;
        try
        {
            var settings = await _host.CallAsync("get_settings");
            if (settings.Result is not { } value || !value.TryGetProperty("managed_favorite_models", out var stored))
                return;
            _favorites = stored.Deserialize<string[]>() ?? [];
            if (_favorites.Length == 0) return;
            var snapshot = await _host.CallAsync("managed_snapshot");
            if (snapshot.Result is { } managed) _snapshot = ManagedSnapshot.Parse(managed.GetRawText());
            try
            {
                var response = await _host.CallAsync("managed_model_catalogue", """{"tool":"claude"}""");
                if (response.Result is { } catalogue) _catalogue = ManagedLaunchCatalogue.Parse(catalogue);
            }
            catch (Exception) { _catalogue = null; }
        }
        catch (Exception) { }
        finally { _busy = false; Render(); }
    }

    private void Render()
    {
        Visibility = _snapshot is not null && _favorites.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
        _content.Children.Clear();
        if (Visibility != Visibility.Visible) return;
        _content.Children.Add(new TextBlock { Text = T("favorites_title"),
            Style = (Style)Application.Current.Resources["TcTitleCardTextStyle"] });
        foreach (var (id, unavailable) in ManagedLaunchCatalogue.FavoriteRows(_favorites, _catalogue))
        {
            var row = new Grid { ColumnSpacing = 8, ColumnDefinitions = { new(), new() { Width = GridLength.Auto } } };
            var label = unavailable ? T("model_favorite_unavailable").Replace("{model}", id) : id;
            row.Children.Add(new TextBlock { Text = label, TextWrapping = TextWrapping.Wrap, VerticalAlignment = VerticalAlignment.Center });
            var remove = new Button { Content = T("remove"), IsEnabled = !_busy };
            AutomationProperties.SetName(remove, T("model_favorite_remove_named").Replace("{model}", id));
            remove.Click += async (_, _) => await RemoveAsync(id);
            Grid.SetColumn(remove, 1);
            row.Children.Add(remove);
            _content.Children.Add(row);
        }
        if (_failed) _content.Children.Add(new TextBlock { Text = T("action_failed"), TextWrapping = TextWrapping.Wrap });
    }

    private async Task RemoveAsync(string id)
    {
        if (_busy) return;
        _busy = true;
        _failed = false;
        try
        {
            var response = await _host.CallAsync("managed_model_favorite_set", JsonSerializer.Serialize(new { model = id, favorite = false }));
            if (response.Result is { } result && result.TryGetProperty("favorites", out var favorites))
                _favorites = favorites.Deserialize<string[]>() ?? [];
            else _failed = true;
        }
        catch (Exception) { _failed = true; }
        finally { _busy = false; Render(); }
    }
}
