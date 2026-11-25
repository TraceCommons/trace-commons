using System;
using System.Threading;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.Controls.Settings;
using TraceCommons.Interop;
using TraceCommons.Interop.FirstRun;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls;

public sealed partial class SettingsView
{
    private static readonly SetUpAgainCopy? AccountSetupWords = SetUpAgainCopy.Load();
    private static readonly AccountChangeCopy? AccountChangeWords = AccountChangeCopy.Copy();
    private static readonly PrivateInferenceCopy? AccountCredentialWords = PrivateInferenceSurface.Copy();
    private AccountSettingsClient? _accountClient;
    private DaemonHost? _accountHost;
    private DaemonStatus? _accountStatus;
    private bool _accountStatusFailed;
    private bool _nearAiSignedIn;
    private bool _accountActionBusy;
    private string _accountSignInNotice = string.Empty;
    private string _accountLeaveNotice = string.Empty;
    private string _accountSetupNotice = string.Empty;
    private long _accountReadSequence;

    private TextBlock? _contributionLine;
    private Button? _contributionRefresh;
    private Button? _contributionRedeem;
    private PasswordBox? _inviteCode;
    private bool _contributionBusy;
    private (string Code, Guid Key)? _inviteAttempt;
    private string? _contributionScope;

    private bool AccountSignedOut => _accountStatus is { LoggedIn: true, AccountSignedIn: false };
    private bool CanSetUpAccountAgain => _accountStatus?.LoggedIn == true && !Settings.HasAccountAttention && AccountSetupWords is not null;

    private void InitializeAccountSettings(DaemonHost host)
    {
        _accountHost = host;
        _accountClient = new AccountSettingsClient((method, parameters) => CallSettingsAccountAsync(host, method, parameters));
        BuildContributionCard();
        AccountConnectionHost.Content = SettingsCards.ReadNotice(failed: false, (_, _) => { });
        Loaded += async (_, _) =>
        {
            host.StatusChanged -= OnSettingsAccountStatusChanged;
            host.StatusChanged += OnSettingsAccountStatusChanged;
            host.Account.Changed -= OnSettingsAccountBoundary;
            host.Account.Changed += OnSettingsAccountBoundary;
            Settings.PropertyChanged -= OnAccountSettingsPropertyChanged;
            Settings.PropertyChanged += OnAccountSettingsPropertyChanged;
            await RefreshSettingsAccountAsync();
        };
        Unloaded += (_, _) =>
        {
            host.StatusChanged -= OnSettingsAccountStatusChanged;
            host.Account.Changed -= OnSettingsAccountBoundary;
            Settings.PropertyChanged -= OnAccountSettingsPropertyChanged;
            _accountReadSequence++;
        };
    }

    private void OnAccountSettingsPropertyChanged(object? sender, System.ComponentModel.PropertyChangedEventArgs e)
    {
        if (e.PropertyName == nameof(Settings.HasAccountAttention))
        {
            DrawSettingsAccount();
        }
    }

    private async void OnSettingsAccountStatusChanged() => await RefreshSettingsAccountAsync();

    private async void OnSettingsAccountBoundary()
    {
        ResetContribution(null);
        await RefreshSettingsAccountAsync();
    }

    private static async Task<DaemonResponse> CallSettingsAccountAsync(DaemonHost host, string method, string parameters = "{}")
    {
        try
        {
            return await host.CallAsync(method, parameters);
        }
        catch (Exception exception) when (exception is TcException or InvalidOperationException)
        {
            return DaemonResponse.Parse("""{"error":{"code":"unavailable","message":"daemon-not-started"}}""");
        }
    }

    private async Task RefreshSettingsAccountAsync()
    {
        if (_accountHost is not { } host)
        {
            return;
        }

        long read = ++_accountReadSequence;
        Task<DaemonResponse> statusCall = CallSettingsAccountAsync(host, DaemonProtocol.Methods.Status);
        Task<DaemonResponse> credentialCall = CallSettingsAccountAsync(host, DaemonProtocol.Methods.NearAiCredentialStatus);
        await Task.WhenAll(statusCall, credentialCall);
        if (read != _accountReadSequence)
        {
            return;
        }

        DaemonResponse statusResponse = await statusCall;
        DaemonResponse credentialResponse = await credentialCall;
        DaemonStatus? status = statusResponse.IsError ? null : statusResponse.ResultAs<DaemonStatus>();
        NearAiCredentialStatus credential = NearAiCredentialSurface.ParseStatus(credentialResponse.Result?.GetRawText());
        bool nearAiSignedIn = !credentialResponse.IsError && credential.SessionState == NearAiEnrollSurface.CredentialStatePresent;
        bool redraw = status?.LoggedIn != _accountStatus?.LoggedIn || status?.AccountSignedIn != _accountStatus?.AccountSignedIn
            || status?.AccountLogin != _accountStatus?.AccountLogin || nearAiSignedIn != _nearAiSignedIn
            || _accountStatusFailed != (status is null);

        _accountStatus = status;
        _accountStatusFailed = status is null;
        _nearAiSignedIn = nearAiSignedIn;
        if (_contributionScope != status?.AccountScope)
        {
            ResetContribution(status?.AccountScope);
        }

        if (redraw)
        {
            DrawSettingsAccount();
        }
    }

