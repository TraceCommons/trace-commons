using TraceCommons.Interop.Shell;

namespace TraceCommons.Interop;

/// <summary>
/// The Private AI tab's heading row (<c>PrivateAIInspectorView.summary</c>,
/// InferenceViews.swift:444-492): the core's status title, then the
/// listener's state in a chip (the menu's one word and dot), the core's full
/// sentence as the row's tooltip and help text, and the subtitle under it,
/// with the credit sentence only while NEAR AI answers calls.
/// </summary>
public sealed record PrivateAIHeading(string Title, string Word, DotStatus Dot, string Sentence, string Subtitle)
{
    /// <summary>The state label that means NEAR AI answers calls (InferenceViews.swift:474-476).</summary>
    public const string Running = "running";

    /// <summary>The row for a reported state, through the core's tables.</summary>
    public static PrivateAIHeading For(PrivateInferenceState state, PrivateInferenceCopy copy) =>
        For(
            state.Label,
            copy,
            PrivateInferenceSurface.StateLine(state, copy),
            NativeMethods.TakeOwnedString(NativeMethods.tc_private_inference_menu_word(state.Label)) ?? string.Empty,
            PrivateInferenceSurface.FromAbiTone(NativeMethods.tc_private_inference_menu_tone(state.Label)));

    /// <summary>
    /// The row from the core's sentence, menu word and tone for
    /// <paramref name="label"/> (<c>statusRow</c>, InferenceViews.swift:483-492):
    /// a core that gave no word draws the sentence, on grey.
    /// </summary>
    public static PrivateAIHeading For(
        string label, PrivateInferenceCopy copy, string sentence, string menuWord, PrivateInferenceTone menuTone)
    {
        string subtitle = label == Running ? copy.Subtitle + " " + copy.SubtitleCredit : copy.Subtitle;
        if (string.IsNullOrEmpty(menuWord))
        {
            return new PrivateAIHeading(copy.InspectorStatus, sentence, DotStatus.Off, sentence, subtitle);
        }

        // MenuPanelStatus.privateAIDot (MenuBarGlassPanel.swift:963-965,
        // PrivateInferenceIndicator.status): clear green, attention amber,
        // refused red, otherwise grey.
        DotStatus dot = menuTone switch
        {
            PrivateInferenceTone.Clear => DotStatus.On,
            PrivateInferenceTone.Attention => DotStatus.Ask,
            PrivateInferenceTone.Refused => DotStatus.Outside,
            _ => DotStatus.Off,
        };
        return new PrivateAIHeading(copy.InspectorStatus, menuWord, dot, sentence, subtitle);
    }
}
