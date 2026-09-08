using System;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// The model-calls destination: everything its page shows, and the one write
/// it can make.
///
/// <para>
/// Not one sentence is composed here. Every word arrives from
/// <c>crates/trace-commons-contributor/src/private_inference_copy.rs</c>
/// across the C ABI, including the destination's own name -- the rail label
/// is the one word a contributor navigates by, and a shell that spelled it
/// itself would go on spelling the old one after a rename in the Rust.
/// </para>
/// <para>
/// The whole page is hidden when those words did not arrive, rather than
/// drawn with a switch and nothing beside it. That shape is what says "on"
/// over a listener that refused to start.
/// </para>
/// <para>
/// <b>The status is read from the tone, never from the switch.</b> The switch
/// reports what was asked for; the tone reports what the daemon says
/// happened. They disagree exactly when it matters -- the switch on, the port
/// already taken -- and every indicator this shell draws for this surface,
/// here and on the rail and in the tray, comes off the tone.
/// </para>
/// </summary>
public sealed class PrivateInferenceViewModel : INotifyPropertyChanged
{
    private readonly DaemonHost _host;

    /// <summary>
    /// Every fixed word, read once across the ABI. Null when the export or
    /// the decode failed, and then the page renders nothing at all.
    /// </summary>
    private readonly PrivateInferenceCopy? _copy = PrivateInferenceSurface.Copy();

    private PrivateInferenceState _state = new(string.Empty, null);
    private bool _on;
    private bool _busy;
    private bool _loaded;

    /// <summary>
    /// Whether the exposure question has been put at all. Read from the same
    /// settings snapshot as the switch, so the first connect and the first-run
    /// offer cannot come to disagree about whether it was asked.
    /// </summary>
    private bool _offerSeen;

    public PrivateInferenceViewModel(DaemonHost host)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>Whether the words arrived at all.</summary>
    public bool Available => _copy is not null;

    public bool ControlsEnabled => !_busy && _copy is not null;

    public string Title => _copy?.SettingsTitle ?? string.Empty;

    public string Subtitle => _copy?.Subtitle ?? string.Empty;

    public string What => _copy?.OfferWhat ?? string.Empty;

    /// <summary>
    /// What turning it on exposes. On this page as well as in the first-run
    /// offer: a contributor who declined and came back later is making the
    /// same decision and is owed the same sentence.
    /// </summary>
    public string Exposure => _copy?.OfferExposure ?? string.Empty;

    public string ToggleText => _copy?.SettingsToggle ?? string.Empty;

    public string AppliesAtOnce => _copy?.SettingsAppliesAtOnce ?? string.Empty;

    /// <summary>Where the switch is. The switch's own position, and nothing else.</summary>
    public bool Enabled => _on;

    /// <summary>The sentence for whatever the daemon last reported.</summary>
    public string StateText => _copy is null
        ? string.Empty
        : PrivateInferenceSurface.StateLine(_state, _copy);

    /// <summary>The reported port, or the empty string when there is none.</summary>
    public string ServingText => PrivateInferenceSurface.ServingLine(_state);

    public bool HasServingText => ServingText.Length > 0;

    private PrivateInferenceTone StateTone => PrivateInferenceSurface.Tone(_state);

    /// <summary>
    /// Whether this page may paint anything as working.
    ///
    /// The tone, and only the tone. Reading <see cref="Enabled"/> here is the
    /// fail-open this surface exists to prevent.
    /// </summary>
    public bool IsWorking => StateTone.ReadsAsWorking();

    // One flag per tone rather than a converted foreground: three of the
    // sentences begin with the same two words, so a colour picked by reading
    // the sentence back would be picked by matching a prefix.

    public bool StateIsNeutral => StateTone == PrivateInferenceTone.Neutral;

    public bool StateIsHeld => StateTone == PrivateInferenceTone.Held;

    public bool StateIsClear => StateTone == PrivateInferenceTone.Clear;

    public bool StateIsAttention => StateTone == PrivateInferenceTone.Attention;

    public bool StateIsRefused => StateTone == PrivateInferenceTone.Refused;

    /// <summary>
    /// The sentence shown when a write could not be confirmed, or the empty
    /// string. The payload's, like everything else here.
    /// </summary>
    public string Notice
    {
        get => _notice;
        private set
        {
            if (_notice == value)
            {
                return;
            }

            _notice = value;
            Raise(nameof(Notice));
            Raise(nameof(HasNotice));
        }
    }

