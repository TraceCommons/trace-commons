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

    private const string SignedInWithRedTeam = """
    {"revision":1,"accounts":[],"sign_in_accounts":[{"id":"c","tool":"claude","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"},{"id":"r","tool":"red_team","connection":"near_ai_signin","label":"NEAR AI","auth_state":"ready"}],"sessions":[],"generations":{},"capabilities":{"managed_launch":true,"near_ai_signin":true,"terminal_launch":true},"copy":{}}
    """;

    private static ManagedSnapshot SignedIn(string tools) =>
        ManagedSnapshot.Parse(SignedInWithRedTeam.Replace("\"copy\":{}", "\"copy\":{},\"tools\":" + tools))!;

    [Fact]
    public void RedTeamGetsAStartButtonOnlyOnceItsRuntimeIsReady()
    {
        const string claude = """{"tool":"claude","installed":true,"install_url":"x"}""";
        var notReady = SignedIn($$"""[{{claude}},{"tool":"red_team","installed":false,"install_url":null}]""");
        Assert.Equal(["claude"], notReady.SignInStartTools);
        Assert.False(notReady.CanLaunch("red_team"));
        var ready = SignedIn($$"""[{{claude}},{"tool":"red_team","installed":true,"install_url":null}]""");
        Assert.Equal(["claude", "red_team"], ready.SignInStartTools);
        Assert.True(ready.CanLaunch("red_team"));
        Assert.Equal("start_red_team", ManagedSignIn.StartKey("red_team"));
    }

    [Fact]
    public void RedTeamIsNeverACodingToolToInstall()
    {
        var snapshot = SignedIn("""[{"tool":"red_team","installed":false,"install_url":null}]""");
        Assert.DoesNotContain(snapshot.MissingInstallTools, t => t.Tool == "red_team");
        Assert.False(snapshot.CanLaunch("red_team"));
    }

    [Fact]
    public void TheSetupStateFollowsTheDaemonsRow()
    {
        Assert.Equal(RedTeamSetup.NotInstalled, Snapshot("""{"tool":"red_team","installed":false,"install_url":null}""").RedTeamState);
        Assert.Equal(RedTeamSetup.Installed, Snapshot("""{"tool":"red_team","installed":true,"install_url":null}""").RedTeamState);
        Assert.Null(Snapshot("""{"tool":"claude","installed":true,"install_url":"x"}""").RedTeamState);
    }

    [Theory]
    [InlineData("sign_out_needed", "red_team_enable_sign_out")]
    [InlineData("restart_needed", "red_team_enable_restart")]
    [InlineData("home_unsupported", "red_team_home_unsupported")]
    [InlineData("declined", "red_team_enable_declined")]
    [InlineData("failed", "red_team_enable_failed")]
    [InlineData("anything else", "red_team_enable_failed")]
    [InlineData(null, "red_team_enable_failed")]
    public void EachEnableOutcomeHasItsOwnWords(string? outcome, string key)
    {
        Assert.Equal(key, ManagedSignIn.EnableOutcomeKey(outcome));
    }
}
