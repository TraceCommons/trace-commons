using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

public sealed partial class ManagedSessionsView
{
    private async Task<ManagedLaunchCatalogue?> ReadCatalogueAsync(string tool, bool cacheOnly = false)
    {
        try
        {
            var result = await CallAsync("managed_model_catalogue", new { tool, cache_only = cacheOnly });
            return ManagedLaunchCatalogue.Parse(result);
        }
        catch (Exception) { return null; }
    }

    private async Task LoadSessionModelNamesAsync(ManagedSnapshot snapshot)
    {
        var tools = snapshot.Sessions.Where(session => session.Model is not null).Select(session => session.Tool)
            .Distinct().Where(tool => !_modelCatalogues.ContainsKey(tool) && _modelCatalogueReads.Add(tool)).ToArray();
        var results = await Task.WhenAll(tools.Select(async tool => (Tool: tool, Catalogue: await ReadCatalogueAsync(tool, cacheOnly: true))));
        bool updated = false;
        foreach (var (tool, catalogue) in results)
        {
            if (catalogue is not { Models.Length: > 0 } || !_modelCatalogues.TryAdd(tool, catalogue)) continue;
            updated = true;
        }
        if (updated && IsLoaded && !_busy) Render();
    }

    private sealed class LaunchFields(ManagedSessionsView view, ManagedCapabilities capabilities)
    {
        internal event Action? Changed;
        internal StackPanel Panel { get; } = new() { Spacing = 8 };
        internal bool Ready { get; private set; }
        internal ManagedLaunchOptions? Choices => Ready ? _readChoices?.Invoke() : null;

        private static readonly JsonSerializerOptions Wire = new() { PropertyNameCaseInsensitive = true };
        private Func<ManagedLaunchOptions>? _readChoices;
        private int _load;
        private string T(string key) => view.T(key);

        internal void Close() => ++_load;

        internal async Task LoadAsync(string tool, string connection, string? cwd, bool reset = false, bool waitForTyping = false)
        {
            int load = ++_load;
            Ready = false;
            _readChoices = null;
            Panel.Children.Clear();
            Changed?.Invoke();
            if (!capabilities.FolderSettings || tool == "red_team")
            {
                if (tool == "opencode")
                    ShowFields(tool, connection, cwd, new(tool, null, false, false, "", null, false), null);
                else
                {
                    Ready = true;
                    Changed?.Invoke();
                }
                return;
            }

            Panel.Children.Add(SessionCaption(T("folder_settings_loading")));
            if (waitForTyping) await Task.Delay(250);
            if (load != _load) return;
            var catalogue = ManagedLaunchOptions.ModelMode(tool, connection) == ManagedModelMode.Catalogue
                ? view.ReadCatalogueAsync(tool) : Task.FromResult<ManagedLaunchCatalogue?>(null);
            ManagedFolderPreferences? folder;
            try
            {
                var method = reset ? "managed_folder_settings_reset" : "managed_folder_settings_get";
                folder = (await view.CallAsync(method, new { tool, cwd })).Deserialize<ManagedFolderPreferences>(Wire);
            }
            catch (Exception) { folder = null; }
            var models = await catalogue;
            if (load != _load) return;
            if (models is { Models.Length: > 0 }) view._modelCatalogues[tool] = models;
            Panel.Children.Clear();
            if (folder?.Tool != tool)
            {
                Panel.Children.Add(SessionCaption(T("folder_settings_failed")));
                var retry = new Button { Content = T("retry") };
                retry.Click += async (_, _) => await LoadAsync(tool, connection, cwd);
                Panel.Children.Add(retry);
                return;
            }
            ShowFields(tool, connection, cwd, folder, models);
        }

