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
/// The redaction-witness settings surface.
///
/// The one thing this file exists to hold down is the tone mapping. The
/// witness tones are numbered 10..14 precisely BECAUSE the routing tones are
/// numbered 0..3 and <see cref="RoutingSurface"/>'s mapper sends anything it
/// does not recognise to <c>Neutral</c>. A witness tone fed through that
/// mapper would paint a refusal -- nothing is being sent at all -- as
/// "nothing to say". The witness mapper is therefore separate, and its
/// unknown arm goes the other way.
/// </summary>
public sealed class WitnessSurfaceTests
{
    /// <summary>
    /// A tone this build does not know is a refusal, never neutral.
    /// </summary>
    /// <remarks>
    /// On a surface about whether raw sessions leave the machine, the safe
    /// reading of "I do not know" is "they are not". Every value the header
    /// adds later is a condition this build has no words for, and the
    /// fail-closed direction is the only one that cannot quietly reassure.
    /// </remarks>
    [Fact]
    public void AnUnknownWitnessToneIsRefusedAndNeverNeutral()
    {
        foreach (int unknown in new[] { -1, 5, 6, 9, 15, 16, 100, int.MinValue, int.MaxValue })
        {
            Assert.Equal(WitnessTone.Refused, WitnessTools.FromAbiTone(unknown));
        }
    }

    /// <summary>
    /// The routing tone numbers are not witness tones.
    /// </summary>
    /// <remarks>
    /// This is the cross-wiring the disjoint range was chosen to catch. If
    /// this shell ever routed the witness tone through
    /// <see cref="RoutingSurface"/>'s table -- or copied its arms -- the
    /// routing numbers would start resolving, and 0..3 would resolve to the
    /// four reassuring readings. They must all be refusals here.
    /// </remarks>
    [Fact]
    public void TheRoutingToneNumbersAreNotWitnessTones()
    {
        foreach (int routingTone in new[] { 0, 1, 2, 3, 4 })
        {
            Assert.Equal(WitnessTone.Refused, WitnessTools.FromAbiTone(routingTone));
        }
    }

    /// <summary>Each witness tone number maps to its own reading.</summary>
    [Fact]
    public void EveryWitnessToneNumberHasItsOwnReading()
    {
        Assert.Equal(WitnessTone.Neutral, WitnessTools.FromAbiTone(10));
        Assert.Equal(WitnessTone.Held, WitnessTools.FromAbiTone(11));
        Assert.Equal(WitnessTone.Clear, WitnessTools.FromAbiTone(12));
        Assert.Equal(WitnessTone.Attention, WitnessTools.FromAbiTone(13));
        Assert.Equal(WitnessTone.Refused, WitnessTools.FromAbiTone(14));
    }

    /// <summary>
    /// No witness and a witness that refuses everything do not read alike.
    /// </summary>
    /// <remarks>
    /// Absent means local redaction, which is a supported arrangement and not
    /// a warning. Refusing-unpinned means nothing uploads at all. A shell that
    /// rendered those the same would tell a contributor everything is fine
    /// through a total outage. Asserted across the real ABI, on both halves of
    /// the rendering -- the sentence and the tone -- because either one alone
    /// leaves the two distinguishable only to somebody reading carefully.
    /// </remarks>
    [Fact]
    public void AbsentAndRefusingUnpinnedDoNotRenderAlike()
    {
        string? absent = WitnessSurface.StateLine(WitnessTools.StateAbsent);
        string? unpinned = WitnessSurface.StateLine(WitnessTools.StateRefusingUnpinned);

        Assert.False(string.IsNullOrWhiteSpace(absent));
        Assert.False(string.IsNullOrWhiteSpace(unpinned));
        Assert.NotEqual(absent, unpinned);

        Assert.Equal(WitnessTone.Neutral, WitnessSurface.StateTone(WitnessTools.StateAbsent));
        Assert.Equal(
            WitnessTone.Refused,
            WitnessSurface.StateTone(WitnessTools.StateRefusingUnpinned));
    }

