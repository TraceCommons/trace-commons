using System;
using System.Collections.Generic;
using System.Globalization;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls.Settings;

/// <summary>
/// Settings > Referrals (W16-D2 (a); <c>ReferralsSection</c>,
/// ReferralsSection.swift): the person's codes, with people joined and the
/// credit, Copy and Turn off per code, Create while the limit allows, and,
/// while the account may still take one, the field to enter someone else's
/// code. An ended session is said as the action alone. Every sentence is
/// the core's.
/// </summary>
public sealed class ReferralsSection : UserControl
{
    private readonly ReferralFlow _flow;
    private readonly StackPanel _root = new() { Spacing = 12 };
    private readonly TextBox _field = new();
    private readonly DispatcherTimer _copiedClear = new() { Interval = TimeSpan.FromSeconds(2) };
    private Button? _apply;
    private TextBlock? _problem;

    public ReferralsSection(DaemonHost host)
    {
        // A container, not a stop: Tab goes to the controls inside it.
        IsTabStop = false;
        ReferralCopy? copy = ReferralCopy.Live;
        _flow = new ReferralFlow(
            new ReferralClient((method, json) => host.CallAsync(method, json)),
            copy is null ? null : new ReferralSurface(copy));
        _flow.Changed += Render;
        _field.TextChanged += (_, _) =>
        {
            _flow.Draft = _field.Text;
            ArmApply();
        };
        _field.KeyDown += async (_, e) =>
        {
            if (e.Key == Windows.System.VirtualKey.Enter)
            {
                e.Handled = true;
                await _flow.ApplyAsync();
            }
        };
        _copiedClear.Tick += (_, _) =>
        {
            _copiedClear.Stop();
            if (_flow.CopiedCodeId is { } id)
            {
                _flow.ClearCopied(id);
            }
        };
        Unloaded += (_, _) => _copiedClear.Stop();
        Content = _root;
        Loaded += async (_, _) => await _flow.RefreshAsync();
    }

    /// <summary>Reads the codes again, after a change of account.</summary>
    public System.Threading.Tasks.Task RefreshAsync() => _flow.RefreshAsync();

    private void ArmApply()
    {
        if (_apply is not null)
        {
            _apply.IsEnabled = _flow.CanApply;
        }

        if (_problem is not null)
        {
            string? line = _flow.FieldProblem;
            _problem.Text = line ?? string.Empty;
            _problem.Visibility = line is null ? Visibility.Collapsed : Visibility.Visible;
        }
    }

    private void Render()
    {
        if (_field.Parent is Panel holder)
        {
            holder.Children.Remove(_field);
        }

        _root.Children.Clear();
        _apply = null;
        _problem = null;
        if (_flow.Surface is not { } surface)
        {
            return;
        }

        if (!_flow.Answered)
        {
            // Not answered yet: a spinner, or the failure line with Retry,
            // never a blank body under the rule (ReferralsSection.swift).
            _root.Children.Add(SettingsCards.ReadNotice(_flow.Failed, async (_, _) => await _flow.RefreshAsync()));
            return;
        }

        if (_flow.SessionEnded)
        {
            _root.Children.Add(SettingsCards.Card(
                surface.Copy["title"],
                SettingsCards.Button(surface.Copy["session_ended"], async (_, _) => await _flow.RefreshAsync())));
            return;
        }

        _root.Children.Add(CodesCard(surface));
        if (ReferralSurface.ShowsEnterField(_flow.Status) || _flow.Status?.Applied == true)
        {
            _root.Children.Add(EnterCard(surface));
        }
    }

    private UIElement CodesCard(ReferralSurface surface)
    {
        var body = new StackPanel { Spacing = 8 };
        body.Children.Add(SettingsCards.Caption(surface.Copy["body"]));
        if (_flow.Listing is { } listing)
        {
            body.Children.Add(Pair(surface.Copy["people_joined"], listing.PeopleJoined.ToString(CultureInfo.CurrentCulture)));
            if (surface.CreditLine(listing, ReferralMoney.Format) is { } credit)
            {
                body.Children.Add(Pair(surface.Copy["credit"], credit));
            }

            foreach (ReferralCodeRow row in listing.Codes)
            {
                body.Children.Add(CodeRow(surface, row));
            }

            if (ReferralSurface.ShowsDisableScope(listing))
            {
                body.Children.Add(SettingsCards.Caption(surface.Copy["disable_scope"]));
            }

            if (surface.LimitLine(listing) is { } limit)
            {
                body.Children.Add(SettingsCards.Caption(limit));
            }
            else
            {
                Button create = SettingsCards.Button(
                    surface.Copy["create_button"], async (_, _) => await _flow.CreateAsync(), surface.Copy["create_accessibility_name"]);
                create.IsEnabled = !_flow.Busy;
                body.Children.Add(create);
            }
        }

        if (_flow.Refusal is { } refusal && refusal is not (ReferralRefusal.NotApplied or ReferralRefusal.InvalidCode))
        {
            body.Children.Add(SettingsCards.Caption(surface.RefusalLine(refusal, _flow.Listing?.MaxLiveCodes)));
        }

        return SettingsCards.Card(surface.Copy["title"], body);
    }

