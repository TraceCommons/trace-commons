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

    private const string Whole =
        "{\"title\":\"T\",\"body\":\"B\",\"private_ai\":null,\"action\":\"A\","
        + "\"confirm_title\":\"CT\",\"confirm_body\":\"CB\",\"confirm_action\":\"CA\","
        + "\"confirm_cancel\":\"CC\",\"failed\":\"F\"}";

    [Fact]
    public void TheStatusGoesToTheAbiVerbatim()
    {
        var seen = new List<string>();
        AccountAttention? attention = AccountAttentions.For(Unreadable, wire =>
        {
            seen.Add(wire);
            return Whole;
        });

        Assert.NotNull(attention);
        Assert.Equal("T", attention!.Title);
        Assert.Equal("B", attention.Body);
        Assert.Null(attention.PrivateAi);
        Assert.Equal("A", attention.Action);
        Assert.Equal("CT", attention.ConfirmTitle);
        Assert.Equal("CB", attention.ConfirmBody);
        Assert.Equal("CA", attention.ConfirmAction);
        Assert.Equal("CC", attention.ConfirmCancel);
        Assert.Equal("F", attention.Failed);
        Assert.Equal(Unreadable, Assert.Single(seen));
    }

    /// <summary>
    /// Only the three account keys reach the ABI, never the whole status,
    /// and a status without them sends nothing.
    /// </summary>
    [Fact]
    public void OnlyTheThreeKeysAreSent()
    {
        using JsonDocument status = JsonDocument.Parse(
            "{\"logged_in\":true,\"queue_depth\":4,\"account_state\":\"needs_attention\","
            + "\"account_state_reason\":\"account_record_unreadable\",\"near_ai_login_unconfirmed\":true}");
        string keys = Assert.IsType<string>(AccountAttentions.Keys(status.RootElement));
        var names = JsonDocument.Parse(keys).RootElement.EnumerateObject().Select(p => p.Name).OrderBy(k => k);
        Assert.Equal(AccountAttentions.StatusKeys.OrderBy(k => k), names);
        Assert.NotNull(AccountAttentions.For(keys));

        using JsonDocument old = JsonDocument.Parse("{\"logged_in\":true,\"paused\":false}");
        Assert.Null(AccountAttentions.Keys(old.RootElement));
        using JsonDocument notObject = JsonDocument.Parse("[]");
        Assert.Null(AccountAttentions.Keys(notObject.RootElement));
    }

    [Fact]
    public void ABlockWithoutTitleOrButtonIsNotShown()
    {
        Assert.Null(AccountAttentions.For(Unreadable, _ => Whole.Replace("\"title\":\"T\"", "\"title\":\"\"")));
        Assert.Null(AccountAttentions.For(Unreadable, _ => Whole.Replace("\"action\":\"A\"", "\"action\":\"\"")));
        Assert.Null(AccountAttentions.For(Unreadable, _ => Whole.Replace("\"confirm_body\":\"CB\"", "\"confirm_body\":\"\"")));
        Assert.Null(AccountAttentions.For(Unreadable, _ => Whole.Replace("\"failed\":\"F\",", "").Replace(",\"failed\":\"F\"", "")));
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

        Assert.Equal("Needs attention", unreadable.Title);
        Assert.Equal(unreadable.Title, refused.Title);
        Assert.Equal(
            "Can't read the account details, so traces can't be sent.",
            unreadable.Body);
        Assert.Equal(
            "Can no longer reach the service this app was set up with. An update may fix it.",
            refused.Body);
        Assert.Null(unreadable.PrivateAi);
        Assert.Equal("Private AI still works with your NEAR AI account.", refused.PrivateAi);
        Assert.Equal("Set up again", unreadable.Action);
    }

    /// <summary>
    /// Through the real ABI: the owner-approved confirmation and the
    /// failure line arrive with the block.
    /// </summary>
    [Fact]
    public void TheLiveAbiCarriesTheConfirmationAndFailureLine()
    {
        AccountAttention attention = Assert.IsType<AccountAttention>(AccountAttentions.For(Unreadable));
        Assert.Equal("Set up this app again?", attention.ConfirmTitle);
        Assert.Equal(
            "This signs out of NEAR AI on this computer. You'll sign in again to keep sending traces.",
            attention.ConfirmBody);
        Assert.Equal("Set up again", attention.ConfirmAction);
        Assert.Equal("Cancel", attention.ConfirmCancel);
        // Owner dropped "Try again." 2026-11-04 (Settings copy pass 2, P57).
        Assert.Equal("Nothing changed.", attention.Failed);
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
