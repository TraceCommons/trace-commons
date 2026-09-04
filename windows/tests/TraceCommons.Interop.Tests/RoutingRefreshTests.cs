using System;
using System.Linq;
using System.Reflection;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The routing surface repainting itself while the pane is open.
///
/// The surface used to read <c>status.routing</c> on load and again only
/// after the contributor wrote something, so a state change while they sat
/// looking at the card was invisible until they touched it. The shell now
/// repaints from the daemon's own <c>StatusChanged</c> event, as the GTK
/// shell always has; <see cref="RoutingRefreshGate"/> is the part of that
/// worth asserting off Windows.
///
/// Wording assertions live in <see cref="RoutingSurfaceTests"/> and every
/// word here still comes from <see cref="RoutingSurface.Copy"/> across the
/// real ABI.
/// </summary>
public class RoutingRefreshTests
{
    private static readonly DateTimeOffset T0 =
        new(2026, 9, 2, 12, 0, 0, TimeSpan.Zero);

    private static RoutingCopy Copy()
    {
        RoutingCopy? copy = RoutingSurface.Copy();
        Assert.NotNull(copy);
        return copy!;
    }

    // --- When an event may ask IronWire ---------------------------------

    /// <summary>
    /// Nothing declared, so there is nothing to ask about. Events keep
    /// arriving on this surface whatever the switch says -- the queue and the
    /// daemon's own status raise them -- and each one must stop here.
    /// </summary>
    [Fact]
    public void AnEventAsksNothingWhileNothingIsDeclared()
    {
        var gate = new RoutingRefreshGate();

        Assert.False(gate.TryBeginProbe(declared: false, T0, out long ticket));
        Assert.False(gate.IsProbeInFlight);
        Assert.Null(gate.AnsweredAt);

        // Declaring it is what makes the same event ask.
        Assert.True(gate.TryBeginProbe(declared: true, T0, out long allowed));
        Assert.True(gate.IsProbeInFlight);
        Assert.Equal(ticket, allowed);
    }

    /// <summary>
    /// A burst of events opens one connection, not one per event.
    /// </summary>
    [Fact]
    public void ASecondEventDoesNotStartASecondCallWhileOneIsInFlight()
    {
        var gate = new RoutingRefreshGate();
        Assert.True(gate.TryBeginProbe(declared: true, T0, out long first));

        Assert.False(gate.TryBeginProbe(declared: true, T0.AddSeconds(1), out _));
        Assert.False(gate.TryBeginProbe(declared: true, T0.AddMinutes(30), out _));

        // Only once the first one is done does the next event get through.
        Assert.True(gate.CompleteWithAnswer(first, T0.AddSeconds(2)));
        Assert.True(gate.TryBeginProbe(declared: true, T0.AddHours(1), out _));
    }

    /// <summary>
    /// An answer already held is not re-asked for, until the backstop.
    /// </summary>
    [Fact]
    public void AHeldAnswerIsNotReAskedForUntilTheBackstopHasPassed()
    {
        var gate = new RoutingRefreshGate();
        Assert.True(gate.TryBeginProbe(declared: true, T0, out long ticket));
        Assert.True(gate.CompleteWithAnswer(ticket, T0));
        Assert.Equal(T0, gate.AnsweredAt);

        DateTimeOffset justInside = T0 + RoutingRefreshGate.EvidenceBackstop - TimeSpan.FromSeconds(1);
        Assert.False(gate.TryBeginProbe(declared: true, justInside, out _));

        DateTimeOffset atTheEdge = T0 + RoutingRefreshGate.EvidenceBackstop;
        Assert.True(gate.TryBeginProbe(declared: true, atTheEdge, out _));
    }

    /// <summary>
    /// A call that did not run is not a fact about any tool, so it takes no
    /// stamp and does not hold the next event off.
    /// </summary>
    [Fact]
    public void ACallThatDidNotAnswerLeavesNoStampAndTheNextEventAsksAgain()
    {
        var gate = new RoutingRefreshGate();
        Assert.True(gate.TryBeginProbe(declared: true, T0, out long ticket));

        Assert.True(gate.CompleteWithoutAnswer(ticket));

        Assert.False(gate.IsProbeInFlight);
        Assert.Null(gate.AnsweredAt);
        Assert.True(gate.TryBeginProbe(declared: true, T0.AddSeconds(1), out _));
    }

    /// <summary>
    /// A check that failed about a declaration since replaced says nothing
    /// either. The contributor is owed the failure of the check they are
    /// looking at, not of one about a port this machine has stopped naming.
    /// </summary>
    [Fact]
    public void AFailedCallAboutTheOldDeclarationIsNotWorthSayingEither()
    {
        var gate = new RoutingRefreshGate();
        Assert.True(gate.TryBeginProbe(declared: true, T0, out long ticket));

        gate.Forget();

        Assert.False(gate.CompleteWithoutAnswer(ticket));
        Assert.False(gate.IsProbeInFlight);
    }

    /// <summary>
    /// A contributor pressing something is owed a fresh answer whatever is
    /// held, which is the one path the backstop does not gate.
    /// </summary>
    [Fact]
    public void APressAsksEvenWithAFreshAnswerHeld()
    {
        var gate = new RoutingRefreshGate();
        Assert.True(gate.TryBeginProbe(declared: true, T0, out long ticket));
        Assert.True(gate.CompleteWithAnswer(ticket, T0));
        Assert.False(gate.TryBeginProbe(declared: true, T0.AddSeconds(1), out _));

        long pressed = gate.BeginProbe();

        Assert.True(gate.IsProbeInFlight);
        Assert.True(gate.CompleteWithAnswer(pressed, T0.AddSeconds(2)));
    }

    // --- What a repaint may not paint -----------------------------------

