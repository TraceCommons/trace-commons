using System;
using System.Collections.Generic;
using System.Collections.ObjectModel;
using System.ComponentModel;
using System.Threading.Tasks;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// The roots screen: which session folders this machine should watch.
/// </summary>
/// <remarks>
/// <para>
/// Reached from a refusal, not from a menu. The daemon will not start until
/// both sources are declared, so until this screen is answered the app can do
/// nothing at all -- which is why it is a window of its own rather than a page
/// inside a shell that has no daemon behind it.
/// </para>
/// <para>
/// Thin by design, like <see cref="OnboardingViewModel"/>. Every decision that
/// can be tested lives in <see cref="SessionRootsDeclaration"/> and
/// <see cref="SessionRootsCopy"/> in the interop assembly, because this
/// assembly cannot be built or tested anywhere but Windows and the consent
/// rules are too important to be reachable only there.
/// </para>
/// <para>
/// <b>Why there is no Windows-specific path logic here.</b>
/// <c>crates/trace-commons-contributor/src/source/discovery.rs</c> is
/// platform-neutral by construction -- <c>dirs::home_dir()</c> plus an
/// injected environment lookup, with no <c>cfg(windows)</c> branch -- so this
/// client shares the one implementation rather than carrying a transcription
/// of it that could drift from the others. That is correct on Windows, not
/// merely convenient:
/// </para>
/// <list type="number">
/// <item>Claude Code's documentation states that on Windows <c>~/.claude</c>
/// resolves to <c>%USERPROFILE%\.claude</c>, and that setting
/// <c>CLAUDE_CONFIG_DIR</c> relocates every path under it -- so both the
/// conventional location and the environment override apply on Windows
/// exactly as they do elsewhere. See
/// https://code.claude.com/docs/en/claude-directory.</item>
/// <item>Codex defaults to <c>%USERPROFILE%\.codex</c> when <c>CODEX_HOME</c>
/// is unset, with rollouts under <c>sessions\</c>.</item>
/// <item><c>dirs::home_dir()</c> returns <c>%USERPROFILE%</c> on Windows, so
/// the shared implementation lands on both of those without a branch.</item>
/// </list>
/// <para>
/// The session store is the <c>projects</c> and <c>sessions</c> SUBdirectory
/// in each case, never the parent dot-directory, which also holds settings and
/// plugins the contributor did not agree to have watched.
/// </para>
/// <para>
/// The one rule this file enforces itself: <b>nothing is pre-selected</b>.
/// Both rows start undecided and <see cref="CanContinue"/> stays false until
/// the contributor has answered each one. Discovery fills the screen in with
/// what is on the machine; it does not answer on their behalf. A pre-ticked
/// box plus a habitual Continue is the shape of consent people click through,
/// and it would recover the silent-scanning behaviour this screen exists to
/// end.
/// </para>
/// </remarks>
public sealed class SessionRootsViewModel : INotifyPropertyChanged
{
    private readonly DaemonHost _host;
    private string _error = string.Empty;
    private bool _isBusy;

    public SessionRootsViewModel(DaemonHost host)
        : this(host, SourceDiscovery.ProbeThisMachine())
    {
    }

    /// <summary>
    /// As above, with discovery injected so a caller can supply candidates it
    /// already has rather than probing the disk twice.
    /// </summary>
    public SessionRootsViewModel(DaemonHost host, IReadOnlyList<SourceCandidate> candidates)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        ArgumentNullException.ThrowIfNull(candidates);

        Claude = Row(candidates, SourceDiscovery.ClaudeCode);
        Codex = Row(candidates, SourceDiscovery.Codex);
        Gemini = Row(candidates, SourceDiscovery.GeminiCli);

        Claude.PropertyChanged += OnRowChanged;
        Codex.PropertyChanged += OnRowChanged;
        Gemini.PropertyChanged += OnRowChanged;

