using System;
using System.Globalization;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;
using Windows.ApplicationModel.DataTransfer;
using Windows.Foundation;

namespace TraceCommons.App.Controls;

/// <summary>
/// The preview sheet's markup and wiring. <see cref="PreviewWindow"/> hosts it.
///
/// Thin by design, like <c>OnboardingWindow</c>: every decision lives in
/// <see cref="PreviewSheetViewModel"/>, which is where the contract behaviours
/// are commented, and the read gate itself lives one layer further down in
/// <see cref="ReadGate"/> so it can be tested off Windows. This file wires
/// clicks and draws the transcript.
/// </summary>
public sealed partial class PreviewSheet : UserControl, IDisposable
{
    public PreviewSheet(DaemonHost host, QueueEntryViewModel entry)
    {
        InitializeComponent();

        ViewModel = new PreviewSheetViewModel(host, entry);
        ViewModel.Decided += OnDecided;

        Loaded += OnFirstLoaded;
    }

    /// <summary>
    /// Effective pixels of horizontal padding inside the transcript card,
    /// taken off the viewport width before working out how many characters
    /// fit across it. Matches the Border's Padding in the markup.
    /// </summary>
    private const double TranscriptBodyPadding = 24.0;

    /// <summary>The body, cut into chunks. Built once when the tab is realized.</summary>
    private TranscriptDocument? _document;

    /// <summary>The chunks typeset right now, and the eviction that bounds them.</summary>
    private TranscriptResidentChunks<RichTextBlock> _resident = new();

    /// <summary>Where each chunk sits vertically, for the two spacers.</summary>
    private TranscriptRowIndex? _rows;

    /// <summary>The chunk range currently in the panel's children.</summary>
    private ChunkRange _shown = ChunkRange.Empty;

    private int _columns;

    /// <summary>
    /// Set while the children and the spacers are being changed. Changing
    /// them changes the scroll extent, which raises ViewChanged, which would
    /// re-enter this and rebuild the window it is halfway through building.
    /// </summary>
    private bool _updatingTranscript;

    public PreviewSheetViewModel ViewModel { get; }

    /// <summary>
    /// Raised once the contributor has decided. The queue window owns the
    /// undo, because recovery has to be on the screen they land on rather
    /// than behind a sheet that has closed.
    /// </summary>
    public event Action<QueueEntryViewModel, PreviewDecision>? Decided;

    /// <summary>
    /// Opens the preview once the sheet is on screen rather than in the
    /// constructor, so the window is visible before the redaction pass starts:
    /// on a large session that pass takes long enough that doing it first
    /// would look like a failure to open.
    /// </summary>
    private async void OnFirstLoaded(object sender, RoutedEventArgs e)
    {
        Loaded -= OnFirstLoaded;
        await ViewModel.LoadAsync();

        // Search is the tab a contributor can answer a question with in five
        // seconds, so the caret starts in it.
        SearchBox.Focus(FocusState.Programmatic);
    }

    private void OnSearchTab(object sender, RoutedEventArgs e) =>
        ViewModel.SelectTab(PreviewTab.Search);

    private void OnWhatsInItTab(object sender, RoutedEventArgs e) =>
        ViewModel.SelectTab(PreviewTab.WhatsInIt);

    private void OnTranscriptTab(object sender, RoutedEventArgs e) =>
        ViewModel.SelectTab(PreviewTab.Transcript);

    private void OnPermissionsTab(object sender, RoutedEventArgs e) =>
        ViewModel.SelectTab(PreviewTab.Permissions);

    /// <summary>
    /// The transcript panel has been realized, which with <c>x:Load</c> means
    /// it is genuinely on screen rather than merely collapsed somewhere in the
    /// tree.
    /// </summary>
    /// <remarks>
    /// This is the ONLY thing that arms the first half of the read gate, and
    /// it is deliberately here rather than in the tab click: a click records
    /// intent, a realization records display, and the gate is only worth
    /// having if it records the second. The body is drawn first and the gate
    /// armed after, so the flag can never lead the pixels.
    ///
    /// <para>
    /// Paging deliberately did NOT change this gate. Every byte being
    /// reachable is not every byte being read, and a gate that waited for a
    /// scroll to the end of 17.5 MB would be defeated by throwing the
    /// scrollbar at the bottom: verifying nothing while reading, to everyone
    /// downstream, as though it verified reading. The gate still claims only
    /// that the first screenful was displayed.
    /// </para>
    /// </remarks>
    private void OnTranscriptRealized(object sender, RoutedEventArgs e)
    {
        BuildTranscriptDocument();
        RefreshTranscript();
        ViewModel.MarkTranscriptShown();
    }

