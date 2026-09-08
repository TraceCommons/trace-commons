using System;
using System.Collections.Generic;
using System.IO;
using System.Text.RegularExpressions;
using Xunit;

namespace TraceCommons.Interop.Tests;

/// <summary>
/// The order in which <c>MainViewModel.ReplacePending</c> publishes what it
/// built and raises what it changed.
/// </summary>
/// <remarks>
/// <para>
/// <c>Pending</c> is an <c>ObservableCollection</c>, and
/// <c>MainWindow.OpenPreview</c> subscribes an open preview sheet to its
/// <c>CollectionChanged</c> precisely so the sheet re-reads its gate when the
/// queue is replaced. That re-read runs through <c>MainViewModel.LiveEntry</c>,
/// which reads the <c>_rowsByEntryId</c> FIELD. The sheet's only notification
/// arrives from the <c>Clear</c> and the <c>Add</c>s, and nothing raises a
/// collection change afterwards -- so publishing the field last means the
/// sheet is notified only while the map holds the previous snapshot, every
/// time. Deterministic, not a race.
/// </para>
///
/// <para>
/// WHAT THIS FILE DOES NOT CLAIM. Whether that was ever visible to a
/// contributor depends on whether <c>x:Bind</c> pulls synchronously on
/// <c>PropertyChanged</c> or defers to the dispatcher queue, which cannot be
/// determined from source and needs a Windows toolchain to settle. If it
/// defers, the read lands after the method returns and there is no observable
/// defect. These tests deliberately do not rest on that question: the
/// ordering is wrong either way, and the order is what they assert. Nothing
/// incorrect is ever sent in either case -- <c>ContributeAsync</c> re-tests
/// the gate at the press, against the fresh map.
/// </para>
///
/// <para>
/// THE DEFECT EXISTS ONLY DURING THE RAISE. Once <c>ReplacePending</c>
/// returns, the field is correct and every end-state assertion agrees with
/// itself, which is why this is asserted about the statement order rather
/// than about a finished queue.
/// </para>
///
/// <para>
/// Asserted about the source for the reason
/// <see cref="EligibilityShellWiringTests"/> gives: <c>TraceCommons.App</c> is
/// a WinUI project and cannot be built, let alone referenced, on the macOS and
/// Linux machines this suite runs on, and the ordering is not observable from
/// anywhere else. The source is copied next to the test assembly by the test
/// project.
/// </para>
/// </remarks>
public class QueueRepublishOrderTests
{
    /// <summary>
    /// Every field a <c>CollectionChanged</c> handler can reach is assigned
    /// before the first observable mutation of the refresh.
    /// </summary>
    [Fact]
    public void TheRowMapIsPublishedBeforeTheQueueRaises()
    {
        string body = MethodBody("MainViewModel.cs", "private void ReplacePending(");

        int firstRaise = FirstObservableEffect(body);
        Assert.True(firstRaise >= 0, "ReplacePending no longer mutates any observable collection.");

        // The map LiveEntry reads. This is the assignment the whole live-entry
        // resolution depends on, and the one that used to sit 55 lines below
        // the Clear that first re-entered the sheet.
        foreach (string publication in new[]
        {
            "_rowsByEntryId = rowsByEntryId;",
            "_previousEntryIds = currentIds;",
            "_groups = groups;",
        })
        {
            int at = body.IndexOf(publication, StringComparison.Ordinal);
            Assert.True(at >= 0, $"ReplacePending no longer contains `{publication}`.");
            Assert.True(
                at < firstRaise,
                $"`{publication}` is assigned AFTER the first observable mutation, at offset "
                + $"{at} against {firstRaise}. The open preview sheet is notified by that "
                + "raise, and LiveEntry would resolve against the previous snapshot.");
        }
    }

    /// <summary>
    /// Nothing observable happens before the publication line.
    /// </summary>
    /// <remarks>
    /// The complement of the test above, and the one that survives a rename.
    /// Publishing early is only half the rule: a raise added ABOVE the
    /// publication -- a status line, a shield, a navigation -- reopens the
    /// same window without touching either assignment.
    /// </remarks>
    [Fact]
    public void NothingIsRaisedBeforeTheRowsAreBuilt()
    {
        string body = MethodBody("MainViewModel.cs", "private void ReplacePending(");
        int firstRaise = FirstObservableEffect(body);
        int lastPublication = body.IndexOf("_groups = groups;", StringComparison.Ordinal);

        Assert.True(lastPublication >= 0);
        Assert.True(
            firstRaise > lastPublication,
            $"an observable effect at offset {firstRaise} precedes the publication line at "
            + $"{lastPublication}.");
    }

