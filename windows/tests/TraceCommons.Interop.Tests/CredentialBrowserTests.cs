using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using TraceCommons.App.ViewModels;
using Xunit;

namespace TraceCommons.Interop.Tests;

public sealed class CredentialBrowserTests
{
    [Theory]
    [InlineData("success", "open,read,poll")]
    [InlineData("refused", "open,cancel,read,poll")]
    [InlineData("exception", "open,cancel,read,poll")]
    [InlineData("malformed", "cancel,read,poll")]
    [InlineData("none", "read,poll")]
    public async Task ContinuationRefreshesBeforePollingAndCancelsFailedLaunch(string outcome, string expected)
    {
        var calls = new List<string>();
        bool statusRead = false;
        NearAiCredentialAttempt? attempt = outcome == "none" ? null : new("attempt",
            outcome == "malformed" ? "not an absolute URL" : "https://example.invalid/sign-in");
        await CredentialBrowser.ContinueAsync(attempt,
            uri =>
            {
                calls.Add("open");
                Assert.Equal("https://example.invalid/sign-in", uri.AbsoluteUri);
                if (outcome == "exception") throw new InvalidOperationException("synthetic launcher failure");
                return Task.FromResult(outcome == "success");
            },
            () => { calls.Add("cancel"); return Task.CompletedTask; },
            () => { calls.Add("read"); statusRead = true; return Task.CompletedTask; },
            () => { Assert.True(statusRead); calls.Add("poll"); return Task.CompletedTask; });
        Assert.Equal(expected, string.Join(",", calls));
    }

    [Fact]
    public async Task FailedStatusReadCannotStartPolling()
    {
        bool polled = false;
        await Assert.ThrowsAsync<InvalidOperationException>(() => CredentialBrowser.ContinueAsync(null,
            _ => throw new InvalidOperationException("must not open"),
            () => throw new InvalidOperationException("must not cancel"),
            () => throw new InvalidOperationException("status unavailable"),
            () => { polled = true; return Task.CompletedTask; }));
        Assert.False(polled);
    }
}
