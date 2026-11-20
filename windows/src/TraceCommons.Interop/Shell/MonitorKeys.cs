namespace TraceCommons.Interop.Shell;

/// <summary>The Monitor window's keys that are the Mac's, beyond the tab and Settings shortcuts.</summary>
public static class MonitorKeys
{
    /// <summary>
    /// Whether Return undoes the last approval: the Mac's one Return binding,
    /// on the undo card while the undo is offered (InspectorPrompts.swift:92-97,
    /// MonitorNotices.swift:673-678). A focused control that takes Return
    /// itself (a button, a field, a list) keeps it.
    /// </summary>
    public static bool ReturnUndoes(bool undoOffered, bool focusTakesReturn) => undoOffered && !focusTakesReturn;
}
