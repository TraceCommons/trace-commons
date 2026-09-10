using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text.Json;
using System.Text.RegularExpressions;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The NEAR AI credential surface as it really crosses the C ABI.
///
/// The thing under test is not the wording: it is that this shell RENDERS and
/// does not DECIDE. Which sentence, which colour and -- above all -- which
/// button belongs beside a state are three branches that live in
/// <c>private_inference_copy.rs</c>, and the button in question opens a
/// browser and mints a key at a third party.
/// </summary>
public class NearAiCredentialTests
{
    [Fact]
    public void InferenceKeyDoesNotImplyACloudSession()
    {
        Assert.Equal(string.Empty, NearAiCredentialSurface.ParseStatus("{\"state\":\"present\"}").SessionState);
        Assert.Equal("absent", NearAiCredentialSurface.ParseStatus("{\"state\":\"present\",\"session_state\":\"absent\"}").SessionState);
        Assert.Equal("present", NearAiCredentialSurface.ParseStatus("{\"state\":\"present\",\"session_state\":\"present\"}").SessionState);
        Assert.Equal(string.Empty, NearAiCredentialSurface.ParseStatus("{\"session_state\":true}").SessionState);
    }

    /// <summary>The five labels the daemon reports, spelled as it spells them.</summary>
    private static readonly string[] Reported =
    {
        "absent", "obtaining", "failed", "cancelled", "present",
    };

    /// <summary>
    /// A label from a daemon this build has never met, and the empty label a
    /// daemon that does not answer the question leaves behind.
    /// </summary>
    private static readonly string[] Unread =
    {
        "a_state_from_a_later_daemon", string.Empty,
    };

    private static PrivateInferenceCopy Copy()
    {
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        Assert.NotNull(copy);
        return copy!;
    }

    private static NearAiCredentialStatus Status(string state) => new(state, null, null);

    /// <summary>
    /// All fourteen credential sentences arrive, and each one arrives
    /// finished. A template with a hole in it would make this shell a second
    /// place the wording lives.
    /// </summary>
    [Fact]
    public void EveryCredentialSentenceArrivesFinished()
    {
        PrivateInferenceCopy copy = Copy();
        string[] credential =
        {
            copy.CredentialTitle, copy.CredentialWhat, copy.CredentialCost,
            copy.CredentialObtain, copy.CredentialCancel, copy.CredentialForget,
            copy.CredentialForgetExplains, copy.CredentialAbsent, copy.CredentialObtaining,
            copy.CredentialFailed, copy.CredentialCancelled, copy.CredentialPresent,
            copy.CredentialUnknown, copy.CredentialUnreported,
        };

        Assert.Equal(14, credential.Length);
        foreach (string sentence in credential)
        {
            Assert.False(string.IsNullOrWhiteSpace(sentence));
            foreach (string marker in new[] { "{}", "{state}", "%@", "%s", "%d" })
            {
                Assert.DoesNotContain(marker, sentence, StringComparison.Ordinal);
            }

            // Every one of them is in the complete-payload check, so a
            // payload arriving with one of them blank is refused whole rather
            // than drawn with a gap where a consequence should be.
            Assert.Contains(sentence, copy.Sentences);
        }
    }

    /// <summary>
    /// The payload has no hole for a key, and this record grows none.
    ///
    /// Asserted about the exported field names rather than about a rendered
    /// screen: the way a key reaches a screen is a field added to carry one,
    /// and a screen drawing a field that does not exist is a compile error
    /// while a field that does exist is one binding away from being drawn.
    /// </summary>
    [Fact]
    public void NoCredentialFieldCanCarryAValue()
    {
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_private_inference_copy());
        Assert.False(string.IsNullOrWhiteSpace(json));

        using JsonDocument document = JsonDocument.Parse(json!);
        var credentialFields = document.RootElement.EnumerateObject()
            .Select(property => property.Name)
            .Where(name => name.StartsWith("credential_", StringComparison.Ordinal))
            .ToList();

        Assert.Equal(14, credentialFields.Count);
        var carriers = new HashSet<string>(StringComparer.Ordinal)
        {
            "key", "keys", "secret", "token", "prefix", "account", "email", "org", "workspace",
            "id", "identity", "user", "username", "sk",
        };

