using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Devices section (W16-D3 (a)) against the Mac's
/// <c>AccountDevicesSurface</c>.
/// </summary>
/// <remarks>
/// Expected values were printed by a throwaway Swift probe that compiles
/// macos/Sources/TCShellCore/AccountDevicesSurface.swift verbatim at
/// b8d4a6d70, linked to this worktree's FFI build, over the live
/// <c>tc_account_devices_copy_json</c>.
/// </remarks>
public class AccountDevicesTests
{
    private static AccountDevicesSurface Surface() => new(AccountDevicesCopy.Live!);

    private static readonly AccountDevice A = new("a", Label: "  ", LastSeenAt: "2026-11-19T10:00:00Z");
    private static readonly AccountDevice B = new("b", Label: " Build box ", ThisDevice: true);
    private static readonly AccountDevice C = new("c", Label: "Laptop");

    [Fact]
    public void TheLiveCopyDecodesEveryFieldTheMacReads()
    {
        // probe: JSONKEYS lists exactly consumedFields.
        Assert.Equal(
            "body,cancel_button,code_command,copied,copy_button,done_button,expired,failed,last_seen,link_accessibility_name,link_body,link_button,linked,session_ended,this_device,title,too_many,unlink_accessibility_name,unlink_button,unlink_confirm,unlink_confirm_accessibility_name,unlink_confirm_button,unnamed,waiting",
            string.Join(",", AccountDevicesCopy.ConsumedFields.OrderBy(k => k, StringComparer.Ordinal)));
        AccountDevicesCopy copy = AccountDevicesCopy.Live!;
        Assert.Equal("Devices", copy.Title);
        Assert.Equal("Last seen {when}", copy.LastSeen);
        Assert.Equal("Nothing changed.", copy.Failed);
    }

    [Fact]
    public void ABlankFieldRefusesTheWholeCopyButALineBreakDoesNot()
    {
        // probe: BLANK true (copied "  "), NEWLINE false (copied "\n"):
        // Swift trims .whitespaces, which holds no line break.
        string json = NativeMethods.TakeOwnedString(NativeMethods.tc_account_devices_copy_json())!;
        Assert.Null(AccountDevicesCopy.Decode(json.Replace("\"copied\":\"Copied\"", "\"copied\":\"  \"")));
        Assert.Contains("\"copied\":\"Copied\"", json);
        Assert.NotNull(AccountDevicesCopy.Decode(json.Replace("\"copied\":\"Copied\"", "\"copied\":\"\\n\"")));
        Assert.Null(AccountDevicesCopy.Decode("[]"));
    }

    [Fact]
    public void NamesLinesAndOrderMatchTheMac()
    {
        AccountDevicesSurface s = Surface();
        // probe: NAME, LASTSEEN, ORDER, CANUNLINK, CONFIRM, UNLINKAX, CONFIRMAX, COMMAND
        Assert.Equal("Unnamed device", s.Name(A));
        Assert.Equal("Build box", s.Name(B));
        Assert.Equal("Last seen <2026-11-19T10:00:00Z>", s.LastSeenLine(A, at => $"<{at}>"));
        Assert.Null(s.LastSeenLine(B, at => $"<{at}>"));
        Assert.Equal("b,a,c", string.Join(",", AccountDevicesSurface.Ordered(new[] { A, B, C }).Select(d => d.DeviceKeyId)));
        Assert.True(AccountDevicesSurface.CanUnlink(A));
        Assert.False(AccountDevicesSurface.CanUnlink(B));
        Assert.Equal("Unlink Laptop? It's signed out and can't upload.", s.UnlinkConfirm(C));
        Assert.Equal("Unlink Unnamed device", s.UnlinkAccessibilityName(A));
        Assert.Equal("Confirm: unlink Laptop", s.UnlinkConfirmAccessibilityName(C));
        Assert.Equal(
            "trace-commons-contributor login --code ABCD-EFGH",
            s.Command(new DeviceCode("k1", "ABCD-EFGH", "2026-11-19T12:00:00.250Z")));
    }

    [Theory]
    // probe: EXPIRY lines (seconds since 1970).
    [InlineData("2026-11-19T12:00:00.250Z", 1795089600.25)]
    [InlineData("2026-11-19T12:00:00Z", 1795089600.0)]
    [InlineData("2026-11-19T12:00:00+02:00", 1795082400.0)]
    [InlineData("tomorrow", null)]
    public void ExpiryIsReadAsTheMacReadsIt(string text, double? seconds)
    {
        DateTimeOffset? at = new DeviceCode("x", "y", text).ExpiryDate;
        Assert.Equal(seconds, at is { } v ? v.ToUnixTimeMilliseconds() / 1000.0 : null);
    }

