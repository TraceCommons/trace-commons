using System;
using System.Collections.Generic;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// The one action a shell may offer for one credential state.
/// </summary>
/// <remarks>
/// The ABI numbering these decode from is a range of its own, disjoint from
/// every tone range for the reason those are disjoint from each other: a
/// shell that cross-wired an action onto a tone mapper would be drawing a
/// button from a colour. <see cref="None"/> is the safe direction and not by
/// analogy -- <see cref="Obtain"/> opens a browser and mints a key at a third
/// party, so drawing it for a state nobody could read is how a contributor
/// ends up holding a second key.
/// </remarks>
public enum CredentialAction
{
    /// <summary>Draw no action. Nothing is known well enough to offer one.</summary>
    None,

    /// <summary>Start the ceremony. The only action that opens a browser.</summary>
    Obtain,

    /// <summary>Stop waiting on the browser.</summary>
    Cancel,

    /// <summary>Remove the stored key from this machine.</summary>
    Forget,
}

/// <summary>
/// What <c>near_ai_credential_status</c> reported.
/// </summary>
/// <remarks>
/// The state is carried as the daemon's own string and handed to the shared
/// table, for the reason <see cref="PrivateInferenceState"/> carries its
/// label that way: a state a later daemon grows would otherwise have to be
/// spelled in this shell before it could be shown, and the shared table
/// already answers an unfamiliar label safely.
///
/// <para>
/// <see cref="AttemptStatus"/> is the ceremony's own lifecycle word and
/// arrives only when the caller named the CURRENT attempt. The wire field is
/// <c>attempt_status</c> rather than <c>status</c> deliberately: a field
/// called <c>status</c> beside one called <c>state</c> is a shell reading the
/// wrong one.
/// </para>
/// </remarks>
public readonly record struct NearAiCredentialStatus(
    string State,
    string? AttemptId,
    string? AttemptStatus,
    string SessionState = "")
{
    /// <summary>
    /// Nothing was reported. The empty state, which the shared table answers
    /// with the sentence saying this daemon does not answer the question --
    /// never with the one saying no key is kept here.
    /// </summary>
    public static NearAiCredentialStatus Unreported => new(string.Empty, null, null);
}

/// <summary>
/// The NEAR AI credential surface, across the C ABI.
///
/// Holds no words and owns no branch. Every sentence, the tone and the action
/// come from
/// <c>crates/trace-commons-contributor/src/private_inference_copy.rs</c>.
/// </summary>
public static class NearAiCredentialSurface
{
    /// <summary>
    /// What the daemon reported, or <see cref="NearAiCredentialStatus.Unreported"/>
    /// when the answer could not be read.
    /// </summary>
    /// <remarks>
    /// A malformed or missing reply is unreported and NOT absent. The two
    /// sentences differ precisely where it matters: one says this daemon did
    /// not answer, the other claims that nothing is stored on this machine,
    /// and a shell that reached the second from a failed call would invite a
    /// second sign-in over a key that is already here.
    /// </remarks>
    public static NearAiCredentialStatus ParseStatus(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return NearAiCredentialStatus.Unreported;
        }

