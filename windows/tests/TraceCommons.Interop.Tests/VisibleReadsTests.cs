using System.Collections.Generic;
using TraceCommons.Interop.Shell;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The Private AI ledger follows the daemon only while its tab is chosen and
/// the window can be seen (one-pass 13; the Mac's <c>.task</c>,
/// InferenceViews.swift:48-56, ends when the view leaves and starts again,
/// reading afresh, when it returns).
/// </summary>
public sealed class VisibleReadsTests
{
    private static (VisibleReads Gate, List<bool> Events) Gate()
    {
        var gate = new VisibleReads();
        var events = new List<bool>();
        gate.Changed += events.Add;
        return (gate, events);
    }

    [Fact]
    public void HidingTheWindowStopsTheTabsReadsAndShowingItReadsAgain()
    {
        (VisibleReads gate, List<bool> events) = Gate();
        gate.SetTabShown(true);
        gate.SetWindowShown(false);
        gate.SetWindowShown(true);
        Assert.Equal(new[] { true, false, true }, events);
    }

    [Fact]
    public void AHiddenWindowNeverStartsReadsWhenTheTabIsChosen()
    {
        (VisibleReads gate, List<bool> events) = Gate();
        gate.SetWindowShown(false);
        gate.SetTabShown(true);
        Assert.Empty(events);
        Assert.False(gate.Reading);
        gate.SetWindowShown(true);
        Assert.Equal(new[] { true }, events);
    }

    [Fact]
    public void AnotherTabStopsReadsAndTheWindowComingBackDoesNotRestartThem()
    {
        (VisibleReads gate, List<bool> events) = Gate();
        gate.SetTabShown(true);
        gate.SetTabShown(false);
        gate.SetWindowShown(false);
        gate.SetWindowShown(true);
        Assert.Equal(new[] { true, false }, events);
    }

    [Fact]
    public void RepeatingAStateRaisesNothing()
    {
        (VisibleReads gate, List<bool> events) = Gate();
        gate.SetTabShown(true);
        gate.SetTabShown(true);
        gate.SetWindowShown(true);
        Assert.Equal(new[] { true }, events);
    }
}