        foreach (string name in credentialFields)
        {
            // Word by word, not by substring: "forget" contains "org", and a
            // substring test that had to be loosened to accommodate that is a
            // substring test nobody trusts afterwards.
            foreach (string word in name.Split('_'))
            {
                Assert.DoesNotContain(word, carriers);
            }
        }
    }

    /// <summary>
    /// Each reported state reaches its own sentence, and the five are five
    /// distinct sentences.
    /// </summary>
    /// <remarks>
    /// The distinctness is what has the teeth. Comparing each label to the
    /// payload field of the same name would keep passing if the Rust table
    /// collapsed two arms together -- and the pair most worth collapsing is
    /// "nothing is stored" against "your attempt failed two minutes ago",
    /// which are different things to be told.
    /// </remarks>
    [Fact]
    public void EachCredentialStateReachesItsOwnSentence()
    {
        PrivateInferenceCopy copy = Copy();
        Assert.Equal(copy.CredentialAbsent, NearAiCredentialSurface.StateLine(Status("absent"), copy));
        Assert.Equal(copy.CredentialObtaining, NearAiCredentialSurface.StateLine(Status("obtaining"), copy));
        Assert.Equal(copy.CredentialFailed, NearAiCredentialSurface.StateLine(Status("failed"), copy));
        Assert.Equal(copy.CredentialCancelled, NearAiCredentialSurface.StateLine(Status("cancelled"), copy));
        Assert.Equal(copy.CredentialPresent, NearAiCredentialSurface.StateLine(Status("present"), copy));

        var distinct = new HashSet<string>(
            Reported.Select(label => NearAiCredentialSurface.StateLine(Status(label), copy)),
            StringComparer.Ordinal);
        Assert.Equal(Reported.Length, distinct.Count);
    }

    /// <summary>
    /// A state this build could not read borrows nothing.
    ///
    /// Not the absent sentence above all: that one is a claim about what this
    /// machine holds, and a shell that reached it from a label it could not
    /// read would be inviting a second sign-in over a key that is already
    /// here. The unreported and unknown sentences are also distinct from each
    /// other -- "this daemon does not answer" and "the state could not be
    /// read" are different facts.
    /// </summary>
    [Fact]
    public void AnUnreadCredentialStateNeverBorrowsAKnownSentence()
    {
        PrivateInferenceCopy copy = Copy();
        var known = new HashSet<string>(
            Reported.Select(label => NearAiCredentialSurface.StateLine(Status(label), copy)),
            StringComparer.Ordinal);

        foreach (string stranger in Unread)
        {
            string line = NearAiCredentialSurface.StateLine(Status(stranger), copy);
            Assert.False(string.IsNullOrWhiteSpace(line));
            Assert.DoesNotContain(line, known);
            Assert.NotEqual(copy.CredentialAbsent, line);

            // And no action either: offering the browser here is exactly how
            // a contributor ends up holding a second key.
            Assert.Equal(CredentialAction.None, NearAiCredentialSurface.Action(Status(stranger)));
            Assert.Null(NearAiCredentialSurface.ActionLabel(
                NearAiCredentialSurface.Action(Status(stranger)), copy));
        }

        Assert.Equal(copy.CredentialUnreported, NearAiCredentialSurface.StateLine(Status(string.Empty), copy));
        Assert.Equal(
            copy.CredentialUnknown,
            NearAiCredentialSurface.StateLine(Status("a_state_from_a_later_daemon"), copy));
        Assert.NotEqual(copy.CredentialUnknown, copy.CredentialUnreported);
    }

    /// <summary>
    /// The tone is the ABI's, and exactly one state may be painted as
    /// working: the one meaning a key is kept here.
    /// </summary>
    [Fact]
    public void OnlyAStoredKeyIsPaintedClear()
    {
        Assert.Equal(PrivateInferenceTone.Clear, NearAiCredentialSurface.Tone(Status("present")));
        Assert.Equal(PrivateInferenceTone.Held, NearAiCredentialSurface.Tone(Status("obtaining")));
        Assert.Equal(PrivateInferenceTone.Refused, NearAiCredentialSurface.Tone(Status("failed")));
        Assert.Equal(PrivateInferenceTone.Neutral, NearAiCredentialSurface.Tone(Status("absent")));
        Assert.Equal(PrivateInferenceTone.Neutral, NearAiCredentialSurface.Tone(Status("cancelled")));

        foreach (string label in Reported.Where(label => label != "present").Concat(Unread))
        {
            Assert.False(
                NearAiCredentialSurface.Tone(Status(label)).ReadsAsWorking(),
                $"{label} reads as a key being kept here");
        }

        Assert.True(NearAiCredentialSurface.Tone(Status("present")).ReadsAsWorking());
    }

    /// <summary>
    /// The action is the ABI's too, and the whole table crosses rather than
    /// being re-derived here.
    /// </summary>
    [Fact]
    public void TheActionForEachStateComesFromTheAbi()
    {
        Assert.Equal(CredentialAction.Obtain, NearAiCredentialSurface.Action(Status("absent")));
        Assert.Equal(CredentialAction.Obtain, NearAiCredentialSurface.Action(Status("failed")));
        Assert.Equal(CredentialAction.Obtain, NearAiCredentialSurface.Action(Status("cancelled")));
        Assert.Equal(CredentialAction.Cancel, NearAiCredentialSurface.Action(Status("obtaining")));
        Assert.Equal(CredentialAction.Forget, NearAiCredentialSurface.Action(Status("present")));

        // The one that must never be reachable from a state nobody could read.
        foreach (string stranger in Unread)
        {
            Assert.NotEqual(CredentialAction.Obtain, NearAiCredentialSurface.Action(Status(stranger)));
        }
    }

    /// <summary>
    /// The action numbering is a range of its own, spelled out rather than
    /// cast, and no tone value decodes into it.
    /// </summary>
    /// <remarks>
    /// The cross-wiring this guards against is a shell handing a tone to the
    /// action mapper. The ranges being disjoint is what makes that wrong for
    /// every value rather than only for the dangerous one -- and here the
    /// dangerous one is <see cref="CredentialAction.Obtain"/>.
    /// </remarks>
    [Fact]
    public void AnUnknownAbiActionOffersNothingAndTheToneNumberingDoesNotDecodeHere()
    {
        Assert.Equal(CredentialAction.None, NearAiCredentialSurface.FromAbiAction(30));
        Assert.Equal(CredentialAction.Obtain, NearAiCredentialSurface.FromAbiAction(31));
        Assert.Equal(CredentialAction.Cancel, NearAiCredentialSurface.FromAbiAction(32));
        Assert.Equal(CredentialAction.Forget, NearAiCredentialSurface.FromAbiAction(33));

        // Every private-inference tone value, and a spread of strangers.
        foreach (int stranger in new[] { 0, 1, 2, 3, 10, 14, 20, 21, 22, 23, 24, 25, 29, 34, -1, 99 })
        {
            Assert.Equal(CredentialAction.None, NearAiCredentialSurface.FromAbiAction(stranger));
        }
    }

    /// <summary>
    /// The cost sentence is on screen wherever obtaining is offered, and what
    /// forgetting does not do is on screen wherever forgetting is.
    /// </summary>
    /// <remarks>
    /// Asserted over every label rather than over the two that matter, so an
    /// arm added to the action table without a consequence sentence beside it
    /// fails here rather than shipping a button that opens a browser with
    /// nothing in front of it.
    /// </remarks>
    [Fact]
    public void EveryConsequenceIsStatedBesideTheActionThatCausesIt()
    {
        PrivateInferenceCopy copy = Copy();
        foreach (string label in Reported.Concat(Unread))
        {
            CredentialAction action = NearAiCredentialSurface.Action(Status(label));
            string? preamble = NearAiCredentialSurface.ActionPreamble(action, copy);
            string? button = NearAiCredentialSurface.ActionLabel(action, copy);

            switch (action)
            {
                case CredentialAction.Obtain:
                    Assert.Equal(copy.CredentialCost, preamble);
                    Assert.Equal(copy.CredentialObtain, button);
                    break;
                case CredentialAction.Forget:
                    Assert.Equal(copy.CredentialForgetExplains, preamble);
                    Assert.Equal(copy.CredentialForget, button);
                    break;
                case CredentialAction.Cancel:
                    Assert.Equal(copy.CredentialCancel, button);
                    break;
                default:
                    Assert.Null(button);
                    Assert.Null(preamble);
                    break;
            }
        }

        // The cost sentence names all three consequences. Pinned literally,
        // because a payload field compared to itself proves nothing and this
        // is the paragraph a friendlier paraphrase would shorten.
        foreach (string consequence in new[]
        {
            // A browser opens, somebody signs in with a company that is not
            // this app, and a key is minted and kept here.
            "browser", "creates an inference key", "on this computer",
            "renewable Cloud sign-in", "can read your account and create more keys",
        })
        {
            Assert.Contains(consequence, copy.CredentialCost, StringComparison.Ordinal);
        }

        // And forgetting says what it does not do: the key keeps working
        // until the contributor removes it in their own account.
        Assert.Contains("from this computer", copy.CredentialForgetExplains, StringComparison.Ordinal);
        Assert.Contains("account", copy.CredentialForgetExplains, StringComparison.Ordinal);
    }

    /// <summary>
    /// A missing payload offers no button at all, rather than an unlabelled
    /// one.
    /// </summary>
    [Fact]
    public void AMissingPayloadOffersNoAction()
    {
        Assert.Null(NearAiCredentialSurface.ActionLabel(CredentialAction.Obtain, null));
        Assert.Null(NearAiCredentialSurface.ActionPreamble(CredentialAction.Obtain, null));
        Assert.Null(NearAiCredentialSurface.ActionPreamble(CredentialAction.Forget, null));
    }

    /// <summary>
    /// The status reply is read whole, and the ceremony's lifecycle word is
    /// read from <c>attempt_status</c>.
    /// </summary>
    /// <remarks>
    /// The second half is the point. A reply carrying <c>status</c> beside
    /// <c>state</c> is exactly the shape this field name was chosen to avoid,
    /// and a shell that read it would report a finished attempt as the state
    /// of the machine.
    /// </remarks>
    [Fact]
    public void TheAttemptFieldsAreReadAndAStatusFieldIsNot()
    {
        NearAiCredentialStatus read = NearAiCredentialSurface.ParseStatus(
            """{"state":"obtaining","attempt_id":"a1","attempt_status":"waiting_for_browser"}""");
        Assert.Equal("obtaining", read.State);
        Assert.Equal("a1", read.AttemptId);
        Assert.Equal("waiting_for_browser", read.AttemptStatus);

        NearAiCredentialStatus decoy = NearAiCredentialSurface.ParseStatus(
            """{"state":"present","status":"complete"}""");
        Assert.Equal("present", decoy.State);
        Assert.Null(decoy.AttemptStatus);
        Assert.Null(decoy.AttemptId);

        // A caller that named no attempt still gets a state, and gets neither
        // echoed back.
        NearAiCredentialStatus resting = NearAiCredentialSurface.ParseStatus("""{"state":"absent"}""");
        Assert.Equal("absent", resting.State);
        Assert.Null(resting.AttemptId);
    }

    /// <summary>
    /// A reply that could not be read is UNREPORTED and never absent.
    /// </summary>
    /// <remarks>
    /// The two sentences differ precisely where it matters: one says this
    /// daemon did not answer the question, the other claims that no key is
    /// kept on this machine. Reaching the second from a failed call is how a
    /// contributor is invited to sign in over a key they already have.
    /// </remarks>
    [Fact]
    public void AnUnreadableReplyIsUnreportedRatherThanAbsent()
    {
        PrivateInferenceCopy copy = Copy();
        foreach (string? malformed in new[] { null, string.Empty, "   ", "not json", "[]", "7" })
        {
            NearAiCredentialStatus status = NearAiCredentialSurface.ParseStatus(malformed);
            Assert.Equal(string.Empty, status.State);
            Assert.Equal(copy.CredentialUnreported, NearAiCredentialSurface.StateLine(status, copy));
            Assert.NotEqual(copy.CredentialAbsent, NearAiCredentialSurface.StateLine(status, copy));
            Assert.Equal(CredentialAction.None, NearAiCredentialSurface.Action(status));
        }

        // A reply whose state is not a string is no state at all.
        Assert.Equal(string.Empty, NearAiCredentialSurface.ParseStatus("""{"state":3}""").State);
    }

    /// <summary>
    /// The browser URL comes from start, with the attempt it belongs to, and
    /// both halves or neither.
    /// </summary>
    /// <remarks>
    /// An attempt id without its URL is a ceremony nobody can finish; a URL
    /// without its id is one nobody can cancel. And a status reply carries no
    /// URL at all, which is what stops a poll from re-serving one.
    /// </remarks>
    [Fact]
    public void TheBrowserUrlArrivesOnlyFromStartAndOnlyWithItsAttempt()
    {
        NearAiCredentialAttempt? started = NearAiCredentialSurface.ParseStart(
            """{"attempt_id":"a1","status":"waiting_for_browser","browser_url":"https://example.invalid/x"}""");
        Assert.NotNull(started);
        Assert.Equal("a1", started!.Value.AttemptId);
        Assert.Equal("https://example.invalid/x", started.Value.BrowserUrl);

        Assert.Null(NearAiCredentialSurface.ParseStart("""{"attempt_id":"a1"}"""));
        Assert.Null(NearAiCredentialSurface.ParseStart("""{"browser_url":"https://example.invalid/x"}"""));
        Assert.Null(NearAiCredentialSurface.ParseStart("""{"attempt_id":"","browser_url":"https://example.invalid/x"}"""));
        Assert.Null(NearAiCredentialSurface.ParseStart("""{"state":"obtaining","attempt_status":"waiting_for_browser"}"""));
        Assert.Null(NearAiCredentialSurface.ParseStart("not json"));
        Assert.Null(NearAiCredentialSurface.ParseStart(null));
    }

    /// <summary>
    /// A cancel names its attempt when there is one; neither call ever sends
    /// an empty id.
    /// </summary>
    [Fact]
    public void ACancelNamesTheAttemptAndAStatusNeedNot()
    {
        using JsonDocument cancel = JsonDocument.Parse(NearAiCredentialSurface.SerializeCancel("a1"));
        JsonProperty only = Assert.Single(cancel.RootElement.EnumerateObject());
        Assert.Equal("attempt_id", only.Name);
        Assert.Equal("a1", only.Value.GetString());

        using JsonDocument named = JsonDocument.Parse(NearAiCredentialSurface.SerializeStatus("a1"));
        Assert.Equal("a1", named.RootElement.GetProperty("attempt_id").GetString());

        foreach (string? none in new[] { null, string.Empty })
        {
            using JsonDocument anonymous = JsonDocument.Parse(NearAiCredentialSurface.SerializeStatus(none));
            Assert.Empty(anonymous.RootElement.EnumerateObject());
        }
    }

    /// <summary>
    /// Whether to keep polling follows the action, so this shell holds no arm
    /// of the state table -- not even the one that looks like a loop
    /// condition.
    /// </summary>
    [Fact]
    public void ThePollWaitsOnExactlyTheStateWithACancel()
    {
        Assert.True(NearAiCredentialSurface.AwaitingBrowser(Status("obtaining")));
        foreach (string settled in Reported.Where(label => label != "obtaining").Concat(Unread))
        {
            Assert.False(
                NearAiCredentialSurface.AwaitingBrowser(Status(settled)),
                $"{settled} would poll forever");
        }
    }

    /// <summary>
    /// A connect the daemon will refuse says why, and a daemon that refuses
    /// nothing says nothing.
    /// </summary>
    /// <remarks>
    /// The absent field is the arm worth the test. A shell that read
    /// <c>destination_credentialed</c> as a boolean would flatten "this
    /// daemon does not gate connects" into "no key here", and would then tell
    /// a contributor to sign in before connecting a tool they can connect
    /// right now.
    /// </remarks>
    [Fact]
    public void ARefusedConnectSaysWhyAndAnUngatedOneSaysNothing()
    {
        PrivateInferenceCopy copy = Copy();

        HarnessListing refused = HarnessSurface.ParseListing(
            """{"harnesses":[],"destination_credentialed":false}""");
        Assert.False(refused.DestinationCredentialed);
        Assert.Equal(0, refused.CredentialedAbiValue);
        Assert.Equal(copy.HarnessNeedsCredential, HarnessSurface.CredentialNotice(refused));

        // A key is here, or the destination is one the contributor runs
        // themselves. Either way there is nothing to explain.
        HarnessListing held = HarnessSurface.ParseListing(
            """{"harnesses":[],"destination_credentialed":true}""");
        Assert.True(held.DestinationCredentialed);
        Assert.Equal(1, held.CredentialedAbiValue);
        Assert.Equal(string.Empty, HarnessSurface.CredentialNotice(held));

        // The field absent, and the field present but unreadable. Both are
        // the third answer, and neither draws a sentence.
        foreach (string ungated in new[]
        {
            """{"harnesses":[]}""",
            """{"harnesses":[],"destination_credentialed":null}""",
            """{"harnesses":[],"destination_credentialed":"yes"}""",
        })
        {
            HarnessListing listing = HarnessSurface.ParseListing(ungated);
            Assert.Null(listing.DestinationCredentialed);
            Assert.True(listing.CredentialedAbiValue < 0);
            Assert.Equal(string.Empty, HarnessSurface.CredentialNotice(listing));
        }
    }

    /// <summary>
    /// That notice is drawn once, beside the rows, and read from the shared
    /// table rather than from a boolean this shell interpreted.
    /// </summary>
    [Fact]
    public void TheConnectNoticeIsDrawnOnceFromTheSharedTable()
    {
        string viewModel = ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs");
        Assert.Contains(
            "HarnessSurface.CredentialNotice(listing)", viewModel, StringComparison.Ordinal);
        Assert.DoesNotContain("DestinationCredentialed", viewModel, StringComparison.Ordinal);

        string markup = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml");
        Assert.Single(Regex.Matches(markup, Regex.Escape("ViewModel.HarnessesCredentialNotice")));

        // Above the rows rather than inside the row template, which is what
        // "once, about the destination" means in markup.
        int notice = markup.IndexOf("ViewModel.HarnessesCredentialNotice", StringComparison.Ordinal);
        int rows = markup.IndexOf("ItemsSource=\"{x:Bind ViewModel.Harnesses", StringComparison.Ordinal);
        Assert.True(notice >= 0 && rows > notice, "the notice is not drawn above the tool rows");
    }

    /// <summary>
    /// A Cancel with no attempt to name is offered, and is sent.
    /// </summary>
    /// <remarks>
    /// The ordinary case, not an edge one: <c>near_ai_credential_status</c>
    /// resolves <c>obtaining</c> from the ceremony on disk with no attempt id,
    /// so an app restarted while the daemon kept running reads <c>obtaining</c>
    /// and is offered Cancel with no id to give. The daemon accepts an unnamed
    /// cancel and stops whatever it is running, so the call goes out with the
    /// field OMITTED -- never sent empty, which names no running attempt and
    /// would be refused.
    /// </remarks>
    [Fact]
    public void ACancelWithNoAttemptToNameIsStillSent()
    {
        foreach (string? none in new[] { null, string.Empty })
        {
            using JsonDocument anonymous = JsonDocument.Parse(
                NearAiCredentialSurface.SerializeCancel(none));
            Assert.Empty(anonymous.RootElement.EnumerateObject());
        }

        // And it is a Cancel the state table really does offer: `obtaining`
        // reached this shell without an attempt id in the first place.
        NearAiCredentialStatus obtaining = Status("obtaining");
        Assert.Null(obtaining.AttemptId);
        Assert.Equal(CredentialAction.Cancel, NearAiCredentialSurface.Action(obtaining));
    }

    /// <summary>
    /// The card sends the press it holds no id for, and the browser failing to
    /// open cancels the ceremony rather than leaving it to time out.
    /// </summary>
    [Fact]
    public void TheCardSendsACancelItCannotNameAndCleansUpAFailedLaunch()
    {
        string viewModel = ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs");

        // No id-shaped guard stands between the press and the call: the whole
        // body between the method and its send must not re-read `_attemptId`
        // as a condition.
        int cancelAt = viewModel.IndexOf(
            "public async Task CancelCredentialAsync()", StringComparison.Ordinal);
        Assert.True(cancelAt >= 0, "the cancel is gone from the view model");
        string body = viewModel[cancelAt..(cancelAt + 500)];
        Assert.DoesNotContain("_attemptId is not", body, StringComparison.Ordinal);
        Assert.Contains(
            "NearAiCredentialSurface.SerializeCancel(_attemptId)", body, StringComparison.Ordinal);

        // Nor does one stand between the state and the control being live.
        Assert.Contains(
            "public bool CredentialControlsEnabled => !_credentialBusy && _copy is not null;",
            viewModel,
            StringComparison.Ordinal);

        // The state sentence is unaffected: what the call can carry does not
        // make the ceremony more or less true, and the card still says one is
        // under way.
        int stateAt = viewModel.IndexOf(
            "public string CredentialStateText =>", StringComparison.Ordinal);
        Assert.True(stateAt >= 0, "the state sentence is gone from the view model");
        Assert.DoesNotContain(
            "_attemptId",
            viewModel[stateAt..(stateAt + 300)],
            StringComparison.Ordinal);

        Assert.Contains("CredentialBrowser.ContinueAsync(attempt,", viewModel, StringComparison.Ordinal);
        Assert.Contains("Windows.System.Launcher.LaunchUriAsync(uri)", viewModel, StringComparison.Ordinal);
        Assert.Contains("CancelCredentialAsync, LoadCredentialAsync, AwaitCredentialAsync", viewModel, StringComparison.Ordinal);
    }

    /// <summary>
    /// No sentence on this surface is authored in this shell.
    ///
    /// Asserted about the source rather than about behaviour, for the reason
    /// the other strict guards give: a hand-written sentence that happened to
    /// match the Rust today would pass every behavioural test above and then
    /// survive a rename in exactly one of the three shells.
    /// </summary>
    [Fact]
    public void NoWordingIsAuthoredInTheCredentialSurface()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "NearAiCredentialSurface.cs.txt");
        Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");

        string uncommented = string.Join(
            "\n",
            File.ReadAllText(path).Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal))
                .Where(line => !line.TrimStart().StartsWith("///", StringComparison.Ordinal)));

        var allowed = new HashSet<string>(StringComparer.Ordinal)
        {
            // Wire fields, the empty parameter body, and the optional
            // session state's empty default when an older daemon omits it.
            "state", "session_state", "attempt_id", "attempt_status", "browser_url", "{}", "",
        };

        foreach (Match match in Regex.Matches(uncommented, "\"([^\"\\\\]|\\\\.)*\""))
        {
            string literal = match.Value[1..^1];
            Assert.True(
                allowed.Contains(literal),
                $"\"{literal}\" is a string literal in NearAiCredentialSurface.cs that is not a "
                + "wire value. Wording on this surface comes from private_inference_copy.rs "
                + "across the ABI.");
        }
    }

    /// <summary>
    /// The card is drawn from the shared sentences, and the button is chosen
    /// by the ABI rather than by this shell.
    /// </summary>
    /// <remarks>
    /// The composition rule is the half worth guarding: the consequence
    /// sentence and the button both come off one call, so a card cannot draw
    /// the button that opens a browser without the line saying what that
    /// costs.
    /// </remarks>
    [Fact]
    public void TheCredentialCardIsDrawnFromTheSharedSentences()
    {
        string markup = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml");
        foreach (string bound in new[]
        {
            "ViewModel.CredentialTitle",
            "ViewModel.CredentialWhat",
            "ViewModel.CredentialStateText",
            "ViewModel.CredentialActionPreamble",
            "ViewModel.HasCredentialActionPreamble",
            "ViewModel.CredentialActionText",
            "ViewModel.HasCredentialAction",
            "ViewModel.CredentialIsRefused",
        })
        {
            Assert.Contains(bound, markup, StringComparison.Ordinal);
        }

        // Every rendered string on the page is a binding, never a literal.
        foreach (Match match in Regex.Matches(markup, "(Text|Content|Header)=\"([^\"]*)\""))
        {
            Assert.StartsWith("{x:Bind", match.Groups[2].Value, StringComparison.Ordinal);
        }

        string viewModel = ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs");
        foreach (string sourced in new[]
        {
            "_copy?.CredentialTitle",
            "_copy?.CredentialWhat",
            "NearAiCredentialSurface.StateLine(_credential, _copy)",
            "NearAiCredentialSurface.Tone(_credential)",
            "NearAiCredentialSurface.Action(",
            "_requiresSession ? _credential with { State = _credential.SessionState } : _credential",
            "NearAiCredentialSurface.ActionLabel(OfferedAction, _copy)",
            "NearAiCredentialSurface.ActionPreamble(OfferedAction, _copy)",
        })
        {
            Assert.Contains(sourced, viewModel, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// Neither the colour nor the button is recovered from the rendered
    /// sentence, and neither is decided in C#.
    /// </summary>
    /// <remarks>
    /// A tone read back off the text would be read off a prefix, and an
    /// action decided here would be a fourth copy of a table that exists
    /// once. The forbidden shapes are the ones a shell reaches for when the
    /// ABI feels like a detour: comparing the state label directly, or
    /// matching on the sentence.
    /// </remarks>
    [Fact]
    public void NeitherTheToneNorTheActionIsDecidedInThisShell()
    {
        string viewModel = ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs");
        foreach (string forbidden in new[]
        {
            "CredentialStateText ==",
            "CredentialStateText.Contains",
            "CredentialStateText.StartsWith",
            "_credential.State ==",
            "_credential.State.Contains",
            "_credential.AttemptStatus ==",
        })
        {
            Assert.DoesNotContain(forbidden, viewModel, StringComparison.Ordinal);
        }

        // The one branch on the action lives in the view model, off the ABI's
        // answer, and the view holds no arm of it: a press is handed to the
        // view model, which decides what it was.
        string codeBehind = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml.cs");
        Assert.Contains("ViewModel.PressCredentialAsync()", codeBehind, StringComparison.Ordinal);
        Assert.DoesNotContain("CredentialAction.", codeBehind, StringComparison.Ordinal);
        Assert.DoesNotContain("NearAiCredentialSurface.Action", codeBehind, StringComparison.Ordinal);
    }

    /// <summary>
    /// The view opens a browser only at a URL the daemon handed back from
    /// start, and no poll re-serves one.
    /// </summary>
    [Fact]
    public void OnlyAStartedCeremonyOpensABrowser()
    {
        string codeBehind = ShellSource("TraceCommons.App/Controls/PrivateInferenceView.xaml.cs");
        Assert.Contains("ViewModel.ContinueCredentialAsync(await ViewModel.PressCredentialAsync())", codeBehind, StringComparison.Ordinal);
        string continuation = ShellSource("TraceCommons.App/ViewModels/CredentialBrowser.cs");
        Assert.Contains("started.BrowserUrl", continuation, StringComparison.Ordinal);

        string viewModel = ShellSource("TraceCommons.App/ViewModels/PrivateInferenceViewModel.cs");
        string start = Region(viewModel, "public async Task<NearAiCredentialAttempt?> StartCredentialAsync(");
        Assert.Contains("ParseStart", start, StringComparison.Ordinal);

        // The status read never parses a start reply, which is what would let
        // a poll hand out a second URL.
        string status = Region(viewModel, "public async Task LoadCredentialAsync(");
        Assert.DoesNotContain("ParseStart", status, StringComparison.Ordinal);
        Assert.Contains("ParseStatus", status, StringComparison.Ordinal);

        // And a status that could not be read falls back to unreported, never
        // to a state this shell made up.
        Assert.Contains("NearAiCredentialStatus.Unreported", status, StringComparison.Ordinal);
    }

    /// <summary>One method's body, from its signature to the line closing it.</summary>
    private static string Region(string source, string signature)
    {
        source = source.Replace("\r\n", "\n", StringComparison.Ordinal);
        int start = source.IndexOf(signature, StringComparison.Ordinal);
        Assert.True(start >= 0, $"{signature} is gone from the view model");
        int end = source.IndexOf("\n    }\n", start, StringComparison.Ordinal);
        Assert.True(end > start, $"{signature} does not close");
        return source[start..end];
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"the shell source was not copied to {path}");
        return File.ReadAllText(path);
    }
}
