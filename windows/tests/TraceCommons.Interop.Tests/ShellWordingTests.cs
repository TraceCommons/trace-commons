using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Text;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// Wording authored in this shell, over the whole shell.
///
/// <para>
/// Two guards already hold single surfaces to the strict rule -- every string
/// literal in <c>RoutingTools.cs</c>, and in the four witness files, must be a
/// wire value, because the sentence beside it crosses the ABI already
/// finished. Those stay exactly as they are. What they could not see is a
/// sentence hand-written in any of the other hundred-odd files: wording
/// authored in one shell survives a rename in the other two, and nothing here
/// noticed.
/// </para>
/// <para>
/// This reads every C# and XAML source of both shell projects and counts the
/// literals that read as a sentence a contributor would be shown. It is a
/// ratchet, not a clean bill of health: the Windows shell today authors most
/// of its own wording -- the <c>*Copy</c> classes in the interop assembly were
/// transcribed from the shared design, and the XAML views carry their own
/// <c>Text</c> and <c>ToolTipService.ToolTip</c> strings. Moving all of that
/// behind the ABI is a project of its own. Every file that does so is recorded
/// below with the exact number of sentences it holds today, so nothing new can
/// be added to it and nothing new can start doing it.
/// </para>
/// </summary>
public class ShellWordingTests
{
    /// <summary>
    /// Files that author wording today, and exactly how much.
    ///
    /// <para>
    /// TODO(shell-copy): every entry here is a file whose wording should be
    /// composed in the Rust contributor crate and read across the C ABI, the
    /// way <c>routing_copy.rs</c> and <c>witness_copy.rs</c> already are.
    /// Until then the number is a CEILING AND A FLOOR both: adding a sentence
    /// fails, and removing one fails too, so that the entry has to be lowered
    /// deliberately as copy moves out. Never raise a number. A new file must
    /// never be added here.
    /// </para>
    /// </summary>
    private static readonly IReadOnlyDictionary<string, int> WordingBaseline =
        new SortedDictionary<string, int>(StringComparer.Ordinal)
        {
            // The interop assembly's copy classes. Each is a transcription of
            // the shared design's wording, kept in the interop assembly rather
            // than in a view model so a machine that cannot build WinUI can
            // still test it -- which is precisely why the same sentences are
            // written a second time in the GTK and macOS shells today.
            { "TraceCommons.Interop/ArmingOffer.cs", 4 },
            { "TraceCommons.Interop/CorrectionCopy.cs", 4 },
            { "TraceCommons.Interop/HealthCopy.cs", 22 },
            { "TraceCommons.Interop/HistoryCopy.cs", 30 },
            { "TraceCommons.Interop/OriginalSearchOutcome.cs", 4 },
            // PendingInviteActivation.cs predates this guard: it merged to
            // main before the baseline landed, so its three notice sentences
            // are recorded rather than refused. It belongs to the same
            // TODO(shell-copy) migration as everything else on this list.
            { "TraceCommons.Interop/PendingInviteActivation.cs", 3 },
            { "TraceCommons.Interop/PreviewCardOutcome.cs", 1 },
            { "TraceCommons.Interop/ProjectIgnoreCopy.cs", 8 },
            { "TraceCommons.Interop/PublicProfileCopy.cs", 44 },
            { "TraceCommons.Interop/RedactionLabels.cs", 3 },
            { "TraceCommons.Interop/RedactionSummary.cs", 7 },
            { "TraceCommons.Interop/ScrubDetectorCopy.cs", 6 },
            { "TraceCommons.Interop/ScrubbingCaveatCopy.cs", 3 },
            { "TraceCommons.Interop/SessionRootsCopy.cs", 14 },
            { "TraceCommons.Interop/SubagentCopy.cs", 2 },
            { "TraceCommons.Interop/SubmitToast.cs", 10 },
            { "TraceCommons.Interop/TrayModel.cs", 11 },
            { "TraceCommons.Interop/UnresolvedBucketCopy.cs", 3 },
            { "TraceCommons.Interop/UpdateProtocol.cs", 10 },
            { "TraceCommons.Interop/VerdictCopy.cs", 4 },
            { "TraceCommons.Interop/WatchCopy.cs", 8 },
            { "TraceCommons.Interop/WeekBandCopy.cs", 1 },
            { "TraceCommons.Interop/WithdrawCopy.cs", 29 },

            // View models that compose a sentence rather than reading one.
            // ContributorSettingsViewModel is the file the settings-screen
            // guard already watches for the witness row specifically; the rest
            // of its wording is unmoved. Four of its sentences moved into
            // WatchCopy.cs, which is why this entry ratcheted down.
            { "TraceCommons.App/ViewModels/ContributorSettingsViewModel.cs", 18 },
            { "TraceCommons.App/ViewModels/HistoryViewModel.cs", 4 },
            { "TraceCommons.App/ViewModels/MainViewModel.cs", 18 },
            { "TraceCommons.App/ViewModels/OnboardingViewModel.cs", 6 },
            { "TraceCommons.App/ViewModels/PreviewSheetViewModel.cs", 10 },
            { "TraceCommons.App/ViewModels/QueueGroupViewModel.cs", 1 },
            { "TraceCommons.App/ViewModels/SessionRootsViewModel.cs", 2 },

            // Window and control code-behind: dialog bodies and one fallback
            // label, written at the call site.
            { "TraceCommons.App/MainWindow.xaml.cs", 3 },
            { "TraceCommons.App/StartupRegistration.cs", 4 },
            { "TraceCommons.App/TrayIcon.cs", 3 },

            // XAML views. Literal Text=, Header=, PlaceholderText= and
            // ToolTipService.ToolTip= content -- the accessibility labels
            // included, which are as much a rename risk as anything drawn.
            { "TraceCommons.App/Controls/HistoryView.xaml", 8 },
            { "TraceCommons.App/Controls/PreviewSheet.xaml", 20 },
            { "TraceCommons.App/Controls/SettingsView.xaml", 16 },
            { "TraceCommons.App/MainWindow.xaml", 18 },
            { "TraceCommons.App/OnboardingWindow.xaml", 16 },
            { "TraceCommons.App/SessionRootsWindow.xaml", 3 },
        };

