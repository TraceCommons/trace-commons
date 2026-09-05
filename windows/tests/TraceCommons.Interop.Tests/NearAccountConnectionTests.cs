using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;
namespace TraceCommons.Interop.Tests;
public sealed class NearAccountConnectionTests
{
    private static DaemonResponse Response(object value) => new() { Result = JsonSerializer.SerializeToElement(value) };
    [Fact]
    public async Task AdapterUsesCoreViewAndWaitWithoutReimplementingTransitions()
    {
        var calls=new List<string>();string? opened=null;bool complete=false;
        var flow=new NearAccountConnection((method,payload)=>{
            Assert.Equal("native_wallet_flow",method);
            using var request=JsonDocument.Parse(payload);var action=request.RootElement.GetProperty("action").GetString();calls.Add(action!);
            object view=action switch {
                "open"=>new{flow_id="fixture",state="Idle",can_check=true,can_edit=true},
                "check"=>new{flow_id="fixture",state="Ready",can_check=true,can_edit=true,can_start=true},
                "start"=>new{flow_id="fixture",state="WaitingForWallet",busy=true,can_cancel=true,wait=true,browser_url="https://commons.example/exact?token=synthetic"},
                "wait"=>new{flow_id="fixture",state="Complete"},
                _=>throw new Exception("unexpected")
            };return Task.FromResult(Response(view));
        },uri=>{opened=uri.AbsoluteUri;return Task.FromResult(true);},()=>true);
        flow.Completed+=()=>complete=true;
        await flow.InitializeAsync();flow.Commons="https://commons.example";await flow.CheckAsync();flow.Account="synthetic.near";await flow.StartAsync();
        Assert.True(complete);Assert.Empty(flow.Account);Assert.Equal("https://commons.example/exact?token=synthetic",opened);
        Assert.Equal(new[]{"open","check","start","wait"},calls);
    }
    [Theory]
    [InlineData(false)]
    [InlineData(true)]
    public async Task BrowserRefusalUsesCoreCancel(bool throws)
    {
        var cancelled=false;
        var flow=new NearAccountConnection((_,payload)=>{
            using var request=JsonDocument.Parse(payload);var action=request.RootElement.GetProperty("action").GetString();
            if(action=="cancel")cancelled=true;
            return Task.FromResult(Response(action=="start"?(object)new{flow_id="fixture",state="WaitingForWallet",wait=true,browser_url="https://commons.example"}:new{flow_id="fixture",state="Ready",can_start=true}));
        },_=>throws ? Task.FromException<bool>(new InvalidOperationException()) : Task.FromResult(false),()=>true);
        await flow.InitializeAsync();await flow.StartAsync();Assert.True(cancelled);
    }
    [Fact]
    public async Task FailedWaitStopsWithoutAutomaticRetryAndKeepsCancelAvailable()
    {
        var waits = 0;
        var flow = new NearAccountConnection((_, payload) => {
            using var request = JsonDocument.Parse(payload);
            var action = request.RootElement.GetProperty("action").GetString();
            if (action == "wait") { waits++; return Task.FromResult(new DaemonResponse()); }
            return Task.FromResult(Response(action == "start"
                ? (object)new { flow_id = "fixture", state = "WaitingForWallet", wait = true, can_cancel = true }
                : new { flow_id = "fixture", state = "Ready", can_start = true }));
        }, _ => Task.FromResult(true), () => true);
        await flow.InitializeAsync(); await flow.StartAsync();
        Assert.Equal(1, waits); Assert.True(flow.CanCancel); Assert.True(flow.Refused);
    }
    [Fact]
    public async Task UnavailableCoreDoesNotInventReadiness()
    {
        var flow=new NearAccountConnection((_,_)=>Task.FromResult(DaemonResponse.Parse("{\"error\":{\"code\":\"unknown\",\"message\":\"unknown\"}}")),_=>Task.FromResult(true),()=>true);
        await flow.InitializeAsync();Assert.False(flow.Supported);Assert.False(flow.CanStart);Assert.False(flow.CanCheck);
    }
}