    /// <summary>
    /// Every state this build names has its own sentence, and none of the
    /// four refusing readings is painted as anything but a refusal.
    /// </summary>
    [Fact]
    public void EveryNamedStateHasItsOwnSentence()
    {
        var seen = new List<string>();
        foreach (int state in new[]
        {
            WitnessTools.StateAbsent,
            WitnessTools.StatePinned,
            WitnessTools.StateRefusingUnpinned,
            WitnessTools.StateRefusingPinMalformed,
            WitnessTools.StateRefusingInferenceReceiptsMissing,
            WitnessTools.StateNotEnrolled,
            WitnessTools.StateUnreadable,
        })
        {
            string? line = WitnessSurface.StateLine(state);
            Assert.False(string.IsNullOrWhiteSpace(line), $"state {state} has no sentence");
            Assert.DoesNotContain(line!, seen);
            seen.Add(line!);
        }

        // Not enrolled is not a refusal: nothing about a witness is being
        // declined, the device simply has no account yet.
        Assert.Equal(WitnessTone.Neutral, WitnessSurface.StateTone(WitnessTools.StateNotEnrolled));

        // An unreadable config is a refusal and not an absence.
        Assert.Equal(WitnessTone.Refused, WitnessSurface.StateTone(WitnessTools.StateUnreadable));
        Assert.Equal(
            WitnessTone.Refused,
            WitnessSurface.StateTone(WitnessTools.StateRefusingPinMalformed));
        Assert.Equal(
            WitnessTone.Refused,
            WitnessSurface.StateTone(WitnessTools.StateRefusingInferenceReceiptsMissing));
    }

    /// <summary>
    /// A state this build cannot name yields no sentence and a refused tone.
    /// </summary>
    /// <remarks>
    /// A shell handed no sentence must render none of its own. The tone is
    /// still answered, and it fails closed, so the branch that shows nothing
    /// is not the branch that reassures.
    /// </remarks>
    [Fact]
    public void AStateThisBuildCannotNameHasNoSentenceAndARefusedTone()
    {
        Assert.Null(WitnessSurface.StateLine(99));
        Assert.Equal(WitnessTone.Refused, WitnessSurface.StateTone(99));
    }

    /// <summary>
    /// The card's words arrive as one whole set, or not at all.
    /// </summary>
    /// <remarks>
    /// A field the Rust stopped exporting would deserialise to the empty
    /// string and render as a blank beside a privacy claim. The whole payload
    /// is refused instead.
    /// </remarks>
    [Fact]
    public void TheWitnessCopyArrivesWholeOrNotAtAll()
    {
        WitnessCopy? copy = WitnessSurface.Copy();
        Assert.NotNull(copy);
        foreach (string word in copy!.Words)
        {
            Assert.False(string.IsNullOrWhiteSpace(word));
        }

        // A payload missing one word is not a partly-filled card.
        Assert.Null(WitnessTools.ParseCopy("{\"heading\":\"x\"}"));
        Assert.Null(WitnessTools.ParseCopy("not json"));
        Assert.Null(WitnessTools.ParseCopy(null));
    }

    /// <summary>
    /// Nothing here claims a session is clean, or calls anything attested.
    /// </summary>
    /// <remarks>
    /// A certificate records what was removed and the risk judged left. The
    /// two words this surface must never print are the two a reader would
    /// most readily take as a guarantee.
    /// </remarks>
    [Fact]
    public void NoSentenceOnThisSurfaceClaimsATraceIsClean()
    {
        var sentences = new List<string>();
        WitnessCopy? copy = WitnessSurface.Copy();
        Assert.NotNull(copy);
        sentences.AddRange(copy!.Words);
        foreach (int state in new[]
        {
            WitnessTools.StateAbsent,
            WitnessTools.StatePinned,
            WitnessTools.StateRefusingUnpinned,
            WitnessTools.StateRefusingPinMalformed,
            WitnessTools.StateRefusingInferenceReceiptsMissing,
            WitnessTools.StateNotEnrolled,
            WitnessTools.StateUnreadable,
        })
        {
            sentences.Add(WitnessSurface.StateLine(state)!);
        }

        foreach (string sentence in sentences)
        {
            Assert.DoesNotContain("attested", sentence, StringComparison.OrdinalIgnoreCase);
            Assert.DoesNotContain("genuine", sentence, StringComparison.OrdinalIgnoreCase);
        }
    }

    /// <summary>
    /// A status payload carrying no state code is refused rather than read as
    /// "no witness configured".
    /// </summary>
    /// <remarks>
    /// This is the same conflation the whole surface exists to prevent,
    /// arriving through the deserialiser instead of through a branch: an
    /// absent <c>state_code</c> binds to the default <c>0</c>, and
    /// <c>0</c> is ABSENT. A payload from a build that renamed the key would
    /// silently report a witness-free machine.
    /// </remarks>
    [Fact]
    public void AStatusPayloadWithoutAStateCodeIsRefusedNotReadAsAbsent()
    {
        Assert.Null(WitnessTools.ParseStatus("{\"state\":\"absent\"}"));
        Assert.Null(WitnessTools.ParseStatus("{\"state_code\":null}"));
        Assert.Null(WitnessTools.ParseStatus("not json"));
        Assert.Null(WitnessTools.ParseStatus(null));
    }