    /// <summary>
    /// The membership diff is taken against the PREVIOUS ids, not the ones
    /// just published.
    /// </summary>
    /// <remarks>
    /// The failure mode the reordering itself could introduce. Moving
    /// <c>_previousEntryIds = currentIds</c> above
    /// <c>PreviewCancellation.EntriesRemoved</c> makes the diff compare a list
    /// against itself: it reports nothing removed, every cancelled preview
    /// goes on being scheduled, and no end-state assertion notices.
    /// </remarks>
    [Fact]
    public void TheRemovalDiffIsTakenBeforeTheIdsArePublished()
    {
        string body = MethodBody("MainViewModel.cs", "private void ReplacePending(");

        int diff = body.IndexOf(
            "PreviewCancellation.EntriesRemoved(_previousEntryIds, currentIds)",
            StringComparison.Ordinal);
        int publication = body.IndexOf("_previousEntryIds = currentIds;", StringComparison.Ordinal);

        Assert.True(diff >= 0, "ReplacePending no longer diffs the previous entry ids.");
        Assert.True(publication >= 0);
        Assert.True(
            diff < publication,
            "the removal diff is taken after _previousEntryIds was overwritten, so it compares "
            + "the new ids against themselves and reports nothing removed.");
    }

    /// <summary>
    /// The coupling this whole file rests on is still there.
    /// </summary>
    /// <remarks>
    /// The ordering rule only matters because something re-enters the view
    /// model from inside the raise. If the sheet ever stops subscribing, or
    /// <c>LiveEntry</c> stops reading the field, these assertions would go on
    /// passing while guarding nothing -- so both ends are named here, and a
    /// change to either one lands on this test rather than silently retiring
    /// it.
    /// </remarks>
    [Fact]
    public void TheSheetStillReEntersTheViewModelFromInsideTheRaise()
    {
        string window = ShellSource("TraceCommons.App/MainWindow.xaml.cs");
        Assert.Contains(
            "ViewModel.Pending.CollectionChanged += OnQueueChanged;", window, StringComparison.Ordinal);
        Assert.Contains("sheet.QueueChanged();", window, StringComparison.Ordinal);

        // And the re-read lands on the field, not on a snapshot passed in.
        Assert.Matches(
            new Regex(
                @"public QueueEntryViewModel\? LiveEntry\(string entryId\) =>\s*"
                + @"_rowsByEntryId\.TryGetValue",
                RegexOptions.Singleline),
            ShellSource("TraceCommons.App/ViewModels/MainViewModel.cs"));
    }

    /// <summary>
    /// The offset of the first statement in <paramref name="body"/> that a
    /// subscriber could observe, or -1.
    /// </summary>
    /// <remarks>
    /// Deliberately wider than <c>Pending</c>: <c>Groups</c> and
    /// <c>OpenFolderEntries</c> are observable too, <c>SetQueueLocation</c>
    /// mutates the third and raises six properties, and a property raise can
    /// reach a converter that reads the view model. Anything a handler could
    /// run on belongs on this side of the line.
    /// </remarks>
    private static int FirstObservableEffect(string body)
    {
        var effects = new Regex(
            @"\b(?:Pending|Groups|OpenFolderEntries)\.(?:Clear|Add|Insert|Remove|RemoveAt)\("
            + @"|\bSetQueueLocation\("
            + @"|\bRaise\("
            + @"|\bRaiseShield\(");

        Match first = effects.Match(body);
        return first.Success ? first.Index : -1;
    }

    /// <summary>
    /// The body of the first method in <paramref name="fileName"/> whose
    /// declaration starts with <paramref name="signature"/>, brace-matched.
    /// </summary>
    private static string MethodBody(string fileName, string signature)
    {
        string source = ShellSource("TraceCommons.App/ViewModels/" + fileName);
        int start = source.IndexOf(signature, StringComparison.Ordinal);
        Assert.True(start >= 0, $"{fileName} has no `{signature}`.");

        int open = source.IndexOf('{', start);
        Assert.True(open >= 0);

        int depth = 0;
        for (int i = open; i < source.Length; i++)
        {
            if (source[i] == '{')
            {
                depth++;
            }
            else if (source[i] == '}')
            {
                depth--;
                if (depth == 0)
                {
                    return source.Substring(open, i - open + 1);
                }
            }
        }

        Assert.Fail($"{fileName}: `{signature}` has no matching close brace.");
        return string.Empty;
    }

    private static string ShellSource(string relativePath)
    {
        string path = Path.Combine(
            AppContext.BaseDirectory, "shell-source", relativePath + ".txt");
        Assert.True(File.Exists(path), $"{relativePath} was not copied to {path}");
        return File.ReadAllText(path).Replace("\r\n", "\n", StringComparison.Ordinal);
    }
}
