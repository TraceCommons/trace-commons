using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// W22: the preview's turn index, pinned to the Mac's
/// macos/Tests/TCShellCoreTests/PreviewTurnsTests.swift and LookInside.swift.
/// </summary>
public class PreviewTurnsTests
{
    /// <summary>PreviewTurnsTests.swift:7-19, byte for byte.</summary>
    private const string Fixture = """
    {
      "entry_id": "0b6f1f9e-3f53-4d43-9f43-6a0b9c7f1a11",
      "body_digest": "sha256:ab",
      "envelope_digest": "sha256:cd",
      "turn_count": 2,
      "turns": [
        {"index": 0, "role": "user_message", "byte_offset": 0, "byte_len": 40},
        {"index": 1, "role": "tool_call", "tool_name": "bash", "byte_offset": 40, "byte_len": 12}
      ],
      "leaves_this_mac": {"ignored": true}
    }
    """;

    [Fact]
    public void TheIndexDecodesWithAnOptionalToolName()
    {
        PreviewTurns turns = PreviewTurns.Parse(Fixture)!;
        Assert.Equal("0b6f1f9e-3f53-4d43-9f43-6a0b9c7f1a11", turns.EntryId);
        Assert.Equal("sha256:ab", turns.BodyDigest);
        Assert.Equal("sha256:cd", turns.EnvelopeDigest);
        Assert.Equal(2, turns.TurnCount);
        Assert.Equal(2, turns.Turns.Count);
        Assert.Null(turns.Turns[0].ToolName);
        Assert.Equal("bash", turns.Turns[1].ToolName);
        Assert.Equal("tool_call", turns.Turns[1].Role);
        Assert.Equal(40, turns.Turns[1].ByteOffset);
        Assert.Equal(12, turns.Turns[1].ByteLen);
    }

    [Fact]
    public void AnIndexIsOnlyForTheBodyItWasAnchoredTo()
    {
        PreviewTurns turns = PreviewTurns.Parse(Fixture)!;
        Assert.True(turns.Indexes("sha256:ab"));
        Assert.False(turns.Indexes("sha256:00"));
    }

    [Fact]
    public void UnreadableIsNothing()
    {
        Assert.Null(PreviewTurns.Parse(null));
        Assert.Null(PreviewTurns.Parse("{}"));
        Assert.Null(PreviewTurns.Parse("not json"));
    }

    /// <summary>
    /// The anchor the core requires (LookInside.swift:13-15): sha256 over the
    /// UTF-8 bytes, lowercase, prefixed. The vector is the SHA-256 of "abc"
    /// (FIPS 180-2), and a non-ASCII body hashes its UTF-8, not UTF-16, bytes.
    /// </summary>
    [Fact]
    public void TheBodyDigestIsSha256OfTheUtf8Bytes()
    {
        Assert.Equal(
            "sha256:ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
            PreviewTurns.DigestOf("abc"));
        Assert.Equal(
            "sha256:" + System.Convert.ToHexString(System.Security.Cryptography.SHA256.HashData(new byte[] { 0xC3, 0xA9 })).ToLowerInvariant(),
            PreviewTurns.DigestOf("é"));
    }

    /// <summary>LookInside.swift:97-108, with the core's Look inside words.</summary>
    [Fact]
    public void ATurnReadsAsTheMacReadsIt()
    {
        PreviewTurns turns = PreviewTurns.Parse(Fixture)!;
        Assert.Equal("1. user message", PreviewTurnFormat.Title(turns.Turns[0]));
        Assert.Equal("2. tool call", PreviewTurnFormat.Title(turns.Turns[1]));
        Assert.Equal("event · bytes 0–40", PreviewTurnFormat.Detail(turns.Turns[0], PreviewTurnFormat.Word));
        Assert.Equal("bash · bytes 40–52", PreviewTurnFormat.Detail(turns.Turns[1], PreviewTurnFormat.Word));
        Assert.Equal("Load turns", PreviewTurnFormat.Word("load_turn_index"));
    }
}
