using System;
using System.Collections.Generic;
using System.Text.Encodings.Web;
using System.Text.Json;

namespace TraceCommons.Interop;

/// <summary>
/// How firmly a witness sentence reads.
///
/// Five values, and the fifth is the reason this is not
/// <see cref="RoutingTone"/>. A configured witness with nothing pinned sends
/// NOTHING AT ALL, and neither tone that could otherwise carry it is honest:
/// <see cref="Attention"/> means something needs fixing but sessions still go
/// out, and <see cref="Neutral"/> reads as off. A refusal is neither.
/// </summary>
public enum WitnessTone
{
    /// <summary>
    /// Says nothing either way. No witness is configured, which is a
    /// supported arrangement -- local redaction -- and not a fault.
    /// </summary>
    Neutral,

    /// <summary>Configured, and no answer has arrived yet.</summary>
    Held,

    /// <summary>Configured, pinned, and working.</summary>
    Clear,

    /// <summary>Something on this machine needs fixing, but sessions still go out.</summary>
    Attention,

    /// <summary>Nothing is being sent at all until this is resolved.</summary>
    Refused,
}

/// <summary>
/// The witness surface's branch decisions that are not the Rust's.
///
/// Nothing in this file is a word. The sentences and the tones cross the ABI
/// already decided; what is left here is the mapping of the ABI's integers
/// onto this shell's types, the parse of two JSON payloads, and the one
/// affordance decision -- whether the editor is open -- that is about controls
/// rather than about wording.
/// </summary>
public static class WitnessTools
{
    // --- TC_WITNESS_STATE_* ---------------------------------------------
    //
    // Spelled out rather than inferred. A value this build does not name is
    // handled by fail-closed defaults, never by falling through to Absent.

    /// <summary>No witness configured. Local redaction runs. NOT a warning.</summary>
    public const int StateAbsent = 0;

    /// <summary>Configured and pinned. Submissions go through the witness.</summary>
    public const int StatePinned = 1;

    /// <summary>Configured, nothing pinned. EVERY SUBMISSION IS REFUSED.</summary>
    public const int StateRefusingUnpinned = 2;

    /// <summary>
    /// Configured, pins unparsable. Also a total refusal, and a different
    /// mistake: somebody who mistyped a measurement must not be told they
    /// pinned none.
    /// </summary>
    public const int StateRefusingPinMalformed = 3;

    /// <summary>
    /// Configured and pinned, refusing because a session's inferences carried
    /// no verified receipts. Reserved: no build returns it yet, and the branch
    /// exists so that the day one does, this shell already has a reading for
    /// it.
    /// </summary>
    public const int StateRefusingInferenceReceiptsMissing = 4;

    /// <summary>
    /// No account on this device, so there is no config to hold a witness.
    /// Not an absence -- absence is a decision somebody made -- and not a
    /// refusal either.
    /// </summary>
    public const int StateNotEnrolled = -1;

    /// <summary>
    /// The config could not be read. NOT an absence: a client whose behaviour
    /// is unknown is not a client redacting locally.
    /// </summary>
    public const int StateUnreadable = -2;

    // --- TC_WITNESS_TONE_* ----------------------------------------------
    //
    // DELIBERATELY DISJOINT FROM TC_ROUTING_TONE_*, which stops at
    // ATTENTION = 3. See FromAbiTone.

    private const int AbiToneNeutral = 10;
    private const int AbiToneHeld = 11;
    private const int AbiToneClear = 12;
    private const int AbiToneAttention = 13;
    private const int AbiToneRefused = 14;

    /// <summary>
    /// The ABI's <c>TC_WITNESS_TONE_*</c>, and NOT
    /// <see cref="RoutingSurface"/>'s table.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Two things about this method are load-bearing and neither is obvious.
    /// </para>
    /// <para>
    /// It is a SEPARATE mapper. The routing tone runs 0..3 and its consumers
    /// -- <c>RoutingSurface.FromAbiTone</c> among them -- spell out their arms
    /// and send everything else to neutral. Feeding a witness tone through
    /// that table would render a refusal, which means nothing is being sent at
    /// all, as "nothing to say". That is the precise failure this surface
    /// exists to prevent, and the disjoint numbering is what makes the mistake
    /// wrong for every value rather than only for the dangerous one.
    /// </para>
    /// <para>
    /// Its unknown arm is <see cref="WitnessTone.Refused"/> and not
    /// <see cref="WitnessTone.Neutral"/>. Every value the ABI adds later is a
    /// condition this build has no words for, and on a surface about whether
    /// raw sessions leave the machine, the safe reading of "I do not know" is
    /// "they are not". A neutral default would turn a future refusal into
    /// silence.
    /// </para>
    /// </remarks>
    public static WitnessTone FromAbiTone(int value) => value switch
    {
        AbiToneNeutral => WitnessTone.Neutral,
        AbiToneHeld => WitnessTone.Held,
        AbiToneClear => WitnessTone.Clear,
        AbiToneAttention => WitnessTone.Attention,
        AbiToneRefused => WitnessTone.Refused,
        _ => WitnessTone.Refused,
    };

