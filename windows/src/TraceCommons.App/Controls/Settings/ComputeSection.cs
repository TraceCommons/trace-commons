using System;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Settings > Compute (W18-D1 (a); <c>ComputeView</c>, ComputeView.swift):
/// the core's introduction, the state card, the allowance (a field while
/// it can be enabled, else its value or the unknown word, never 0), and
/// Enable, or Resume, Pause and Disable once consent is granted. A failure
/// is said in the core's words with Retry; nothing reads as working before
/// the controller answered. The controller is the app's own and opened once
/// against this app's state directory; a sample launch opens nothing.
/// </summary>
public sealed class ComputeSection : UserControl
{
    private readonly ComputeModel _model;
    private readonly StackPanel _root = new() { Spacing = 12 };
    private readonly TextBox _allowance = new();
    private readonly DispatcherTimer _clock = new() { Interval = TimeSpan.FromSeconds(1) };
    private readonly string _configDir;
    private readonly bool _sample;
    private Button? _enable;
    private bool _enableAllowed;
    private object? _drawn;

    public ComputeSection(DaemonHost host, ComputeModel model)
    {
        _model = model;
        _configDir = host.ConfigDir;
        _sample = host.IsSample;
        _model.Changed += Render;
        // Typing only arms Enable: rebuilding would take the field's focus.
        _allowance.TextChanged += (_, _) => ArmEnable();
        Content = _root;

        // The Mac reads the controller each second while its section is up;
        // here the clock also stops while the window is hidden or minimised.
        _clock.Tick += async (_, _) => await _model.RefreshAsync();
        Loaded += async (_, _) =>
        {
            WindowClocks.Changed -= OnClocks;
            WindowClocks.Changed += OnClocks;
            if (_sample)
            {
                // Sample data writes nothing to this machine.
                _model.Refuse("compute-sample");
            }
            else
            {
                await _model.StartAsync(_configDir);
            }

            if (WindowClocks.Running)
            {
                _clock.Start();
            }
        };
        Unloaded += (_, _) =>
        {
            WindowClocks.Changed -= OnClocks;
            _clock.Stop();
        };
        Render();
    }

    private void OnClocks(bool running)
    {
        if (running && IsLoaded)
        {
            _clock.Start();
        }
        else
        {
            _clock.Stop();
        }
    }

    private void ArmEnable()
    {
        if (_enable is not null)
        {
            _enable.IsEnabled = _enableAllowed && ComputeAllowance.Parse(_allowance.Text) is not null;
        }
    }

    private void Render()
    {
        // The clock reads the controller each second: redraw only what
        // changed, so the field keeps its focus and its text (checklist O2).
        object drawn = (_model.Snapshot, _model.FailureLabel, _model.ControlsBusy);
        if (Equals(drawn, _drawn))
        {
            return;
        }

        _drawn = drawn;
        _enable = null;
        _root.Children.Clear();
        // The field is kept across redraws: let go of its old parent first.
        if (_allowance.Parent is Panel holder)
        {
            holder.Children.Remove(_allowance);
        }
        if (_model.Snapshot is { } snapshot)
        {
            _root.Children.Add(SettingsCards.Caption(snapshot.Copy.Introduction));
            var state = new StackPanel { Spacing = 4 };
            state.Children.Add(SettingsCards.Text(snapshot.Title, "TcTitleCardTextStyle"));
            if (snapshot.Detail.Length > 0)
            {
                state.Children.Add(SettingsCards.Caption(snapshot.Detail));
            }

            _root.Children.Add(SettingsCards.Card(string.Empty, state));

            var allowance = new StackPanel { Spacing = 6 };
            if (snapshot.CanEnable)
            {
                _allowance.Header = snapshot.Copy.AllowanceLabel;
                _allowance.IsEnabled = !_model.ControlsBusy;
                if (_allowance.Text.Length == 0 && snapshot.RamAllowanceGib is { } typed)
                {
                    _allowance.Text = typed.ToString(System.Globalization.CultureInfo.InvariantCulture);
                }

                allowance.Children.Add(_allowance);
            }
            else
            {
                allowance.Children.Add(SettingsCards.Text(snapshot.Copy.AllowanceLabel, "TcEyebrowTextStyle"));
                allowance.Children.Add(snapshot.RamAllowanceGib is { } value
                    ? SettingsCards.Text(value.ToString(System.Globalization.CultureInfo.InvariantCulture), "TcMetricValueTextStyle")
                    : SettingsCards.Caption(UnknownWord()));
            }

            allowance.Children.Add(SettingsCards.Caption(snapshot.Copy.AllowanceDetail));
            _root.Children.Add(SettingsCards.Card(string.Empty, allowance));

            var buttons = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
            bool busy = _model.ControlsBusy;
            if (snapshot.ConsentGranted)
            {
                buttons.Children.Add(Command(snapshot.Copy.Resume, new ComputeCommand.Resume(), !busy && snapshot.Available && snapshot.CanResume));
                buttons.Children.Add(Command(snapshot.Copy.Pause, new ComputeCommand.Pause(), !busy && snapshot.CanPause));
                buttons.Children.Add(Command(snapshot.Copy.Disable, new ComputeCommand.Disable(), !busy));
            }
            else
            {
                Button enable = SettingsCards.Button(snapshot.Copy.Enable, async (_, _) =>
                {
                    if (ComputeAllowance.Parse(_allowance.Text) is { } gib)
                    {
                        await _model.PerformAsync(new ComputeCommand.Enable(gib));
                    }
                });
                _enable = enable;
                _enableAllowed = !busy && snapshot.Available && snapshot.CanEnable;
                ArmEnable();
                buttons.Children.Add(enable);
            }

            if (busy)
            {
                buttons.Children.Add(new ProgressRing { IsActive = true, Width = 14, Height = 14 });
            }

            _root.Children.Add(buttons);
        }
        else if (_model.FailureLabel is not null)
        {
            // Failed: the core's line, else its unknown word; never a spinner.
            if (_model.Copy?.Retry is { } retry && !_sample)
            {
                Button again = SettingsCards.Button(retry, async (_, _) => await _model.RetryAsync());
                again.IsEnabled = !_model.ControlsBusy;
                _root.Children.Add(again);
            }

            _root.Children.Add(SettingsCards.Caption(
                string.IsNullOrEmpty(_model.Copy?.Unavailable) ? UnknownWord() : _model.Copy!.Unavailable!));
        }
        else
        {
            // Not answered yet: a spinner, never a control that reads as working.
            _root.Children.Add(new ProgressRing { IsActive = true, Width = 16, Height = 16, HorizontalAlignment = HorizontalAlignment.Left });
        }
    }

    private Button Command(string label, ComputeCommand command, bool enabled)
    {
        Button button = SettingsCards.Button(label, async (_, _) => await _model.PerformAsync(command));
        button.IsEnabled = enabled;
        return button;
    }

    /// <summary>The core's word for an answer it does not have, else a dash.</summary>
    private static string UnknownWord()
    {
        string unknown = MonitorShellCopy.Top("unknown");
        return unknown.Length > 0 ? unknown : SettingsListRow.Placeholder;
    }
}