    /// <summary>
    /// The surfaces whose wording already comes from Rust. Nothing may ever
    /// buy them an allowance here: eight of the nine are held to the strict
    /// every-literal rule by <c>NoWordingIsAuthoredInThisShell</c>,
    /// <c>NoWordingIsAuthoredInTheWitnessSurface</c> and
    /// <c>NoWordingIsAuthoredInTheConsentSurface</c>, and a baseline entry
    /// would be a quieter way of undoing that. The ninth,
    /// <c>RoutingSurface.cs</c>, is the routing surface's other half: no
    /// strict guard names it, so zero here is the only thing holding it.
    /// </summary>
    /// <remarks>
    /// The consent trio -- <c>ConsentCopy.cs</c>, <c>ConsentSurface.cs</c>
    /// and <c>ReadGate.cs</c> -- is held to the strict rule by
    /// <c>NoWordingIsAuthoredInTheConsentSurface</c>. <c>ReadGate.cs</c> is
    /// listed even though it now holds only the rule: it is where the four
    /// consent sentences used to be, and a zero here is what makes one
    /// coming back a failure rather than a re-negotiated allowance.
    /// </remarks>
    private static readonly string[] RustOwnedSurfaces =
    {
        "TraceCommons.Interop/RoutingTools.cs",
        "TraceCommons.Interop/RoutingSurface.cs",
        "TraceCommons.Interop/WitnessTools.cs",
        "TraceCommons.Interop/WitnessSurface.cs",
        "TraceCommons.Interop/NearAccountConnection.cs",
        "TraceCommons.Interop/AdmissionPreparation.cs",
        "TraceCommons.Interop/ConsentCopy.cs",
        "TraceCommons.Interop/ConsentSurface.cs",
        "TraceCommons.Interop/ReadGate.cs",
    };

