using System;
using System.IO;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The deep-link parser and the completion store.
///
/// Both live in the interop layer rather than the WinUI app so they can be
/// exercised here, off Windows. The screens they feed cannot be, but the two
/// pieces with actual decisions in them can.
/// </summary>
public sealed class DeepLinkTests
{
    [Fact]
    public void InviteIsPulledOutOfADeepLink()
    {
        string? invite = DeepLink.InviteFrom(
            "tracecommons://enroll?invite=https%3A%2F%2Fissuer.example%2Fonboard%23CODE");

        Assert.Equal("https://issuer.example/onboard#CODE", invite);
    }

    [Fact]
    public void SchemeAndHostAreCaseInsensitive()
    {
        // A handler registration need not preserve the case anyone typed,
        // and the macOS and Rust parsers accept either case too. One invite
        // mail goes to contributors on all three platforms.
        string? invite = DeepLink.InviteFrom(
            "TraceCommons://ENROLL?invite=https%3A%2F%2Fi.example%2Fo%23C");

        Assert.Equal("https://i.example/o#C", invite);
    }

    [Theory]
    // Registering a scheme handler means this is asked about every argument
    // the app is ever launched with, so none of these may throw.
    [InlineData("https://example.com/")]
    [InlineData("tracecommons://open?x=1")]
    [InlineData("--state-dir")]
    [InlineData("tracecommons://enroll")]
    [InlineData("tracecommons://enroll?invite=")]
    [InlineData("")]
    [InlineData(null)]
    public void EverythingElseIsNotAnInvite(string? argument)
    {
        Assert.Null(DeepLink.InviteFrom(argument));
    }

    [Fact]
    public void AWindowsPathIsNotAnInvite()
    {
        // argv[0] is a path, and the handler hands us argv.
        Assert.Null(DeepLink.InviteFrom(@"C:\Program Files\TraceCommons\TraceCommons.exe"));
    }

    [Fact]
    public void AnInviteWithAnAmpersandSurvivesTheQuerySplit()
    {
        // The invite is a whole URL living inside a query parameter, so its
        // own encoded separators must not be mistaken for ours.
        string? invite = DeepLink.InviteFrom(
            "tracecommons://enroll?invite=https%3A%2F%2Fi.example%2Fo%3Fa%3D1%26b%3D2%23C&x=1");

        Assert.Equal("https://i.example/o?a=1&b=2#C", invite);
    }
}

public sealed class OnboardingStateTests : IDisposable
{
    private readonly string _directory =
        Path.Combine(Path.GetTempPath(), "tc-onboarding-" + Guid.NewGuid().ToString("N"));

    private string StorePath => Path.Combine(_directory, "onboarded.json");

    [Fact]
    public void NoTenantIsNeverComplete()
    {
        // Before `enroll` there is no tenant, so nothing could have been
        // finished -- and onboarding must run rather than be skipped.
        var state = new OnboardingState(StorePath);

        Assert.False(state.IsComplete(null));
        Assert.False(state.IsComplete(string.Empty));
    }

    [Fact]
    public void CompletionSurvivesAReload()
    {
        new OnboardingState(StorePath).MarkComplete("tenant-a");

        Assert.True(new OnboardingState(StorePath).IsComplete("tenant-a"));
    }

    [Fact]
    public void OneTenantsCompletionIsNotAnothers()
    {
        // The invariant the per-tenant scheme exists for: finishing for one
        // commons must not skip the scopes screen for a different one.
        var state = new OnboardingState(StorePath);
        state.MarkComplete("tenant-a");

        Assert.True(state.IsComplete("tenant-a"));
        Assert.False(state.IsComplete("tenant-b"));
    }

    [Fact]
    public void MarkingTwiceDoesNotDuplicate()
    {
        var state = new OnboardingState(StorePath);
        state.MarkComplete("tenant-a");
        state.MarkComplete("tenant-a");

        Assert.Contains("tenant-a", File.ReadAllText(StorePath));
        Assert.True(state.IsComplete("tenant-a"));
    }

    [Fact]
    public void AnUnreadableStoreAsksAgainRatherThanAssumingDone()
    {
        // Asymmetric costs: showing a screen again is cheap, skipping the
        // consent decision is not. Corruption resolves toward asking.
        Directory.CreateDirectory(_directory);
        File.WriteAllText(StorePath, "{ this is not a json array");

        Assert.False(new OnboardingState(StorePath).IsComplete("tenant-a"));
    }

    public void Dispose()
    {
        if (Directory.Exists(_directory))
        {
            Directory.Delete(_directory, recursive: true);
        }
    }
}

/// <summary>
/// The invite host, across the real FFI boundary.
///
/// These call the Rust cdylib, so they prove the marshalling as well as the
/// answer -- which is the reason this layer is plain net8.0.
/// </summary>
public sealed class InviteTests
{
    [Fact]
    public void IssuerHostCrossesTheBoundaryWithoutTheCode()
    {
        string? host = Invite.IssuerHost("https://issuer.tracecommons.ai/onboard#VQWWPGYSG8Y4LTP6");

        Assert.Equal("issuer.tracecommons.ai", host);
        Assert.DoesNotContain("VQWWPGYSG8Y4LTP6", host);
    }

    [Theory]
    [InlineData("VQWWPGYSG8Y4LTP6")]
    [InlineData("https://issuer.tracecommons.ai/onboard")]
    [InlineData("not a url")]
    [InlineData("")]
    [InlineData(null)]
    public void AnythingUnusableIsNull(string? invite)
    {
        // One failure sentence for the whole path means the caller must not
        // be able to tell these apart.
        Assert.Null(Invite.IssuerHost(invite));
    }

    [Fact]
    public void NonAsciiHostSurvivesTheUtf8Boundary()
    {
        // The marshalling is LPUTF8Str on the way in and PtrToStringUTF8 on
        // the way out; a punycode host proves neither end mangles it.
        string? host = Invite.IssuerHost("https://xn--bcher-kva.example/onboard#CODE");

        Assert.Equal("xn--bcher-kva.example", host);
    }
}
