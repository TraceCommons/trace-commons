using System;
using System.Collections.Generic;
using System.Text.Json;
using System.Threading.Tasks;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

public class AccountSettingsTests
{
    [Fact]
    public async Task NearAiSignInRestoresCredentialsAndRereadsTheAccountSession()
    {
        var calls = new List<string>();
        var answers = new Queue<string>(new[]
        {
            """{"result":{"sign_in":"near_ai","signed_in":false}}""",
            """{"error":{"code":"unavailable","message":"near_ai_sign_in_required"}}""",
            """{"result":{"signed_in":true}}""",
            """{"result":{"signed_in":true}}""",
        });
        var client = new AccountSettingsClient((method, _) =>
        {
            calls.Add(method);
            return Task.FromResult(DaemonResponse.Parse(answers.Dequeue()));
        });

        Assert.True(await client.SignInAsync(() =>
        {
            calls.Add("browser");
            return Task.FromResult(true);
        }));

        Assert.Equal(new[] { "account_session_status", "near_ai_account_sign_in", "browser", "near_ai_account_sign_in", "account_session_status" }, calls);
    }

    [Fact]
    public async Task SignOutNeedsAnAffirmativeReply()
    {
        var client = new AccountSettingsClient((_, _) => Task.FromResult(DaemonResponse.Parse("""{"result":{"signed_out":false}}""")));
        Assert.Null(await client.SignOutAsync());
    }

    [Fact]
    public async Task InviteRedemptionStaysScopedToTheAccountAndUsesTheRetryKey()
    {
        var key = Guid.NewGuid();
        var client = new AccountSettingsClient((method, parameters) =>
        {
            Assert.Equal("account_invite_redeem", method);
            using JsonDocument args = JsonDocument.Parse(parameters);
            Assert.Equal("invite", args.RootElement.GetProperty("invite_code").GetString());
            Assert.Equal(key, args.RootElement.GetProperty("idempotency_key").GetGuid());
            Assert.Equal("pressed-account", args.RootElement.GetProperty("account_scope").GetString());
            return Task.FromResult(DaemonResponse.Parse("""{"result":{"account_scope":"another-account","line":"unrelated"}}"""));
        });

        Assert.Null(await client.ContributionAsync("pressed-account", "invite", key));
    }
}
