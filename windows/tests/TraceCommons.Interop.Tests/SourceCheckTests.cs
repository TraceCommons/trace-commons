using System;
using System.IO;
using System.Linq;
using TraceCommons.Interop;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The settings screen's session-source rows, read across the real ABI.
///
/// Driven through the actual export rather than a fixture: a fixture would
/// assert that this file agrees with itself, and what has to be true is that
/// this shell prints the sentence the Rust picked for the mode it was given.
/// </summary>
public class SourceCheckTests
{
    private static string Line(string tool, string mode)
    {
        string? line = SourceChecks.CheckLine(tool, mode);
        Assert.NotNull(line);
        return line!;
    }

    /// <summary>
    /// The defect, pinned. "off" and "unset" shared one sentence, because
    /// this shell branched on ClaudeRootConfigured -- which is
    /// (mode == "watch") and so false for both. A contributor who declared
    /// Claude Code off was told its sessions were being read from the usual
    /// place, which is false in the fail-open direction.
    ///
    /// "unset" keeps saying sessions are read, because they are: an
    /// undeclared claude or codex source is scanned at its conventional
    /// location. Saying otherwise would be the same bug pointing the other
    /// way, and worse.
    /// </summary>
    [Fact]
    public void EachSourceModeGetsItsOwnSentence()
    {
        Assert.Equal("Claude Code sessions folder set", Line(SourceChecks.Claude, "watch"));
        Assert.Equal(
            "Claude Code sessions read from the usual place",
            Line(SourceChecks.Claude, "unset"));
        Assert.Equal(
            "Claude Code marked not used, so nothing is opened for it",
            Line(SourceChecks.Claude, "off"));
        Assert.Equal(
            "Codex marked not used, so nothing is opened for it",
            Line(SourceChecks.Codex, "off"));
    }

    /// <summary>
    /// No mode's sentence contains another's. "Private" is a substring of
    /// "Not private", and a Contains check on this surface has matched the
    /// wrong branch that way before; the "off" line is therefore not the
    /// "unset" line with a negation bolted on.
    /// </summary>
    [Fact]
    public void NoModesSentenceContainsAnothers()
    {
        foreach (string tool in new[] { SourceChecks.Claude, SourceChecks.Codex })
        {
            string[] lines = new[] { Line(tool, "watch"), Line(tool, "unset"), Line(tool, "off") };
            for (int i = 0; i < lines.Length; i++)
            {
                for (int j = 0; j < lines.Length; j++)
                {
                    if (i == j)
                    {
                        continue;
                    }

                    Assert.NotEqual(lines[i], lines[j]);
                    Assert.DoesNotContain(lines[i], lines[j], StringComparison.Ordinal);
                }
            }
        }
    }

    /// <summary>
    /// A mode this build does not know reads as "unset", never as "off". An
    /// older daemon sends no *_source_mode at all and the snapshot defaults
    /// it to the empty string; claiming nothing is read from a folder that
    /// is being scanned is the worse of the two errors.
    /// </summary>
    [Fact]
    public void AnUnknownModeNeverClaimsNothingIsRead()
    {
        string unset = Line(SourceChecks.Claude, "unset");
        foreach (string mode in new[] { string.Empty, "OFF", "disabled", "watching" })
        {
            Assert.Equal(unset, Line(SourceChecks.Claude, mode));
        }
    }

    /// <summary>
    /// A tool key this build does not have is refused by name, not answered
    /// with some other tool's sentence.
    /// </summary>
    [Fact]
    public void AnUnknownToolIsRefusedAsUnknownSourceTool()
    {
        Assert.Null(SourceChecks.CheckLine("claude-code", "watch"));
        Assert.Equal(
            "unknown-source-tool",
            NativeMethods.BorrowedString(NativeMethods.tc_last_error()));
    }

    /// <summary>
    /// The view model asks the Rust for the row and does not write one.
    ///
    /// Asserted about the call site, not about the wording helper: a test
    /// that only exercised <see cref="SourceChecks"/> would keep passing
    /// while the settings screen went on rendering its own two hand-written
    /// sentences beside it, which is exactly the state this change found.
    /// </summary>
    [Fact]
    public void TheSettingsScreenAsksForTheRowRatherThanWritingIt()
    {
        string path = Path.Combine(AppContext.BaseDirectory, "ContributorSettingsViewModel.cs.txt");
        Assert.True(File.Exists(path), $"the view model source was not copied to {path}");
        string source = File.ReadAllText(path);
        string uncommented = string.Join(
            "\n",
            source.Split('\n').Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));

        Assert.Contains(
            "AddSourceRow(SourceChecks.Claude, settings.ClaudeSourceMode)",
            uncommented,
            StringComparison.Ordinal);
        Assert.Contains(
            "AddSourceRow(SourceChecks.Codex, settings.CodexSourceMode)",
            uncommented,
            StringComparison.Ordinal);
        Assert.Contains(
            "SourceChecks.CheckLine(tool, sourceMode)",
            uncommented,
            StringComparison.Ordinal);

        // The sentences themselves, and the boolean that could not tell the
        // two false-branch facts apart, are gone from this shell.
        foreach (string forbidden in new[]
        {
            "sessions folder set",
            "usual place",
            "ClaudeRootConfigured",
            "CodexRootConfigured",
        })
        {
            Assert.DoesNotContain(forbidden, uncommented, StringComparison.Ordinal);
        }
    }
}