        Rows = new ObservableCollection<SourceRowViewModel> { Claude, Codex, Gemini };
    }

    /// <summary>Raised once the daemon has started with the declaration.</summary>
    public event Action? Finished;

    public event PropertyChangedEventHandler? PropertyChanged;

    public string Title => SessionRootsCopy.Title;

    public string Body => SessionRootsCopy.Body;

    public SourceRowViewModel Claude { get; }

    public SourceRowViewModel Codex { get; }

    /// <summary>The Gemini CLI row. Offered like the others, but it cannot
    /// block Continue -- see <c>SessionRootsDeclaration.IsComplete</c>.</summary>
    public SourceRowViewModel Gemini { get; }

    /// <summary>Both rows, for a bound list.</summary>
    public ObservableCollection<SourceRowViewModel> Rows { get; }

    /// <summary>
    /// Whether both sources have been answered. Bound to Continue's
    /// IsEnabled.
    /// </summary>
    public bool CanContinue => !_isBusy && Declaration().IsComplete;

    /// <summary>
    /// Why Continue is disabled, or empty once it is not. Shown as well as
    /// the disabled button: an unanswered source is not "no", and a greyed
    /// out control does not say which of the two is still waiting.
    /// </summary>
    public string Hint => Declaration().IsComplete ? string.Empty : SessionRootsCopy.Incomplete;

    /// <summary>The failure sentence, or empty.</summary>
    public string Error
    {
        get => _error;
        private set
        {
            if (!string.Equals(_error, value, StringComparison.Ordinal))
            {
                _error = value;
                Raise(nameof(Error));
                Raise(nameof(HasError));
            }
        }
    }

    public bool HasError => _error.Length > 0;

    public bool IsBusy
    {
        get => _isBusy;
        private set
        {
            if (_isBusy != value)
            {
                _isBusy = value;
                Raise(nameof(IsBusy));
                Raise(nameof(CanContinue));
            }
        }
    }

    /// <summary>
    /// Declares the roots and starts the daemon in one call.
    /// </summary>
    /// <remarks>
    /// One call rather than "write settings, then start": the refusal is
    /// evaluated after the settings are applied, so passing them to the start
    /// is what turns the contributor's answer into a running daemon without a
    /// restart. See <see cref="DaemonHost.StartAsync"/>.
    /// </remarks>
    public async Task ContinueAsync()
    {
        string? settings = Declaration().SettingsJson();
        if (settings is null)
        {
            // Should be unreachable while Continue is bound to CanContinue,
            // and cheap insurance if that binding is ever changed.
            Error = SessionRootsCopy.Incomplete;
            return;
        }

        IsBusy = true;
        Error = string.Empty;
        try
        {
            await _host.StartAsync(settings).ConfigureAwait(true);
            Finished?.Invoke();
        }
        catch (TcException exception)
        {
            // A refusal here means the declaration this screen built was not
            // one the daemon accepts, which is this shell's bug rather than
            // the contributor's, so it does not get the "answer both" hint.
            Error = exception.IsRootsNotDeclared
                ? "Trace Commons could not record that answer. Please report this."
                : "Could not start. Another Trace Commons instance may already be running.";
        }
        finally
        {
            IsBusy = false;
        }
    }

    private SessionRootsDeclaration Declaration() => new()
    {
        Claude = Claude.Decision,
        Codex = Codex.Decision,
        Gemini = Gemini.Decision,
    };

    private static SourceRowViewModel Row(
        IReadOnlyList<SourceCandidate> candidates,
        string source)
    {
        SourceCandidate candidate = SourceDiscovery.For(candidates, source)
            ?? new SourceCandidate { Source = source };
        return new SourceRowViewModel(candidate);
    }

    private void OnRowChanged(object? sender, PropertyChangedEventArgs e)
    {
        Raise(nameof(CanContinue));
        Raise(nameof(Hint));
    }

    private void Raise(string? name) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}

/// <summary>
/// One agent's row on the roots screen: what discovery found, and what the
/// contributor answered about it.
/// </summary>
public sealed class SourceRowViewModel : INotifyPropertyChanged
{
    private string _path;
    private SourceDecisionKind _kind = SourceDecisionKind.Undecided;

    public SourceRowViewModel(SourceCandidate candidate)
    {
        ArgumentNullException.ThrowIfNull(candidate);

        // Pre-filling the PATH is not pre-selecting the ANSWER. The folder is
        // shown so the contributor can see what they would be agreeing to;
        // the answer stays Undecided until they press one of the buttons.
        _path = candidate.Path;
        AgentName = SessionRootsCopy.AgentName(candidate.Source);
        Evidence = SessionRootsCopy.Evidence(candidate);
        IsRelocated = candidate.RelocatedByEnv;
        RelocatedNote = SessionRootsCopy.RelocatedNote(candidate.Source);
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    public string AgentName { get; }

    /// <summary>What discovery found, as one sentence.</summary>
    public string Evidence { get; }

    /// <summary>Whether an environment variable moved this store.</summary>
    public bool IsRelocated { get; }

    public string RelocatedNote { get; }

    public string DoNotUseLabel => SessionRootsCopy.DoNotUse;

    /// <summary>
    /// Shown on every row, not only when discovery came back empty. See
    /// <see cref="SessionRootsCopy.ManualHint"/>.
    /// </summary>
    public string ManualHint => SessionRootsCopy.ManualHint;

    /// <summary>
    /// The folder this row would watch. Editable, so a contributor whose
    /// store is somewhere discovery did not look can still name it.
    ///
    /// Editing it clears an existing answer: the folder that was agreed to is
    /// no longer the folder in the box, and carrying the old Watch forward
    /// would declare a path the contributor never saw when they pressed the
    /// button.
    /// </summary>
    public string Path
    {
        get => _path;
        set
        {
            string next = value ?? string.Empty;
            if (string.Equals(_path, next, StringComparison.Ordinal))
            {
                return;
            }

            _path = next;
            if (_kind == SourceDecisionKind.Watch)
            {
                _kind = SourceDecisionKind.Undecided;
                RaiseAnswer();
            }

            Raise(nameof(Path));
        }
    }

    /// <summary>This row's answer.</summary>
    public SourceDecision Decision => _kind switch
    {
        SourceDecisionKind.Watch => SourceDecision.Watch(_path),
        SourceDecisionKind.Off => SourceDecision.Off,
        _ => SourceDecision.Undecided,
    };

    public bool IsWatchSelected => _kind == SourceDecisionKind.Watch;

    public bool IsOffSelected => _kind == SourceDecisionKind.Off;

    /// <summary>Answer: watch the folder in <see cref="Path"/>.</summary>
    public void ChooseWatch()
    {
        _kind = SourceDecisionKind.Watch;
        RaiseAnswer();
    }

    /// <summary>Answer: this agent is not used on this machine.</summary>
    public void ChooseOff()
    {
        _kind = SourceDecisionKind.Off;
        RaiseAnswer();
    }

    private void RaiseAnswer()
    {
        Raise(nameof(Decision));
        Raise(nameof(IsWatchSelected));
        Raise(nameof(IsOffSelected));
    }

    private void Raise(string? name) =>
        PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));
}
