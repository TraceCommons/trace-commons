using System;
using System.IO;
using System.Linq;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// A composition visual has no <c>Translation</c> property until
/// <c>ElementCompositionPreview.SetIsTranslationEnabled</c> is called for its
/// element, and <c>StopAnimation("Translation.Y")</c> on such a visual throws
/// E_INVALIDARG ("The specified property was not found or cannot be
/// animated"). Thrown from the window's constructor, that is a stowed
/// exception and the app dies before its first frame: on tc-win-az5,
/// 2026-11-19, every sample launch that showed a notice card did, because
/// the first card was put at rest before it had ever slid in.
/// </summary>
/// <remarks>
/// <c>TraceCommons.App</c> is a WinUI project and cannot be referenced from
/// this suite, so this is asserted about the sources, as the other shell
/// wiring tests are: every method that starts or stops an animation on
/// <c>Translation</c> enables translation first.
/// </remarks>
public class CompositionTranslationTests
{
    [Fact]
    public void EveryTranslationAnimationEnablesTranslationFirstInTheSameMethod()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source", "TraceCommons.App");
        Assert.True(Directory.Exists(root), $"shell sources were not copied to {root}");

        int seen = 0;
        foreach (string file in Directory.EnumerateFiles(root, "*.cs.txt", SearchOption.AllDirectories))
        {
            string[] lines = File.ReadAllText(file).Replace("\r\n", "\n", StringComparison.Ordinal).Split('\n');
            for (int i = 0; i < lines.Length; i++)
            {
                string line = lines[i];
                bool touchesTranslation =
                    (line.Contains("StopAnimation(\"Translation", StringComparison.Ordinal)
                     || line.Contains("StartAnimation(\"Translation", StringComparison.Ordinal)
                     || (line.Contains("Animate(", StringComparison.Ordinal) && line.Contains("\"Translation", StringComparison.Ordinal)))
                    && !line.TrimStart().StartsWith("//", StringComparison.Ordinal);
                if (!touchesTranslation)
                {
                    continue;
                }

                seen++;
                int start = i;
                while (start > 0 && !IsMemberHeader(lines[start]))
                {
                    start--;
                }

                string before = string.Join("\n", lines.Skip(start).Take(i - start));
                Assert.True(
                    before.Contains("SetIsTranslationEnabled(", StringComparison.Ordinal),
                    $"{Path.GetFileName(file)} line {i + 1} animates Translation without enabling it first in the same method: {line.Trim()}");
            }
        }

        Assert.True(seen > 0, "no Translation animation found; the notice cards' reset has moved and this test reads nothing");
    }

    private static bool IsMemberHeader(string line)
    {
        string t = line.TrimStart();
        return (t.StartsWith("private ", StringComparison.Ordinal)
                || t.StartsWith("public ", StringComparison.Ordinal)
                || t.StartsWith("internal ", StringComparison.Ordinal)
                || t.StartsWith("protected ", StringComparison.Ordinal))
            && t.Contains('(', StringComparison.Ordinal);
    }
}