    /// <summary>
    /// The address and signing key cross verbatim; the count crosses as a
    /// count.
    /// </summary>
    /// <remarks>
    /// A screen that will not show what it is asking a contributor to trust
    /// with their raw session is not a settings screen. These two fields are
    /// the ABI's named exemption from the no-identifiers rule, and nothing
    /// else about the witness path crosses it.
    /// </remarks>
    [Fact]
    public void AStatusPayloadIsReadVerbatim()
    {
        WitnessStatus? status = WitnessTools.ParseStatus(
            "{\"state\":\"refusing_unpinned\",\"state_code\":2,"
            + "\"refusal\":\"witness_expected_measurement\","
            + "\"url\":\"https://witness.example\",\"signing_address\":\"0xabc\","
            + "\"pinned_measurement_count\":0}");

        Assert.NotNull(status);
        Assert.Equal(2, status!.StateCode);
        Assert.Equal("https://witness.example", status.Url);
        Assert.Equal("0xabc", status.SigningAddress);
        Assert.Equal(0, status.PinnedMeasurementCount);
        Assert.Equal("witness_expected_measurement", status.Refusal);
    }

    /// <summary>
    /// Measurements are typed a line at a time and cross as a JSON array.
    /// </summary>
    /// <remarks>
    /// A list, not a value: an image upgrade moves the measurement and leaves
    /// the signing address alone, so the new one is added before the fleet
    /// rolls. Blank lines and stray whitespace are dropped rather than
    /// written, because an empty entry is what the ABI refuses as an unpinned
    /// witness -- and an unpinned witness is a total upload outage.
    /// </remarks>
    [Fact]
    public void MeasurementsCrossAsAJsonArrayOfTheNonEmptyLines()
    {
        Assert.Equal(
            "[\"mrtd=aa,mrconfigid=bb\",\"mrtd=cc,mrconfigid=dd\"]",
            WitnessTools.SerializeMeasurements(
                "mrtd=aa,mrconfigid=bb\n\n  mrtd=cc,mrconfigid=dd  \r\n"));

        // Nothing typed is an empty array, which the ABI refuses. This shell
        // does not turn it into something else on the way.
        Assert.Equal("[]", WitnessTools.SerializeMeasurements("   \n \r\n"));
        Assert.Equal("[]", WitnessTools.SerializeMeasurements(null));
    }

    /// <summary>
    /// A refusal has a way out: the editor is open on every refusing state.
    /// </summary>
    /// <remarks>
    /// Not on the two readings that are not refusals. Absent is the ordinary
    /// arrangement and opening an editor over it would present local
    /// redaction as something needing repair; a device with no account has
    /// nothing to answer here yet. Everything else -- including a state this
    /// build cannot name -- opens, on the same fail-closed reasoning as the
    /// tone.
    /// </remarks>
    [Fact]
    public void ARefusalOpensTheEditorAndAnOrdinaryStateDoesNot()
    {
        Assert.False(WitnessTools.EditorOpensFor(WitnessTools.StateAbsent));
        Assert.False(WitnessTools.EditorOpensFor(WitnessTools.StatePinned));
        Assert.False(WitnessTools.EditorOpensFor(WitnessTools.StateNotEnrolled));

        Assert.True(WitnessTools.EditorOpensFor(WitnessTools.StateRefusingUnpinned));
        Assert.True(WitnessTools.EditorOpensFor(WitnessTools.StateRefusingPinMalformed));
        Assert.True(WitnessTools.EditorOpensFor(WitnessTools.StateRefusingInferenceReceiptsMissing));
        Assert.True(WitnessTools.EditorOpensFor(WitnessTools.StateUnreadable));
        Assert.True(WitnessTools.EditorOpensFor(99));
    }

    /// <summary>
    /// The pinned measurements read back verbatim, and round trip.
    /// </summary>
    /// <remarks>
    /// The array the status payload carries is exactly what
    /// <c>tc_witness_configure</c> takes, so the editor is pre-filled from it
    /// and hands it straight back. NOTHING MAY BE REFORMATTED IN BETWEEN: a
    /// shell that re-emits a pin from a parsed form can re-emit it wrongly,
    /// and would rewrite a pin nobody touched.
    /// </remarks>
    [Fact]
    public void ThePinnedMeasurementsRoundTripUntouched()
    {
        string[] stored =
        {
            "mrtd=abab,mrconfigid=cdcd",
            "mrtd=efef",
            // A stored entry this build cannot parse comes back as it is
            // stored, so a contributor can see the typo. It must survive the
            // round trip too, or saving would delete their work.
            "mrtd=NOTHEX,,mrconfigid",
        };

        string text = WitnessTools.JoinMeasurements(stored);
        Assert.Equal(
            "[\"mrtd=abab,mrconfigid=cdcd\",\"mrtd=efef\",\"mrtd=NOTHEX,,mrconfigid\"]",
            WitnessTools.SerializeMeasurements(text));

        // And through the editor's own text, which is what the box holds.
        Assert.Equal(stored, text.Split('\n'));

        // No entries is an empty box, not a box with a blank line in it.
        Assert.Equal(string.Empty, WitnessTools.JoinMeasurements(Array.Empty<string>()));
        Assert.Equal(string.Empty, WitnessTools.JoinMeasurements(null));
    }