    /// <summary>
    /// Cuts the body into chunks, once, when the tab is first realized.
    /// </summary>
    /// <remarks>
    /// A scan rather than a layout, and cheap enough not to need a
    /// background thread: 17.5 MB chunks in single-digit milliseconds on the
    /// macOS reference. What it must never become is a walk that grows
    /// faster than the body, which would be the original hang moved one
    /// function along; there is a wall-clock test on it in the interop
    /// suite.
    /// </remarks>
    private void BuildTranscriptDocument()
    {
        _document = new TranscriptDocument(ViewModel.Transcript);
        _resident = new TranscriptResidentChunks<RichTextBlock>();
        _shown = ChunkRange.Empty;
        _columns = 0;
        _rows = null;

        if (TranscriptChunks is not null)
        {
            TranscriptChunks.Children.Clear();
        }

        if (TranscriptSizeCaption is not null)
        {
            TranscriptSizeCaption.Text = string.Format(
                CultureInfo.InvariantCulture,
                "{0}, all of it.",
                QueueEntryViewModel.FormatBytes(_document.TotalBytes));
        }
    }

    /// <summary>
    /// Moves the resident window to cover what is on screen, typesetting the
    /// chunks that came into it and dropping the ones that fell out.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Eviction is the load-bearing half. A window that only ever added
    /// chunks would pass every "is it under the ceiling" check early in a
    /// scroll and then run the reader out of memory further down a 17.5 MB
    /// body: the same failure as the original, arriving more slowly. The
    /// policy itself lives in
    /// <see cref="TranscriptResidentChunks{TRendered}"/> so it can be
    /// asserted against real byte counts on a machine with no WinUI at all.
    /// </para>
    /// <para>
    /// The children are diffed rather than rebuilt: the window is contiguous
    /// and moves a chunk at a time, so a scroll costs one chunk of layout
    /// rather than a window's worth. Re-adding an element that was already
    /// laid out would cost the whole window on every scroll event.
    /// </para>
    /// </remarks>
    private void RefreshTranscript()
    {
        if (_document is null || TranscriptChunks is null || TranscriptPanel is null)
        {
            return;
        }

        MeasureTranscriptColumns();
        if (_rows is null)
        {
            return;
        }

        ChunkRange visible = TranscriptViewport.VisibleChunks(
            _rows,
            TranscriptScrollIntoBody(),
            TranscriptPanel.ViewportHeight);

        _resident.Update(_document, visible, RenderTranscriptChunk);
        ChunkRange next = _resident.Window;
        if (next == _shown)
        {
            return;
        }

        _updatingTranscript = true;
        try
        {
            // A jump lands somewhere with nothing in common with what is on
            // screen. Diffing that is a rebuild with extra steps.
            if (next.End <= _shown.Start || next.Start >= _shown.End)
            {
                TranscriptChunks.Children.Clear();
                _shown = new ChunkRange(next.Start, next.Start);
            }

            while (_shown.Start < next.Start && !_shown.IsEmpty)
            {
                TranscriptChunks.Children.RemoveAt(0);
                _shown = new ChunkRange(_shown.Start + 1, _shown.End);
            }

            while (_shown.End > next.End && !_shown.IsEmpty)
            {
                TranscriptChunks.Children.RemoveAt(TranscriptChunks.Children.Count - 1);
                _shown = new ChunkRange(_shown.Start, _shown.End - 1);
            }

            while (_shown.Start > next.Start)
            {
                int index = _shown.Start - 1;
                if (_resident.TryGet(index, out RichTextBlock block))
                {
                    TranscriptChunks.Children.Insert(0, block);
                }

                _shown = new ChunkRange(index, _shown.End);
            }

            while (_shown.End < next.End)
            {
                if (_resident.TryGet(_shown.End, out RichTextBlock block))
                {
                    TranscriptChunks.Children.Add(block);
                }

                _shown = new ChunkRange(_shown.Start, _shown.End + 1);
            }

            (double above, double below) = TranscriptViewport.Spacers(_rows, next);
            TranscriptSpacerAbove.Height = above;
            TranscriptSpacerBelow.Height = below;
        }
        finally
        {
            _updatingTranscript = false;
        }
    }

