namespace TraceCommons.Interop.Shell;

/// <summary>The status a Private AI dot is drawn in (<c>GlassStatus</c>).</summary>
public enum DotStatus
{
    /// <summary>No dot: the core gave the state no label.</summary>
    None,
    /// <summary>Green: on, answering, with somewhere to pass calls on to.</summary>
    On,
    /// <summary>Amber: something on this machine wants attention.</summary>
    Ask,
    /// <summary>Red: asked for, and the listener refused or is not working.</summary>
    Outside,
    /// <summary>Grey: neutral or held, no readiness claimed.</summary>
    Off,
}

/// <summary>
/// The Private AI tab's dot (<c>MonitorWindowView.inferenceDot</c>,
/// MonitorWindowView.swift:853-856, and <c>PrivateInferenceIndicator</c>,
/// PrivateInferenceView.swift:33-52): no dot without a state label, else the
/// core's tone drawn as clear on, attention ask, refused outside, and neutral
/// or held off. Its text equivalent is the core's sentence for the same state
/// (<c>inferenceDotDescription</c>, :859-862).
/// </summary>
public static class PrivateAIDot
{
    /// <summary>The dot's size across, points (Navigation.swift:70, <c>GlassStatusDot(status, size: 6)</c>).</summary>
    public const double Size = 6;

    /// <summary>The dot for a reported state, through the core's menu table.</summary>
    public static DotStatus Of(PrivateInferenceState state) =>
        string.IsNullOrEmpty(state.Label)
            ? DotStatus.None
            : For(
                state.Label,
                NativeMethods.TakeOwnedString(NativeMethods.tc_private_inference_menu_word(state.Label)) ?? string.Empty,
                PrivateInferenceSurface.FromAbiTone(NativeMethods.tc_private_inference_menu_tone(state.Label)));

    /// <summary>
    /// The dot for a state <paramref name="label"/>, given the core's menu
    /// word and tone for it (<c>PrivateInferenceSurface.menuState</c>,
    /// PrivateInferenceSurface.swift:651-657): grey when the menu has no word.
    /// </summary>
    public static DotStatus For(string label, string menuWord, PrivateInferenceTone tone) =>
        string.IsNullOrEmpty(label)
            ? DotStatus.None
            : string.IsNullOrEmpty(menuWord)
            ? DotStatus.Off
            : tone switch
            {
                PrivateInferenceTone.Clear => DotStatus.On,
                PrivateInferenceTone.Attention => DotStatus.Ask,
                PrivateInferenceTone.Refused => DotStatus.Outside,
                _ => DotStatus.Off,
            };
}
