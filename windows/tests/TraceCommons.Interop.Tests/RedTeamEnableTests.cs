using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>Red Team on Windows: the daemon's red_team row says why the host
/// cannot run it, and whether the app offers "Enable Red Team".</summary>
public sealed class RedTeamEnableTests
{
    private static ManagedSnapshot Snapshot(string tool) => ManagedSnapshot.Parse(
        """{"revision":0,"accounts":[],"sessions":[],"generations":{},"capabilities":{},"copy":{},"tools":[""" + tool + "]}")!;

    [Fact]
    public void AMissingFeatureOrGroupOffersEnable()
    {
        foreach (var problem in new[] { "hyper_v_missing", "not_in_hyper_v_administrators" })
        {
            var snapshot = Snapshot($$"""{"tool":"red_team","installed":false,"install_url":null,"host_problem":"{{problem}}","enable_offered":true}""");
            Assert.True(snapshot.OffersRedTeamEnable, problem);
            Assert.False(snapshot.RedTeamUnsupportedEdition, problem);
        }
    }

    [Fact]
    public void WindowsHomeIsExplainedAndOffersNoButton()
    {
        var snapshot = Snapshot("""{"tool":"red_team","installed":false,"install_url":null,"host_problem":"home_edition","enable_offered":false}""");
        Assert.False(snapshot.OffersRedTeamEnable);
        Assert.True(snapshot.RedTeamUnsupportedEdition);
    }

    [Fact]
    public void ANeedsSetupRowWithoutAHostProblemOffersNoEnable()
    {
        var snapshot = Snapshot("""{"tool":"red_team","installed":false,"install_url":null}""");
        Assert.False(snapshot.OffersRedTeamEnable);
        Assert.False(snapshot.RedTeamUnsupportedEdition);
    }

    [Fact]
    public void AnInstalledRowOrNoRowOffersNothing()
    {
        Assert.False(Snapshot("""{"tool":"red_team","installed":true,"install_url":null}""").OffersRedTeamEnable);
        Assert.False(Snapshot("""{"tool":"claude","installed":true,"install_url":"x"}""").OffersRedTeamEnable);
    }
}
