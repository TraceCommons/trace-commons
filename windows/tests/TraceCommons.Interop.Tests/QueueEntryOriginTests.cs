using System.Text.Json;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The queue names what a trace came FROM, not how it is stored.
///
/// An imported Antigravity conversation is staged as a trajectory file and
/// read by the <c>trajectory</c> adapter, so the adapter name alone labelled
/// it "Letta trajectory" -- the storage format, and not the word the
/// contributor typed to collect it.
///
/// What is covered here is the decode: this test project references
/// TraceCommons.Interop only, so QueueEntryViewModel's label mapping (which
/// consumes DeclaredSource) is not reachable from it. That mapping has no
/// tests today either way; this at least pins that the value survives the
/// wire, which is the half that would silently break.
/// </summary>
public sealed class QueueEntryOriginTests
{
    private static QueueEntry Decode(string json) =>
        JsonSerializer.Deserialize<QueueEntry>(json)!;

    [Fact]
    public void AnImportedConversationCarriesBothItsAdapterAndItsOrigin()
    {
        QueueEntry entry = Decode(
            """
            {
              "entry_id": "e1",
              "source": "trajectory",
              "declared_source": "antigravity"
            }
            """);

        Assert.Equal("trajectory", entry.Source);
        Assert.Equal("antigravity", entry.DeclaredSource);
    }

    /// <summary>
    /// A native session declares nothing, and a daemon predating the field
    /// sends nothing. Both must decode rather than throw.
    /// </summary>
    [Fact]
    public void AnEntryWithoutTheFieldStillDecodes()
    {
        QueueEntry entry = Decode(
            """
            {
              "entry_id": "e2",
              "source": "claude-code"
            }
            """);

        Assert.Equal("claude-code", entry.Source);
        Assert.Null(entry.DeclaredSource);
    }

    /// <summary>An explicit null is the same answer as an absent key.</summary>
    [Fact]
    public void AnExplicitNullOriginIsNotAnEmptyString()
    {
        QueueEntry entry = Decode(
            """
            {
              "entry_id": "e3",
              "source": "codex",
              "declared_source": null
            }
            """);

        Assert.Null(entry.DeclaredSource);
    }
}
