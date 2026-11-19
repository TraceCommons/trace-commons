using System.Text.Json;
using System.Text.Json.Serialization;

namespace TraceCommons.Interop;

public sealed record OnboardingCopy
{
    [JsonPropertyName("welcome_body")] public string WelcomeBody { get; init; } = "";
    [JsonPropertyName("done_body")] public string DoneBody { get; init; } = "";
    [JsonPropertyName("notification_purpose")] public string NotificationPurpose { get; init; } = "";
    [JsonPropertyName("notification_heading")] public string NotificationHeading { get; init; } = "";
    [JsonPropertyName("notification_offer")] public string NotificationOffer { get; init; } = "";
    [JsonPropertyName("notification_allowed")] public string NotificationAllowed { get; init; } = "";
    [JsonPropertyName("notification_denied")] public string NotificationDenied { get; init; } = "";
    [JsonPropertyName("notification_unknown")] public string NotificationUnknown { get; init; } = "";
    [JsonPropertyName("notification_not_asked")] public string NotificationNotAsked { get; init; } = "";
    [JsonPropertyName("notification_allow")] public string NotificationAllow { get; init; } = "";
    [JsonPropertyName("not_now")] public string NotNow { get; init; } = "";
    [JsonPropertyName("system_settings")] public string SystemSettings { get; init; } = "";
    [JsonPropertyName("watcher_start_failed")] public string WatcherStartFailed { get; init; } = "";

    public static OnboardingCopy? Load()
    {
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_onboarding_copy());
        if (string.IsNullOrEmpty(json)) return null;
        try { return JsonSerializer.Deserialize<OnboardingCopy>(json); }
        catch (JsonException) { return null; }
    }
}
