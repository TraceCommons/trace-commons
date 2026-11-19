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