    /// <summary>
    /// Whether the address-and-pin editor starts open for a state.
    /// </summary>
    /// <remarks>
    /// <para>
    /// A refusal must have a way out. Every refusing state on this surface is
    /// fixed by the same three fields, so the editor holding them is opened
    /// rather than left behind a disclosure a contributor has no reason to
    /// suspect.
    /// </para>
    /// <para>
    /// This is a decision about controls, not about wording or tone: the
    /// sentence and the colour both come from the Rust, and nothing here may
    /// second-guess them. Closed for exactly two readings -- the ordinary
    /// arrangement, where opening an editor would present local redaction as
    /// something needing repair, and a device with no account, which has
    /// nothing to answer here yet. A state this build cannot name opens, on
    /// the same reasoning as <see cref="FromAbiTone"/>'s default.
    /// </para>
    /// </remarks>
    public static bool EditorOpensFor(int stateCode) =>
        stateCode != StateAbsent && stateCode != StatePinned && stateCode != StateNotEnrolled;

    /// <summary>
    /// The card's words, or null when the payload will not parse or arrived
    /// incomplete.
    /// </summary>
    /// <remarks>
    /// Null, never a partly-filled record. A screen rendering a blank where a
    /// sentence about privacy belongs is worse than a screen rendering
    /// nothing, and one rendering a C#-authored sentence is worse than both.
    /// </remarks>
    public static WitnessCopy? ParseCopy(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            WitnessCopy? copy = JsonSerializer.Deserialize<WitnessCopy>(json!);
            if (copy is null)
            {
                return null;
            }

            foreach (string word in copy.Words)
            {
                if (string.IsNullOrEmpty(word))
                {
                    return null;
                }
            }

            return copy;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>
    /// The witness configuration, or null when the payload will not parse or
    /// carries no state code.
    /// </summary>
    /// <remarks>
    /// THE MISSING-KEY CASE IS THE POINT. An absent <c>state_code</c> binds to
    /// the default zero and zero is <see cref="StateAbsent"/>, so a payload
    /// from a build that renamed the key would be read as a witness-free
    /// machine -- the exact conflation between "no witness" and "a witness
    /// that refuses everything" the whole surface exists to prevent, arriving
    /// through the deserialiser instead of through a branch.
    /// </remarks>
    public static WitnessStatus? ParseStatus(string? json)
    {
        if (string.IsNullOrWhiteSpace(json))
        {
            return null;
        }

        try
        {
            using (JsonDocument document = JsonDocument.Parse(json!))
            {
                if (document.RootElement.ValueKind != JsonValueKind.Object
                    || !document.RootElement.TryGetProperty("state_code", out JsonElement code)
                    || code.ValueKind != JsonValueKind.Number)
                {
                    return null;
                }
            }

            WitnessStatus? status = JsonSerializer.Deserialize<WitnessStatus>(json!);
            return status?.StateCodeOrNull is null ? null : status;
        }
        catch (JsonException)
        {
            return null;
        }
    }

    /// <summary>
    /// The typed measurements, as the JSON array
    /// <c>tc_witness_configure</c> takes.
    /// </summary>
    /// <remarks>
    /// One measurement set per line, because it is a LIST: an image upgrade
    /// moves the measurement and leaves the signing address alone, so the new
    /// one is added before the fleet rolls and a client holding only the old
    /// one refuses the upgraded witness.
    ///
    /// Blank lines are dropped rather than written. The ABI refuses an empty
    /// array outright -- it will not write an unpinned witness, because that
    /// is a client refusing every submission from the moment it is saved -- so
    /// an empty result is passed through as an empty array and declined there,
    /// not turned into something else on the way.
    /// </remarks>
    public static string SerializeMeasurements(string? text)
    {
        var entries = new List<string>();
        foreach (string line in (text ?? string.Empty).Split('\n'))
        {
            string trimmed = line.Trim();
            if (trimmed.Length > 0)
            {
                entries.Add(trimmed);
            }
        }

        return JsonSerializer.Serialize(
            entries,
            new JsonSerializerOptions { Encoder = JavaScriptEncoder.UnsafeRelaxedJsonEscaping });
    }
}
