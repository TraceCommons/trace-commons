using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.Controls.Settings;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls;

public sealed partial class SettingsView
{
    private void ApplyPrivateAISettingsGate()
    {
        if (PrivateAILedgerFeedHost.Content is PrivateAILedgerFeedCard card)
            card.SetGate(_gate == SettingsGate.Open);
    }

    private sealed class PrivateAILedgerFeedCard : UserControl
    {
        private readonly InsightsLedgerFeedSwitch _setting;
        private readonly ToggleSwitch _toggle;
        private readonly TextBlock _refusal;
        private bool _gateOpen;
        private bool _rendering;

        public PrivateAILedgerFeedCard(DaemonHost host)
        {
            _setting = new InsightsLedgerFeedSwitch((method, json) => host.CallAsync(method, json));
            string label = InsightsSettingsWords.Text("analytics_setting_ledger_feed");
            _toggle = new ToggleSwitch { Header = label, OnContent = string.Empty, OffContent = string.Empty };
            AutomationProperties.SetName(_toggle, label);
            _refusal = SettingsCards.Caption(string.Empty);
            var body = new StackPanel { Spacing = 8 };
            body.Children.Add(_toggle);
            body.Children.Add(_refusal);
            Content = SettingsCards.Card(string.Empty, body);
            Visibility = Visibility.Collapsed;
            _toggle.Toggled += OnToggled;
        }

        public async void SetGate(bool open)
        {
            if (open == _gateOpen) return;
            _gateOpen = open;
            if (open) await _setting.ReadAsync();
            Draw();
        }

        private void Draw()
        {
            Visibility = _gateOpen && _setting.On.HasValue && _toggle.Header is string { Length: > 0 }
                ? Visibility.Visible : Visibility.Collapsed;
            _rendering = true;
            _toggle.IsOn = _setting.On == true;
            _toggle.IsEnabled = _gateOpen && !_setting.Busy;
            _refusal.Text = _setting.Refusal ?? string.Empty;
            _refusal.Visibility = _refusal.Text.Length > 0 ? Visibility.Visible : Visibility.Collapsed;
            _rendering = false;
        }

        private async void OnToggled(object sender, RoutedEventArgs e)
        {
            if (_rendering || !_gateOpen || _setting.Busy || _setting.On is not bool on || _toggle.IsOn == on)
                return;

            bool wanted = _toggle.IsOn;
            var write = _setting.SetAsync(wanted);
            Draw();
            await write;
            Draw();
        }
    }
}
