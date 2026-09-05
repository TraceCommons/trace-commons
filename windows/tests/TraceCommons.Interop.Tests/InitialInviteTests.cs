using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class InitialInviteTests
{
    private const string Protocol = "tracecommons://enroll?invite=protocol-invite";
    private static readonly string[] Arguments = { "TraceCommons.exe", "tracecommons://enroll?invite=argument-invite" };

    [Fact]
    public void PackagedProtocolPayloadWinsOverCommandLine()
    {
        Assert.Equal("protocol-invite", DeepLink.InitialInvite(Protocol, Arguments));
    }

    [Theory]
    [InlineData("")]
    [InlineData("https://enroll?invite=wrong-scheme")]
    [InlineData("tracecommons://other?invite=wrong-host")]
    [InlineData("tracecommons://enroll")]
    public void InvalidProtocolPayloadCannotSelectADifferentArgvInvite(string payload)
    {
        Assert.Null(DeepLink.InitialInvite(payload, Arguments));
    }

    [Fact]
    public void OrdinaryLaunchRetainsExistingArgumentParsing()
    {
        Assert.Equal("argument-invite", DeepLink.InitialInvite(null, Arguments));
        Assert.Null(DeepLink.InitialInvite(null, new[] { "TraceCommons.exe", "--ordinary-switch" }));
    }
}
