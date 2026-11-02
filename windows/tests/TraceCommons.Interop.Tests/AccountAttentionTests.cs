using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The account section's "needs attention" status: <c>status</c> handed to
/// the ABI verbatim, and the words it returns.
/// </summary>
public sealed class AccountAttentionTests
{
    private const string Unreadable =
        "{\"logged_in\":true,\"account_state\":\"needs_attention\","
        + "\"account_state_reason\":\"account_record_unreadable\",\"near_ai_login_unconfirmed\":false}";

    private const string RefusedWithLogin =
        "{\"logged_in\":true,\"account_state\":\"needs_attention\","
        + "\"account_state_reason\":\"account_address_refused\",\"near_ai_login_unconfirmed\":true}";

    [Fact]
    public void TheStatusGoesToTheAbiVerbatim()
    {
        var seen = new List<string>();
        AccountAttention? attention = AccountAttentions.For(Unreadable, wire =>
        {
            seen.Add(wire);
            return "{\"title\":\"T\",\"body\":\"B\",\"private_ai\":null,\"action\":\"A\"}";
        });

        Assert.NotNull(attention);
        Assert.Equal("T", attention!.Title);
        Assert.Equal("B", attention.Body);
        Assert.Null(attention.PrivateAi);
        Assert.Equal("A", attention.Action);
        Assert.Equal(Unreadable, Assert.Single(seen));
    }

    [Fact]
    public void ABlockWithoutTitleOrButtonIsNotShown()
    {
        Assert.Null(AccountAttentions.For(Unreadable, _ => "{\"title\":\"\",\"action\":\"A\"}"));
        Assert.Null(AccountAttentions.For(Unreadable, _ => "{\"title\":\"T\",\"action\":\"\"}"));
        Assert.Null(AccountAttentions.For(Unreadable, _ => "not json"));
        Assert.Null(AccountAttentions.For(Unreadable, _ => null));
        Assert.Null(AccountAttentions.For(null, _ => throw new InvalidOperationException()));
    }

    /// <summary>
    /// Through the real ABI: each reason gets its own sentence, the Private
    /// AI line follows the stored login, and the button is "Set up again".
    /// </summary>
    [Fact]
    public void TheLiveAbiWordsBothReasons()
    {
        AccountAttention unreadable = Assert.IsType<AccountAttention>(AccountAttentions.For(Unreadable));
        AccountAttention refused = Assert.IsType<AccountAttention>(AccountAttentions.For(RefusedWithLogin));

        Assert.Equal("This app's account needs attention", unreadable.Title);
        Assert.Equal(unreadable.Title, refused.Title);
        Assert.Equal(
            "This app can't read its account details on this Mac. Set it up again to keep sending traces.",
            unreadable.Body);
        Assert.Equal(
            "This app can no longer reach the service it was set up with. Update the app, or set it up again.",
            refused.Body);
        Assert.Null(unreadable.PrivateAi);
        Assert.Equal("Private AI still works with your NEAR AI account.", refused.PrivateAi);
        Assert.Equal("Set up again", unreadable.Action);
    }

    /// <summary>
    /// A healthy account and a daemon too old to send the keys show nothing.
    /// </summary>
    [Fact]
    public void AHealthyAccountAndAnOlderDaemonShowNothing()
    {
        Assert.Null(AccountAttentions.For(
            "{\"logged_in\":true,\"account_state\":\"ok\",\"account_state_reason\":null,"
            + "\"near_ai_login_unconfirmed\":false}"));
        Assert.Null(AccountAttentions.For("{\"logged_in\":true,\"paused\":false}"));
        Assert.Null(AccountAttentions.For("null"));
    }

    [Fact]
    public void TheExportedFieldSetIsExactlyTheConsumedFields()
    {
        string json = NativeMethods.TakeOwnedString(NativeMethods.tc_account_attention_copy_json(RefusedWithLogin))
            ?? throw new InvalidOperationException("tc_account_attention_copy_json returned NULL");
        var keys = JsonDocument.Parse(json).RootElement.EnumerateObject().Select(p => p.Name).OrderBy(k => k);
        Assert.Equal(AccountAttention.ConsumedFields.OrderBy(k => k), keys);
    }
}