    /// <summary>
    /// The count's sentence crosses the ABI, and is absent where there is no
    /// witness to count for.
    /// </summary>
    /// <remarks>
    /// Rendered or not rendered -- never replaced with a bare numeral, which
    /// is a shell inventing wording by omission, and never with a placeholder.
    /// </remarks>
    [Fact]
    public void TheCountsSentenceIsCarriedAndTheEntriesMatchIt()
    {
        WitnessStatus? status = WitnessTools.ParseStatus(
            "{\"state\":\"pinned\",\"state_code\":1,\"refusal\":null,"
            + "\"url\":\"https://witness.example\",\"signing_address\":\"0xabc\","
            + "\"pinned_measurement_count\":2,"
            + "\"pinned_measurement_line\":\"2 measurements are pinned.\","
            + "\"pinned_measurements\":[\"mrtd=aa\",\"mrtd=bb\"]}");

        Assert.NotNull(status);
        Assert.Equal("2 measurements are pinned.", status!.PinnedMeasurementLine);
        Assert.Equal(new[] { "mrtd=aa", "mrtd=bb" }, status.PinnedMeasurements);

        // The count and the list are one fact. A payload where they disagree
        // is not one this build produces, and the assertion is here so a
        // shell never renders a sentence about a number it did not read.
        Assert.Equal(status.PinnedMeasurementCount, status.PinnedMeasurements.Length);

        // A payload from the build before these keys existed still parses,
        // and answers no sentence and no entries rather than a placeholder.
        WitnessStatus? older = WitnessTools.ParseStatus(
            "{\"state\":\"absent\",\"state_code\":0,\"pinned_measurement_count\":0}");
        Assert.NotNull(older);
        Assert.Null(older!.PinnedMeasurementLine);
        Assert.Empty(older.PinnedMeasurements);
    }

