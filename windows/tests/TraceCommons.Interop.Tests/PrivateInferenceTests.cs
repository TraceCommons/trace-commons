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
/// The private-inference surface as it really crosses the C ABI.
///
/// Everything asserted against the payload the Rust exported, except the one
/// deliberate literal pin below: comparing a payload field to itself would
/// keep passing if every sentence changed at once, and this suite is meant to
/// turn red with the macOS and Linux ones when a word moves.
/// </summary>
public class PrivateInferenceTests
{
    [Fact]
    public void WriteConfirmationDistinguishesMissingAndExplicitFalse()
    {
        foreach (bool? seen in new bool?[] { null, false, true })
        foreach (bool? on in new bool?[] { null, false, true })
        {
            var settings = JsonSerializer.Deserialize<DaemonSettingsSnapshot>(JsonSerializer.Serialize(new { private_inference_offer_seen = seen, private_inference = on }));
            Assert.Equal(seen == true, PrivateInferenceSurface.WriteConfirmed(null, settings));
            Assert.Equal(seen == true && on == true, PrivateInferenceSurface.WriteConfirmed(true, settings));
            Assert.Equal(seen == true && on == false, PrivateInferenceSurface.WriteConfirmed(false, settings));
        }
        Assert.False(PrivateInferenceSurface.WriteConfirmed(false, null));
    }

    private static PrivateInferenceCopy Copy()
    {
        PrivateInferenceCopy? copy = PrivateInferenceSurface.Copy();
        Assert.NotNull(copy);
        return copy!;
    }

    private static PrivateInferenceState State(string label, ushort? port = null) =>
        new(label, port);

    /// <summary>
    /// Every field the Rust exports is decoded here, and this shell invents
    /// none. Set equality in both directions: a field the Rust grew and this
    /// record dropped would sail past a one-way check.
    /// </summary>
    [Fact]
    public void EveryExportedFieldIsDecodedAndNoneIsInvented()
    {
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_private_inference_copy());
        Assert.False(string.IsNullOrWhiteSpace(json));

        using JsonDocument document = JsonDocument.Parse(json!);
        var exported = new HashSet<string>(StringComparer.Ordinal);
        foreach (JsonProperty property in document.RootElement.EnumerateObject())
        {
            exported.Add(property.Name);
        }

        var declared = new HashSet<string>(StringComparer.Ordinal);
        foreach (var property in typeof(PrivateInferenceCopy).GetProperties())
        {
            foreach (var attribute in property.GetCustomAttributes(
                typeof(System.Text.Json.Serialization.JsonPropertyNameAttribute), false))
            {
                declared.Add(
                    ((System.Text.Json.Serialization.JsonPropertyNameAttribute)attribute).Name);
            }
        }

