using System;
using System.ComponentModel;
using System.Globalization;
using TraceCommons.Interop;

namespace TraceCommons.App.ViewModels;

/// <summary>
/// One queue row, formatted for display.
///
/// Mostly a read-only projection of <see cref="QueueEntry"/>: it exposes what
/// a row shows, and it deliberately exposes NO raw transcript content -- the
/// full body is reachable only through an explicit preview sheet, which is
/// the C ABI's single content exemption and should stay a deliberate act
/// rather than something a list view can leak by binding to the wrong
/// property.
///
/// The one mutable, notifying piece is <see cref="Preview"/>: the card's own
/// preview, requested through the daemon's bounded scheduler
/// (<c>preview_request</c>) and filled in later by a <c>preview_ready</c>
/// event. It carries an opening-prompt excerpt and a scrubbing receipt when
/// ready, which the sheet's own <c>PreviewSummary</c> already carries in full
/// -- this is the same content exemption, on a second, smaller surface, so it
/// follows the same rule: display only, never a log line.
/// </summary>
public sealed class QueueEntryViewModel : INotifyPropertyChanged
{
    private readonly QueueEntry _entry;
    private PreviewCardOutcome? _preview;

    public QueueEntryViewModel(QueueEntry entry)
    {
        _entry = entry ?? throw new ArgumentNullException(nameof(entry));
    }

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>
    /// What the daemon's preview scheduler has said about this card, or null
    /// before <c>preview_request</c> has been made or answered -- which reads
    /// the same as <see cref="IsPreviewPending"/>, so a fresh row and one
    /// still queued or running look identical: both are honestly "not yet
    /// known", and a card must not guess between them.
    /// </summary>
    public PreviewCardOutcome? Preview
    {
        get => _preview;
        set
        {
            if (ReferenceEquals(_preview, value))
            {
                return;
            }

            _preview = value;
            Raise(nameof(Preview));
            Raise(nameof(IsPreviewPending));
            Raise(nameof(HasOpeningPrompt));
            Raise(nameof(OpeningPromptText));
            Raise(nameof(IsTooLargeToPreview));
            Raise(nameof(TooLargeText));
            Raise(nameof(HasScrubbingReceipt));
            Raise(nameof(ScrubbingReceiptText));
            Raise(nameof(MatchedNothing));
        }
    }

    /// <summary>No answer yet: never requested, or queued, or running.</summary>
    public bool IsPreviewPending => _preview is null || _preview.IsPending;

    public bool HasOpeningPrompt =>
        _preview is { IsReady: true, Summary.OpeningPrompt.Length: > 0 };

    public string OpeningPromptText => _preview?.Summary?.OpeningPrompt ?? string.Empty;

    public bool IsTooLargeToPreview => _preview?.IsTooLarge == true;

    /// <summary>
    /// "too large to preview (367.5 MB)" -- the fixed line plus a stat of the
    /// file, and NOTHING resembling a would-send figure. See
    /// <see cref="PreviewCardOutcome.TooLargeText"/> and the design spec's
    /// "Rejected alternatives": a number derived from anything but the
    /// envelope that would actually be sent is a false number on a consent
    /// surface.
    /// </summary>
    public string TooLargeText
    {
        get
        {
            if (_preview is not { IsTooLarge: true } preview)
            {
                return string.Empty;
            }

            return string.Format(
                CultureInfo.CurrentCulture,
                "{0} ({1})",
                PreviewCardOutcome.TooLargeText,
                FormatBytes(preview.RawSessionBytes));
        }
    }

    public bool HasScrubbingReceipt => _preview is { IsReady: true, Summary: not null };

    public string ScrubbingReceiptText => _preview?.Summary?.RedactionReceipt ?? string.Empty;

    /// <summary>
    /// Whether this session's preview reports that no pattern fired.
    /// </summary>
    /// <remarks>
    /// False until a preview lands, which is the honest reading: not knowing
    /// is not the same as knowing nothing matched, and a rail that raised an
    /// alarm about every card still loading would be raising it about
    /// nothing. Counts REMOVALS, so a session whose only count is a surviving
    /// secret reads as nothing matched here too, which is true and is the
    /// state the gold chip exists to say.
    /// </remarks>
    public bool MatchedNothing =>
        _preview is { IsReady: true, Summary: not null }
        && RedactionLabels.Total(_preview.Summary.Redactions) == 0;