    private void DrawSettingsAccount()
    {
        AccountCredentialHost.Visibility = AccountSignedOut ? Visibility.Collapsed : Visibility.Visible;
        DevicesHost.Visibility = AccountSignedOut ? Visibility.Collapsed : Visibility.Visible;
        var body = new StackPanel { Spacing = 8 };
        if (_accountStatus is not { } status)
        {
            body.Children.Add(SettingsCards.ReadNotice(_accountStatusFailed, async (_, _) => await RefreshSettingsAccountAsync()));
        }
        else
        {
            string title = !status.LoggedIn ? SettingsLegacyWords.Word("not_connected")
                : AccountSignedOut ? MonitorShellCopy.Top("signed_out")
                : (!Settings.HasAccountAttention ? AccountChangeWords?.Render(AccountChangeWords.Connected, provider: status.AccountLogin) : null)
                    ?? SettingsLegacyWords.Word("connected");
            body.Children.Add(SettingsCards.Text(title, "TcTitleCardTextStyle"));
            if (!AccountSignedOut)
            {
                body.Children.Add(SettingsCards.GlassChip(
                    MonitorShellCopy.Shell(status.LoggedIn ? "connection_ready" : "connection_local_only"), muted: !status.LoggedIn));
            }

            if (!status.LoggedIn)
            {
                body.Children.Add(SettingsCards.Caption(SettingsLegacyWords.Word("queued_nothing_sent")));
            }
            else if (AccountSignedOut)
            {
                Button signIn = SettingsCards.Button(MonitorShellCopy.Top("sign_in"), async (_, _) => await SignInToSettingsAccountAsync(), primary: true);
                signIn.IsEnabled = !_accountActionBusy;
                body.Children.Add(signIn);
            }

            if (_accountSignInNotice.Length > 0)
            {
                body.Children.Add(SettingsCards.Caption(_accountSignInNotice));
            }
        }

        AccountConnectionHost.Content = SettingsCards.Card(SettingsSections.NavName(SettingsSection.Account, SettingsNavWordsLive.Value) ?? string.Empty, body);
        DrawAccountLeave();
    }

    private void DrawAccountLeave()
    {
        var body = new StackPanel { Spacing = 8 };
        if ((_accountStatus?.AccountSignedIn == true || _nearAiSignedIn) && AccountChangeWords is { SignOut: { } signOut, SignOutBody: { } description })
        {
            body.Children.Add(SettingsCards.Text(description));
            Button button = SettingsCards.Button(signOut, async (_, _) => await SignOutOfSettingsAccountAsync());
            button.IsEnabled = !_accountActionBusy;
            body.Children.Add(button);
        }

        if (_accountLeaveNotice.Length > 0)
        {
            body.Children.Add(SettingsCards.Caption(_accountLeaveNotice));
        }

        if (CanSetUpAccountAgain && AccountSetupWords is { } setup)
        {
            Button button = SettingsCards.Button(setup.Action, OnSetUpAgain);
            button.IsEnabled = !_accountActionBusy && Settings.IsNotBusy;
            body.Children.Add(button);
            if (_accountSetupNotice.Length > 0)
            {
                body.Children.Add(SettingsCards.Caption(_accountSetupNotice));
            }
        }

        AccountLeaveHost.Content = body.Children.Count == 0 ? null : SettingsCards.Card(string.Empty, body);
        AccountLeaveHost.Visibility = body.Children.Count == 0 ? Visibility.Collapsed : Visibility.Visible;
    }

    private async Task SignInToSettingsAccountAsync()
    {
        if (_accountHost is not { } host || _accountClient is not { } client || _accountActionBusy)
        {
            return;
        }

        _accountActionBusy = true;
        _accountSignInNotice = string.Empty;
        _accountLeaveNotice = string.Empty;
        DrawSettingsAccount();
        bool cancelled = false;
        try
        {
            bool signedIn = await client.SignInAsync(async () =>
            {
                NearAILoginOutcome outcome = await FirstRunDaemon.OpenNearAILoginAsync(
                    async (method, parameters) => await CallSettingsAccountAsync(host, method, parameters),
                    async uri => await Windows.System.Launcher.LaunchUriAsync(uri),
                    _ => { },
                    fresh: true,
                    CancellationToken.None);
                cancelled = outcome == NearAILoginOutcome.Cancelled;
                return outcome == NearAILoginOutcome.SignedIn;
            });
            if (signedIn)
            {
                host.Account.Boundary();
            }
            else if (!cancelled)
            {
                _accountSignInNotice = MonitorShellCopy.Top("request_failed");
            }

            await RefreshSettingsAccountAsync();
        }
        finally
        {
            _accountActionBusy = false;
            DrawSettingsAccount();
        }
    }

