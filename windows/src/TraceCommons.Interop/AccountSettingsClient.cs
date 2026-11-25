using System;
using System.Text.Json;
using System.Text.Json.Serialization;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

public sealed record SetUpAgainCopy(
    [property: JsonPropertyName("action")] string Action,
    [property: JsonPropertyName("confirm_title")] string ConfirmTitle,
    [property: JsonPropertyName("confirm_body")] string ConfirmBody,
    [property: JsonPropertyName("confirm_action")] string ConfirmAction,
    [property: JsonPropertyName("confirm_cancel")] string ConfirmCancel,
    [property: JsonPropertyName("failed")] string Failed)
{
    public static SetUpAgainCopy? Load()
    {
        try
        {
            string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_set_up_again_copy_json());
            return json is null ? null : JsonSerializer.Deserialize<SetUpAgainCopy>(json);
        }
        catch (JsonException)
        {
            return null;
        }
    }
}

/// <summary>The Commons account operations shared by Settings' sign-in entry points.</summary>
public sealed class AccountSettingsClient(Func<string, string, Task<DaemonResponse>> call)
{
    private const string SessionStatus = "account_session_status";
    private const string NearAiSignIn = "near_ai_account_sign_in";

    public async Task<bool> SignInAsync(Func<Task<bool>> refreshNearAiCredential)
    {
        DaemonResponse before = await call(SessionStatus, "{}");
        if (before.IsError || before.Result is not { ValueKind: JsonValueKind.Object } session)
        {
            return false;
        }

        string method = Text(session, "sign_in") == "near_ai" ? NearAiSignIn : "account_sign_in";
        DaemonResponse result = await call(method, "{}");
        if (method == NearAiSignIn && result.Error?.Message == "near_ai_sign_in_required")
        {
            if (!await refreshNearAiCredential())
            {
                return false;
            }

            result = await call(method, "{}");
        }

        if (result.IsError)
        {
            return false;
        }

        DaemonResponse after = await call(SessionStatus, "{}");
        return IsTrue(after, "signed_in");
    }

    public async Task<DaemonResponse?> SignOutAsync()
    {
        DaemonResponse result = await call("account_sign_out", "{}");
        return IsTrue(result, "signed_out") ? result : null;
    }

    public async Task<string?> ContributionAsync(string scope, string? code, Guid? idempotencyKey)
    {
        DaemonResponse response = code is null
            ? await call("account_contribution_status", JsonSerializer.Serialize(new { account_scope = scope }))
            : await call("account_invite_redeem", JsonSerializer.Serialize(new { invite_code = code, idempotency_key = idempotencyKey, account_scope = scope }));

        return !response.IsError && response.Result is { ValueKind: JsonValueKind.Object } result && Text(result, "account_scope") == scope
            ? Text(result, "line")
            : null;
    }

    private static bool IsTrue(DaemonResponse response, string name) =>
        !response.IsError && response.Result is { ValueKind: JsonValueKind.Object } result
        && result.TryGetProperty(name, out JsonElement value) && value.ValueKind == JsonValueKind.True;

    private static string? Text(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement text) && text.ValueKind == JsonValueKind.String ? text.GetString() : null;
}