    private static UIElement Pair(string label, string value)
    {
        var grid = new Grid { ColumnSpacing = 8 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.Children.Add(SettingsCards.Caption(label));
        TextBlock shown = SettingsCards.Text(value);
        Grid.SetColumn(shown, 1);
        grid.Children.Add(shown);
        return grid;
    }

    private UIElement CodeRow(ReferralSurface surface, ReferralCodeRow row)
    {
        var grid = new Grid { ColumnSpacing = 8 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        var left = new StackPanel { Spacing = 2 };
        var title = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6 };
        TextBlock code = SettingsCards.Text(surface.ShownCode(row), row.Code is null ? "TcCaptionTextStyle" : "TcMonoCodeTextStyle");
        code.IsTextSelectionEnabled = true;
        title.Children.Add(code);
        if (surface.StateLabel(row) is { } state)
        {
            title.Children.Add(SettingsCards.Text(state, "TcMonoChipTextStyle"));
        }

        left.Children.Add(title);
        left.Children.Add(SettingsCards.Caption(surface.JoinedLine(row.RefereeCount)));
        grid.Children.Add(left);

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 6, VerticalAlignment = VerticalAlignment.Top };
        if (ReferralSurface.CanCopy(row) && row.Code is { } shown)
        {
            string label = _flow.CopiedCodeId == row.CodeId ? surface.Copy["copied"] : surface.Copy["copy_button"];
            actions.Children.Add(SettingsCards.Button(label, (_, _) =>
            {
                // A clipboard another process holds open is "not copied": no
                // "Copied" confirmation, and no exception into the click.
                if (!ClipboardCopy.TryCopy(shown))
                {
                    return;
                }

                _flow.Copied(row);
                _copiedClear.Stop();
                _copiedClear.Start();
            }, surface.CopyAccessibilityName(row)));
        }

        if (ReferralSurface.CanDisable(row))
        {
            Button off = SettingsCards.Button(
                surface.Copy["disable_button"], async (_, _) => await _flow.DisableAsync(row), surface.DisableAccessibilityName(row));
            off.IsEnabled = !_flow.Busy;
            actions.Children.Add(off);
        }

        Grid.SetColumn(actions, 1);
        grid.Children.Add(actions);
        return grid;
    }

    private UIElement EnterCard(ReferralSurface surface)
    {
        var body = new StackPanel { Spacing = 8 };
        if (_flow.Status?.Applied == true)
        {
            body.Children.Add(SettingsCards.Text(surface.Copy["applied_status"]));
            return SettingsCards.Card(surface.Copy["enter_title"], body);
        }

        if (_flow.Status?.ApplyUntil is { } until && Date(until) is { } date)
        {
            body.Children.Add(SettingsCards.Caption(surface.EnterScope(date)));
        }

        _field.Header = surface.Copy["field_label"];
        _field.PlaceholderText = surface.Copy["field_placeholder"];
        if (_field.Text != _flow.Draft)
        {
            _field.Text = _flow.Draft;
        }

        _apply = SettingsCards.Button(
            surface.Copy["apply_button"], async (_, _) => await _flow.ApplyAsync(), surface.Copy["apply_accessibility_name"]);
        _apply.VerticalAlignment = VerticalAlignment.Bottom;
        var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 8 };
        row.Children.Add(_field);
        row.Children.Add(_apply);
        body.Children.Add(row);

        _problem = SettingsCards.Caption(string.Empty);
        body.Children.Add(_problem);
        if (_flow.FieldProblem is null && _flow.Refusal is ReferralRefusal.NotApplied or ReferralRefusal.InvalidCode)
        {
            body.Children.Add(SettingsCards.Caption(surface.RefusalLine(_flow.Refusal.Value)));
        }

        ArmApply();
        return SettingsCards.Card(surface.Copy["enter_title"], body);
    }

    /// <summary>The server's RFC 3339 date as a short date in the system's words.</summary>
    private static string? Date(string text) =>
        DateTimeOffset.TryParse(text, CultureInfo.InvariantCulture, DateTimeStyles.RoundtripKind, out DateTimeOffset at)
            ? at.LocalDateTime.ToString("d", CultureInfo.CurrentCulture)
            : null;
}
