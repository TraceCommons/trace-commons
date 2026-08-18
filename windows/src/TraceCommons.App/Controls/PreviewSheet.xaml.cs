using System;
using System.Collections.Generic;
using Microsoft.UI.Text;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Documents;
using Microsoft.UI.Xaml.Media;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;

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
    /// </remarks>
    private void OnTranscriptRealized(object sender, RoutedEventArgs e)
    {
        DrawTranscript();
        ViewModel.MarkTranscriptShown();
    }

    /// <summary>
    /// Draws the redacted body with its redaction markers picked out.
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
    /// put hundreds of elements in a 169 KB transcript for a rounded corner.
    /// So the chip's wash becomes the chip's colour: bold, in the brand's
    /// "weigh this" gold, which the design system already carries at text
    /// contrast in both themes. The property that matters survives either way,
    /// which is that the marker reads as an object placed in the text rather
    /// than as damage done to it. The Linux shell records the same compromise
    /// for the same reason.
    /// </para>
    /// <para>
    /// Nothing in here is logged. The transcript is the ABI's one content
    /// exemption and it goes to the screen and nowhere else.
    /// </para>
    /// </remarks>
    private void DrawTranscript()
    {
        if (TranscriptBody is null)
        {
            return;
        }

        TranscriptBody.Blocks.Clear();

        string body = ViewModel.Transcript;
        var paragraph = new Paragraph();

        if (body.Length == 0)
        {
            // An empty redacted body is a real outcome and it must not look
            // like a rendering failure.
            paragraph.Inlines.Add(new Run { Text = "(empty)" });
            TranscriptBody.Blocks.Add(paragraph);
            return;
        }

        var markerBrush = (Brush)Application.Current.Resources["TcGoldTextBrush"];

        IReadOnlyList<TranscriptRun> runs = TranscriptMarkers.Split(body);
        foreach (TranscriptRun run in runs)
        {
            var inline = new Run { Text = body.Substring(run.Start, run.Length) };
            if (run.IsMarker)
            {
                inline.FontWeight = FontWeights.Bold;
                inline.Foreground = markerBrush;
            }

            paragraph.Inlines.Add(inline);
        }

        TranscriptBody.Blocks.Add(paragraph);
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
