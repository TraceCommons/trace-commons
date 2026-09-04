using System;
using System.Linq;
using System.Reflection;
using System.Text.Json;
using System.Text.Json.Serialization;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The routing vocabulary, read across the real ABI.
///
/// These drive the actual exports rather than parsing a fixture: a fixture
/// would assert that this file agrees with itself, and what has to be true is
/// that this shell prints the words the Rust defines.
/// </summary>
public class RoutingCopyTests
{
    private static RoutingCopy Copy()
    {
        RoutingCopy? copy = RoutingSurface.Copy();
        Assert.NotNull(copy);
        return copy!;
    }

    /// <summary>
    /// The words this shell will print, pinned as literals.
    ///
    /// The one deliberately hand-written assertion here, and the reason it
    /// exists: every other check compares the payload to itself and would keep
    /// passing if all four words were renamed. This is the tripwire that a
    /// word changed at all -- change one in the Rust and this goes red, which
    /// is what proves this shell reads the shared source rather than a copy of
    /// its own. The GTK and macOS suites make the same assertion, so a rename
    /// turns all three red together.
    /// </summary>
    [Fact]
    public void TheSharedWordsAreTheOnesThisShellReceives()
    {
        RoutingCopy copy = Copy();
        Assert.Equal("Private", copy.WordPrivate);
        Assert.Equal("Sends direct", copy.WordDirect);
        Assert.Equal("Not known", copy.WordUnknown);
        Assert.Equal("Not used", copy.WordNotUsed);
    }

    /// <summary>
    /// Exactly one word claims privacy, and none denies it. "Private" is a
    /// substring of "Not private", so a vocabulary carrying both is one
    /// <c>Contains</c> away from showing the wrong verdict.
    /// </summary>
    [Fact]
    public void OnlyTheWiredWordClaimsPrivacyAndNoneDeniesIt()
    {
        RoutingCopy copy = Copy();
        Assert.Contains("privat", copy.WordPrivate.ToLowerInvariant(), StringComparison.Ordinal);
        foreach (string word in new[] { copy.WordDirect, copy.WordUnknown, copy.WordNotUsed })
        {
            Assert.DoesNotContain("privat", word.ToLowerInvariant(), StringComparison.Ordinal);
        }
    }

    /// <summary>
    /// No word contains any other, in either direction, case-insensitively.
    /// </summary>
    [Fact]
    public void NoWordContainsAnotherSoContainsCannotMatchTheWrongOne()
    {
        string[] words = Copy().Words;
        for (int i = 0; i < words.Length; i++)
        {
            for (int j = 0; j < words.Length; j++)
            {
                if (i == j)
                {
                    continue;
                }

                Assert.NotEqual(words[i], words[j]);
                Assert.DoesNotContain(
                    words[j].ToLowerInvariant(),
                    words[i].ToLowerInvariant(),
                    StringComparison.Ordinal);
            }
        }
    }

    /// <summary>
    /// Every string on the surface arrives filled. An empty one would render
    /// as a blank beside a tool name rather than as a failure anyone could
    /// see.
    /// </summary>
    [Fact]
    public void EveryWordOnTheSurfaceArrivesNonEmpty()
    {
        RoutingCopy copy = Copy();
        var strings = typeof(RoutingCopy)
            .GetProperties()
            .Where(p => p.PropertyType == typeof(string))
            .ToList();

        // What "every" means comes from the payload the Rust exported, not
        // from a number kept here. A literal count was edited by every copy
        // addition, caught nothing the loop below does not, and did not catch
        // the drift actually worth catching -- a field the Rust exports and
        // this shell dropped, which sails past a count as long as somebody
        // remembers to decrement it.
        string? json = NativeMethods.TakeOwnedString(NativeMethods.tc_routing_copy());
        Assert.False(string.IsNullOrWhiteSpace(json));
        using JsonDocument payload = JsonDocument.Parse(json!);
        string[] exported = payload.RootElement
            .EnumerateObject()
            .Select(field => field.Name)
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToArray();
        string[] declared = strings
            .Select(p => p.GetCustomAttribute<JsonPropertyNameAttribute>()?.Name ?? p.Name)
            .OrderBy(name => name, StringComparer.Ordinal)
            .ToArray();

        // Non-vacuous: the surface has words, and this shell reads every one
        // the Rust sends and invents none of its own.
        Assert.NotEmpty(exported);
        Assert.Equal(exported, declared);

        foreach (var property in strings)
        {
            string? value = (string?)property.GetValue(copy);
            Assert.False(string.IsNullOrEmpty(value), $"{property.Name} arrived empty");
        }
    }

    /// <summary>
    /// The sentences arrive finished. This shell never fills in a hole, so no
    /// format marker survives and no wording of its own surrounds them.
    /// </summary>
    [Fact]
    public void TheSentencesArriveAssembledAndNotAsTemplates()
    {
        string? named = RoutingSurface.TokenLine(@"C:\Users\x\.ironwire\control.token");
        Assert.NotNull(named);
        Assert.Contains(@"C:\Users\x\.ironwire\control.token", named!, StringComparison.Ordinal);

        string? unnamed = RoutingSurface.TokenLine(null);
        Assert.NotNull(unnamed);
        Assert.DoesNotContain(@"C:\Users\x", unnamed!, StringComparison.Ordinal);
        Assert.NotEqual(named, unnamed);

        string? withPort = RoutingSurface.UnreachableLine(8463);
        Assert.NotNull(withPort);
        Assert.Contains("8463", withPort!, StringComparison.Ordinal);

        // No port tried must not become "port 0".
        string? noPort = RoutingSurface.UnreachableLine(null);
        Assert.NotNull(noPort);
        Assert.DoesNotContain("0", noPort!, StringComparison.Ordinal);

        Assert.Equal("Last checked an hour ago", RoutingSurface.LastChecked("an hour ago"));

        foreach (string sentence in new[] { named!, unnamed!, withPort!, noPort! })
        {
            foreach (string marker in new[] { "{}", "{0}", "{path}", "{port}", "%s", "%d" })
            {
                Assert.DoesNotContain(marker, sentence, StringComparison.Ordinal);
            }
        }
    }

    /// <summary>
    /// A "last checked" with no time is refused rather than rendered as
    /// "Last checked " with nothing after it.
    /// </summary>
    [Fact]
    public void ALastCheckedWithNoTimeIsRefused()
    {
        Assert.Null(RoutingSurface.LastChecked(string.Empty));
    }

    /// <summary>
    /// The payload half, without the dylib: a truncated or empty envelope is
    /// refused whole rather than decoded into blank words.
    /// </summary>
    [Fact]
    public void APayloadMissingAWordIsRefusedRatherThanRenderedBlank()
    {
        Assert.Null(RoutingSurface.Parse(null));
        Assert.Null(RoutingSurface.Parse("   "));
        Assert.Null(RoutingSurface.Parse("{ not json"));
        Assert.Null(RoutingSurface.Parse("{\"word_private\":\"Private\"}"));
    }
}
