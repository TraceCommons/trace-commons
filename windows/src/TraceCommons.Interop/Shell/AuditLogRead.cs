using System;
using System.Collections.Generic;
using System.Threading.Tasks;

namespace TraceCommons.Interop.Shell;

/// <summary>Where the change log's read stands (<c>SettingsRead</c>, SettingsStateRow.swift:36-44).</summary>
public enum AuditLogState
{
    /// <summary>Asked, and not answered yet.</summary>
    Awaiting,
    /// <summary>Answered at least once; what it said is real, even if stale.</summary>
    Answered,
    /// <summary>The last call failed and nothing has answered before it.</summary>
    Failed,
}

/// <summary>
/// The change log's read of <c>list_audit</c> (<c>ChangesSection</c>,
/// ChangesSection.swift:11-15; <c>AppModel.refreshAudit</c>). The log is
/// empty until the call answers, and a failed call leaves it so; "No
/// changes yet." is said only once a read answered with no entries,
/// because before that it would be a count nothing reported. Once
/// answered, a later failure keeps what the last answer said. A reply
/// overtaken by a newer read is dropped. Every error reads as a failed
/// read; nothing throws out. Continuations resume on the caller's context.
/// </summary>
public sealed class AuditLogRead
{
    private readonly Func<string, string, Task<DaemonResponse>> _call;
    private long _ticket;

    public AuditLogRead(Func<string, string, Task<DaemonResponse>> call)
    {
        _call = call;
    }

    public AuditLogState State { get; private set; } = AuditLogState.Awaiting;

    /// <summary>The last answered log, newest first; empty until one answers.</summary>
    public IReadOnlyList<AuditSettingEntry> Entries { get; private set; } = Array.Empty<AuditSettingEntry>();

    /// <summary>The read notice stands in the log's place until a read answers.</summary>
    public bool ShowsReadNotice => State != AuditLogState.Answered;

    /// <summary>Whether the read failed with nothing answered before it (the notice's failure line).</summary>
    public bool ReadFailed => State == AuditLogState.Failed;

    /// <summary>"No changes yet.": only an answered, empty log.</summary>
    public bool ShowsNothingChanged => State == AuditLogState.Answered && Entries.Count == 0;

    /// <summary>Reads the log once (the newest twenty, as before).</summary>
    public async Task ReadAsync()
    {
        long ticket = ++_ticket;
        AuditSettingsPayload? payload = null;
        try
        {
            DaemonResponse response = await _call(DaemonProtocol.Methods.ListAudit, "{\"limit\":20}").ConfigureAwait(true);
            if (!response.IsError)
            {
                payload = response.ResultAs<AuditSettingsPayload>();
            }
        }
        catch (Exception)
        {
            payload = null;
        }

        if (ticket != _ticket)
        {
            return;
        }

        if (payload is not null)
        {
            Entries = payload.Entries ?? new List<AuditSettingEntry>();
            State = AuditLogState.Answered;
        }
        else if (State != AuditLogState.Answered)
        {
            State = AuditLogState.Failed;
        }
    }
}
