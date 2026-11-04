using System;
using System.Threading.Tasks;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// The three-step dialog for changing the NEAR AI login an account uses
/// (F10, spec section 2), the Windows half of macOS's
/// <c>AccountLoginChangeSheet</c>.
/// </summary>
/// <remarks>
/// <para>
/// One ContentDialog for the whole ceremony: <see cref="DialogGuard"/>
/// forbids a second dialog while one is open, so the steps redraw this one
/// rather than replace it. The primary button never closes it on its own
/// (<c>args.Cancel</c>); the driver closes it when the ceremony is done or
/// cancelled.
/// </para>
/// <para>
/// Every word is the core's, through <see cref="AccountLoginChangePage"/>:
/// nothing here writes copy. This class chooses type and spacing. Switch is
/// never the default button: it moves the account.
/// </para>
/// </remarks>
internal sealed class AccountLoginChangeDialog
{
    private readonly AccountLoginChangeFlow _flow;
    private readonly ContentDialog _dialog;
    private readonly TextBlock _heading = new() { TextWrapping = TextWrapping.Wrap, FontWeight = FontWeights.SemiBold };
    private readonly StackPanel _lines = new() { Spacing = 6 };
    private readonly TextBox _walletAccount = new();
    private readonly ProgressRing _working = new() { IsActive = false, Width = 20, Height = 20, HorizontalAlignment = HorizontalAlignment.Left };
    private readonly TextBlock _error = new() { TextWrapping = TextWrapping.Wrap };
    private bool _closingByFlow;

    private AccountLoginChangeDialog(XamlRoot root, AccountLoginChangeFlow flow, string? walletAccountLabel)
    {
        _flow = flow;
        _walletAccount.Header = walletAccountLabel;
        _walletAccount.TextChanged += (_, _) => _flow.WalletAccount = _walletAccount.Text;

        var panel = new StackPanel { Spacing = 10 };
        panel.Children.Add(_heading);
        panel.Children.Add(_lines);
        panel.Children.Add(_walletAccount);
        panel.Children.Add(_working);
        panel.Children.Add(_error);

        _dialog = new ContentDialog
        {
            XamlRoot = root,
            Content = panel,
        };
        _dialog.PrimaryButtonClick += OnPrimary;
        _dialog.Closing += OnClosing;
        _dialog.Opened += (_, _) => Draw();
        _flow.Changed += OnFlowChanged;
    }

    /// <summary>
    /// Shows the dialog for a ceremony the driver has begun, and returns once
    /// it closed, whichever way.
    /// </summary>
    public static async Task ShowAsync(XamlRoot root, AccountLoginChangeFlow flow, string? walletAccountLabel)
    {
        ArgumentNullException.ThrowIfNull(root);
        ArgumentNullException.ThrowIfNull(flow);
        if (!flow.Presented || flow.Page is null)
        {
            return;
        }

        var dialog = new AccountLoginChangeDialog(root, flow, walletAccountLabel);
        dialog.Draw();
        try
        {
            await DialogGuard.ShowOnceAsync(dialog._dialog);
        }
        finally
        {
            flow.Changed -= dialog.OnFlowChanged;
            if (flow.Presented)
            {
                // Closed without the driver (a dialog that could not be
                // shown): end the ceremony rather than leave it held.
                await flow.CancelAsync();
            }
        }
    }

    private void OnFlowChanged(object? sender, EventArgs e)
    {
        if (!_flow.Presented)
        {
            Hide();
            return;
        }

        if (_flow.Page is null)
        {
            // The words for this state cannot be drawn: close rather than
            // show a placeholder.
            _ = _flow.CancelAsync();
            Hide();
            return;
        }

        Draw();
    }

    private void Draw()
    {
        if (_flow.Page is not { } page)
        {
            return;
        }

        _dialog.Title = page.Title;
        _heading.Text = page.Heading ?? string.Empty;
        _heading.Visibility = page.Heading is null ? Visibility.Collapsed : Visibility.Visible;

        _lines.Children.Clear();
        foreach (string line in page.Lines)
        {
            _lines.Children.Add(new TextBlock { Text = line, TextWrapping = TextWrapping.Wrap });
        }

        // The NEAR account name, typed as at sign-up; its label is the
        // wallet join's own. Not drawn without that label.
        bool asks = page.AsksWalletAccount && _walletAccount.Header is not null;
        _walletAccount.Visibility = asks ? Visibility.Visible : Visibility.Collapsed;
        _walletAccount.IsEnabled = !_flow.Working;
        if (_walletAccount.Text != _flow.WalletAccount)
        {
            _walletAccount.Text = _flow.WalletAccount;
        }

        _working.IsActive = _flow.Working;
        _working.Visibility = _flow.Working ? Visibility.Visible : Visibility.Collapsed;

        _error.Text = page.Error ?? string.Empty;
        _error.Visibility = page.Error is null ? Visibility.Collapsed : Visibility.Visible;

        _dialog.CloseButtonText = page.Cancel;
        _dialog.PrimaryButtonText = page.Primary;
        _dialog.IsPrimaryButtonEnabled = _flow.CanPressPrimary;

        // Switch is never one Enter away: it moves the account.
        _dialog.DefaultButton = page.IsDefault ? ContentDialogButton.Primary : ContentDialogButton.None;

        NamePrimaryButton(page.PrimaryAccessibilityName);
    }

    /// <summary>
    /// The primary button's accessibility name, which can say more than its
    /// label (step 1 names the login used last time; Switch names the new
    /// one). ContentDialog does not expose the button, so it is found by its
    /// template name once the dialog is drawn.
    /// </summary>
    private void NamePrimaryButton(string name)
    {
        AutomationProperties.SetName(_dialog, _flow.Page?.Title ?? string.Empty);
        if (Find(_dialog, "PrimaryButton") is Button primary)
        {
            AutomationProperties.SetName(primary, name);
        }
    }

    private static DependencyObject? Find(DependencyObject parent, string name)
    {
        int count = VisualTreeHelper.GetChildrenCount(parent);
        for (int index = 0; index < count; index++)
        {
            DependencyObject child = VisualTreeHelper.GetChild(parent, index);
            if (child is FrameworkElement element && element.Name == name)
            {
                return child;
            }

            if (Find(child, name) is { } found)
            {
                return found;
            }
        }

        return null;
    }

    private void OnPrimary(ContentDialog sender, ContentDialogButtonClickEventArgs args)
    {
        // The driver decides when the dialog closes: a step's button starts
        // a browser leg or confirms, and the dialog stays up for the next.
        args.Cancel = true;
        if (_flow.CanPressPrimary)
        {
            _ = _flow.PrimaryAsync();
        }
    }

    private void OnClosing(ContentDialog sender, ContentDialogClosingEventArgs args)
    {
        if (_closingByFlow)
        {
            return;
        }

        // Cancel or Escape: the daemon drops the ceremony and any login held
        // for it.
        _closingByFlow = true;
        _ = _flow.CancelAsync();
    }

    private void Hide()
    {
        if (_closingByFlow)
        {
            return;
        }

        _closingByFlow = true;
        _dialog.Hide();
    }
}
