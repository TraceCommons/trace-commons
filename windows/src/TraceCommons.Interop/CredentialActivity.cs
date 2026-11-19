using System;

namespace TraceCommons.Interop;

/// <summary>
/// Whether a NEAR AI sign-in or credential write is in flight, the one flag
/// the Private AI screen's credential card sets and every surface that must
/// wait for it reads (the Mac's <c>credentialBusy</c>, AppModel.swift:1104).
/// It is a type rather than a <c>Func&lt;bool&gt;</c> so the spending
/// prompt cannot be handed some other busy flag: the switch's own read and
/// write (<c>ControlsEnabled</c>) are not a credential action.
/// </summary>
public sealed class CredentialActivity
{
    private bool _busy;

    public bool Busy
    {
        get => _busy;
        set
        {
            if (_busy == value) return;
            _busy = value;
            Changed?.Invoke(value);
        }
    }

    public event Action<bool>? Changed;
}
