using System;
using System.Linq;
using System.Collections.Generic;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Cards the Mac's Home draws that Windows did not, ported as a class
/// (owner, 2026-11-19), each on the Mac's condition and in the core's words.
/// </summary>
public class HomeCardsTests
{
    private static readonly HomeOverviewModel.Figures Figures = new(
        n => n.ToString(System.Globalization.CultureInfo.InvariantCulture),
        c => c.ToString(System.Globalization.CultureInfo.InvariantCulture),
        (_, _) => string.Empty);

    private static string Home(string key) => MonitorShellCopy.Section("home_history", key);

    private static HomeOverviewModel Build(DaemonStatus? status, bool missionsOn = false, bool catalogueRead = false) =>
        HomeOverviewModel.From(
            new HomeOverviewModel.Reads(status, null, null, null, null, null, null, false, missionsOn, null, null, null, catalogueRead),
            MonitorShellCopy.Top, Home, string.Empty, Figures);

    /// <summary>NoCreditNotice.showsOnHome: signed in and without the model-training use, the core's line and Open Settings.</summary>
    [Fact]
    public void TheNoCreditLineShowsWhileModelTrainingIsOff()
    {
        HomeOverviewModel model = Build(new DaemonStatus { LoggedIn = true, ConsentScopes = new List<string> { "insights" } });
        Assert.NotNull(model.NoCredit);
        Assert.Equal(Home("no_credit_body"), model.NoCredit!.Line);
        Assert.Equal("Open Settings", model.NoCredit.Action);
    }

    [Fact]
    public void TheNoCreditLineGoesOnceTheUseIsOnOrNoOneIsSignedIn()
    {
        Assert.Null(Build(new DaemonStatus { LoggedIn = true, ConsentScopes = new List<string> { "model_training" } }).NoCredit);
        Assert.Null(Build(new DaemonStatus { LoggedIn = false }).NoCredit);
        Assert.Null(Build(null).NoCredit);
    }

    /// <summary>HomeViews.swift:274-277: the catalogue's card while missions are on and the catalogue was read.</summary>
    [Fact]
    public void TheCatalogueCardShowsWhileMissionsAreOnAndItWasRead()
    {
        HomeOverviewModel model = Build(null, missionsOn: true, catalogueRead: true);
        Assert.NotNull(model.Catalogue);
        Assert.Equal("Mission catalogue", model.Catalogue!.Title);
        Assert.Equal("What the commons is asking for.", model.Catalogue.Subtitle);
        Assert.Null(Build(null, missionsOn: false, catalogueRead: true).Catalogue);
        Assert.Null(Build(null, missionsOn: true, catalogueRead: false).Catalogue);
    }

    [Fact]
    public void ANewCardIsADifferentHome()
    {
        HomeOverviewModel without = Build(null, missionsOn: true, catalogueRead: false);
        HomeOverviewModel with = Build(null, missionsOn: true, catalogueRead: true);
        Assert.False(without.Same(with));
        Assert.True(with.Same(Build(null, missionsOn: true, catalogueRead: true)));
    }
}

/// <summary>
/// Home's Private AI section (HomeViews.swift:466-490): once the daemon runs
/// and the managed snapshot was read; signed in, the calls answered and how
/// many are verified over the network window; signed out, the core's line and
/// Sign in.
/// </summary>
public class HomePrivateAITests
{
    private static readonly HomeOverviewModel.Figures Figures = new(
        n => n.ToString(System.Globalization.CultureInfo.InvariantCulture),
        c => c.ToString(System.Globalization.CultureInfo.InvariantCulture),
        (_, _) => string.Empty);

    private static ManagedSnapshot Snapshot(bool signedIn) => new(
        1, Array.Empty<ManagedAccount>(), Array.Empty<ManagedSession>(), new Dictionary<string, ulong>(),
        new ManagedCapabilities(false, false, null, signedIn),
        new Dictionary<string, string> { ["sign_in_near_ai"] = "Sign in", ["sign_in_near_ai_accessibility"] = "Sign in to NEAR AI" });

    private static HomeOverviewModel Build(DaemonStatus? status, ManagedSnapshot? managed, NetworkInferenceSummary? network = null) =>
        HomeOverviewModel.From(
            new HomeOverviewModel.Reads(status, null, null, null, null, null, null, false, false, null, null, null, false, managed, network),
            MonitorShellCopy.Top, k => MonitorShellCopy.Section("home_history", k), string.Empty, Figures);

    [Fact]
    public void SignedInTheRowShowsCallsAndVerifiedOverTheNetworkWindow()
    {
        var network = new NetworkInferenceSummary(true, 24, true,
            new InferenceRouteTotal(10, null, null, 7), new InferenceRouteTotal(2, null, null, 0), new InferenceRouteTotal(1, null, null, 1));
        HomePrivateAI? section = Build(new DaemonStatus(), Snapshot(signedIn: true), network).PrivateAI;
        Assert.NotNull(section);
        Assert.Equal(MonitorShellCopy.Section("home_history", "section_private_ai"), section!.Heading);
        Assert.Null(section.SignIn);
        Assert.Equal(new[] { "13", "8" }, section.Figures!.Select(f => f.Value));
        Assert.Equal(new[] { MonitorShellCopy.Top("calls"), MonitorShellCopy.Top("proof_verified") }, section.Figures!.Select(f => f.Label));
    }

    [Fact]
    public void SignedInWithoutANetworkReadTheFiguresAreDashes()
    {
        HomePrivateAI section = Build(new DaemonStatus(), Snapshot(signedIn: true)).PrivateAI!;
        Assert.All(section.Figures!, f => Assert.Equal(HomeFacts.Dash, f.Value));
    }

