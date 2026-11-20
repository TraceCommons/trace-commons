namespace TraceCommons.Interop.Shell;

/// <summary>
/// Which window messages say the system's motion setting changed. Windows
/// broadcasts WM_SETTINGCHANGE with the SystemParametersInfo action in
/// wParam when "Animation effects" is switched; UISettings'
/// AnimationsEnabledChanged was not raised for it on the test box, so a map
/// stopped by turning motion off never started again when it was turned
/// back on.
/// </summary>
public static class MotionSettingMessage
{
    public const uint WmSettingChange = 0x001A;

    /// <summary>SPI_SETCLIENTAREAANIMATION, the switch UISettings.AnimationsEnabled reads.</summary>
    public const ulong SpiSetClientAreaAnimation = 0x1043;

    /// <summary>SPI_SETANIMATION, the window open and close animation, switched with it.</summary>
    public const ulong SpiSetAnimation = 0x0049;

    /// <summary>Whether this message may have changed the motion setting.</summary>
    public static bool MayChangeMotion(uint message, ulong wParam) =>
        message == WmSettingChange && (wParam == SpiSetClientAreaAnimation || wParam == SpiSetAnimation);
}