    /// <summary>
    /// The declaration changed while a call was in flight, so its answer is
    /// about a port or folder this machine is no longer pointed at. It is
    /// discarded rather than painted.
    /// </summary>
    [Fact]
    public void AnAnswerAboutTheOldDeclarationIsDiscardedRatherThanPainted()
    {
        var gate = new RoutingRefreshGate();
        Assert.True(gate.TryBeginProbe(declared: true, T0, out long ticket));

        gate.Forget();

        Assert.False(gate.CompleteWithAnswer(ticket, T0.AddSeconds(1)));
        // And the discarded answer took no stamp, so the next event asks
        // about the declaration that is actually held now.
        Assert.Null(gate.AnsweredAt);
        Assert.True(gate.TryBeginProbe(declared: true, T0.AddSeconds(2), out _));
    }

    /// <summary>
    /// Forgetting drops the held stamp too, so turning the switch back on
    /// cannot reuse a verdict taken about the previous declaration.
    /// </summary>
    [Fact]
    public void ForgettingDropsAHeldAnswerAndNotOnlyAnInFlightOne()
    {
        var gate = new RoutingRefreshGate();
        Assert.True(gate.TryBeginProbe(declared: true, T0, out long ticket));
        Assert.True(gate.CompleteWithAnswer(ticket, T0));

        gate.Forget();

        Assert.Null(gate.AnsweredAt);
        Assert.True(gate.TryBeginProbe(declared: true, T0.AddSeconds(1), out _));
    }

    /// <summary>
    /// Waiting for the first rows is not a fault, and a repaint must not make
    /// it into one.
    /// </summary>
    /// <remarks>
    /// A contributor who just moved the switch sees this state until the
    /// reader's next tick, which is the single most likely moment for an
    /// event to arrive. Repainting it must keep saying the waiting sentence,
    /// held, and must never reach for the check-unavailable line or the
    /// nothing-declared one.
    /// </remarks>
    [Fact]
    public void RepaintingTheAwaitingRowsStateNeverTurnsItIntoAFault()
    {
        RoutingCopy copy = Copy();

        for (int repaint = 0; repaint < 3; repaint++)
        {
            RoutingStatusLine line = RoutingTools.StatusLine(
                copy,
                RoutingTools.AwaitingRows,
                T0.AddMinutes(-1),
                T0.AddSeconds(repaint));

            Assert.Equal(copy.StateWaiting, line.Text);
            Assert.Equal(RoutingTone.Held, line.Tone);
            Assert.NotEqual(copy.CheckUnavailable, line.Text);
            Assert.NotEqual(copy.StateOff, line.Text);
        }

        // And it is not the state that asks for something. Asserted on
        // awaiting_rows itself rather than on the shape of the tone enum:
        // this test is about one state's reading, and the vocabulary of
        // readings grows -- it grew when `token_unreadable` arrived, which
        // is a different state and is where `Attention` belongs.
        Assert.Equal(RoutingTone.Held, RoutingTools.StateTone(RoutingTools.AwaitingRows));
        Assert.NotEqual(RoutingTone.Attention, RoutingTools.StateTone(RoutingTools.AwaitingRows));
        Assert.NotEqual(RoutingTone.Neutral, RoutingTools.StateTone(RoutingTools.AwaitingRows));
    }

    /// <summary>
    /// The stamp is per-process. A daemon that restarted reports none, and a
    /// repaint drops the one it was showing rather than carrying it forward
    /// as something this install has been true since.
    /// </summary>
    [Fact]
    public void ARepaintAfterTheDaemonRestartedDropsTheStampRatherThanKeepingIt()
    {
        RoutingCopy copy = Copy();

        RoutingStatusLine before = RoutingTools.StatusLine(
            copy,
            RoutingTools.RowsSeen,
            T0.AddMinutes(-2),
            T0);
        Assert.NotNull(before.LastChecked);

        // Same state, no stamp: the daemon came back up.
        RoutingStatusLine after = RoutingTools.StatusLine(
            copy,
            RoutingTools.RowsSeen,
            null,
            T0.AddSeconds(1));

        Assert.Null(after.LastChecked);
        Assert.Equal(copy.StateReading, after.Text);
    }

    /// <summary>
    /// The repaint decision reads no word and no declaration switch.
    /// </summary>
    /// <remarks>
    /// Per-tool words come from <c>probe_routed_tools</c>. A refresh path
    /// that could see a rendered string is one edit away from deriving a word
    /// from it, so nothing on this type's surface is a string at all -- the
    /// only bool it takes is named for the declaration and is a reason not to
    /// ask, never an input to a verdict.
    /// </remarks>
    [Fact]
    public void TheRefreshDecisionCannotSeeAWord()
    {
        Type[] forbidden =
        {
            typeof(string),
            typeof(RoutingCopy),
            typeof(RoutingToolRow),
            typeof(RoutingModes),
            typeof(ToolWiring),
            typeof(RoutingEvidence),
        };

        foreach (MethodInfo method in typeof(RoutingRefreshGate)
                     .GetMethods(BindingFlags.Public | BindingFlags.Instance | BindingFlags.Static)
                     .Where(m => m.DeclaringType == typeof(RoutingRefreshGate)))
        {
            Assert.DoesNotContain(method.ReturnType, forbidden);
            foreach (ParameterInfo parameter in method.GetParameters())
            {
                Type type = parameter.ParameterType;
                Assert.DoesNotContain(
                    type.IsByRef ? type.GetElementType()! : type,
                    forbidden);
            }
        }

        foreach (PropertyInfo property in typeof(RoutingRefreshGate)
                     .GetProperties(BindingFlags.Public | BindingFlags.Instance | BindingFlags.Static))
        {
            Assert.DoesNotContain(property.PropertyType, forbidden);
        }
    }
}
