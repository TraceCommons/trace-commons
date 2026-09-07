using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Controls;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;

namespace TraceCommons.App.Controls;

/// <summary>
/// The model-calls destination's markup and wiring.
///
/// Thin by design, like <see cref="SettingsView"/> and <see cref="HistoryView"/>:
/// every word and every decision lives in
/// <see cref="PrivateInferenceViewModel"/>, and the shapes it depends on live
/// one layer further down in TraceCommons.Interop so they can be tested off
/// Windows.
/// </summary>
public sealed partial class PrivateInferenceView : UserControl
{
    public PrivateInferenceView(DaemonHost host)
    {
        InitializeComponent();

        ViewModel = new PrivateInferenceViewModel(host);
        Loaded += OnFirstLoaded;
    }

    public PrivateInferenceViewModel ViewModel { get; }

    /// <summary>
    /// Reads the switch once the page is on screen rather than in the
    /// constructor, matching <see cref="HistoryView"/>: an IPC call that runs
    /// before the pane has been laid out reads as the nav click having done
    /// nothing.
    /// </summary>
    private async void OnFirstLoaded(object sender, RoutedEventArgs e)
    {
        Loaded -= OnFirstLoaded;
        await ViewModel.LoadAsync();
    }

    /// <summary>
    /// Hands a settings snapshot the window already has to the page, so a
    /// change made in the tray or in another shell lands here without a
    /// second read.
    /// </summary>
    public void Fill(DaemonSettingsSnapshot? settings) => ViewModel.Fill(settings);

    /// <summary>Flips the switch from outside the page, for the accelerator.</summary>
    /// <remarks>
    /// The accelerator only: it is pressed with this window in front of the
    /// contributor, so both directions are answered on screen. The tray has
    /// no such guarantee and gets <see cref="TurnOffAsync"/> instead.
    /// </remarks>
    public Task ToggleAsync() => ViewModel.SetAsync(!ViewModel.Enabled);

    /// <summary>Stops answering model calls, for the tray's one write.</summary>
    /// <remarks>
    /// Sets the value rather than inverting it. The tray may reduce what this
    /// computer answers and may not enlarge it, and a flip would do the
    /// second whenever the menu it was pressed from was stale.
    /// </remarks>
    public Task TurnOffAsync() => ViewModel.SetAsync(false);

    /// <summary>
    /// "Send this tool's calls here", for one row.
    /// </summary>
    /// <remarks>
    /// The first connect puts the exposure question first. Connecting a tool is
    /// what makes the exposure real, so a contributor who has never been asked
    /// is asked here, with the shared paragraph and the same two answers as the
    /// first-run offer -- and an explicit accept, never an implied one.
    /// </remarks>
    private async void OnConnectHarness(object sender, RoutedEventArgs e)
    {
        if (IdOf(sender) is not { Length: > 0 } id)
        {
            return;
        }

        if (ViewModel.ConnectNeedsExposure && !await AskExposureAsync())
        {
            return;
        }

        await PlanThenConfirmAsync(id, HarnessSurface.Connect);
    }

    /// <summary>
    /// "Stop sending this tool's calls here", for one row. No exposure
    /// question: taking a tool back off exposes nothing.
    /// </summary>
    private async void OnDisconnectHarness(object sender, RoutedEventArgs e)
    {
        if (IdOf(sender) is not { Length: > 0 } id)
        {
            return;
        }

        await PlanThenConfirmAsync(id, HarnessSurface.Disconnect);
    }

    /// <summary>
    /// The exposure question, before the first connect. True only on an
    /// explicit accept that the daemon confirmed.
    /// </summary>
    /// <remarks>
    /// <c>ContentDialogResult.None</c> is the dialog never having appeared, and
    /// it is a no, not a yes: a contributor who was never shown the paragraph
    /// has not agreed to anything.
    /// </remarks>
    private async Task<bool> AskExposureAsync()
    {
        var body = new StackPanel { Spacing = 8 };
        body.Children.Add(Paragraph(ViewModel.What));
        body.Children.Add(Paragraph(ViewModel.Exposure));
        body.Children.Add(Paragraph(ViewModel.OfferNoRepoint));

        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = ViewModel.OfferTitle,
            Content = body,
            PrimaryButtonText = ViewModel.OfferAccept,
            CloseButtonText = ViewModel.OfferDecline,

            // Declining is what Enter and Escape both do. Turning this on
            // changes what anything else on this computer may send through;
            // it does not get to be the thing a stray keypress commits.
            DefaultButton = ContentDialogButton.Close,
        };

        // Three outcomes, not two. Primary is the accept. Close is a decline
        // and is recorded, so the question is not put again. None is the
        // dialog never having appeared -- nothing is recorded, because nobody
        // was asked, and it is emphatically not a yes.
        ContentDialogResult answer = await DialogGuard.ShowOnceAsync(dialog);
        if (answer == ContentDialogResult.None)
        {
            return false;
        }

        if (answer != ContentDialogResult.Primary)
        {
            await ViewModel.DeclineExposureAsync();
            return false;
        }