        try
        {
            using JsonDocument document = JsonDocument.Parse(json);
            if (document.RootElement.ValueKind != JsonValueKind.Object)
            {
                return NearAiCredentialStatus.Unreported;
            }

            return new NearAiCredentialStatus(
                ReadString(document.RootElement, StateField) ?? string.Empty,
                ReadString(document.RootElement, AttemptIdField),
                ReadString(document.RootElement, AttemptStatusField),
                ReadString(document.RootElement, "session_state") ?? string.Empty);
        }
        catch (JsonException)
        {
            return NearAiCredentialStatus.Unreported;
        }
    }

    /// <summary>
    /// Where to open the browser, handed back by <c>near_ai_credential_start</c>
    /// together with the attempt it belongs to. Null when either is missing.
    /// </summary>
    /// <remarks>
    /// THIS COMES ONCE. No poll re-serves it, so a shell that lost it has to
    /// start a second ceremony to get another -- a second browser tab in
    /// front of somebody already looking at one. The two are returned
    /// together because an attempt id without its URL is a ceremony nobody
    /// can finish, and a URL without its id is one nobody can cancel.
    /// </remarks>
    public static NearAiCredentialAttempt? ParseStart(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            using JsonDocument document = JsonDocument.Parse(json);
            if (document.RootElement.ValueKind != JsonValueKind.Object)
            {
                return null;
            }

            string? attempt = ReadString(document.RootElement, AttemptIdField);
            string? url = ReadString(document.RootElement, BrowserUrlField);
            return attempt is { Length: > 0 } && url is { Length: > 0 }
                ? new NearAiCredentialAttempt(attempt, url)
                : null;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>
    /// The sentence for one state. Falls back to the payload's could-not-read
    /// sentence when the Rust caught a panic -- never to the one saying no
    /// key is kept here.
    /// </summary>
    public static string StateLine(NearAiCredentialStatus status, PrivateInferenceCopy copy)
    {
        ArgumentNullException.ThrowIfNull(copy);
        return NativeMethods.TakeOwnedString(
                NativeMethods.tc_near_ai_credential_state_line(status.State))
            ?? copy.CredentialUnknown;
    }

    /// <summary>The tone that sentence is painted in.</summary>
    public static PrivateInferenceTone Tone(NearAiCredentialStatus status) =>
        PrivateInferenceSurface.FromAbiTone(
            NativeMethods.tc_near_ai_credential_state_tone(status.State));

    /// <summary>The one action a shell may offer for this state.</summary>
    public static CredentialAction Action(NearAiCredentialStatus status) =>
        FromAbiAction(NativeMethods.tc_near_ai_credential_action(status.State));

    /// <summary>The words on that action's control, or null where there is none.</summary>
    public static string? ActionLabel(CredentialAction action, PrivateInferenceCopy? copy) =>
        copy is null
            ? null
            : action switch
            {
                CredentialAction.Obtain => copy.CredentialObtain,
                CredentialAction.Cancel => copy.CredentialCancel,
                CredentialAction.Forget => copy.CredentialForget,
                _ => null,
            };

    /// <summary>
    /// The sentence that must be on screen wherever that action is offered,
    /// or null where the action carries no consequence to state.
    /// </summary>
    /// <remarks>
    /// NOT A DECORATION AND NOT OPTIONAL. Obtain opens a browser, signs the
    /// contributor in with a company that is not this app, and mints a key
    /// that is then kept here; forget removes a key locally while leaving it
    /// valid at the service until the contributor removes it in their own
    /// account. Pairing each consequence with its own button HERE, once, is
    /// what stops three shells each deciding that a button reads well enough
    /// on its own.
    /// </remarks>
    public static string? ActionPreamble(CredentialAction action, PrivateInferenceCopy? copy) =>
        copy is null
            ? null
            : action switch
            {
                CredentialAction.Obtain => copy.CredentialCost,
                CredentialAction.Forget => copy.CredentialForgetExplains,
                _ => null,
            };

    /// <summary>
    /// The <c>near_ai_credential_cancel</c> body: the attempt id when this
    /// shell holds one, the empty body when it does not.
    /// </summary>
    /// <remarks>
    /// The daemon accepts an unnamed cancel and stops whatever sign-in it is
    /// running, so a shell that cannot name the attempt still sends one. The
    /// field is OMITTED rather than sent empty -- an empty id names no running
    /// attempt and would be refused exactly as a wrong one is.
    /// </remarks>
    public static string SerializeCancel(string? attemptId) =>
        attemptId is { Length: > 0 }
            ? JsonSerializer.Serialize(
                new Dictionary<string, string> { [AttemptIdField] = attemptId })
            : EmptyParams;

    /// <summary>
    /// The <c>near_ai_credential_status</c> body for a caller that can name
    /// the attempt, or the empty body for one that cannot.
    /// </summary>
    /// <remarks>
    /// The resting state comes back either way. Naming the attempt is what
    /// buys back <c>attempt_status</c>, and it is echoed only to a caller
    /// that already knew the id.
    /// </remarks>
    public static string SerializeStatus(string? attemptId) =>
        attemptId is { Length: > 0 }
            ? JsonSerializer.Serialize(
                new Dictionary<string, string> { [AttemptIdField] = attemptId })
            : EmptyParams;

    /// <summary>
    /// Whether the poll should keep going: the ceremony is still in flight.
    /// </summary>
    /// <remarks>
    /// Asked off the ACTION rather than off the label, so this shell holds no
    /// arm of the state table. Cancel is the action for exactly the one state
    /// that is still under way, which is the same state a poll is waiting on.
    /// </remarks>
    public static bool AwaitingBrowser(NearAiCredentialStatus status) =>
        Action(status) == CredentialAction.Cancel;

    /// <summary>
    /// The ABI value, spelled out rather than cast.
    ///
    /// Anything unknown is <see cref="CredentialAction.None"/>. That is the
    /// whole point of the range being its own: a tone value arriving here,
    /// through a cross-wiring or a later ABI, must offer nothing rather than
    /// land on obtain by arithmetic.
    /// </summary>
    internal static CredentialAction FromAbiAction(int value) =>
        value switch
        {
            AbiActionNone => CredentialAction.None,
            AbiActionObtain => CredentialAction.Obtain,
            AbiActionCancel => CredentialAction.Cancel,
            AbiActionForget => CredentialAction.Forget,
            _ => CredentialAction.None,
        };

    private static string? ReadString(JsonElement element, string name) =>
        element.TryGetProperty(name, out JsonElement value)
        && value.ValueKind == JsonValueKind.String
            ? value.GetString()
            : null;

    private const string StateField = "state";
    private const string AttemptIdField = "attempt_id";
    private const string AttemptStatusField = "attempt_status";
    private const string BrowserUrlField = "browser_url";
    private const string EmptyParams = "{}";

    private const int AbiActionNone = 30;
    private const int AbiActionObtain = 31;
    private const int AbiActionCancel = 32;
    private const int AbiActionForget = 33;
}

/// <summary>
/// A ceremony that was started: the attempt to poll and cancel by, and the
/// one URL to open. Both, or neither.
/// </summary>
public readonly record struct NearAiCredentialAttempt(string AttemptId, string BrowserUrl);
