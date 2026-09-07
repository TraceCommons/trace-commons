namespace TraceCommons.Interop;

/// <summary>
/// What a press on the model-calls row does.
/// </summary>
/// <remarks>
/// Two values and not a toggle, because the two directions are not
/// symmetrical. Turning it OFF only ever reduces what this computer will
/// answer, so it is safe from a menu with nothing else on screen. Turning it
/// ON changes what anything else running here may send through, charged to
/// the contributor's own accounts, and the sentence that says so is the
/// reason model calls became a destination rather than a settings switch. A
/// menu press that enabled it would route around that sentence, so from the
/// menu the on direction opens the screen and writes nothing.
/// </remarks>
public enum PrivateInferenceTrayAction
{
    /// <summary>Raise the window at the model-calls destination. Writes nothing.</summary>
    OpenDestination,

    /// <summary>Stop answering model calls on this computer.</summary>
    StopAnswering,
}

/// <summary>
/// The model-calls entry in the notification-area menu: what it says, and
/// whether anything on it may be drawn as working.
///
/// <para>
/// Computed here rather than in the tray class so it can be tested off
/// Windows, and so the one rule that matters is written once. That rule is
/// that <see cref="ReadsAsWorking"/> comes from the daemon's reported state
/// and never from <see cref="On"/>. The switch is what was asked for; the
/// state is what happened. A contributor who turned it on over a port that
/// was already taken has <c>On == true</c> and a listener that never
/// started, and a tray that lit a glyph from the switch would tell them
/// their model calls are being answered here when nothing is answering.
/// </para>
/// <para>
/// <see cref="On"/> is still carried, because it is what decides which of
/// the two asymmetric actions the row offers -- see
/// <see cref="PrivateInferenceTrayAction"/>. It is the switch's own position,
/// never a claim about the world, and nothing that paints may read it.
/// </para>
/// </summary>
public readonly record struct PrivateInferenceTrayEntry
{
    /// <summary>
    /// Whether the entry belongs in the menu at all. False when the words did
    /// not arrive: a menu row carrying a switch and no sentence beside it is
    /// the shape that says "on" over a listener that refused to start, and
    /// the settings card refuses to render for the same reason.
    /// </summary>
    public bool Available { get; init; }

    /// <summary>The destination's own name, as the Rust spells it.</summary>
    public string Label { get; init; }

    /// <summary>The sentence for the reported state.</summary>
    public string StateText { get; init; }

    /// <summary>Where the switch is. Picks the action below, and nothing else.</summary>
    public bool On { get; init; }

    /// <summary>The one action the row offers, in the words the Rust spells.</summary>
    public string ActionText { get; init; }

    /// <summary>
    /// What a press on the action row does. Off it opens the destination and
    /// writes nothing; on it stops answering.
    /// </summary>
    public PrivateInferenceTrayAction Action => ActionFor(On);

    /// <summary>
    /// The same rule, for a caller that has no entry -- the keyboard chord.
    /// </summary>
    /// <remarks>
    /// It is here rather than restated at the call site because the chord is
    /// the surface where getting it wrong is worst: it fires with nothing on
    /// screen, so a chord that flipped the switch could open a listener to
    /// everything on this machine without the exposure sentence ever being
    /// shown. Off, it opens the destination and writes nothing; on, it stops
    /// answering. One table, asked twice.
    /// </remarks>
    public static PrivateInferenceTrayAction ActionFor(bool on) =>
        on ? PrivateInferenceTrayAction.StopAnswering : PrivateInferenceTrayAction.OpenDestination;

    /// <summary>
    /// Whether pressing the action row writes a setting at all. False
    /// whenever it is off: that is the safety claim this type exists to make,
    /// and it is asserted directly rather than inferred from the wording.
    /// </summary>
    public bool ActionWrites => Action == PrivateInferenceTrayAction.StopAnswering;

    /// <summary>
    /// Whether anything on this entry may be drawn as working. Derived from
    /// the reported state alone.
    /// </summary>
    public bool ReadsAsWorking { get; init; }

    /// <summary>
    /// The entry for one payload, one reported state and one switch position.
    /// </summary>
    public static PrivateInferenceTrayEntry For(
        PrivateInferenceCopy? copy, PrivateInferenceState state, bool on)
    {
        if (copy is null)
        {
            return new PrivateInferenceTrayEntry
            {
                Available = false,
                Label = string.Empty,
                StateText = string.Empty,
                ActionText = string.Empty,
                On = false,
                ReadsAsWorking = false,
            };
        }

        return new PrivateInferenceTrayEntry
        {
            Available = true,
            Label = copy.Destination,
            StateText = PrivateInferenceSurface.StateLine(state, copy),
            ActionText = on ? copy.TrayTurnOff : copy.TrayOpenToTurnOn,
            On = on,

            // The state, never `on`. See the class remarks.
            ReadsAsWorking = PrivateInferenceSurface.Tone(state).ReadsAsWorking(),
        };
    }
}