        Assert.Equal(declared, exported);
    }

    /// <summary>
    /// The sentence this whole surface exists to print. Pinned literally,
    /// because a payload field compared to itself proves nothing.
    /// </summary>
    [Fact]
    public void TheOfferSaysWhatTurningItOnExposes()
    {
        PrivateInferenceCopy copy = Copy();
        Assert.Contains("anything else running", copy.OfferExposure, StringComparison.Ordinal);
        Assert.Contains("accounts", copy.OfferExposure, StringComparison.Ordinal);
        Assert.Contains("shared", copy.OfferExposure, StringComparison.Ordinal);
    }

    /// <summary>
    /// Every sentence arrives finished. A template with a hole in it would
    /// make this shell a second place the wording lives.
    /// </summary>
    [Fact]
    public void EverySentenceArrivesFinished()
    {
        foreach (string sentence in Copy().Sentences)
        {
            Assert.False(string.IsNullOrWhiteSpace(sentence));
            foreach (string marker in new[] { "{}", "{port}", "%@", "%s", "%d" })
            {
                Assert.DoesNotContain(marker, sentence, StringComparison.Ordinal);
            }
        }
    }

    /// <summary>
    /// <c>Sentences</c> holds every decoded sentence and no fewer.
    ///
    /// <see cref="EveryExportedFieldIsDecodedAndNoneIsInvented"/> pins the
    /// record to the Rust, and <see cref="EverySentenceArrivesFinished"/>
    /// pins the sentences to being finished -- but nothing tied the two
    /// together, so a field added to the Rust, the record and the decoder
    /// while being forgotten in this hand-written array would pass both and
    /// never be checked for a template hole. The count is what has the
    /// teeth: containment alone would accept a short array.
    /// </summary>
    [Fact]
    public void EveryDecodedSentenceIsInTheCompletePayloadCheck()
    {
        PrivateInferenceCopy copy = Copy();
        var decoded = new List<string>();
        foreach (var property in typeof(PrivateInferenceCopy).GetProperties())
        {
            if (property.PropertyType != typeof(string))
            {
                continue;
            }

            if (property.GetCustomAttributes(
                typeof(System.Text.Json.Serialization.JsonPropertyNameAttribute), false).Length == 0)
            {
                continue;
            }

            decoded.Add((string)property.GetValue(copy)!);
        }

        Assert.NotEmpty(decoded);
        Assert.Equal(decoded.Count, copy.Sentences.Length);
        foreach (string sentence in decoded)
        {
            Assert.Contains(sentence, copy.Sentences);
        }
    }

    /// <summary>
    /// A payload missing the exposure sentence is refused whole, not rendered
    /// with a gap where it should be.
    /// </summary>
    [Fact]
    public void APayloadMissingASentenceIsRefusedRatherThanRenderedBlank()
    {
        Assert.Null(PrivateInferenceSurface.Parse("""{"offer_title":"T"}"""));
        Assert.Null(PrivateInferenceSurface.Parse("not json"));
        Assert.Null(PrivateInferenceSurface.Parse(""));
    }

    /// <summary>
    /// The seven labels the daemon can report each get their own sentence,
    /// and every stranger reads as off.
    /// </summary>
    [Fact]
    public void EachStateRendersTheSentenceTheRustExports()
    {
        PrivateInferenceCopy copy = Copy();
        Assert.Equal(copy.StateOff, PrivateInferenceSurface.StateLine(State("off"), copy));
        Assert.Equal(copy.StateRunning, PrivateInferenceSurface.StateLine(State("running"), copy));
        Assert.Equal(
            copy.StateRunningNoBackends,
            PrivateInferenceSurface.StateLine(State("running_no_backends"), copy));
        Assert.Equal(
            copy.StateRunningElsewhere,
            PrivateInferenceSurface.StateLine(State("running_elsewhere"), copy));
        Assert.Equal(
            copy.StatePortInUse, PrivateInferenceSurface.StateLine(State("port_in_use"), copy));
        Assert.Equal(
            copy.StateStartFailed, PrivateInferenceSurface.StateLine(State("start_failed"), copy));
        Assert.Equal(copy.StateCrashed, PrivateInferenceSurface.StateLine(State("crashed"), copy));
        Assert.Equal(
            copy.StateUnknown,
            PrivateInferenceSurface.StateLine(State("a_state_from_a_later_daemon"), copy));
        Assert.Equal(copy.StateUnreported, PrivateInferenceSurface.StateLine(State(string.Empty), copy));
        Assert.Equal(copy.StateStopping, PrivateInferenceSurface.StateLine(State("stopping"), copy));
        Assert.Equal(PrivateInferenceTone.Held, PrivateInferenceSurface.Tone(State("stopping")));
    }

    /// <summary>
    /// Exactly one state may be painted as working, and it is not the one
    /// with nowhere to send a call. That is why the state exists.
    /// </summary>
    [Fact]
    public void OnlyAListenerWithSomewhereToSendIsPaintedClear()
    {
        Assert.Equal(PrivateInferenceTone.Clear, PrivateInferenceSurface.Tone(State("running")));
        Assert.Equal(
            PrivateInferenceTone.Attention,
            PrivateInferenceSurface.Tone(State("running_no_backends")));
        Assert.NotEqual(
            PrivateInferenceTone.Clear,
            PrivateInferenceSurface.Tone(State("running_no_backends")));
        Assert.Equal(
            PrivateInferenceTone.Held, PrivateInferenceSurface.Tone(State("running_elsewhere")));
        Assert.Equal(PrivateInferenceTone.Neutral, PrivateInferenceSurface.Tone(State("off")));
        foreach (string failure in new[] { "port_in_use", "start_failed", "crashed" })
        {
            Assert.Equal(PrivateInferenceTone.Refused, PrivateInferenceSurface.Tone(State(failure)));
        }

        Assert.Equal(
            PrivateInferenceTone.Neutral,
            PrivateInferenceSurface.Tone(State("a_state_from_a_later_daemon")));
    }

    /// <summary>
    /// Every refusal names the way out, and the sticky one says it is sticky.
    /// </summary>
    [Fact]
    public void EveryRefusalNamesTheWayOut()
    {
        PrivateInferenceCopy copy = Copy();
        foreach (string sentence in new[] { copy.StatePortInUse, copy.StateStartFailed, copy.StateCrashed })
        {
            Assert.Contains("off and on again", sentence, StringComparison.Ordinal);
        }

        Assert.Contains("will not retry by itself", copy.StateCrashed, StringComparison.Ordinal);
    }

    /// <summary>
    /// The ABI numbering is spelled out and anything unknown is neutral --
    /// never the working light. The routing numbering must not decode here:
    /// that cross-wiring is what the disjoint ranges exist to make wrong for
    /// every value rather than only for the dangerous one.
    /// </summary>
    [Fact]
    public void AnUnknownToneIsNeutralAndTheRoutingNumberingDoesNotDecodeHere()
    {
        Assert.Equal(PrivateInferenceTone.Held, PrivateInferenceSurface.FromAbiTone(21));
        Assert.Equal(PrivateInferenceTone.Clear, PrivateInferenceSurface.FromAbiTone(22));
        Assert.Equal(PrivateInferenceTone.Attention, PrivateInferenceSurface.FromAbiTone(23));
        Assert.Equal(PrivateInferenceTone.Refused, PrivateInferenceSurface.FromAbiTone(24));
        foreach (int stranger in new[] { 0, 1, 2, 3, 10, 14, 20, 25, -1, 99 })
        {
            Assert.Equal(PrivateInferenceTone.Neutral, PrivateInferenceSurface.FromAbiTone(stranger));
        }
    }

    [Fact]
    public void TheServingSentenceNamesAPortOrIsEmpty()
    {
        Assert.Equal(string.Empty, PrivateInferenceSurface.ServingLine(State("port_in_use")));
        Assert.Equal(string.Empty, PrivateInferenceSurface.ServingLine(State("running", 0)));
        Assert.Contains(
            "8463", PrivateInferenceSurface.ServingLine(State("running", 8463)),
            StringComparison.Ordinal);
    }

    /// <summary>
    /// Whether to ask crosses the ABI, so the three shells cannot come to
    /// disagree about who has already been asked.
    /// </summary>
    [Fact]
    public void WhetherToOfferCrossesTheAbi()
    {
        Assert.True(PrivateInferenceSurface.ShouldOffer(answered: false, on: false));
        Assert.False(PrivateInferenceSurface.ShouldOffer(answered: true, on: false));
        Assert.False(PrivateInferenceSurface.ShouldOffer(answered: false, on: true));
        Assert.False(PrivateInferenceSurface.ShouldOffer(answered: true, on: true));
    }

    /// <summary>
    /// Declining records the answer and writes no switch. Writing it as false
    /// would make a refusal indistinguishable from a change.
    /// </summary>
    [Fact]
    public void DecliningWritesTheMarkerAlone()
    {
        using JsonDocument declined =
            JsonDocument.Parse(PrivateInferenceSurface.SerializeOfferAnswer(accepted: false));
        JsonProperty only = Assert.Single(declined.RootElement.EnumerateObject());
        Assert.Equal(PrivateInferenceSurface.OfferSeenKey, only.Name);
        Assert.True(only.Value.GetBoolean());

        using JsonDocument accepted =
            JsonDocument.Parse(PrivateInferenceSurface.SerializeOfferAnswer(accepted: true));
        Assert.True(
            accepted.RootElement.GetProperty(PrivateInferenceSurface.SettingsKey).GetBoolean());
        Assert.True(
            accepted.RootElement.GetProperty(PrivateInferenceSurface.OfferSeenKey).GetBoolean());
    }

    /// <summary>
    /// The settings switch answers the question too, so a contributor who
    /// found it themselves is not asked about it later.
    /// </summary>
    [Theory]
    [InlineData(true)]
    [InlineData(false)]
    public void TheSettingsSwitchAlsoAnswersTheQuestion(bool on)
    {
        using JsonDocument json = JsonDocument.Parse(PrivateInferenceSurface.SerializeSwitch(on));
        Assert.Equal(
            on, json.RootElement.GetProperty(PrivateInferenceSurface.SettingsKey).GetBoolean());
        Assert.True(
            json.RootElement.GetProperty(PrivateInferenceSurface.OfferSeenKey).GetBoolean());
    }

    /// <summary>
    /// A daemon that never heard of the keys reads as off and unanswered,
    /// which is what makes the offer appear once after an upgrade.
    /// </summary>
    [Fact]
    public void ADaemonWithoutTheKeysReadsAsOffAndUnanswered()
    {
        DaemonSettingsSnapshot? snapshot =
            JsonSerializer.Deserialize<DaemonSettingsSnapshot>("""{"quiescence_secs":45}""");
        Assert.NotNull(snapshot);
        Assert.False(snapshot!.PrivateInferenceOn);
        Assert.False(snapshot.PrivateInferenceAnswered);
        Assert.Null(snapshot.PrivateInferenceReport);
        Assert.Equal(
            string.Empty, PrivateInferenceState.From(snapshot.PrivateInferenceReport).Label);
    }

    /// <summary>
    /// The state object is read as the daemon sends it, label and port.
    /// </summary>
    [Fact]
    public void TheReportedStateIsReadWhole()
    {
        DaemonSettingsSnapshot? snapshot = JsonSerializer.Deserialize<DaemonSettingsSnapshot>(
            """{"private_inference":true,"private_inference_offer_seen":true,"private_inference_state":{"state":"running","port":8463}}""");
        Assert.NotNull(snapshot);
        Assert.True(snapshot!.PrivateInferenceOn);
        Assert.True(snapshot.PrivateInferenceAnswered);
        PrivateInferenceState state = PrivateInferenceState.From(snapshot.PrivateInferenceReport);
        Assert.Equal("running", state.Label);
        Assert.Equal((ushort)8463, state.Port);
    }

    /// <summary>
    /// The quit sentence is added only while the switch is on, and it is the
    /// payload's words rather than this shell's.
    /// </summary>
    [Fact]
    public void TheQuitSentenceIsOnlyAddedWhenTheSwitchIsOn()
    {
        PrivateInferenceCopy copy = Copy();
        Assert.Null(PrivateInferenceSurface.QuitDetail(on: false, State(string.Empty), copy));
        Assert.Null(PrivateInferenceSurface.QuitDetail(on: true, State(string.Empty), null));
        Assert.Equal(copy.QuitAlsoStops, PrivateInferenceSurface.QuitDetail(on: true, State(string.Empty), copy));
        Assert.Equal(copy.QuitAlsoStops, PrivateInferenceSurface.QuitDetail(false, State("stopping"), copy));
        Assert.Null(PrivateInferenceSurface.QuitDetail(true, State("running_elsewhere"), copy));
        Assert.Null(PrivateInferenceSurface.QuitDetail(true, State("off"), copy));
    }

    /// <summary>
    /// No sentence on this surface is authored in this shell.
    ///
    /// Asserted about the source rather than about behaviour, for the reason
    /// <c>NoWordingIsAuthoredInThisShell</c> gives: a hand-written sentence
    /// that happened to match the Rust today would pass every behavioural
    /// test here and then survive a rename in exactly one of the three
    /// shells.
    /// </summary>
    [Fact]
    public void NoWordingIsAuthoredInThePrivateInferenceSurface()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "PrivateInferenceSurface.cs.txt");
        Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");
        string source = File.ReadAllText(path);

        string uncommented = string.Join(
            "\n",
            source.Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal))
                .Where(line => !line.TrimStart().StartsWith("///", StringComparison.Ordinal)));

        var allowed = new HashSet<string>(StringComparer.Ordinal)
        {
            // The two set_settings wire keys, and nothing else.
            "private_inference",
            "private_inference_offer_seen",
        };

        foreach (Match match in Regex.Matches(uncommented, "\"([^\"\\\\]|\\\\.)*\""))
        {
            string literal = match.Value[1..^1];
            Assert.True(
                allowed.Contains(literal),
                $"\"{literal}\" is a string literal in PrivateInferenceSurface.cs that is not a "
                + "wire value. Wording on this surface comes from private_inference_copy.rs "
                + "across the ABI.");
        }
    }

    /// <summary>
    /// The offer's card in the main window draws every sentence from the view
    /// model, and the view model draws every one of them from the payload.
    ///
    /// The paragraph at issue is the one saying what turning the switch on
    /// exposes: a shorter, friendlier paraphrase of it would render, would
    /// pass every behavioural test, and would be the one thing on this card
    /// that must not be paraphrased.
    /// </summary>
    [Fact]
    public void TheOfferIsDrawnFromTheSharedSentences()
    {
        string markup = File.ReadAllText(
            Path.Combine(AppContext.BaseDirectory, "MainWindow.xaml.txt"));
        int start = markup.IndexOf("HasPrivateInferenceOffer", StringComparison.Ordinal);
        Assert.True(start >= 0, "the offer card is not in the main window");
        int end = markup.IndexOf("</Border>", start, StringComparison.Ordinal);
        Assert.True(end > start, "the offer card's border does not close");
        string card = markup[start..end];

        foreach (string bound in new[]
        {
            "ViewModel.PrivateInferenceOfferTitle",
            "ViewModel.PrivateInferenceOfferWhat",
            "ViewModel.PrivateInferenceOfferExposure",
            "ViewModel.PrivateInferenceOfferNoRepoint",
            "ViewModel.PrivateInferenceOfferAskedOnce",
            "ViewModel.PrivateInferenceOfferAccept",
            "ViewModel.PrivateInferenceOfferDecline",
        })
        {
            Assert.Contains(bound, card, StringComparison.Ordinal);
        }

        // Every Text= and Content= on the card is a binding, never a literal.
        foreach (Match match in Regex.Matches(card, "(Text|Content)=\"([^\"]*)\""))
        {
            Assert.StartsWith("{x:Bind", match.Groups[2].Value, StringComparison.Ordinal);
        }

        string viewModel = File.ReadAllText(
            Path.Combine(AppContext.BaseDirectory, "MainViewModel.cs.txt"));
        foreach (string sourced in new[]
        {
            "_privateInferenceCopy?.OfferTitle",
            "_privateInferenceCopy?.OfferWhat",
            "_privateInferenceCopy?.OfferExposure",
            "_privateInferenceCopy?.OfferNoRepoint",
            "_privateInferenceCopy?.OfferAskedOnce",
            "_privateInferenceCopy?.OfferAccept",
            "_privateInferenceCopy?.OfferDecline",
        })
        {
            Assert.Contains(sourced, viewModel, StringComparison.Ordinal);
        }

        // And whether to ask is the shared table's decision, not this
        // window's: three shells each deciding when to interrupt somebody is
        // three chances to re-ask a contributor who already said no. The
        // exact call shape, including the has-the-daemon-answered guard, is
        // pinned by TheOfferIsGatedOnHavingHeardFromTheDaemon.
        Assert.Contains(
            "PrivateInferenceSurface.ShouldOffer(",
            viewModel,
            StringComparison.Ordinal);
        Assert.DoesNotContain(
            "!_privateInferenceAnswered && !_privateInferenceOn",
            viewModel,
            StringComparison.Ordinal);
    }

    /// <summary>
    /// The page that holds the switch picks its colour from the tone, never
    /// by reading the sentence back. Three of the seven sentences begin with
    /// the same two words, so a colour recovered from the text would be
    /// recovered from a prefix match.
    ///
    /// <para>
    /// It reads the destination rather than the settings card, because the
    /// settings card no longer reports a state at all: it holds a pointer,
    /// and the control and its indicator moved together.
    /// </para>
    /// </summary>
    [Fact]
    public void ThePrivateInferenceStateIsPaintedFromTheToneAndNotFromItsOwnText()
    {
        string viewModel = File.ReadAllText(
            Path.Combine(AppContext.BaseDirectory, "PrivateInferenceViewModel.cs.txt"));
        foreach (string forbidden in new[]
        {
            "StateText ==",
            "StateText.Contains",
            "StateText.StartsWith",
        })
        {
            Assert.DoesNotContain(forbidden, viewModel, StringComparison.Ordinal);
        }

        Assert.Contains(
            "PrivateInferenceSurface.Tone(_state)",
            viewModel,
            StringComparison.Ordinal);

        string markup = File.ReadAllText(
            Path.Combine(AppContext.BaseDirectory, "PrivateInferenceView.xaml.txt"));

        // The refusal wears the refusal colour, and the exposure sentence is
        // beside the switch as well as in the offer.
        Assert.Contains("ViewModel.StateIsRefused", markup, StringComparison.Ordinal);
        Assert.Contains("TcCoralTextBrush", markup, StringComparison.Ordinal);
        Assert.Contains("ViewModel.Exposure", markup, StringComparison.Ordinal);
        foreach (Match match in Regex.Matches(markup, "(Text|Content|Header)=\"([^\"]*)\""))
        {
            Assert.StartsWith("{x:Bind", match.Groups[2].Value, StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// The offer is never put to somebody before the daemon has answered.
    ///
    /// A window holds "answered" and "on" as booleans, and both default to
    /// false -- which is exactly the shape that means "ask". So an unguarded
    /// shell renders the offer from construction, before get_settings lands,
    /// and indefinitely against a daemon it cannot reach: shown to a
    /// contributor who already declined, by a window that has not yet learned
    /// they did.
    ///
    /// The guard is a third input to the shared table rather than a flag each
    /// shell ANDs for itself, for the reason the table exists at all.
    /// </summary>
    [Fact]
    public void NothingIsOfferedBeforeTheDaemonHasAnswered()
    {
        Assert.False(
            PrivateInferenceSurface.ShouldOffer(known: false, answered: false, on: false),
            "an unanswered get_settings must not read as an unanswered question");
        Assert.False(PrivateInferenceSurface.ShouldOffer(known: false, answered: true, on: false));
        Assert.False(PrivateInferenceSurface.ShouldOffer(known: false, answered: false, on: true));

        // Once it has answered, the two-input rule applies unchanged.
        Assert.True(PrivateInferenceSurface.ShouldOffer(known: true, answered: false, on: false));
        Assert.False(PrivateInferenceSurface.ShouldOffer(known: true, answered: true, on: false));
        Assert.False(PrivateInferenceSurface.ShouldOffer(known: true, answered: false, on: true));
    }

    /// <summary>
    /// The main window asks the guarded overload, and learns it has an answer
    /// BEFORE the early return that skips a settings read whose values happen
    /// to match the defaults -- which is every first read against a daemon
    /// with the switch off and the question unasked.
    /// </summary>
    [Fact]
    public void TheOfferIsGatedOnHavingHeardFromTheDaemon()
    {
        string viewModel = File.ReadAllText(
            Path.Combine(AppContext.BaseDirectory, "MainViewModel.cs.txt"));

        Assert.Contains(
            "PrivateInferenceSurface.ShouldOffer(\n            _privateInferenceKnown,",
            viewModel.Replace("\r\n", "\n"),
            StringComparison.Ordinal);

        string setter = MethodBody(viewModel, "public void SetPrivateInference(");
        int flag = setter.IndexOf("_privateInferenceKnown = true;", StringComparison.Ordinal);
        int earlyReturn = setter.IndexOf("== _privateInferenceAnswered", StringComparison.Ordinal);
        Assert.True(flag >= 0, "the window never records that the daemon answered");
        Assert.True(
            flag < earlyReturn,
            "the known flag is set after the values-coincide early return, so the first "
            + "answer that matches the defaults never lands");
    }

    /// <summary>
    /// A refused or failed write snaps the switch back to what the daemon
    /// holds, rather than leaving it wherever the contributor dragged it.
    ///
    /// The toggle is bound one-way to <c>Enabled</c>, so the only thing that
    /// returns it to the daemon's value is a change notification. Every path
    /// out of the write must raise one: the early return, which fires when a
    /// second press arrives while one is in flight, the error frame, and the
    /// catch, which is where a daemon that refused lands. Without it the page
    /// says "on" over a listener that was never started, which is the shape
    /// this whole surface is built to avoid.
    ///
    /// It reads the destination's view model, which owns the switch now.
    /// </summary>
    [Fact]
    public void AFailedSwitchWriteSnapsTheToggleBack()
    {
        string viewModel = File.ReadAllText(
            Path.Combine(AppContext.BaseDirectory, "PrivateInferenceViewModel.cs.txt"));
        string body = MethodBody(viewModel, "public async Task SetAsync(");

        const string raise = "Raise(nameof(Enabled));";
        int guardReturn = body.IndexOf("return;", StringComparison.Ordinal);
        Assert.True(guardReturn >= 0, "the busy/unloaded guard is gone");
        Assert.Contains(
            raise,
            body[..guardReturn],
            StringComparison.Ordinal);

        // The refusal path proper: a well-formed error frame. The call
        // returned, so neither the guard nor the catch fires, and Fill is
        // never reached -- which is precisely the case where the daemon said
        // no and the toggle is still sitting where the contributor put it.
        int elseAt = body.IndexOf("else", StringComparison.Ordinal);
        Assert.True(elseAt >= 0, "the error frame is no longer handled");
        int catchAt = body.IndexOf("catch", StringComparison.Ordinal);
        Assert.True(catchAt >= 0, "the write no longer catches");
        Assert.True(elseAt < catchAt, "the else being asserted is not the one inside the try");
        Assert.Contains(raise, body[elseAt..catchAt], StringComparison.Ordinal);

        Assert.Contains(raise, body[catchAt..], StringComparison.Ordinal);

        // All three, and no more: every path out of this method either wrote
        // the value or corrected it.
        Assert.Equal(
            3,
            Regex.Matches(body, Regex.Escape(raise)).Count);
    }

    [Theory]
    [InlineData("\n")]
    [InlineData("\r\n")]
    public void MethodBodyAcceptsBothCheckoutLineEndings(string newline)
    {
        string source = string.Join(newline, new[]
        {
            "    public void Example()", "    {", "        return;", "    }", ""
        });
        Assert.Contains("return;", MethodBody(source, "public void Example("));
    }

    /// <summary>
    /// The body of one method, from its signature to the line that closes it
    /// at method indentation. Crude on purpose: it is reading C# this suite
    /// cannot compile, and a parser here would be a second thing to get
    /// wrong.
    /// </summary>
    private static string MethodBody(string source, string signature)
    {
        // Git checks source fixtures out with CRLF on Windows. The guard
        // examines C# structure, not the checkout's line-ending convention.
        source = source.Replace("\r\n", "\n", StringComparison.Ordinal);
        int start = source.IndexOf(signature, StringComparison.Ordinal);
        Assert.True(start >= 0, $"{signature} is gone from the view model");
        int end = source.IndexOf("\n    }\n", start, StringComparison.Ordinal);
        Assert.True(end > start, $"{signature} does not close");
        return source[start..end];
    }

    /// <summary>
    /// Not one string literal lives in either view model's private-inference
    /// region.
    ///
    /// The surface scan above covers PrivateInferenceSurface.cs, and a
    /// paraphrase would never go there -- it would go where the sentence is
    /// handed to a control, as a fallback on the payload read:
    /// <c>_privateInferenceCopy?.OfferExposure ?? "we handle your model calls
    /// here"</c>. That renders, it is friendlier, it is wrong, and it passes
    /// every other test in this file.
    ///
    /// Both regions legitimately contain NO literals at all -- they are
    /// nameof, payload reads and shared-surface calls -- so the allowed set is
    /// empty rather than curated. Comments are stripped first: the prose in
    /// them quotes words on purpose.
    /// </summary>
    [Theory]
    [InlineData(
        "MainViewModel.cs.txt",
        "// --- The offer to answer model calls on this computer",
        "// --- The daily-budget banner")]
    [InlineData(
        "ContributorSettingsViewModel.cs.txt",
        "// Answering model calls on this computer. The switch, the exposure",
        "<summary>Whether the witness actions may be pressed.</summary>")]
    public void NoWordingIsAuthoredInThePrivateInferenceViewModels(
        string file, string opens, string closes)
    {
        string source = File.ReadAllText(Path.Combine(AppContext.BaseDirectory, file));
        int start = source.IndexOf(opens, StringComparison.Ordinal);
        Assert.True(start >= 0, $"{file} no longer has a private-inference region");
        int end = source.IndexOf(closes, start, StringComparison.Ordinal);
        Assert.True(end > start, $"{file}'s private-inference region does not close");
        string region = source[start..end];

        string uncommented = string.Join(
            "\n",
            region.Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));

        foreach (Match match in Regex.Matches(uncommented, "\"([^\"\\\\]|\\\\.)*\""))
        {
            Assert.Fail(
                $"{match.Value} is a string literal in {file}'s private-inference region. "
                + "Every sentence on this surface comes from private_inference_copy.rs across "
                + "the ABI; a fallback beside a payload read is a paraphrase that renders.");
        }
    }
}