    private async Task SignOutOfSettingsAccountAsync()
    {
        if (_accountHost is not { } host || _accountClient is not { } client || _accountActionBusy)
        {
            return;
        }

        _accountActionBusy = true;
        _accountSignInNotice = string.Empty;
        _accountLeaveNotice = string.Empty;
        DrawSettingsAccount();
        try
        {
            DaemonResponse? answer = await client.SignOutAsync();
            if (answer is null)
            {
                _accountLeaveNotice = AccountSetupWords?.Failed ?? MonitorShellCopy.Top("request_failed");
            }
            else
            {
                _accountLeaveNotice = NearAiCredentialSurface.UnenrollNotice(answer, AccountCredentialWords);
                host.Account.Boundary();
            }

            await RefreshSettingsAccountAsync();
        }
        finally
        {
            _accountActionBusy = false;
            DrawSettingsAccount();
        }
    }

    private void BuildContributionCard()
    {
        if (AccountCredentialWords is not { } copy || string.IsNullOrEmpty(copy.AccountContributionHeading)
            || string.IsNullOrEmpty(copy.AccountContributionRefreshAction) || string.IsNullOrEmpty(copy.AccountContributionInviteCode)
            || string.IsNullOrEmpty(copy.AccountContributionRedeemAction))
        {
            AccountContributionHost.Visibility = Visibility.Collapsed;
            return;
        }

        var body = new StackPanel { Spacing = 8 };
        _contributionLine = SettingsCards.Caption(copy.AccountContributionRefresh);
        _contributionRefresh = SettingsCards.Button(copy.AccountContributionRefreshAction, async (_, _) => await UpdateContributionAsync(redeem: false));
        _inviteCode = new PasswordBox { Header = copy.AccountContributionInviteCode };
        _inviteCode.PasswordChanged += (_, _) => DrawContributionButtons();
        _contributionRedeem = SettingsCards.Button(copy.AccountContributionRedeemAction, async (_, _) => await UpdateContributionAsync(redeem: true));
        body.Children.Add(_contributionLine);
        body.Children.Add(_contributionRefresh);
        body.Children.Add(_inviteCode);
        body.Children.Add(_contributionRedeem);
        if (copy.AccountContributionPendingCredit.Length > 0)
        {
            body.Children.Add(SettingsCards.Caption(copy.AccountContributionPendingCredit));
        }

        AccountContributionHost.Content = SettingsCards.Card(copy.AccountContributionHeading, body);
        DrawContributionButtons();
    }

    private void ResetContribution(string? scope)
    {
        _contributionScope = scope;
        _inviteAttempt = null;
        if (_inviteCode is not null && _contributionLine is not null)
        {
            _inviteCode.Password = string.Empty;
            _contributionLine.Text = AccountCredentialWords?.AccountContributionRefresh ?? string.Empty;
        }

        DrawContributionButtons();
    }

    private void DrawContributionButtons()
    {
        if (_inviteCode is null || _contributionRefresh is null || _contributionRedeem is null)
        {
            return;
        }

        bool ready = !_contributionBusy && _contributionScope is not null;
        _contributionRefresh.IsEnabled = ready;
        _inviteCode.IsEnabled = ready;
        _contributionRedeem.IsEnabled = ready && !string.IsNullOrWhiteSpace(_inviteCode.Password);
    }

    private async Task UpdateContributionAsync(bool redeem)
    {
        if (_accountHost is not { } host || _accountClient is not { } client || _contributionScope is not { } scope
            || _contributionLine is null || _inviteCode is null || AccountCredentialWords is not { } copy || _contributionBusy)
        {
            return;
        }

        string? code = redeem ? _inviteCode.Password.Trim() : null;
        if (code is { Length: 0 })
        {
            return;
        }

        if (code is not null && _inviteAttempt?.Code != code)
        {
            _inviteAttempt = (code, Guid.NewGuid());
        }

        long generation = host.Account.Current;
        _contributionBusy = true;
        _contributionLine.Text = copy.AccountContributionChecking;
        DrawContributionButtons();
        try
        {
            string? line = await client.ContributionAsync(scope, code, _inviteAttempt?.Key);
            if (!host.Account.IsCurrent(generation) || _contributionScope != scope)
            {
                return;
            }

            _contributionLine.Text = line ?? copy.AccountContributionUnavailable;
            if (line is not null && redeem)
            {
                _inviteCode.Password = string.Empty;
                _inviteAttempt = null;
            }
        }
        finally
        {
            _contributionBusy = false;
            DrawContributionButtons();
        }
    }
}