        return await ViewModel.AcceptExposureAsync();
    }

    /// <summary>
    /// Works out one tool's edit, shows exactly what it would do, and writes
    /// only on an explicit confirmation.
    /// </summary>
    /// <remarks>
    /// The two steps never collapse into one. The daemon holds the worked-out
    /// edit and hands back an id; this shows it and hands that id straight
    /// back, adding nothing. A plan that turned out not to be committable --
    /// nothing to change, a file that could not be read, a tool that is not
    /// there -- is still shown, with the cancel alone: the contributor pressed
    /// a button and is owed an answer either way.
    ///
    /// A plan the daemon has forgotten, or one whose file moved underneath it,
    /// is not a failed write. Nothing was written. The list is read again so
    /// the rows say where things actually stand.
    /// </remarks>
    private async Task PlanThenConfirmAsync(string id, string action)
    {
        HarnessPlan? plan = await ViewModel.PlanAsync(id, action);
        if (plan is null)
        {
            await ViewModel.LoadHarnessesAsync();
            return;
        }

        var body = new StackPanel { Spacing = 8 };
        if (plan.Path is { Length: > 0 } path)
        {
            body.Children.Add(Monospaced(path));
        }

        // The changes verbatim, in the shared crate's own words. Nothing here
        // summarises them, and nothing here reorders them.
        foreach (string change in plan.Changes)
        {
            body.Children.Add(Monospaced(change));
        }

        // Occupied rides alongside the outcome and is never one. A slot the
        // contributor already had a value in was left exactly as it was: this
        // is reported, never drawn as a fault, and never paired with an action
        // that would take the slot.
        if (plan.HasOccupied)
        {
            body.Children.Add(Paragraph(ViewModel.SlotTaken));
            foreach (HarnessOccupiedSlot slot in plan.Occupied)
            {
                body.Children.Add(Monospaced(slot.Slot));
                body.Children.Add(Monospaced(slot.Current));
            }
        }

        // Every outcome that writes nothing says why. A file that could not be
        // read is a refusal that needs a human, not "nothing to change"; so is
        // a tool that is not here, and a path this build could not work out.
        // The shared table keeps them apart, and this shell holds no arm of
        // its own.
        string outcomeSentence = PrivateInferenceViewModel.OutcomeSentence(plan.OutcomeLabel);
        if (!string.IsNullOrEmpty(outcomeSentence))
        {
            body.Children.Add(Paragraph(outcomeSentence));
        }

        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = ViewModel.PreviewTitle,
            Content = body,
            CloseButtonText = ViewModel.PreviewCancel,

            // Leaving the file alone is what Enter and Escape both do.
            DefaultButton = ContentDialogButton.Close,
        };

        // The confirm button exists only where there is a minted plan to
        // commit. Every other outcome gets the cancel alone, so a dialog can
        // never offer to write something the daemon refused to work out.
        if (plan.IsCommittable)
        {
            dialog.PrimaryButtonText = ViewModel.PreviewConfirm;
        }

        ContentDialogResult answer = await DialogGuard.ShowOnceAsync(dialog);
        if (answer != ContentDialogResult.Primary || !plan.IsCommittable || plan.PlanId is null)
        {
            return;
        }

        // The preview is already gone by the time this returns. On any refusal
        // the view model has put the unconfirmed-write sentence up and nothing
        // was written; the list is read again either way, so the rows say
        // where things actually stand rather than where the preview said they
        // would. NOTHING IS RETRIED AND NOTHING IS RE-PLANNED: the daemon
        // takes the plan out of its store before it re-checks the file and
        // before it writes, so no refusal leaves anything to commit again --
        // and re-planning here would be deciding a write on the contributor's
        // behalf that they were never shown. Pressing the button again is
        // theirs to do.
        await ViewModel.CommitAsync(plan.PlanId);
        await ViewModel.LoadHarnessesAsync();
    }

    /// <summary>Which row a button press came from.</summary>
    private static string? IdOf(object sender) =>
        sender is FrameworkElement element ? element.Tag as string : null;

    private static TextBlock Paragraph(string text) =>
        new() { Text = text, TextWrapping = TextWrapping.Wrap };

    /// <summary>
    /// A path or a change, drawn as what it is: an exact string in somebody's
    /// file, selectable so it can be checked against the file itself.
    /// </summary>
    private static TextBlock Monospaced(string text) =>
        new()
        {
            Text = text,
            TextWrapping = TextWrapping.Wrap,
            IsTextSelectionEnabled = true,
            FontFamily = new Microsoft.UI.Xaml.Media.FontFamily("Consolas"),
        };

    /// <summary>
    /// The switch itself.
    ///
    /// Guarded against the programmatic fill: the toggle is bound one-way to
    /// the view model, so re-rendering it after a write raises Toggled again
    /// and would echo the value straight back at the daemon.
    /// </summary>
    private async void OnToggled(object sender, RoutedEventArgs e)
    {
        if (sender is not ToggleSwitch toggle || toggle.IsOn == ViewModel.Enabled)
        {
            return;
        }

        await ViewModel.SetAsync(toggle.IsOn);
    }
}
