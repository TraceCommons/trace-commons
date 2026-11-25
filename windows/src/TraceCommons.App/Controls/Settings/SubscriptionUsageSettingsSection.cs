using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls.Settings;

/// <summary>Local permission to read Codex and Claude subscription usage.</summary>
internal sealed class SubscriptionUsageSettingsSection : UserControl
{
    private readonly SubscriptionUsageModel _model;

    internal SubscriptionUsageSettingsSection(SubscriptionUsageModel model)
    {
        IsTabStop = false;
        _model = model;
        Loaded += async (_, _) =>
        {
            _model.Changed += OnChanged;
            Draw();
            await _model.LoadAsync();
        };
        Unloaded += (_, _) => _model.Changed -= OnChanged;
    }

    private void OnChanged() => DispatcherQueue.TryEnqueue(() =>
    {
        if (IsLoaded) Draw();
    });

    private void Draw()
    {
        bool? codex = _model.CodexEnabled;
        bool? claude = _model.ClaudeEnabled;
        if (!_model.CanChange || (codex is null && claude is null)
            || string.IsNullOrEmpty(InsightsSettingsWords.Text("analytics_subscriptions_setting_codex")))
        {
            Content = null;
            return;
        }

        var body = new StackPanel { Spacing = 8 };
        if (codex is bool codexEnabled)
        {
            AddPermission(body, "codex", codexEnabled);
        }

        if (claude is bool claudeEnabled)
        {
            AddPermission(body, "claude", claudeEnabled);
        }
        if (_model.FailureKey is { } key)
        {
            body.Children.Add(SettingsCards.Caption(InsightsSettingsWords.Text(key)));
        }

        string? focus = SettingsCards.FocusedKey(this);
        SettingsCards.Replace(this, SettingsCards.Card(InsightsSettingsWords.Text("analytics_subscriptions_title"), body), focus);
    }

    private void AddPermission(StackPanel body, string provider, bool enabled)
    {
        body.Children.Add(SettingsCards.Caption(InsightsSettingsWords.Text($"analytics_subscriptions_{provider}_detail")));
        var toggle = SettingsCards.Key(new ToggleSwitch
        {
            Header = InsightsSettingsWords.Text($"analytics_subscriptions_setting_{provider}"),
            IsOn = enabled,
            IsEnabled = _model.ChangingProviders.Count == 0,
            OnContent = string.Empty,
            OffContent = string.Empty,
        }, $"insights/{provider}");
        toggle.Toggled += async (_, _) =>
        {
            bool? current = provider == "codex" ? _model.CodexEnabled : _model.ClaudeEnabled;
            if (toggle.IsOn != current && _model.ChangingProviders.Count == 0)
            {
                await _model.SetEnabledAsync(provider, toggle.IsOn);
            }
        };
        body.Children.Add(toggle);
    }
}