    /// <summary>
    /// Words a sentence has and an identifier, a wire key, a resource key or a
    /// format pattern does not.
    ///
    /// <para>
    /// This is what separates authored prose from the rest, and it is
    /// deliberately a function-word test rather than a "has a space" test:
    /// <c>"Segoe UI Variable Display"</c>, <c>"MMMM d, yyyy"</c> and
    /// <c>"0,0,0,12"</c> are not wording, and <c>"Watch this folder"</c>,
    /// <c>"Still being scored"</c> and <c>"Back to the folder list"</c> are.
    /// </para>
    /// </summary>
    private static readonly HashSet<string> FunctionWords = new(
        @"a an and are as at be been being but by can cannot could did do does for from
          had has have how if in into is isn't it it's its just may never no not nothing of off on once only or
          so some still such than that the their them then there they this those to until up was we were what
          when where which while who will with would you your yours yet anything something everything already
          always about after again all any because before both each else ever every here more most much
          must need needs same see should since take takes tell these too under use used using very"
            .Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries),
        StringComparer.Ordinal);

    private static readonly Regex StringLiteral = new("\"([^\"\\\\]|\\\\.)*\"", RegexOptions.Compiled);

    private static readonly Regex CharLiteral = new("'(\\\\.|[^'\\\\])'", RegexOptions.Compiled);

    private static readonly Regex XamlComment = new("<!--.*?-->", RegexOptions.Compiled | RegexOptions.Singleline);

    private static readonly Regex XamlAttributeValue = new("=\"([^\"]*)\"", RegexOptions.Compiled);

    private static readonly Regex XamlTextNode = new(">([^<>]+)<", RegexOptions.Compiled);

    /// <summary>
    /// No file in this shell authors more wording than it did when this guard
    /// was written, and no file starts authoring wording that did not.
    /// </summary>
    [Fact]
    public void NoWordingIsAuthoredInThisShellBeyondTheRecordedBaseline()
    {
        var scanned = ScanShellSources();

        // A copy that silently stopped happening would turn this test into a
        // pass over nothing, which is the failure mode the single-file guards
        // already protect against by name.
        Assert.True(
            scanned.Count >= 100,
            $"only {scanned.Count} shell sources were copied to shell-source/; "
            + "the whole tree of both projects is expected. See the csproj.");

        var failures = new List<string>();

        foreach (var (relativePath, wording) in scanned.OrderBy(pair => pair.Key, StringComparer.Ordinal))
        {
            int allowed = WordingBaseline.TryGetValue(relativePath, out int budget) ? budget : 0;
            if (wording.Count == allowed)
            {
                continue;
            }

            if (wording.Count > allowed)
            {
                failures.Add(
                    $"{relativePath}: {wording.Count} authored sentences, baseline allows {allowed}. "
                    + $"First one over the line: \"{wording[allowed]}\"");
            }
            else
            {
                failures.Add(
                    $"{relativePath}: {wording.Count} authored sentences, baseline still allows {allowed}. "
                    + "Wording moved out -- lower the entry (or delete it at zero).");
            }
        }

        foreach (string recorded in WordingBaseline.Keys)
        {
            if (!scanned.ContainsKey(recorded))
            {
                failures.Add($"{recorded}: recorded in the baseline but no longer in the shell. Delete the entry.");
            }
        }

        Assert.True(
            failures.Count == 0,
            "Wording on this shell's surfaces comes from the Rust contributor crate across the ABI.\n"
            + "A sentence written here is one the other two shells will not get, and one a rename in\n"
            + "the Rust will not reach.\n\n"
            + string.Join("\n", failures));
    }

    /// <summary>
    /// The surfaces the Rust already owns hold no wording at all, and hold no
    /// baseline entry either.
    /// </summary>
    [Fact]
    public void TheRustOwnedSurfacesAreNotGivenAWordingAllowance()
    {
        var scanned = ScanShellSources();

        foreach (string surface in RustOwnedSurfaces)
        {
            Assert.False(
                WordingBaseline.ContainsKey(surface),
                $"{surface} has a wording baseline entry. Its wording comes from Rust and the strict "
                + "per-literal guard says so; an allowance here would quietly undo that.");

            Assert.True(
                scanned.ContainsKey(surface),
                $"{surface} was not among the copied shell sources; the guard would pass over nothing.");

            Assert.Empty(scanned[surface]);
        }
    }

    /// <summary>
    /// Reads every copied shell source and returns the wording each one
    /// authors, keyed by its path relative to <c>windows/src</c>.
    /// </summary>
    private static Dictionary<string, List<string>> ScanShellSources()
    {
        string root = Path.Combine(AppContext.BaseDirectory, "shell-source");
        Assert.True(Directory.Exists(root), $"the shell sources were not copied to {root}");

        var scanned = new Dictionary<string, List<string>>(StringComparer.Ordinal);
        foreach (string file in Directory.EnumerateFiles(root, "*.txt", SearchOption.AllDirectories))
        {
            string relative = Path.GetRelativePath(root, file).Replace('\\', '/');
            relative = relative[..^".txt".Length];
            scanned[relative] = relative.EndsWith(".xaml", StringComparison.Ordinal)
                ? AuthoredWordingInXaml(File.ReadAllText(file))
                : AuthoredWordingInCSharp(File.ReadAllText(file));
        }

        return scanned;
    }

    /// <summary>
    /// The sentences a C# source authors.
    /// </summary>
    /// <remarks>
    /// Comment lines go first, for the reason the routing guard gives: prose
    /// about the wire may quote it, and nothing in a comment is rendered.
    /// Char literals go next so that a <c>'"'</c> cannot unbalance the literal
    /// scan. What is left is filtered by statement: a message handed to
    /// <c>throw</c> or to <c>Debug.WriteLine</c> is read by whoever is
    /// debugging this and by nobody else, and holding those to the shared copy
    /// would be a refinement of nothing.
    /// </remarks>
    private static List<string> AuthoredWordingInCSharp(string source)
    {
        string uncommented = string.Join(
            "\n",
            source.Split('\n').Where(line => !line.TrimStart().StartsWith("//", StringComparison.Ordinal)));
        uncommented = CharLiteral.Replace(uncommented, "''");

        var wording = new List<string>();
        foreach (Match match in StringLiteral.Matches(uncommented))
        {
            if (IsDeveloperFacing(uncommented, match.Index))
            {
                continue;
            }

            string literal = match.Value[1..^1];
            if (ReadsAsASentence(literal))
            {
                wording.Add(literal);
            }
        }

        return wording;
    }

    /// <summary>
    /// The sentences a XAML source authors: attribute values and text nodes
    /// both, since <c>&lt;TextBlock&gt;Nothing yet&lt;/TextBlock&gt;</c> draws
    /// exactly what <c>Text="Nothing yet"</c> does.
    /// </summary>
    private static List<string> AuthoredWordingInXaml(string source)
    {
        string uncommented = XamlComment.Replace(source, string.Empty);

        var wording = new List<string>();
        foreach (Match match in XamlAttributeValue.Matches(uncommented))
        {
            if (ReadsAsASentence(match.Groups[1].Value))
            {
                wording.Add(match.Groups[1].Value);
            }
        }

        foreach (Match match in XamlTextNode.Matches(uncommented))
        {
            if (ReadsAsASentence(match.Groups[1].Value))
            {
                wording.Add(match.Groups[1].Value);
            }
        }

        return wording;
    }

    /// <summary>
    /// True where the literal at <paramref name="index"/> is an exception
    /// message, a debug line or a <c>nameof</c> neighbour rather than
    /// something a contributor is shown.
    /// </summary>
    private static bool IsDeveloperFacing(string source, int index)
    {
        int boundary = source.LastIndexOfAny(new[] { ';', '{', '}' }, Math.Max(index - 1, 0));
        string statement = source[(boundary + 1)..index];
        return statement.Contains("throw", StringComparison.Ordinal)
            || statement.Contains("Debug.WriteLine", StringComparison.Ordinal)
            || statement.Contains("Debug.Assert", StringComparison.Ordinal)
            || statement.Contains("nameof", StringComparison.Ordinal);
    }

    /// <summary>
    /// True where the literal reads as a sentence somebody wrote for a
    /// contributor to read.
    /// </summary>
    private static bool ReadsAsASentence(string literal)
    {
        string text = literal.Replace('\n', ' ').Replace('\r', ' ');
        if (!text.Contains(' ', StringComparison.Ordinal))
        {
            return false;
        }

        // A binding, a markup extension or a resource reference. Whatever it
        // resolves to is somebody else's literal and is counted there.
        if (text.TrimStart().StartsWith("{", StringComparison.Ordinal))
        {
            return false;
        }

        var words = text
            .Split((char[]?)null, StringSplitOptions.RemoveEmptyEntries)
            .Select(LettersOnly)
            .Where(word => word.Length > 0)
            .ToList();

        return words.Count >= 2 && words.Any(FunctionWords.Contains);
    }

    private static string LettersOnly(string token)
    {
        var builder = new StringBuilder(token.Length);
        foreach (char c in token)
        {
            if ((c >= 'A' && c <= 'Z') || (c >= 'a' && c <= 'z') || c == '\'')
            {
                builder.Append(char.ToLowerInvariant(c));
            }
        }

        return builder.ToString();
    }
}