    /// <summary>
    /// Whether part of this conversation was left out to fit its byte budget.
    /// </summary>
    /// <remarks>
    /// A load-time fact on the entry, not a property of the preview, so it is
    /// as true while the card still reads "Loading preview…" as after one
    /// lands. See <see cref="SubagentText"/>.
    /// </remarks>
    public bool WasTrimmed => _entry.SubagentsDropped > 0;

    /// <summary>
    /// Whether this session can actually be contributed, and everything the
    /// row draws about it: the sentence, its tone, and whether a send control
    /// may be offered at all.
    /// </summary>
    /// <remarks>
    /// Resolved once, through the shared crate. NOTHING BELOW BRANCHES ON THE
    /// STATE STRING -- three shells each deciding which rows get a send button
    /// is three chances to offer one beside a session the server will refuse,
    /// which is the defect this surface exists to remove.
    ///
    /// <para>
    /// A load-time fact on the entry, exactly as <see cref="WasTrimmed"/> is,
    /// so it is as true while the card still reads "Loading preview…" as
    /// after one lands. A row must not be able to reach a send through a card
    /// that never got a preview.
    /// </para>
    /// </remarks>
    private ContributionEligibilityDecision Eligibility =>
        ContributionEligibilitySurface.Decide(_entry);

    /// <summary>
    /// Whether a send control may be drawn for this row.
    /// </summary>
    /// <remarks>
    /// True for a row the daemon never asked the question of -- an invited
    /// contributor has no eligibility question, and this slice must not take
    /// an offer away from a queue that was never in doubt.
    /// </remarks>
    public bool CanContribute => Eligibility.OffersContribute;

    /// <summary>
    /// Whether this row carries a sentence about its eligibility. False for
    /// every row of an invited contributor's queue: absent is not a state,
    /// and a caveat on work that carries no question is a caveat invented
    /// here.
    /// </summary>
    public bool HasEligibilityText => Eligibility.HasStateLine;

    /// <summary>The sentence itself, from the shared crate.</summary>
    public string EligibilityText => Eligibility.StateLine ?? string.Empty;

    /// <summary>
    /// The one state with something to do about it: a setting decides
    /// whether future sessions are contributable.
    /// </summary>
    public bool EligibilityIsAttention =>
        HasEligibilityText && Eligibility.Tone == PrivateInferenceTone.Attention;

    /// <summary>This session has what a contribution needs.</summary>
    public bool EligibilityIsClear =>
        HasEligibilityText && Eligibility.Tone == PrivateInferenceTone.Clear;

    /// <summary>
    /// Everything else, drawn in the ordinary ink.
    /// </summary>
    /// <remarks>
    /// Deliberately the COMPLEMENT of the other two rather than a test for
    /// <see cref="PrivateInferenceTone.Neutral"/>. A permanent ineligibility
    /// is neutral today, and a later ABI answering a tone this build does not
    /// draw an arm for would otherwise leave the row silent -- which is the
    /// one outcome forbidden here, because the row is being shown either way
    /// and has to say why it carries no send control.
    /// </remarks>
    public bool EligibilityIsPlain =>
        HasEligibilityText && !EligibilityIsAttention && !EligibilityIsClear;

    /// <summary>
    /// Whether a second sentence naming the reason belongs under it.
    /// </summary>
    /// <remarks>
    /// Absent on every eligible row, and on a reason label this build does
    /// not know -- for which the shared crate answers nothing rather than
    /// guessing, and the row draws nothing rather than adding a detail nobody
    /// established.
    /// </remarks>
    public bool HasEligibilityReason => Eligibility.HasReasonLine;

    /// <summary>That sentence, from the shared crate.</summary>
    public string EligibilityReasonText => Eligibility.ReasonLine ?? string.Empty;

    /// <summary>
    /// Whether this session carries proof of the model call that produced it,
    /// and everything the row draws about it: the sentence, and its tone.
    /// </summary>
    /// <remarks>
    /// <b>ANSWERED FOR EVERY ROW, unlike <see cref="Eligibility"/>.</b> The
    /// two sentences sit together and read as siblings, which makes it
    /// natural to hang this off the eligibility line's presence. Doing so
    /// would blank it for exactly the contributors it was added for: an
    /// invited contributor has no eligibility question, so their rows carry
    /// no eligibility line at all, and the mark is the fact about their trace
    /// that the credit it earns will turn on.
    ///
    /// <para>
    /// Resolved through the shared crate, and a load-time fact on the entry
    /// exactly as <see cref="Eligibility"/> is, so it is as true while the
    /// card still reads "Loading preview..." as after one lands.
    /// </para>
    /// </remarks>
    private AttestationMarkDecision Attestation =>
        AttestationMarkSurface.Decide(_entry);

