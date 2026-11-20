using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Monitor window's Return key against the Mac's: its one Return
/// binding is Undo, on the approval's undo card, and only while the undo is
/// offered (InspectorPrompts.swift:92-97 and MonitorNotices.swift:673-678,
/// <c>.keyboardShortcut(.defaultAction)</c> inside <c>if undo.offerUndo</c>).
/// On Windows a focused control that takes Return itself keeps it: Return on
/// a focused button presses that button, never Undo instead.
/// </summary>
public class MonitorKeysTests
{
    [Theory]
    [InlineData(true, false, true)]
    [InlineData(true, true, false)]
    [InlineData(false, false, false)]
    [InlineData(false, true, false)]
    public void ReturnUndoesOnlyWhileTheUndoIsOfferedAndFocusLeavesIt(bool undoOffered, bool focusTakesReturn, bool undoes) =>
        Assert.Equal(undoes, MonitorKeys.ReturnUndoes(undoOffered, focusTakesReturn));
}
