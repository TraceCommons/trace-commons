using System;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// One coding tool's row on the model-calls screen.
///
/// <para>
/// Not one sentence is composed here. The tool's name is the shared crate's,
/// read off the wire -- no shell and no copy constant spells it, so a tool
/// renamed upstream is not still called the old thing on Windows. Every other
/// word comes from <see cref="PrivateInferenceCopy"/> across the C ABI, and
/// every decision -- what the state means, which button may be offered --
/// comes from the shared branch tables in <see cref="HarnessSurface"/>.
/// </para>
/// <para>
/// <b>The row is painted from the state, never from
/// <see cref="HarnessRow.Connected"/>.</b> Connected proves a settings file
/// has a value in it. It proves nothing about a call, and the two disagree
/// exactly when it matters -- a tool pointed here whose every call goes
/// somewhere else.
/// </para>
/// </summary>
public sealed class HarnessRowViewModel
{
    private readonly HarnessRow _row;
    private readonly PrivateInferenceCopy _copy;

    public HarnessRowViewModel(HarnessRow row, PrivateInferenceCopy copy)
    {
        _row = row ?? throw new ArgumentNullException(nameof(row));
        _copy = copy ?? throw new ArgumentNullException(nameof(copy));
    }

    /// <summary>The id an action is planned against. Never drawn.</summary>
    public string Id => _row.Id;

    /// <summary>The tool's own name, as the shared crate reports it.</summary>
    public string Name => _row.Name;

    /// <summary>The file an action here would change, or the empty string.</summary>
    public string ConfigPath => _row.ConfigPath ?? string.Empty;

    public bool HasConfigPath => _row.HasConfigPath;

    /// <summary>
    /// The command that does the same thing outside this app, verbatim and
    /// selectable. A contributor who would rather not have an app edit their
    /// files gets the way to do it themselves.
    /// </summary>
    public string ConnectCommand => _row.ConnectCommand;

    public bool HasConnectCommand => _row.ConnectCommand.Length > 0;

    /// <summary>The sentence this row shows, which is not always its state's.</summary>
    /// <remarks>
    /// The daemon's own label goes across the ABI and the sentence comes
    /// back. This shell does not choose between the payload's fields, and
    /// neither do the other two: it is one table, asked three times.
    ///
    /// A tool that is not on this machine is the exception, and it is not a
    /// state at all -- it gets the missing-tool sentence instead. The row
    /// stays listed, because a tool left out cannot be told apart from a tool
    /// this app was never taught about, and it may not keep the not-connected
    /// sentence, which claims a tool's own settings still send its calls
    /// wherever they went before.
    /// </remarks>
    public string StateText => HarnessSurface.RowSentence(_row, _copy);

    public bool HasStateText => StateText.Length > 0;

    /// <summary>
    /// When a call from this tool was last answered here, or the empty
    /// string -- drawn as no line at all.
    /// </summary>
    /// <remarks>
    /// Assembled across the ABI from a number of seconds this shell works
    /// out. Read at draw time rather than latched, so a row that sat on
    /// screen does not keep claiming a call arrived a minute ago.
    /// </remarks>
    public string LastCallText =>
        HarnessSurface.LastCallSentence(_row.LastCallAt, DateTimeOffset.UtcNow);

    public bool HasLastCallText => LastCallText.Length > 0;

    /// <summary>
    /// Whether this row may be painted as working. The state, and only the
    /// state: answering alone.
    /// </summary>
    /// <remarks>
    /// A call that arrived in a protocol family two connected tools both speak
    /// is deliberately not this. The sentence beside it still says what this
    /// computer did; what it may not do is claim this tool is the one doing
    /// it.
    /// </remarks>
    public bool ReadsAsWorking => _row.ReadsAsWorking;

    /// <summary>The state line in the plain colour: everything that is not answering.</summary>
    public bool StateIsPlain => HasStateText && !ReadsAsWorking;

    public bool IsSettingsExpanded { get; set; }

    public string SettingsTitle => _copy.HarnessPreviewTitle;

    public string ConnectText => _copy.HarnessConnect;

    public string DisconnectText => _copy.HarnessDisconnect;

    public bool CanConnect => _row.CanConnect;

    public bool CanDisconnect => _row.CanDisconnect;
}