    /// <summary>The sentence itself, from the shared crate.</summary>
    public string AttestationText => Attestation.MarkLine;

    /// <summary>
    /// Whether a witness certificate is held for the bytes this row was
    /// pinned to, true after either witness route.
    /// </summary>
    /// <remarks>
    /// The row's own answer, straight off the entry. Not the attestation
    /// mark above: that says whether the session carries a copy of the model
    /// call that produced it, this says whether a certificate is held over
    /// the reviewed bytes.
    /// </remarks>
    public bool HoldsCertificate => _entry.HoldsCertificate;

    /// <summary>
    /// The one mark with something to do about it: a setting decides whether
    /// future sessions carry proof.
    /// </summary>
    public bool AttestationIsAttention =>
        Attestation.HasMarkLine && Attestation.Tone == PrivateInferenceTone.Attention;

    /// <summary>This session carries a checkable copy of its model call.</summary>
    public bool AttestationIsClear =>
        Attestation.HasMarkLine && Attestation.Tone == PrivateInferenceTone.Clear;

    /// <summary>
    /// Everything else, drawn in the ordinary ink.
    /// </summary>
    /// <remarks>
    /// Deliberately the COMPLEMENT of the other two rather than a test for
    /// <see cref="PrivateInferenceTone.Neutral"/>, for the reason
    /// <see cref="EligibilityIsPlain"/> is: a later ABI answering a tone this
    /// build draws no arm for would otherwise leave the row saying nothing
    /// about a question every row is supposed to answer.
    /// </remarks>
    public bool AttestationIsPlain =>
        Attestation.HasMarkLine && !AttestationIsAttention && !AttestationIsClear;

    /// <summary>
    /// Whether a second sentence naming the reason belongs under it.
    /// </summary>
    /// <remarks>
    /// THE KEY'S PRESENCE DECIDES THIS, NOT THE MARK. An attested session
    /// never carries a reason and both unattested marks always do, so
    /// deciding from the mark looks right until <c>unknown</c>: it carries
    /// none when the row was never evaluated, and carries one when a send was
    /// refused because the receipt service was down. The shared crate answers
    /// nothing for an absent or unfamiliar label, so asking it is the branch.
    /// </remarks>
    public bool HasAttestationReason => Attestation.HasReasonLine;

    /// <summary>
    /// That sentence, from the shared crate. Never the eligibility reason
    /// sentence for the same label, which says a request was refused.
    /// </summary>
    public string AttestationReasonText => Attestation.ReasonLine ?? string.Empty;

