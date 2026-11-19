using System;
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