    /// <summary>
    /// This shell authors no wording on the witness surface.
    ///
    /// Read from the implementation's own source for the reason
    /// <c>NoWordingIsAuthoredInThisShell</c> gives about routing: a
    /// hand-written word that happened to match the Rust would pass every
    /// behavioural test here and then survive a rename in exactly one of the
    /// three shells. Every string literal in <c>WitnessTools.cs</c> and
    /// <c>WitnessSurface.cs</c> must be a wire value.
    /// </summary>
    [Fact]
    public void NoWordingIsAuthoredInTheWitnessSurface()
    {
        var allowed = new HashSet<string>(StringComparer.Ordinal)
        {
            // tc_witness_status_json's keys.
            "state_code", "", "Unsupported", "refused", "neutral", "Complete",
            "native_wallet_flow", "open", "check", "start", "wait", "cancel",
            "flow_id", "state", "busy", "can_edit", "can_check", "can_start",
            "can_cancel", "message", "tone", "glyph", "browser_url",
            "prepare_admission_session", "methods", "view", "ready",
        };

        foreach (string file in new[] { "WitnessTools.cs.txt", "WitnessSurface.cs.txt", "NearAccountConnection.cs.txt", "AdmissionPreparation.cs.txt" })
        {
            string path = Path.Combine(AppContext.BaseDirectory, file);
            Assert.True(File.Exists(path), $"the implementation source was not copied to {path}");
            string uncommented = string.Join(
                "\n",
                File.ReadAllText(path)
                    .Split('\n')
                    .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));

            foreach (Match match in Regex.Matches(uncommented, "\"([^\"\\\\]|\\\\.)*\""))
            {
                string literal = match.Value[1..^1];
                Assert.True(
                    allowed.Contains(literal),
                    $"\"{literal}\" is a string literal in {file} that is not a wire value. "
                    + "Wording on this surface comes from witness_copy.rs across the ABI.");
            }
        }
    }

    /// <summary>
    /// The card paints a refusal of its own, rather than borrowing the
    /// attention branch.
    /// </summary>
    /// <remarks>
    /// Asserted about the XAML because nothing else can be: the WinUI project
    /// does not build on the machines this suite runs on, so without this the
    /// half of the surface a contributor actually looks at is unchecked. A
    /// refused reading rendered through <c>IsAttention</c> would say something
    /// on this machine needs fixing while sessions still go out -- and none
    /// are going out at all.
    /// </remarks>
    [Fact]
    public void TheWitnessCardPaintsARefusalOfItsOwn()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "SettingsView.xaml.txt");
        Assert.True(File.Exists(path), $"the settings view was not copied to {path}");
        string xaml = File.ReadAllText(path);

        foreach (string bind in new[]
        {
            "Settings.WitnessStateIsNeutral",
            "Settings.WitnessStateIsHeld",
            "Settings.WitnessStateIsClear",
            "Settings.WitnessStateIsAttention",
            "Settings.WitnessStateIsRefused",
            "Settings.WitnessLastResultIsRefused",
            // The count's sentence, on its own visibility. Shown when the ABI
            // had one and not otherwise -- never a placeholder, and never a
            // bare numeral this shell composed.
            "Settings.WitnessMeasurementLine",
            "Settings.HasWitnessMeasurementLine",
        })
        {
            Assert.Contains(bind, xaml, StringComparison.Ordinal);
        }

        // The refusal is coral, which no other reading on this card is.
        Assert.Contains("TcCoralTextBrush", xaml, StringComparison.Ordinal);
    }

    /// <summary>
    /// The view model reads the witness tone through the witness mapper.
    /// </summary>
    /// <remarks>
    /// The failure this guards is one line long: <c>RoutingSurface</c> is
    /// already in scope on that screen, and its mapper compiles against an
    /// <c>int</c> just as happily. It would send every witness tone to
    /// <c>Neutral</c> and paint a total upload outage as "nothing to say".
    /// </remarks>
    [Fact]
    public void TheSettingsScreenReadsTheWitnessToneThroughTheWitnessMapper()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "ContributorSettingsViewModel.cs.txt");
        Assert.True(File.Exists(path), $"the view model source was not copied to {path}");
        string uncommented = string.Join(
            "\n",
            File.ReadAllText(path)
                .Split('\n')
                .Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal))
                .Where(line => !line.TrimStart().StartsWith("///", StringComparison.Ordinal)));

        Assert.Contains("WitnessSurface.StateTone(", uncommented, StringComparison.Ordinal);
        Assert.Contains("WitnessSurface.LastResultTone(", uncommented, StringComparison.Ordinal);

        // No witness tone is ever recovered from a rendered sentence, and no
        // witness reading is ever taken from the routing table.
        Assert.DoesNotContain("RoutingSurface.StateTone(WitnessTone", uncommented, StringComparison.Ordinal);
        Assert.DoesNotContain("RoutingTone _witness", uncommented, StringComparison.Ordinal);

        // The measurements box is pre-filled from the read-back, through the
        // helper that touches nothing. A box built any other way is a box that
        // can rewrite a pin nobody edited -- and one left empty makes an
        // untouched configuration indistinguishable from a cleared one, so
        // changing only the URL would refuse.
        Assert.Contains(
            "WitnessTools.JoinMeasurements(status.PinnedMeasurements)",
            uncommented,
            StringComparison.Ordinal);
        Assert.Contains("status?.PinnedMeasurementLine", uncommented, StringComparison.Ordinal);
    }

    /// <summary>
    /// A device with no account answers "not enrolled", and the status call
    /// declines rather than reporting an absence.
    /// </summary>
    /// <remarks>
    /// Across the real ABI, against a directory that holds no config. The
    /// distinction is the one the header spells out: a null status is never
    /// "no witness", which is a successful call reporting state absent.
    /// </remarks>
    [Fact]
    public void AnUnenrolledDirectoryIsNotEnrolledAndNotAbsent()
    {
        string dir = Path.Combine(
            Path.GetTempPath(),
            "tcw-" + Guid.NewGuid().ToString("n").Substring(0, 8));
        Directory.CreateDirectory(dir);
        try
        {
            Assert.Equal(WitnessTools.StateNotEnrolled, WitnessSurface.TrustState(dir));

            WitnessReadResult status = WitnessSurface.Status(dir);
            Assert.Null(status.Status);
            Assert.False(string.IsNullOrEmpty(status.Error));

            // And configuring one there fails with a label, not a silent
            // half-write.
            Assert.NotEqual(
                0,
                WitnessSurface.Configure(dir, "https://witness.example", "0xabc", "[\"mrtd=aa\"]").Code);
        }
        finally
        {
            Directory.Delete(dir, recursive: true);
        }
    }
}