    private string _notice = string.Empty;

    public bool HasNotice => _notice.Length > 0;

    /// <summary>
    /// Reads the switch and the reported state once the page is on screen.
    /// </summary>
    public async Task LoadAsync()
    {
        _busy = true;
        Raise(nameof(ControlsEnabled));
        try
        {
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.GetSettings)
                .ConfigureAwait(true);
            Fill(response.ResultAs<DaemonSettingsSnapshot>());
            _loaded = true;
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(LoadAsync));
        }
        finally
        {
            _busy = false;
            Raise(nameof(ControlsEnabled));
        }

        // After the switch, not before it: the tool rows are read against a
        // page that already knows whether anything answers here, so a connect
        // pressed the moment the list appears is planned with the exposure
        // question already settled either way.
        await LoadHarnessesAsync().ConfigureAwait(true);

        // And what this computer answers calls with. Read last because it is
        // the one thing on the page that no settings snapshot carries: the
        // window pushes the switch and the listener state, and nothing pushes
        // this.
        await LoadCredentialAsync().ConfigureAwait(true);
    }

    /// <summary>
    /// Takes a settings snapshot this page did not ask for -- the window
    /// hands one over whenever the daemon reports a change, so the page and
    /// the rail badge cannot come to disagree.
    /// </summary>
    public void Fill(DaemonSettingsSnapshot? settings)
    {
        if (settings is null)
        {
            return;
        }

        _on = settings.PrivateInferenceOn;
        _offerSeen = settings.PrivateInferenceAnswered;
        _state = PrivateInferenceState.From(settings.PrivateInferenceReport);

        // A snapshot the window pushed is a settings read that landed, and
        // this page can write from it. Setting the flag only in LoadAsync left
        // SetAsync silently dropping every write on a page that was filled
        // from outside before its own round trip finished -- no write, and no
        // notice either, which is the shape a contributor reads as the switch
        // being broken.
        _loaded = true;
        RaiseEverythingDerived();
    }

    /// <summary>
    /// Writes the switch and renders from the daemon's echo, never
    /// optimistically: the echo carries the only thing that knows whether the
    /// listener actually started. The marker rides along, so a contributor
    /// who found the switch here is not asked the question later.
    /// </summary>
    /// <remarks>
    /// Every path out of this method either wrote the value or corrected it.
    /// The toggle is bound one-way, so it has already moved to wherever the
    /// contributor dragged it, and only a change notification puts it back.
    /// </remarks>
    public async Task SetAsync(bool on)
    {
        if (!_loaded || _busy || _copy is null)
        {
            Raise(nameof(Enabled));
            return;
        }

        _busy = true;
        Raise(nameof(ControlsEnabled));
        try
        {
            DaemonResponse response = await _host
                .CallAsync(
                    DaemonProtocol.Methods.SetSettings,
                    PrivateInferenceSurface.SerializeSwitch(on))
                .ConfigureAwait(true);
            if (response.ResultAs<DaemonSettingsSnapshot>() is { } settings &&
                PrivateInferenceSurface.WriteConfirmed(on, settings))
            {
                Notice = string.Empty;
                Fill(settings);
            }
            else
            {
                // The reply did not confirm the full write. The state LINE is
                // left to the next refresh -- nothing here invents a sentence
                // -- but the SWITCH is snapped back to what the daemon last
                // reported, because a stuck switch is a claim.
                Notice = _copy.WriteUnconfirmed;
                Raise(nameof(Enabled));
            }
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(SetAsync));
            Notice = _copy.WriteUnconfirmed;
            Raise(nameof(Enabled));
        }
        finally
        {
            _busy = false;
            Raise(nameof(ControlsEnabled));
        }
    }

    // ---------------------------------------------------------------------
    // The tools on this computer.
    //
    // The list leads and the switch is below it: connecting one tool is the
    // thing a contributor came here to do, and the switch is the kill switch
    // over all of them. Every action is one tool at a time -- nothing here
    // writes two config files, and there is no "connect all".
    // ---------------------------------------------------------------------

    /// <summary>The tools found on this computer, in the order the daemon listed them.</summary>
    public ObservableCollection<HarnessRowViewModel> Harnesses { get; } = new();

    public string HarnessesTitle => _copy?.HarnessesTitle ?? string.Empty;

    public string HarnessesWhat => _copy?.HarnessesWhat ?? string.Empty;

    /// <summary>
    /// What today's calls cost, assembled on the far side, or the empty
    /// string when nobody could measure it.
    /// </summary>
    /// <remarks>
    /// Empty is drawn as no line at all, and the scope sentence below is
    /// hidden with it: a qualification of a figure that is not on screen
    /// reads as a qualification of the list.
    /// </remarks>
    public string HarnessesSpend => _harnessSpend;

    /// <summary>What that figure leaves out. Drawn only beside the figure.</summary>
    public string HarnessesSpendScope => _copy?.HarnessesSpendScope ?? string.Empty;

    public bool HasHarnessesSpend => _harnessSpend.Length > 0;

    private string _harnessSpend = string.Empty;

    /// <summary>
    /// Said in terms of what was looked for, never in terms of which tools
    /// exist. The catalog channel is inert in this build, so this list is what
    /// this app knows how to look for and not a claim about the machine.
    /// </summary>
    public string HarnessesNoneFound => _copy?.HarnessesNoneFound ?? string.Empty;

    public bool HasHarnesses => Harnesses.Count > 0;

    public bool HasNoHarnesses => _harnessesRead && Harnesses.Count == 0;

    private bool _harnessesRead;

    /// <summary>
    /// Reads the tool list. Called after every action, because an action
    /// changed a file and the row that described it is now stale.
    /// </summary>
    public async Task LoadHarnessesAsync()
    {
        if (_copy is null)
        {
            return;
        }

        try
        {
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.HarnessList)
                .ConfigureAwait(true);
            if (response.IsError || response.Result is null)
            {
                return;
            }

            HarnessListing listing = HarnessSurface.ParseListing(response.Result.Value.GetRawText());
            Harnesses.Clear();
            foreach (HarnessRow row in listing.Harnesses)
            {
                Harnesses.Add(new HarnessRowViewModel(row, _copy));
            }

            // Read from the listing, never held over from the last one: an
            // amount a later read could not measure must go, not linger.
            _harnessSpend = HarnessSurface.SpendSentence(listing);

            _harnessesRead = true;
            Raise(nameof(HasHarnesses));
            Raise(nameof(HasNoHarnesses));
            Raise(nameof(HarnessesSpend));
            Raise(nameof(HasHarnessesSpend));
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(LoadHarnessesAsync));
        }
    }

    // ---------------------------------------------------------------------
    // The credential this computer answers calls with.
    //
    // A key minted at NEAR AI and kept on this machine. NOTHING HERE RENDERS
    // ONE: no property carries a key, a prefix, an id or an account name,
    // and the payload has no hole for one.
    //
    // The sentence, the tone and the action all come from the shared table
    // across the ABI. This page picks none of the three: the button in
    // question opens a browser and mints a key at a third party, and a shell
    // that decided for itself which button belongs beside which state is a
    // shell that will one day offer a second sign-in to somebody who already
    // has a key.
    // ---------------------------------------------------------------------

    private NearAiCredentialStatus _credential = NearAiCredentialStatus.Unreported;

    /// <summary>
    /// The attempt this page started, or null. Held so a cancel can name it
    /// and a poll can ask for its lifecycle word; cleared once the ceremony
    /// is no longer in flight.
    /// </summary>
    private string? _attemptId;

    private bool _credentialBusy;

    public string CredentialTitle => _copy?.CredentialTitle ?? string.Empty;

    public string CredentialWhat => _copy?.CredentialWhat ?? string.Empty;

    /// <summary>The sentence for whatever the daemon last reported.</summary>
    public string CredentialStateText => _copy is null
        ? string.Empty
        : NearAiCredentialSurface.StateLine(_credential, _copy);

    private PrivateInferenceTone CredentialTone => NearAiCredentialSurface.Tone(_credential);

    // One flag per tone, for the reason the listener row has them: the tone
    // is what picks the colour, and a colour picked by reading the sentence
    // back would be picked by matching a prefix.

    public bool CredentialIsNeutral => CredentialTone == PrivateInferenceTone.Neutral;

    public bool CredentialIsHeld => CredentialTone == PrivateInferenceTone.Held;

    public bool CredentialIsClear => CredentialTone == PrivateInferenceTone.Clear;

    public bool CredentialIsRefused => CredentialTone == PrivateInferenceTone.Refused;

    /// <summary>The one action the shared table allows for this state.</summary>
    private CredentialAction OfferedAction => NearAiCredentialSurface.Action(_credential);

    /// <summary>The words on that action's button, or the empty string.</summary>
    public string CredentialActionText =>
        NearAiCredentialSurface.ActionLabel(OfferedAction, _copy) ?? string.Empty;

    /// <summary>
    /// The consequence sentence that belongs beside that button, or the
    /// empty string where the action has none to state.
    /// </summary>
    /// <remarks>
    /// Drawn from the same call that decides the button, so the two cannot
    /// come apart: the cost of obtaining one is on screen wherever obtaining
    /// is offered, and what forgetting does not do is on screen wherever
    /// forgetting is.
    /// </remarks>
    public string CredentialActionPreamble =>
        NearAiCredentialSurface.ActionPreamble(OfferedAction, _copy) ?? string.Empty;

    public bool HasCredentialActionPreamble => CredentialActionPreamble.Length > 0;

    /// <summary>
    /// Whether there is an action to draw at all. A state this build could
    /// not read offers nothing.
    /// </summary>
    public bool HasCredentialAction =>
        OfferedAction != CredentialAction.None && _copy is not null;

    public bool CredentialControlsEnabled => !_credentialBusy && _copy is not null;

    /// <summary>
    /// Presses whatever the shared table offers for the state this machine is
    /// in, and hands back a started ceremony for the caller to open a browser
    /// on. Null for every other action.
    /// </summary>
    /// <remarks>
    /// THE BRANCH IS HERE AND NOT IN THE VIEW. Which of the three a press
    /// performs follows from the ABI's action for the reported state, so the
    /// markup cannot come to hold an arm of its own -- and the one arm that
    /// mints a key at a third party cannot be reached from a state this build
    /// could not read.
    /// </remarks>
    public async Task<NearAiCredentialAttempt?> PressCredentialAsync()
    {
        switch (OfferedAction)
        {
            case CredentialAction.Obtain:
                return await StartCredentialAsync().ConfigureAwait(true);
            case CredentialAction.Cancel:
                await CancelCredentialAsync().ConfigureAwait(true);
                return null;
            case CredentialAction.Forget:
                await ForgetCredentialAsync().ConfigureAwait(true);
                return null;
            default:
                return null;
        }
    }

    /// <summary>
    /// Reads what this machine holds. Names the attempt when this page
    /// started one, which is what buys back the ceremony's own lifecycle
    /// word; the resting state comes back either way.
    /// </summary>
    public async Task LoadCredentialAsync()
    {
        if (_copy is null)
        {
            return;
        }

        try
        {
            DaemonResponse response = await _host
                .CallAsync(
                    DaemonProtocol.Methods.NearAiCredentialStatus,
                    NearAiCredentialSurface.SerializeStatus(_attemptId))
                .ConfigureAwait(true);

            // A call that failed is UNREPORTED, never absent: one says this
            // daemon did not answer, the other claims nothing is stored here,
            // and reaching the second from a failed call is how a contributor
            // is invited to sign in over a key they already have.
            _credential = response.IsError || response.Result is null
                ? NearAiCredentialStatus.Unreported
                : NearAiCredentialSurface.ParseStatus(response.Result.Value.GetRawText());
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(LoadCredentialAsync));
            _credential = NearAiCredentialStatus.Unreported;
        }

        if (!NearAiCredentialSurface.AwaitingBrowser(_credential))
        {
            _attemptId = null;
        }

        RaiseCredential();
    }

    /// <summary>
    /// Begins the ceremony and hands back where to open the browser, or null
    /// when the daemon refused.
    /// </summary>
    /// <remarks>
    /// THE URL COMES ONCE. It is returned to the caller rather than held on
    /// this page, so nothing here can re-serve a stale one, and a poll never
    /// answers with it: getting another means starting a second ceremony,
    /// which is a second browser tab in front of somebody already looking at
    /// one.
    /// </remarks>
    public async Task<NearAiCredentialAttempt?> StartCredentialAsync()
    {
        if (_copy is null || _credentialBusy)
        {
            return null;
        }

        _credentialBusy = true;
        Raise(nameof(CredentialControlsEnabled));
        try
        {
            DaemonResponse response = await _host
                .CallAsync(DaemonProtocol.Methods.NearAiCredentialStart)
                .ConfigureAwait(true);
            if (response.IsError || response.Result is null)
            {
                Notice = _copy.WriteUnconfirmed;
                return null;
            }

            NearAiCredentialAttempt? attempt =
                NearAiCredentialSurface.ParseStart(response.Result.Value.GetRawText());
            if (attempt is null)
            {
                Notice = _copy.WriteUnconfirmed;
                return null;
            }

            Notice = string.Empty;
            _attemptId = attempt.Value.AttemptId;
            return attempt;
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(StartCredentialAsync));
            Notice = _copy.WriteUnconfirmed;
            return null;
        }
        finally
        {
            _credentialBusy = false;
            Raise(nameof(CredentialControlsEnabled));
        }
    }

    /// <summary>
    /// Stops waiting on the browser. Does nothing when this page cannot name
    /// the attempt: the daemon refuses an unnamed cancel, so nothing here can
    /// stop a ceremony some other shell started.
    /// </summary>
    public async Task CancelCredentialAsync()
    {
        if (_copy is null || _attemptId is not { Length: > 0 } attempt)
        {
            return;
        }

        await CallThenRefreshAsync(
                DaemonProtocol.Methods.NearAiCredentialCancel,
                NearAiCredentialSurface.SerializeCancel(attempt))
            .ConfigureAwait(true);
    }

    /// <summary>
    /// Removes the stored key from this machine.
    /// </summary>
    /// <remarks>
    /// Local, and the page says so beside the button:
    /// <see cref="CredentialActionPreamble"/> is the sentence saying the key
    /// stays valid at the service until the contributor removes it in their
    /// own account. This shell cannot do that for them and does not claim to.
    /// </remarks>
    public async Task ForgetCredentialAsync()
    {
        if (_copy is null)
        {
            return;
        }

        await CallThenRefreshAsync(DaemonProtocol.Methods.NearAiCredentialForget)
            .ConfigureAwait(true);
    }

    /// <summary>
    /// Waits for a ceremony that is under way to finish, re-reading the state
    /// until it stops being in flight.
    /// </summary>
    /// <remarks>
    /// The state is a fact about this machine that a browser tab changes, so
    /// nothing arrives here unasked. The loop stops on the first read that is
    /// no longer waiting on the browser -- including a cancel or a failure --
    /// and gives up after the cap rather than polling a socket forever behind
    /// a page nobody is looking at.
    /// </remarks>
    public async Task AwaitCredentialAsync()
    {
        for (int poll = 0; poll < CredentialPollLimit; poll++)
        {
            if (!NearAiCredentialSurface.AwaitingBrowser(_credential))
            {
                return;
            }

            await Task.Delay(CredentialPollInterval).ConfigureAwait(true);
            await LoadCredentialAsync().ConfigureAwait(true);
        }
    }

    /// <summary>
    /// One credential write, then a re-read. The state is never inferred from
    /// the reply: what this machine holds afterwards is a question only the
    /// next status answers.
    /// </summary>
    private async Task CallThenRefreshAsync(string method, string? parameters = null)
    {
        _credentialBusy = true;
        Raise(nameof(CredentialControlsEnabled));
        try
        {
            DaemonResponse response = await (parameters is null
                    ? _host.CallAsync(method)
                    : _host.CallAsync(method, parameters))
                .ConfigureAwait(true);
            Notice = response.IsError && _copy is not null ? _copy.WriteUnconfirmed : string.Empty;
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(CallThenRefreshAsync));
            if (_copy is not null)
            {
                Notice = _copy.WriteUnconfirmed;
            }
        }
        finally
        {
            _credentialBusy = false;
            Raise(nameof(CredentialControlsEnabled));
        }

        await LoadCredentialAsync().ConfigureAwait(true);
    }

    private void RaiseCredential()
    {
        Raise(nameof(CredentialStateText));
        Raise(nameof(CredentialIsNeutral));
        Raise(nameof(CredentialIsHeld));
        Raise(nameof(CredentialIsClear));
        Raise(nameof(CredentialIsRefused));
        Raise(nameof(CredentialActionText));
        Raise(nameof(CredentialActionPreamble));
        Raise(nameof(HasCredentialActionPreamble));
        Raise(nameof(HasCredentialAction));
    }

    private const int CredentialPollLimit = 150;

    private static readonly TimeSpan CredentialPollInterval = TimeSpan.FromSeconds(2);

    /// <summary>
    /// Whether a connect has to put the exposure question first.
    /// </summary>
    /// <remarks>
    /// Wider than the first-run offer, and asked off the LISTENER rather than
    /// off the marker: a connect that would reopen a listener the contributor
    /// turned off is the exposure decision again, whatever they answered the
    /// first time. The branch itself lives in
    /// <see cref="HarnessSurface.ConnectNeedsExposure"/>, in the same shape
    /// macOS and GTK use.
    /// </remarks>
    public bool ConnectNeedsExposure => HarnessSurface.ConnectNeedsExposure(_on);

    /// <summary>What turning it on exposes, and the two answers to it.</summary>
    public string OfferTitle => _copy?.OfferTitle ?? string.Empty;

    public string OfferNoRepoint => _copy?.OfferNoRepoint ?? string.Empty;

    public string OfferAccept => _copy?.OfferAccept ?? string.Empty;

    public string OfferDecline => _copy?.OfferDecline ?? string.Empty;

    public string PreviewTitle => _copy?.HarnessPreviewTitle ?? string.Empty;

    public string PreviewConfirm => _copy?.HarnessPreviewConfirm ?? string.Empty;

    public string PreviewCancel => _copy?.HarnessPreviewCancel ?? string.Empty;

    /// <summary>
    /// A slot the contributor already had a value in, which was left exactly
    /// as it was. Reported, never offered.
    /// </summary>
    public string SlotTaken => _copy?.HarnessSlotTaken ?? string.Empty;

    /// <summary>
    /// The sentence about a tool holding an old setting in a process that is
    /// still running, or the empty string. Set only after a write actually
    /// happened, and shown as a plain line rather than in
    /// <see cref="Notice"/>: nothing went wrong.
    /// </summary>
    public string Restart
    {
        get => _restart;
        private set
        {
            if (_restart == value)
            {
                return;
            }

            _restart = value;
            Raise(nameof(Restart));
            Raise(nameof(HasRestart));
        }
    }

    private string _restart = string.Empty;

    public bool HasRestart => _restart.Length > 0;

    /// <summary>A settings file that could not be read, and was therefore refused.</summary>
    /// <summary>
    /// The sentence a plan's outcome carries, from the shared table.
    /// </summary>
    /// <remarks>
    /// Replaces an <c>UnreadableConfig</c> property this view model held and
    /// the view's <c>== Unparseable</c> branch above it. That was one arm of a
    /// table written three times, and the four other non-committable outcomes
    /// had no arm at all -- their preview opened with a title, a path, no
    /// changes and a way out.
    /// </remarks>
    public static string OutcomeSentence(string? outcomeLabel) =>
        HarnessSurface.OutcomeSentence(outcomeLabel);

    /// <summary>
    /// Answers the exposure question with a yes and turns the destination on,
    /// in the one write that records both. Returns whether the daemon
    /// confirmed it.
    /// </summary>
    /// <remarks>
    /// The same body <see cref="PrivateInferenceSurface.SerializeOfferAnswer"/>
    /// builds for the first-run offer, and read back the same way: the echo
    /// carries the only thing that knows whether the listener actually
    /// started, and a connect planned against a listener that refused to start
    /// would be planned against nothing.
    /// </remarks>
    public async Task<bool> AcceptExposureAsync()
    {
        if (_copy is null)
        {
            return false;
        }

        try
        {
            DaemonResponse response = await _host
                .CallAsync(
                    DaemonProtocol.Methods.SetSettings,
                    PrivateInferenceSurface.SerializeOfferAnswer(accepted: true))
                .ConfigureAwait(true);
            if (response.ResultAs<DaemonSettingsSnapshot>() is { } settings &&
                PrivateInferenceSurface.WriteConfirmed(true, settings))
            {
                Notice = string.Empty;
                Fill(settings);
                return true;
            }
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(AcceptExposureAsync));
        }

        Notice = _copy.WriteUnconfirmed;
        return false;
    }

    /// <summary>
    /// Records that the exposure question was put and declined. The marker
    /// alone: the switch is already off, and writing it would make a refusal
    /// indistinguishable from a change.
    /// </summary>
    public async Task DeclineExposureAsync()
    {
        try
        {
            DaemonResponse response = await _host
                .CallAsync(
                    DaemonProtocol.Methods.SetSettings,
                    PrivateInferenceSurface.SerializeOfferAnswer(accepted: false))
                .ConfigureAwait(true);
            Fill(response.ResultAs<DaemonSettingsSnapshot>());
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(DeclineExposureAsync));
        }
    }

    /// <summary>
    /// Works out one tool's edit and writes nothing. Null when the daemon
    /// refused outright, in which case <see cref="Notice"/> already says so
    /// where there are words for it.
    /// </summary>
    public async Task<HarnessPlan?> PlanAsync(string id, string action)
    {
        if (_copy is null)
        {
            return null;
        }

        try
        {
            DaemonResponse response = await _host
                .CallAsync(
                    DaemonProtocol.Methods.HarnessPlan,
                    HarnessSurface.SerializePlan(id, action))
                .ConfigureAwait(true);
            if (response.IsError || response.Result is null)
            {
                // A connect with nothing answering here is a fact about this
                // computer, and the way out is the switch above -- not a
                // retry, and not a sentence written here about a file.
                Notice = HarnessSurface.NoDestination(response.Error)
                    ? string.Empty
                    : _copy.WriteUnconfirmed;
                return null;
            }

            Notice = string.Empty;
            return HarnessSurface.ParsePlan(response.Result.Value.GetRawText());
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(PlanAsync));
            Notice = _copy.WriteUnconfirmed;
            return null;
        }
    }

    /// <summary>
    /// Makes an edit that was already shown, by handing back the plan id the
    /// daemon minted. Null on any refusal, and nothing was written.
    /// </summary>
    /// <remarks>
    /// There is no retry and no automatic re-plan. The plan is taken out of
    /// the daemon's store before the file is re-checked and before the write,
    /// so nothing survives a refusal to commit again -- and re-planning on the
    /// contributor's behalf would be deciding a write they were not shown. The
    /// caller closes the preview, shows the unconfirmed-write sentence, and
    /// reads the list again; pressing the button once more is theirs to do.
    /// </remarks>
    public async Task<HarnessCommit?> CommitAsync(string planId)
    {
        if (_copy is null)
        {
            return null;
        }

        try
        {
            DaemonResponse response = await _host
                .CallAsync(
                    DaemonProtocol.Methods.HarnessCommit,
                    HarnessSurface.SerializeCommit(planId))
                .ConfigureAwait(true);
            if (response.IsError || response.Result is null)
            {
                // Every refusal is the same refusal. The daemon takes the plan
                // out of its store before it re-checks the file and before it
                // writes, so an expired plan, a spent one, a file that moved
                // and a failed write all leave nothing to commit again -- and
                // nothing written either. One sentence, no retry.
                Notice = _copy.WriteUnconfirmed;
                return null;
            }

            Notice = string.Empty;

            // Whatever was already running is holding the old setting until it
            // is started again. Shown once a write has actually happened;
            // before that there is nothing to restart for.
            HarnessCommit? committed = HarnessSurface.ParseCommit(response.Result.Value.GetRawText());
            if (committed is { Committed: true })
            {
                Restart = _copy.HarnessNeedsRestart;
            }

            return committed;
        }
        catch
        {
            System.Diagnostics.Trace.TraceWarning(nameof(CommitAsync));
            Notice = _copy.WriteUnconfirmed;
            return null;
        }
    }

    private void RaiseEverythingDerived()
    {
        Raise(nameof(Enabled));
        Raise(nameof(StateText));
        Raise(nameof(ServingText));
        Raise(nameof(HasServingText));
        Raise(nameof(IsWorking));
        Raise(nameof(StateIsNeutral));
        Raise(nameof(StateIsHeld));
        Raise(nameof(StateIsClear));
        Raise(nameof(StateIsAttention));
        Raise(nameof(StateIsRefused));
    }

    private void Raise(string? name) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}