    private void Raise(string name) => PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(name));

    /// <summary>The daemon's identifier for this entry; used to open a preview.</summary>
    public string EntryId => _entry.EntryId;

    /// <summary>
    /// The daemon's identifier for this session's project, as
    /// <c>entry_value</c> publishes it -- the one a project-group submit must
    /// send as <c>approve</c>'s <c>project_id</c>. Never the display string
    /// <see cref="ProjectLabel"/>, which the daemon does not treat as an
    /// identifier. May be empty for an entry the daemon did not associate
    /// with a project.
    /// </summary>
    public string ProjectId => _entry.ProjectId ?? string.Empty;

    /// <summary>
    /// The project this session belongs to. Falls back to the id, then to a
    /// fixed label -- never to a path, which the daemon does not send and this
    /// row must not invent.
    /// </summary>
    public string ProjectLabel =>
        !string.IsNullOrWhiteSpace(_entry.ProjectLabel) ? _entry.ProjectLabel!
        : !string.IsNullOrWhiteSpace(_entry.ProjectId) ? _entry.ProjectId!
        : "Unknown project";

    /// <summary>
    /// Where this session actually ran, when that is not the project root.
    /// </summary>
    /// <remarks>
    /// Key normalization walks up to the enclosing repository, so two sibling
    /// subdirectories of one repo become one folder. That is the merge the
    /// folder-first queue wanted, and this line is what pays for it: the
    /// sessions are grouped by repo and still say individually where they
    /// ran.
    ///
    /// Empty both when the daemon predates the field and when the session ran
    /// at the root -- the daemon sends null in the second case rather than
    /// repeating the project's own path, so the row draws this only when it
    /// says something. Display only, like every path on this surface.
    /// </remarks>
    public string SessionPath => _entry.SessionPath ?? string.Empty;

    /// <summary>Whether there is a session path worth a line of its own.</summary>
    public bool HasSessionPath => SessionPath.Length > 0;

    /// <summary>
    /// The agent that produced the session, in the words a contributor uses
    /// for it rather than the raw source token.
    ///
    /// The macOS and Linux clients both map these, and a card that reads
    /// "claude-code" where the other two read "Claude Code" is three clients
    /// naming the same thing three ways. An unrecognised token is tidied
    /// rather than replaced: the daemon may name an agent this build has
    /// never heard of, and printing it is more useful than hiding it.
    ///
    /// What the transcript DECLARES wins over the adapter that stores it.
    /// An imported Antigravity conversation is a trajectory file, and
    /// calling it "Letta trajectory" names the format rather than the tool
    /// the contributor used.
    /// </summary>
    public string Source => (_entry.DeclaredSource ?? _entry.Source) switch
    {
        null or "" => "—",
        "claude-code" or "claude_code" => "Claude Code",
        "codex" => "Codex",
        "gemini-cli" or "gemini_cli" => "Gemini CLI",
        "antigravity" => "Antigravity",
        "trajectory" or "letta_trajectory" => "Letta trajectory",
        string other when string.IsNullOrWhiteSpace(other) => "—",
        string other => CultureInfo.CurrentCulture.TextInfo.ToTitleCase(
            other.Replace('_', ' ').Replace('-', ' ')),
    };

    public string State => string.IsNullOrWhiteSpace(_entry.State) ? "—" : _entry.State!;

    /// <summary>
    /// Why the entry is in its state. Already written to be read by a
    /// contributor, so it is shown verbatim rather than remapped here -- a
    /// second vocabulary in the UI would drift from the daemon's.
    /// </summary>
    public string? ReasonLabel => _entry.ReasonLabel;

    public bool HasReason => !string.IsNullOrWhiteSpace(_entry.ReasonLabel);

    /// <summary>Human-readable size, binary units to match what Explorer shows.</summary>
    public string SizeText => FormatBytes(_entry.SizeBytes);

    /// <summary>
    /// What this one card actually covers, and -- the half the contract makes
    /// mandatory -- whether any of it was left out to fit. See
    /// <see cref="SubagentCopy"/>.
    ///
    /// Not a property of the preview: both counts are load-time facts carried
    /// on the entry itself, so this line is as true while the card still reads
    /// "Loading preview…" as it is after one lands. A trimmed conversation
    /// must not be able to reach a decision through a card that never got a
    /// preview.
    /// </summary>
    public string SubagentText => SubagentCopy.Line(_entry.SubagentCount, _entry.SubagentsDropped);

    /// <summary>
    /// Whether there is anything to say at all. A session that delegated
    /// nothing and dropped nothing carries no line about subagents -- never a
    /// row reading zero.
    /// </summary>
    public bool HasSubagentText => SubagentText.Length > 0;

    /// <summary>
    /// When the session was discovered, in the viewer's local time. The daemon
    /// sends an RFC 3339 timestamp; an unparsable one degrades to a dash
    /// rather than to the epoch, which would read as a real date.
    /// </summary>
    public string DiscoveredText =>
        DateTimeOffset.TryParse(
            _entry.DiscoveredAt,
            CultureInfo.InvariantCulture,
            DateTimeStyles.RoundtripKind,
            out DateTimeOffset parsed)
            ? parsed.ToLocalTime().ToString("g", CultureInfo.CurrentCulture)
            : "—";

    /// <summary>
    /// Retry state, shown only when the daemon is actually retrying, so a
    /// healthy row carries no noise.
    /// </summary>
    public bool HasAttempts => _entry.Attempts > 0;

    public string AttemptsText => _entry.Attempts == 1
        ? "1 attempt"
        : $"{_entry.Attempts} attempts";

    internal static string FormatBytes(long bytes)
    {
        if (bytes < 0)
        {
            return "—";
        }

        string[] units = { "B", "KB", "MB", "GB" };
        double value = bytes;
        int unit = 0;

        while (value >= 1024 && unit < units.Length - 1)
        {
            value /= 1024;
            unit++;
        }

        // No decimal place on bytes, one everywhere else: "1.4 MB" is useful,
        // "1437.0 B" is not.
        return unit == 0
            ? string.Format(CultureInfo.CurrentCulture, "{0:0} {1}", value, units[unit])
            : string.Format(CultureInfo.CurrentCulture, "{0:0.#} {1}", value, units[unit]);
    }
}