    [Theory]
    // probe: REFUSAL lines.
    [InlineData("account-session-required", AccountDevicesRefusal.SessionEnded)]
    [InlineData("device_code_too_many", AccountDevicesRefusal.TooMany)]
    [InlineData("account_devices_rate_limited", AccountDevicesRefusal.TooMany)]
    [InlineData("unparseable-response", AccountDevicesRefusal.Failed)]
    [InlineData("", AccountDevicesRefusal.Failed)]
    public void RefusalsRouteFromTheDaemonsLabel(string label, AccountDevicesRefusal want)
    {
        Assert.Equal(want, AccountDevicesRefusals.From(label));
    }

    [Fact]
    public void TheLinkPhaseSettlesOnlyForTheCodeOnScreen()
    {
        var code = new DeviceCode("k1", "ABCD-EFGH", "2026-11-19T12:00:00.250Z");
        DeviceLinkPhase w = new DeviceLinkPhase.Waiting(code);
        // probe: AFTER, MISSED, CHECKED lines.
        Assert.Equal(new DeviceLinkPhase.Linked("d9"), w.After(new DeviceCodeStatus("redeemed", "d9"), "k1"));
        Assert.Equal(new DeviceLinkPhase.Expired(), w.After(new DeviceCodeStatus("expired"), "k1"));
        Assert.Same(w, w.After(new DeviceCodeStatus("pending"), "k1"));
        Assert.Same(w, w.After(new DeviceCodeStatus("redeemed"), "k2"));
        Assert.Equal(new DeviceLinkPhase.Idle(), new DeviceLinkPhase.Idle().After(new DeviceCodeStatus("redeemed"), "k1"));
        Assert.Same(w, w.AfterMissed(4, "k1"));
        Assert.Equal(new DeviceLinkPhase.Refused(AccountDevicesRefusal.Failed), w.AfterMissed(5, "k1"));
        Assert.Same(w, w.AfterMissed(9, "k2"));
        DateTimeOffset expiry = code.ExpiryDate!.Value;
        Assert.Same(w, w.Checked(expiry.AddMilliseconds(-1), "k1"));
        Assert.Equal(new DeviceLinkPhase.Expired(), w.Checked(expiry, "k1"));
        Assert.Same(w, w.Checked(expiry.AddSeconds(9), "k2"));
    }

    [Fact]
    public async Task TheClientSendsTheMacsMethodsAndParameters()
    {
        var sent = new List<(string Method, string Params)>();
        Task<DaemonResponse> Call(string method, string parameters)
        {
            sent.Add((method, parameters));
            string result = method switch
            {
                "account_devices_list" => """{"devices":[{"device_key_id":"a","label":"Laptop","this_device":true,"contributed_traces":3},{"device_key_id":"b"}]}""",
                "account_device_code_mint" => """{"code_id":"k1","code":"ABCD","expires_at":"2026-11-19T12:00:00Z"}""",
                "account_device_code_status" => """{"state":"redeemed","device_key_id":"d9"}""",
                _ => "{}",
            };
            return Task.FromResult(DaemonResponse.Parse($$"""{"id":1,"result":{{result}}}"""));
        }

        var client = new AccountDevicesClient(Call);
        IReadOnlyList<AccountDevice> devices = await client.ListAsync();
        Assert.Equal(new AccountDevice("a", "Laptop", ThisDevice: true, ContributedTraces: 3), devices[0]);
        Assert.Equal(new AccountDevice("b"), devices[1]);
        Assert.Null(devices[1].ContributedTraces);
        Assert.Equal(new DeviceCode("k1", "ABCD", "2026-11-19T12:00:00Z"), await client.MintAsync(null));
        Assert.Equal(new DeviceCodeStatus("redeemed", "d9"), await client.StatusAsync("k1"));
        await client.RenameAsync("b", "Build box");
        await client.RevokeAsync("b");
        await client.MintAsync("Server");
        Assert.Equal(
            new[]
            {
                ("account_devices_list", "{}"),
                ("account_device_code_mint", "{}"),
                ("account_device_code_status", """{"code_id":"k1"}"""),
                ("account_device_rename", """{"device_key_id":"b","label":"Build box"}"""),
                ("account_device_revoke", """{"device_key_id":"b"}"""),
                ("account_device_code_mint", """{"label":"Server"}"""),
            },
            sent.ToArray());
    }

    [Fact]
    public async Task ADaemonRefusalCarriesItsLabelAndRoutes()
    {
        var client = new AccountDevicesClient((_, _) => Task.FromResult(
            DaemonResponse.Parse("""{"id":1,"error":{"code":"unavailable","message":"device_code_too_many"}}""")));
        DaemonRefusedException e = await Assert.ThrowsAsync<DaemonRefusedException>(() => client.MintAsync(null));
        Assert.Equal(AccountDevicesRefusal.TooMany, AccountDevicesRefusals.From(e.Label));
        Assert.Equal("Too many codes. Try again in a few minutes.", Surface().RefusalLine(AccountDevicesRefusals.From(e.Label)));
    }
}
