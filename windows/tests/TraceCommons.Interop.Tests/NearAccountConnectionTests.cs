using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;
namespace TraceCommons.Interop.Tests;

public sealed class NearAccountConnectionTests
{
    private static DaemonResponse Response(object value) => new() { Result = JsonSerializer.SerializeToElement(value) };
    private static readonly string[] Methods = { "near_account_capabilities", "near_account_start", "near_account_status", "near_account_cancel" };
    [Theory]
    [InlineData("http://commons.example/wallet")]
    [InlineData("https://elsewhere.example/wallet")]
    [InlineData("https://commons.example:444/wallet")]
    [InlineData("https://user@commons.example/wallet")]
    [InlineData("file:///tmp/wallet")]
    public void WalletRequiresExactHttpsOrigin(string destination) => Assert.Null(NearAccountConnection.BrowserDestination("https://commons.example", destination));

    [Fact]
    public async Task UnsupportedDaemonNeverStartsOrChecks()
    {
        var calls = new List<string>();
        var flow = new NearAccountConnection((method, _) => { calls.Add(method); return Task.FromResult(Response(new { methods = new[] { "near_account_start" } })); }, _ => Task.FromResult(true), () => true);
        await flow.InitializeAsync(); flow.Commons = "https://commons.example"; flow.Account = "synthetic.near";
        await flow.CheckAsync(); await flow.StartAsync();
        Assert.False(flow.Supported); Assert.Equal(new[] { "hello" }, calls);
    }
    [Fact]
    public async Task CompletedCeremonyClearsAccountAndUsesOnlyExactDaemonBrowserUrl()
    {
        var calls = new List<string>(); string? opened = null; var completed = false;
        var flow = new NearAccountConnection((method, payload) => {
            calls.Add(method);
            object result = method switch {
                "hello" => new { methods = Methods },
                "near_account_capabilities" => new { ready = true },
                "near_account_start" => new { status = "waiting_for_wallet", attempt_id = "fixture", browser_url = "https://commons.example/wallet?ceremony=synthetic" },
                "near_account_status" => new { status = "complete" },
                _ => throw new Exception("unexpected call")
            };
            if (method == "near_account_start") { using var json = JsonDocument.Parse(payload); Assert.Equal("synthetic.near", json.RootElement.GetProperty("account_id").GetString()); Assert.Equal(2, json.RootElement.EnumerateObject().Count()); }
            return Task.FromResult(Response(result));
        }, uri => { opened = uri.AbsoluteUri; return Task.FromResult(true); }, () => true);
        flow.Completed += () => completed = true;
        await flow.InitializeAsync(); flow.Commons = "https://commons.example"; await flow.CheckAsync(); flow.Account = "synthetic.near"; await flow.StartAsync();
        Assert.True(completed); Assert.Empty(flow.Account); Assert.False(flow.Busy);
        Assert.Equal("https://commons.example/wallet?ceremony=synthetic", opened);
        Assert.Equal(new[] { "hello", "near_account_capabilities", "near_account_start", "near_account_status" }, calls);
    }
    [Fact]
    public async Task ClosingDuringStartCancelsReturnedAttemptWithoutOpeningBrowser()
    {
        var started = new TaskCompletionSource<DaemonResponse>(); var cancelled = false; var opened = false;
        var flow = new NearAccountConnection((method, _) => method switch {
            "hello" => Task.FromResult(Response(new { methods = Methods })),
            "near_account_capabilities" => Task.FromResult(Response(new { ready = true })),
            "near_account_start" => started.Task,
            "near_account_cancel" => Cancel(),
            _ => throw new Exception("unexpected")
        }, _ => { opened = true; return Task.FromResult(true); }, () => true);
        Task<DaemonResponse> Cancel() { cancelled = true; return Task.FromResult(Response(new { status = "cancelled" })); }
        await flow.InitializeAsync(); flow.Commons = "https://commons.example"; await flow.CheckAsync(); flow.Account = "synthetic.near";
        var pending = flow.StartAsync(); await flow.CloseAsync();
        started.SetResult(Response(new { status = "waiting_for_wallet", attempt_id = "fixture", browser_url = "https://commons.example/wallet" }));
        await pending; Assert.True(cancelled); Assert.False(opened); Assert.False(flow.Busy);
    }
    [Fact]
    public async Task UnknownStatusRemainsCancellableAndDoesNotEnroll()
    {
        var cancelled = false;
        var flow = new NearAccountConnection((method, _) => {
            if (method == "near_account_cancel") cancelled = true;
            return Task.FromResult(Response(method switch {
                "hello" => (object)new { methods = Methods },
                "near_account_capabilities" => new { ready = true },
                "near_account_start" => new { status = "waiting_for_wallet", attempt_id = "fixture", browser_url = "https://commons.example/wallet" },
                _ => new { status = "future-status", sensitive_error = "never show this" }
            }));
        }, _ => Task.FromResult(true), () => true);
        await flow.InitializeAsync(); flow.Commons = "https://commons.example"; await flow.CheckAsync(); flow.Account = "synthetic.near"; await flow.StartAsync();
        Assert.True(flow.Busy); Assert.True(flow.CanCancel); Assert.DoesNotContain("sensitive", flow.Message);
        await flow.CancelAsync(); Assert.True(cancelled); Assert.False(flow.Busy);
    }
}