    /// <summary>
    /// Where the scroll is, measured from the top of the transcript body
    /// rather than the top of the panel.
    /// </summary>
    /// <remarks>
    /// The caption and the copy row scroll with the transcript, so the
    /// <c>ScrollViewer</c>'s own offset is a few dozen pixels ahead of the
    /// body's. Taking the body host's position inside the scrolled content
    /// is independent of where the scroll currently is, so this does not
    /// feed back on itself.
    /// </remarks>
    private double TranscriptScrollIntoBody()
    {
        if (TranscriptPanel is null || TranscriptBodyHost is null || TranscriptContent is null)
        {
            return 0.0;
        }

        try
        {
            Point top = TranscriptBodyHost
                .TransformToVisual(TranscriptContent)
                .TransformPoint(new Point(0.0, 0.0));
            return Math.Max(0.0, TranscriptPanel.VerticalOffset - top.Y);
        }
        catch (ArgumentException)
        {
            // TransformToVisual throws if the two elements are not in the
            // same tree yet, which can happen on the first layout pass. The
            // top of the body is the right answer then anyway.
            return 0.0;
        }
    }

    /// <summary>
    /// Recomputes the row index when the width changes, since a narrower
    /// sheet wraps more and a chunk's placeholder has to grow with it.
    /// </summary>
    private void MeasureTranscriptColumns()
    {
        if (_document is null || TranscriptPanel is null)
        {
            return;
        }

        double usable = TranscriptPanel.ViewportWidth - TranscriptBodyPadding;
        int columns = TranscriptViewport.Columns(usable);
        if (columns == _columns && _rows is not null)
        {
            return;
        }

        _columns = columns;
        _rows = new TranscriptRowIndex(_document, columns);
    }

    /// <summary>
    /// Typesets one chunk, with its redaction markers picked out.
    /// </summary>
    /// <remarks>
    /// <para>
    /// Markers stay VISIBLE rather than becoming deletions, which is the
    /// point: a hole tells a contributor nothing, a mark tells them the
    /// pipeline was standing right there.
    /// </para>
    /// <para>
    /// The design draws each as a chip with a wash behind it. A WinUI
    /// <see cref="Run"/> carries a foreground and a weight but no background
    /// and no box, and building one <c>InlineUIContainer</c> per marker would
    /// put hundreds of elements in a chunk for a rounded corner. So the
    /// chip's wash becomes the chip's colour: bold, in the brand's "weigh
    /// this" gold, which the design system already carries at text contrast
    /// in both themes. The property that matters survives either way, which
    /// is that the marker reads as an object placed in the text rather than
    /// as damage done to it. The Linux shell records the same compromise for
    /// the same reason.
    /// </para>
    /// <para>
    /// The scan runs over ONE CHUNK, never the whole body. The chunker
    /// refuses to cut through a marker precisely so that this is safe: a
    /// marker split across two separately-typeset chunks would draw as two
    /// halves in body type, and half a marker reads as content that was
    /// never scrubbed. Both halves of that guarantee use
    /// <see cref="TranscriptMarkers"/>'s one pattern, so they cannot drift
    /// apart about what a marker is.
    /// </para>
    /// <para>
    /// Nothing in here is logged. The transcript is the ABI's one content
    /// exemption and it goes to the screen and nowhere else.
    /// </para>
    /// </remarks>
    private RichTextBlock RenderTranscriptChunk(int index)
    {
        var block = new RichTextBlock
        {
            FontFamily = (FontFamily)Application.Current.Resources["TcMonoFontFamily"],
            FontSize = (double)Application.Current.Resources["TcMonoTranscriptFontSize"],
            Foreground = (Brush)Application.Current.Resources["TcInkPrimaryBrush"],
            IsTextSelectionEnabled = true,
        };

        string text = _document is null ? string.Empty : _document.TextOf(index);
        var paragraph = new Paragraph();

        if (text.Length == 0)
        {
            paragraph.Inlines.Add(new Run { Text = string.Empty });
            block.Blocks.Add(paragraph);
            return block;
        }

        var markerBrush = (Brush)Application.Current.Resources["TcGoldTextBrush"];

        foreach (TranscriptRun run in TranscriptMarkers.Split(text))
        {
            var inline = new Run { Text = text.Substring(run.Start, run.Length) };
            if (run.IsMarker)
            {
                inline.FontWeight = FontWeights.Bold;
                inline.Foreground = markerBrush;
            }

            paragraph.Inlines.Add(inline);
        }

        block.Blocks.Add(paragraph);
        return block;
    }