        private void ShowFields(string tool, string connection, string? cwd,
            ManagedFolderPreferences folder, ManagedLaunchCatalogue? catalogue)
        {
            var mode = ManagedLaunchOptions.ModelMode(tool, connection);
            var model = new ComboBox { Header = T("model_choice"), HorizontalAlignment = HorizontalAlignment.Stretch };
            var modelText = new TextBox { Header = T("model"), Text = folder.Model ?? "" };
            var cli = new TextBox { Header = T("cli_options_label"), Text = folder.CliOptions ?? "" };
            var background = new ComboBox { Header = T("background_model_title"), HorizontalAlignment = HorizontalAlignment.Stretch };
            var trace = new ToggleSwitch { Header = T(capabilities.FolderSettings ? "collect_managed_trace" : "collect_traces"), IsOn = folder.CollectTraces };
            var unavailable = SessionCaption(T("folder_model_unavailable"));
            bool keepSaved = mode == ManagedModelMode.Catalogue && catalogue is not null &&
                folder.Model is string saved && !catalogue.Models.Any(item => item.Id == saved);
            string? backgroundId = folder.BackgroundModel;
            int backgroundRead = 0;
            int load = _load;

            if (mode == ManagedModelMode.Catalogue)
            {
                var favorites = new StackPanel { Spacing = 4 };
                var star = new Button { VerticalAlignment = VerticalAlignment.Bottom };
                var starRow = new Grid { ColumnSpacing = 8, ColumnDefinitions = { new(), new() { Width = GridLength.Auto } } };
                Grid.SetColumn(star, 1);
                starRow.Children.Add(model);
                starRow.Children.Add(star);
                FillModels(model, catalogue?.OrderedModels() ?? [], catalogue?.Default, keepSaved ? null : folder.Model);
                Panel.Children.Add(starRow);
                if (keepSaved) Panel.Children.Add(unavailable);
                bool offersBackground = capabilities.BackgroundModel && ManagedLaunchOptions.OffersBackground(tool, connection);
                if (offersBackground)
                {
                    AutomationProperties.SetName(background, T("background_model_choice"));
                    ToolTipService.SetToolTip(background, T("background_model_info"));
                    Panel.Children.Add(background);
                    cli.TextChanged += async (_, _) => await UpdateBackgroundAsync();
                    _ = UpdateBackgroundAsync();
                }
                Panel.Children.Add(favorites);
                var favoriteError = SessionCaption("");
                favoriteError.Visibility = Visibility.Collapsed;
                Panel.Children.Add(favoriteError);
                bool filling = false, saving = false;
                model.SelectionChanged += async (_, _) =>
                {
                    if (filling) return;
                    keepSaved = false;
                    unavailable.Visibility = Visibility.Collapsed;
                    favoriteError.Visibility = Visibility.Collapsed;
                    RenderFavorites();
                    if (offersBackground) await UpdateBackgroundAsync();
                };
                star.Click += async (_, _) =>
                {
                    if (ModelId(model) is { } selected)
                        await SetFavoriteAsync(selected, catalogue?.Favorites?.Contains(selected) != true);
                };
                RenderFavorites();
                if (tool == "claude" && folder.ProjectWorkflows) Panel.Children.Add(SessionCaption(T("project_workflows")));

                void RenderFavorites()
                {
                    string? selected = ModelId(model);
                    bool starred = selected is not null && catalogue?.Favorites?.Contains(selected) == true;
                    string label = T(starred ? "model_favorite_remove" : "model_favorite_add");
                    star.Content = new FontIcon { Glyph = starred ? "\uE735" : "\uE734", FontSize = 16 };
                    AutomationProperties.SetName(star, label);
                    ToolTipService.SetToolTip(star, label);
                    star.Visibility = selected is not null && catalogue?.Models.Any(item => item.Id == selected) == true
                        ? Visibility.Visible : Visibility.Collapsed;
                    star.IsEnabled = !saving;
                    favorites.Children.Clear();
                    foreach (string stale in catalogue?.UnavailableFavorites ?? [])
                    {
                        var row = new Grid { ColumnSpacing = 8, ColumnDefinitions = { new(), new() { Width = GridLength.Auto } } };
                        row.Children.Add(SessionCaption(T("model_favorite_unavailable").Replace("{model}", stale)));
                        var remove = new Button { Content = T("remove"), IsEnabled = !saving };
                        AutomationProperties.SetName(remove, T("model_favorite_remove_named").Replace("{model}", stale));
                        remove.Click += async (_, _) => await SetFavoriteAsync(stale, false);
                        Grid.SetColumn(remove, 1);
                        row.Children.Add(remove);
                        favorites.Children.Add(row);
                    }
                }

                async Task SetFavoriteAsync(string id, bool favorite)
                {
                    if (saving || catalogue is null) return;
                    saving = true;
                    favoriteError.Visibility = Visibility.Collapsed;
                    RenderFavorites();
                    try
                    {
                        var result = await view.CallAsync("managed_model_favorite_set", new { model = id, favorite });
                        if (_load != load) return;
                        var stored = result.GetProperty("favorites").Deserialize<string[]>() ?? [];
                        catalogue = catalogue with
                        {
                            Favorites = stored,
                            UnavailableFavorites = catalogue.UnavailableFavorites?.Where(stored.Contains).ToArray(),
                        };
                        var selection = ModelId(model);
                        filling = true;
                        try { FillModels(model, catalogue.OrderedModels(), catalogue.Default, selection); }
                        finally { filling = false; }
                    }
                    catch (Exception error)
                    {
                        favoriteError.Text = T(error is Refused { Code: "managed-model-unavailable" } ? "model_unavailable" : "action_failed");
                        favoriteError.Visibility = Visibility.Visible;
                    }
                    finally
                    {
                        saving = false;
                        if (_load == load) RenderFavorites();
                    }
                }
            }
            else if (mode == ManagedModelMode.Text) Panel.Children.Add(modelText);
            if (capabilities.FolderSettings)
            {
                ToolTipService.SetToolTip(cli, T("cli_options_tooltip"));
                Panel.Children.Add(cli);
            }
            Panel.Children.Add(trace);
            Panel.Children.Add(SessionCaption(T(capabilities.FolderSettings ? "managed_collection_scope" : "collection_scope")));
            if (capabilities.FolderSettings)
            {
                Panel.Children.Add(SessionCaption(T("folder_settings_scope")));
                if (folder.Remembered)
                {
                    var reset = new Button { Content = T("folder_settings_reset") };
                    AutomationProperties.SetName(reset, T("folder_settings_reset_accessibility").Replace("{tool}", T(tool)));
                    reset.Click += async (_, _) => await LoadAsync(tool, connection, cwd, reset: true);
                    Panel.Children.Add(reset);
                }
            }
            _readChoices = () => new(mode == ManagedModelMode.Text ? modelText.Text : ModelId(model), trace.IsOn,
                cli.Text, keepSaved, background.IsEnabled ? ModelId(background) : backgroundId);
            Ready = true;
            modelText.TextChanged += (_, _) => Changed?.Invoke();
            Changed?.Invoke();

            async Task UpdateBackgroundAsync()
            {
                int read = ++backgroundRead;
                if (background.IsEnabled && background.Items.Count > 0) backgroundId = ModelId(background);
                background.IsEnabled = false;
                var main = ModelId(model) ?? catalogue?.Default;
                var options = cli.Text;
                if (catalogue is not null && !string.IsNullOrWhiteSpace(options))
                {
                    await Task.Delay(250);
                    if (_load != load || read != backgroundRead) return;
                    try
                    {
                        var result = await view.CallAsync("managed_cli_model", new { tool, cli_options = options });
                        var named = result.GetProperty("model").GetString();
                        if (catalogue.Models.Any(item => item.Id == named)) main = named;
                    }
                    catch (Exception) { }
                }
                if (_load != load || read != backgroundRead) return;
                if (main is not null && catalogue?.Background?.GetValueOrDefault(main) is { } entry
                    && catalogue.BackgroundSets is { } sets && entry.Set >= 0 && entry.Set < sets.Length)
                {
                    var models = catalogue.Models.Concat(catalogue.BackgroundModels ?? []).ToLookup(item => item.Id);
                    var allowed = sets[entry.Set];
                    if (!allowed.Contains(backgroundId)) backgroundId = null;
                    FillModels(background, allowed.Select(id => models[id].FirstOrDefault() ?? new(id, null, null)),
                        entry.Default, backgroundId);
                }
                else
                {
                    if (catalogue is not null) backgroundId = null;
                    FillModels(background, [], null, backgroundId);
                }
                background.IsEnabled = background.Items.Count > 1;
            }
        }

        private static string? ModelId(ComboBox picker) => (picker.SelectedItem as ComboBoxItem)?.Tag as string;

        private void FillModels(ComboBox picker, IEnumerable<ManagedCatalogueModel> models,
            string? defaultId, string? selected)
        {
            var values = models.ToArray();
            picker.Items.Clear();
            picker.Items.Add(new ComboBoxItem { Content = defaultId is null ? T("model_default_unknown")
                : T("model_default").Replace("{model}", values.FirstOrDefault(item => item.Id == defaultId)?.Label ?? defaultId) });
            if (selected is not null && !values.Any(item => item.Id == selected))
                values = [.. values, new(selected, null, null)];
            foreach (var item in values)
            {
                string tier = item.Tier is null ? "" : T("model_group_" + item.Tier);
                var choice = new ComboBoxItem { Tag = item.Id, Content = tier.Length == 0 ? item.Label : $"{item.Label} · {tier}" };
                ToolTipService.SetToolTip(choice, item.Id);
                picker.Items.Add(choice);
            }
            picker.SelectedItem = picker.Items.Cast<ComboBoxItem>().FirstOrDefault(item => Equals(item.Tag, selected)) ?? picker.Items[0];
        }
    }
}