    [Fact]
    public void SignedOutTheCardSaysTheCoresLineAndSignIn()
    {
        HomePrivateAI section = Build(new DaemonStatus(), Snapshot(signedIn: false)).PrivateAI!;
        Assert.Null(section.Figures);
        Assert.Equal(MonitorShellCopy.Section("home_history", "private_ai_sign_in"), section.SignIn!.Line);
        Assert.Equal("Sign in", section.SignIn.Action);
        Assert.Equal("Sign in to NEAR AI", section.SignInName);
    }

    [Fact]
    public void NothingIsDrawnBeforeTheDaemonRunsOrTheSnapshotWasRead()
    {
        Assert.Null(Build(null, Snapshot(signedIn: true)).PrivateAI);
        Assert.Null(Build(new DaemonStatus(), null).PrivateAI);
    }
}

/// <summary>
/// Beside the Account heading while its figures wait on a sign-in
/// (<c>accountSignIn</c>, HomeViews.swift:445-470): the core's "No data",
/// then Sign in. Its NEAR AI half: the balance answered that no sign-in is
/// kept or that NEAR AI did not accept it (<c>NearAiSignInOffer.balanceOffers</c>).
/// The account-session half has no sign-in on Windows, so a credit summary
/// waiting on the account session alone draws nothing.
/// </summary>
public class HomeAccountSignInTests
{
    private static readonly HomeOverviewModel.Figures Figures = new(
        n => n.ToString(System.Globalization.CultureInfo.InvariantCulture),
        c => c.ToString(System.Globalization.CultureInfo.InvariantCulture),
        (_, _) => string.Empty);

    private static readonly ManagedSnapshot Managed = new(
        1, Array.Empty<ManagedAccount>(), Array.Empty<ManagedSession>(), new Dictionary<string, ulong>(),
        new ManagedCapabilities(false, false, null, false),
        new Dictionary<string, string> { ["sign_in_near_ai"] = "Sign in", ["sign_in_near_ai_accessibility"] = "Sign in to NEAR AI" });

    private static NearAiBalance Balance(string state) => new(state, null, null, null, null, null);

    private static HomeOverviewModel Build(NearAiBalance? balance, ManagedSnapshot? managed = null, CommonsCredit? credit = null, bool accountReadPending = false) =>
        HomeOverviewModel.From(
            new HomeOverviewModel.Reads(new DaemonStatus(), null, credit, balance, null, null, null, false, false, null, null, null, false, managed ?? Managed, AccountReadPending: accountReadPending),
            MonitorShellCopy.Top, k => MonitorShellCopy.Section("home_history", k), "Balance", Figures);

    [Fact]
    public void PendingAccountShowsLoadingUntilTheFirstReadCompletes()
    {
        HomeOverviewModel loading = Build(null, accountReadPending: true);
        HomeOverviewModel signedOut = Build(Balance("no_session"));

        Assert.True(loading.AccountReadPending);
        Assert.Empty(loading.AccountTiles);
        Assert.Null(loading.AccountSignIn);
        Assert.NotEmpty(loading.TraceTiles);
        Assert.False(signedOut.AccountReadPending);
        Assert.NotEmpty(signedOut.AccountTiles);
        Assert.NotNull(signedOut.AccountSignIn);
        Assert.False(loading.Same(signedOut));
    }

    [Theory]
    [InlineData("no_session")]
    [InlineData("session_expired")]
    public void WhileTheBalanceWaitsOnASignInTheHeadingSaysNoDataAndSignIn(string state)
    {
        HomeAccountSignIn? signIn = Build(Balance(state)).AccountSignIn;
        Assert.NotNull(signIn);
        Assert.Equal(MonitorShellCopy.Section("home_history", "account_no_data"), signIn!.NoData);
        Assert.Equal("No data", signIn.NoData);
        Assert.Equal("Sign in", signIn.SignIn);
        Assert.Equal("Sign in to NEAR AI", signIn.SignInName);
    }

    /// <summary>A known balance, an unreachable NEAR AI or an unread balance is not a sign-in problem.</summary>
    [Theory]
    [InlineData("known")]
    [InlineData("unavailable")]
    [InlineData("no_organization")]
    [InlineData("")]
    public void OtherwiseNothingIsDrawn(string state)
    {
        Assert.Null(Build(Balance(state)).AccountSignIn);
        Assert.Null(Build(null).AccountSignIn);
    }

    [Fact]
    public void TheAccountSessionAloneHasNoSignInHere()
    {
        var credit = new CommonsCredit(true, null, false, null, AccountSessionRequired: true);
        Assert.Null(Build(Balance("known"), credit: credit).AccountSignIn);
        Assert.NotNull(Build(Balance("no_session"), credit: credit).AccountSignIn);
    }

    [Fact]
    public void WithoutTheSignInWordsNothingIsDrawn()
    {
        var bare = Managed with { Copy = new Dictionary<string, string>() };
        Assert.Null(Build(Balance("no_session"), bare).AccountSignIn);
        Assert.Null(HomeOverviewModel.From(
            new HomeOverviewModel.Reads(new DaemonStatus(), null, null, Balance("no_session"), null, null, null, false, false, null, null, null, false, null),
            MonitorShellCopy.Top, k => MonitorShellCopy.Section("home_history", k), "Balance", Figures).AccountSignIn);
    }

    [Fact]
    public void TheSignInIsADifferentHome()
    {
        Assert.False(Build(Balance("known")).Same(Build(Balance("no_session"))));
        Assert.True(Build(Balance("no_session")).Same(Build(Balance("no_session"))));
    }
}