    private void OnTranscriptScrolled(object sender, ScrollViewerViewChangedEventArgs e)
    {
        if (_updatingTranscript)
        {
            return;
        }

        RefreshTranscript();
    }

    private void OnTranscriptResized(object sender, SizeChangedEventArgs e)
    {
        if (_updatingTranscript)
        {
            return;
        }

        RefreshTranscript();
    }

    /// <summary>
    /// Puts the whole redacted body on the clipboard.
    /// </summary>
    /// <remarks>
    /// Selection inside the transcript covers one block at a time, which is
    /// intrinsic to paging rather than a detail that could be fixed with more
    /// care: a chunk that is not typeset has nothing to select. This is the
    /// deliberate trade. Whole-body selection is lost, whole-body copying is
    /// gained, and copying is what the selection was for. It is also bounded
    /// work regardless of size, because it is a string copy and not a layout.
    /// </remarks>
    private void OnCopyWholeTranscript(object sender, RoutedEventArgs e)
    {
        if (_document is null)
        {
            return;
        }

        var package = new DataPackage { RequestedOperation = DataPackageOperation.Copy };
        package.SetText(_document.WholeText());
        Clipboard.SetContent(package);

        if (sender is Button button)
        {
            button.Content = "Copied";
        }
    }

    /// <summary>
    /// Searches as the term is typed.
    /// </summary>
    /// <remarks>
    /// Reads the box rather than the view model, for the same reason
    /// <c>OnboardingWindow</c> does: the order of the two-way binding's push
    /// and this event is not guaranteed, so reading the view model here would
    /// search for the previous keystroke.
    /// </remarks>
    private void OnNeedleChanged(object sender, TextChangedEventArgs e)
    {
        if (sender is TextBox box)
        {
            ViewModel.Needle = box.Text;
            ViewModel.RunSearch();
        }
    }

    private void OnSearchClick(object sender, RoutedEventArgs e) => ViewModel.RunSearch();

    private void OnRecentSearchClick(object sender, RoutedEventArgs e)
    {
        if (sender is Button { Content: string term })
        {
            SearchBox.Text = term;
        }
    }

    private async void OnNotThisOne(object sender, RoutedEventArgs e)
    {
        await ViewModel.DismissAsync();
    }

    private async void OnContribute(object sender, RoutedEventArgs e)
    {
        await ViewModel.ContributeAsync();
    }

    private void OnClose(object sender, RoutedEventArgs e) => CloseRequested?.Invoke();

    /// <summary>Raised when the sheet is finished with, for any reason.</summary>
    public event Action? CloseRequested;

    /// <summary>
    /// One sheet, one session, one decision: both decisions close the window
    /// and put the contributor back on the queue.
    /// </summary>
    private void OnDecided(PreviewDecision decision)
    {
        Decided?.Invoke(ViewModel.Entry, decision);
        CloseRequested?.Invoke();
    }

    /// <summary>
    /// Frees the preview.
    ///
    /// The native body dies here, which is what bounds the ABI's one content
    /// exemption to a sheet that is open.
    /// </summary>
    public void Dispose()
    {
        ViewModel.Decided -= OnDecided;
        ViewModel.Dispose();
    }
}
