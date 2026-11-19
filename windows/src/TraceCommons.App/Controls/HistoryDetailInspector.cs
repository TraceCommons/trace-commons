using System;
using System.Collections.Generic;
using System.Linq;
using System.Text.Json;
using System.Threading.Tasks;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Media;
using TraceCommons.App.ViewModels;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls;

/// <summary>
/// An opened History record's details in the inspector (W21): the Mac's
/// <c>HistoryDetailInspector</c> (Views/Monitor/HistoryInspector.swift) over
/// its <c>SessionDetailView</c> (Views/SessionDetailView.swift) and
/// <c>SessionContributionOverview</c>.
/// </summary>
/// <remarks>
/// <para>Every word is the core's: the public-run table
/// (<c>tc_public_run_copy</c>), its error lines, and the monitor table's
/// held sentence and dismiss word. Without the public-run table the detail
/// draws nothing and offers nothing (SessionDetailView.swift:30-32).</para>
/// <para>Withdraw is not drawn here: it stays on the History row, so the page
/// has one (HistoryInspector.swift:10-12). Skills and Train coding models
/// are W20's and are not drawn by this slice.</para>
/// <para>Publishing is consent: nothing is preselected, the editor is
/// validated only by the core (<c>tc_public_run_validate_editor</c>), a review
/// of the exact public page comes before Publish, at most four evidence items
/// are chosen, and every daemon error is read and said in the core's words.
/// A refused publish leaves the record as it was; an unpublish returns it to
/// private at the version the daemon names.</para>
/// <para>Built in code, as <see cref="GoPublicDialog"/> is; rebuilt only on a
/// state change (a read, a mode change, a theme change), never on a
/// keystroke, so typing keeps focus. Field edits update their counters, the
/// core's verdict and the Review button in place.</para>
/// </remarks>
public sealed class HistoryDetailInspector : UserControl
{
    private readonly DaemonHost _host;
    private readonly StackPanel _root = new() { Spacing = 16 };

    private readonly HistoryDetailState _state = new();
    private bool _loading;
    private bool _working;
    private string? _dismissedError;

    private SessionDetail? CurrentDetail => _state.Detail;

    // The editor's state (SessionDetailView.swift:183-197).
    private bool _configured;
    private bool _editingPublished;
    private PublicRunDraft? _reviewDraft;
    private string _title = string.Empty;
    private string _outcome = string.Empty;
    private string _workflow = string.Empty;
    private string _source = string.Empty;
    private bool _includeCorrection;
    private string? _permission;
    private IReadOnlySet<string> _evidence = new HashSet<string>(StringComparer.Ordinal);

    // Controls a keystroke updates in place.
    private TextBlock? _problemLine;
    private Button? _reviewButton;
    private readonly Dictionary<TextBox, (TextBlock Counter, int Limit)> _counters = new();
    private readonly List<CheckBox> _evidenceBoxes = new();

    public HistoryDetailInspector(DaemonHost host, HistoryRecordViewModel record)
    {
        _host = host ?? throw new ArgumentNullException(nameof(host));
        Record = record ?? throw new ArgumentNullException(nameof(record));
        Content = new ScrollViewer
        {
            Content = _root,
            Padding = new Thickness(22, 14, 22, 22),
            HorizontalScrollMode = ScrollMode.Disabled,
            IsTabStop = false,
        };
        Background = Res<Brush>("TcSurfaceCardBrush");
        AutomationProperties.SetAutomationId((UIElement)Content, "HistoryDetailInspector");
        Loaded += OnLoaded;
        ActualThemeChanged += (_, _) => Render();
    }

    /// <summary>The record this inspector is about.</summary>
    public HistoryRecordViewModel Record { get; private set; }

    private static PublicRunCopy? Copy => PublicRunCopy.Current;

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        Loaded -= OnLoaded;
        Render();
        await LoadAsync();
    }

    /// <summary>
    /// The list was redrawn: keep showing the same submission with its
    /// refreshed record, so the tag agrees with the row (HistoryInspector.swift:61-64).
    /// </summary>
    public void Refresh(HistoryRecordViewModel record)
    {
        Record = record;
        Render();
    }

    /// <summary>
    /// Reads the detail, keeping the one already held until the daemon
    /// answers (<c>AppModel.loadSessionDetail</c>, AppModel.swift:4195-4258):
    /// a reload never takes the editor or its draft away.
    /// </summary>
    public async Task LoadAsync()
    {
        if (_loading || _working)
        {
            return;
        }

        // With a detail held, a reload touches only the reading line, so the
        // editor keeps its caret, its draft and the pane its scroll; the page
        // is rebuilt only when what the daemon said has changed.
        SessionDetail? held = CurrentDetail;
        _loading = true;
        _state.ReadStarted();
        RefreshStatus();
        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.HistoryDetail, PublicRunEditor.SubmissionParams(Record.SubmissionId))
            .ConfigureAwait(true);
        _loading = false;
        string? raw = null;
        if (response.IsError || response.Result is not JsonElement result)
        {
            _state.ReadFailed(response.Error?.Message);
        }
        else if (SessionDetail.Parse(result) is { } detail)
        {
            raw = result.GetRawText();
            SetDetail(detail);
        }
        else
        {
            _state.ReadFailed("session-detail-unavailable");
        }

        if (held is not null && CurrentDetail is null)
        {
            // The account ended or changed under the read: nothing it typed
            // or chose stays (the Mac drops the editor with the detail).
            ResetEditor();
        }

        bool unchanged = held is not null && CurrentDetail is not null
            && (raw is null || raw == _lastRaw);
        if (raw is not null)
        {
            _lastRaw = raw;
        }

        if (unchanged && _statusSlot is not null)
        {
            RefreshStatus();
        }
        else
        {
            Render();
        }
    }

    private string? _lastRaw;
    private StackPanel? _statusSlot;

    private void ResetEditor()
    {
        _configured = false;
        _editingPublished = false;
        _reviewDraft = null;
        _title = _outcome = _workflow = _source = string.Empty;
        _includeCorrection = false;
        _permission = null;
        _evidence = new HashSet<string>(StringComparer.Ordinal);
        _lastRaw = null;
    }

    /// <summary>The reading line, or the failed read with Retry, in its own slot.</summary>
    private void RefreshStatus()
    {
        if (_statusSlot is null || Copy is not { } copy)
        {
            Render();
            return;
        }

        _statusSlot.Children.Clear();
        if (_loading)
        {
            _statusSlot.Children.Add(Caption(copy["reading_record"], "TcInkSecondaryBrush"));
        }
        else if (_state.ReadError is { } error)
        {
            var notice = Card();
            var inner = new StackPanel { Spacing = 8 };
            inner.Children.Add(Text(error, "TcBodyTextStyle", "TcStatusOutsideTextBrush"));
            Button retry = ActionButton(copy["retry_read"], primary: false);
            retry.Click += async (_, _) => await LoadAsync();
            inner.Children.Add(retry);
            notice.Child = inner;
            _statusSlot.Children.Add(notice);
        }
    }

    private void SetDetail(SessionDetail detail)
    {
        if (CurrentDetail?.OwnerScopeSha256 is { } previous && detail.OwnerScopeSha256 is { } scope && previous != scope)
        {
            ResetEditor();
        }

        int? before = CurrentDetail?.Publication?.Version;
        _state.ReadSucceeded(detail);
        if (detail.Publication is not null && detail.Publication.Version != before)
        {
            _reviewDraft = null;
            _editingPublished = false;
        }

        if (!_configured)
        {
            _configured = true;
            if (detail.Publication is { } page)
            {
                Populate(page);
            }
            else
            {
                _source = detail.RetainedSourceSlug ?? string.Empty;
            }
        }
    }

    // ---- Drawing ---------------------------------------------------------

    private void Render()
    {
        _root.Children.Clear();
        _counters.Clear();
        _evidenceBoxes.Clear();
        _problemLine = null;
        _reviewButton = null;
        _statusSlot = null;

        PublicRunCopy? copy = Copy;
        _root.Children.Add(Summary(copy));
        foreach (string line in Explanations())
        {
            _root.Children.Add(Caption(line, "TcInkSecondaryBrush"));
        }

        if (copy is null)
        {
            return;
        }

        var heading = new StackPanel { Spacing = 2 };
        TextBlock title = Text(copy["session_detail"], "TcTitleSectionTextStyle");
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level2);
        heading.Children.Add(title);
        heading.Children.Add(Caption($"{Record.ProjectLabel} · {Record.SubmittedText}", "TcInkSecondaryBrush"));
        _root.Children.Add(heading);

        _statusSlot = new StackPanel();
        _root.Children.Add(_statusSlot);
        RefreshStatus();

        if (CurrentDetail is { } detail)
        {
            Overview(copy, detail);
            bool completed = ContributionGate.WithdrawalCompleted(Record.Status, Record.WithdrawnHere);
            if (ContributionGate.IsActive(detail, Record.Status, completed))
            {
                _root.Children.Add(Editor(copy, detail));
            }
            else if (!ContributionGate.IsWithdrawn(detail, Record.Status, completed))
            {
                _root.Children.Add(Caption(copy["publication_after_acceptance"], "TcInkSecondaryBrush"));
            }
        }
    }

    /// <summary>
    /// The status in the core's words, said once: until the detail's
    /// overview says it (HistoryInspector.swift:57-89).
    /// </summary>
    private UIElement Summary(PublicRunCopy? copy)
    {
        var panel = new StackPanel { Spacing = 6 };
        panel.Children.Add(Caption(Record.SubmittedText, "TcInkTertiaryBrush"));
        if (HistoryInspection.TagsStatus(recordResolves: true, CurrentDetail)
            && HistoryInspection.StatusWord(copy, CurrentDetail?.ContributionStatus ?? Record.Status) is { } word)
        {
            panel.Children.Add(StatusTag(word, HistoryInspection.Tone(CurrentDetail?.ContributionStatus ?? Record.Status)));
        }

        return panel;
    }

    /// <summary>
    /// The server's own reasons, without digests; a held record it said
    /// nothing about reads the core's held sentence (HistoryInspector.swift:107-121).
    /// </summary>
    private IEnumerable<string> Explanations()
    {
        IReadOnlyList<string> lines = HistoryInspection.ExplanationLines(new[] { Record.Explanations });
        if (lines.Count == 0 && Record.Status == "quarantined")
        {
            string held = MonitorShellCopy.Top("held_explanation");
            return held.Length == 0 ? lines : new[] { held };
        }

        return lines;
    }

    /// <summary>The overview's four cards (SessionContributionOverview.swift:13-126).</summary>
    private void Overview(PublicRunCopy copy, SessionDetail detail)
    {
        string Missing(string ordinary) => detail.ContentUnavailable == true ? copy["unavailable_value"] : ordinary;

        if (detail.ContentUnavailable == true)
        {
            var notice = Card();
            notice.Child = Text(copy["content_unavailable"], "TcBodyTextStyle");
            _root.Children.Add(notice);
        }

        StackPanel task = EyebrowCard(copy["task"]);
        task.Children.Add(Value(detail.Task, Missing(copy["no_task"])));
        task.Children.Add(Eyebrow(copy["outcome"]));
        task.Children.Add(Value(copy.TaskOutcomeLabel(detail.TaskSuccess), Missing(copy["outcome_unavailable"])));
        if (copy.FeedbackLabel(detail.UserFeedback) is { } feedback)
        {
            task.Children.Add(Caption(feedback, "TcInkSecondaryBrush"));
        }

        StackPanel correction = EyebrowCard(copy["decisive_correction"]);
        correction.Children.Add(Value(detail.HumanCorrection, Missing(copy["no_correction"])));

        StackPanel evidence = EyebrowCard(copy["supporting_evidence"]);
        evidence.Children.Add(Caption(copy["observed_in_version"], "TcInkSecondaryBrush"));
        if (detail.Evidence.Count == 0)
        {
            evidence.Children.Add(Value(null, Missing(copy["no_evidence"])));
        }

        foreach (SessionEvidence item in detail.Evidence)
        {
            evidence.Children.Add(Eyebrow(copy.EvidenceKindLabel(item.Kind)));
            evidence.Children.Add(Selectable(item.Excerpt, "TcCaptionTextStyle"));
        }

        StackPanel details = EyebrowCard(copy["contribution_details"]);
        details.Children.Add(KeyValue(copy["processing_status"], copy.HistoryStatusLabel(detail.ContributionStatus ?? Record.Status), mono: false));
        details.Children.Add(Caption(copy["permitted_uses"], "TcInkSecondaryBrush"));
        if (detail.PermittedUses is { Count: > 0 } uses)
        {
            foreach (string use in uses)
            {
                details.Children.Add(StatusTag(copy.PermittedUseLabel(use), StatusTone.On));
            }
        }
        else
        {
            details.Children.Add(Caption(
                detail.PermittedUses is null ? copy["permitted_uses_unavailable"] : copy["no_permitted_uses"],
                "TcInkSecondaryBrush"));
        }

        details.Children.Add(Eyebrow(copy["contributed_version"]));
        details.Children.Add(KeyValue(copy["envelope_version"], detail.ContributedVersion, mono: true));
        details.Children.Add(KeyValue(copy["consent_policy_version"], detail.ConsentPolicyVersion, mono: true));
        details.Children.Add(KeyValue(copy["redaction_version"], detail.RedactionPipelineVersion, mono: true));
    }

    // ---- The public-run editor (SessionDetailView.swift:179-532) ------------

    private UIElement Editor(PublicRunCopy copy, SessionDetail detail)
    {
        StackPanel card = EyebrowCard(copy["public_workflow"], add: false);
        if (detail.Publication is { } page && !_editingPublished)
        {
            Published(copy, card, page);
        }
        else if (_reviewDraft is { } draft)
        {
            Review(copy, card, detail, draft);
        }
        else
        {
            EditFields(copy, card, detail);
        }

        // A failed request, under the buttons it is about, with a Dismiss
        // link after it (SessionDetailView.swift:207-216).
        if (_state.PublicationError is { } error && error != _dismissedError)
        {
            var row = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
            row.Children.Add(Text(error, "TcCaptionTextStyle", "TcStatusOutsideTextBrush"));
            string dismiss = MonitorWords.Dismiss;
            if (dismiss.Length > 0)
            {
                var link = new HyperlinkButton { Content = dismiss };
                AutomationProperties.SetName(link, dismiss);
                link.Click += (_, _) =>
                {
                    _dismissedError = error;
                    Render();
                };
                row.Children.Add(link);
            }

            card.Children.Add(row);
        }

        return Wrap(card);
    }

    private void Published(PublicRunCopy copy, StackPanel card, PublicRunPage page)
    {
        card.Children.Add(StatusTag(copy["published"], StatusTone.On));
        card.Children.Add(Text(page.Title, "TcTitleCardTextStyle"));
        card.Children.Add(Text(page.OutcomeSummary, "TcBodyTextStyle", "TcInkSecondaryBrush"));
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        if (page.PublicUrl is { } url && Uri.TryCreate(url, UriKind.Absolute, out Uri? uri) && uri.Scheme == Uri.UriSchemeHttps)
        {
            Button open = ActionButton(copy["open_page"], primary: true);
            open.Click += async (_, _) => await Windows.System.Launcher.LaunchUriAsync(uri);
            actions.Children.Add(open);
        }

        Button edit = ActionButton(copy["edit_page"], primary: false);
        edit.Click += (_, _) =>
        {
            Populate(page);
            _editingPublished = true;
            Render();
        };
        actions.Children.Add(edit);
        Button unpublish = ActionButton(_working ? copy["unpublishing"] : copy["unpublish"], primary: false);
        unpublish.IsEnabled = !_working && !_loading;
        unpublish.Click += async (_, _) => await UnpublishAsync();
        actions.Children.Add(unpublish);
        card.Children.Add(actions);
    }

    private void EditFields(PublicRunCopy copy, StackPanel card, SessionDetail detail)
    {
        card.Children.Add(Caption(copy["publication_disclosure"], "TcInkSecondaryBrush"));
        card.Children.Add(Field(copy["page_title"], _title, PublicRunEditor.TitleLimit, multiline: false, v => _title = v));
        card.Children.Add(Field(copy["public_outcome"], _outcome, PublicRunEditor.OutcomeLimit, multiline: true, v => _outcome = v));
        card.Children.Add(Field(copy["reusable_instructions"], _workflow, PublicRunEditor.WorkflowLimit, multiline: true, v => _workflow = v));

        var evidence = new StackPanel { Spacing = 6 };
        evidence.Children.Add(Eyebrow(copy["supporting_evidence"]));
        evidence.Children.Add(Caption(copy["select_evidence"], "TcInkSecondaryBrush"));
        foreach (SessionEvidence item in detail.Evidence)
        {
            string kind = copy.EvidenceKindLabel(item.Kind);
            var label = new StackPanel { Spacing = 2 };
            label.Children.Add(Text(kind, "TcCaptionTextStyle"));
            var excerpt = Text(item.Excerpt, "TcCaptionTextStyle", "TcInkSecondaryBrush");
            excerpt.MaxLines = 4;
            excerpt.TextTrimming = TextTrimming.CharacterEllipsis;
            label.Children.Add(excerpt);
            var box = new CheckBox { Content = label, Tag = item.EventId, IsChecked = _evidence.Contains(item.EventId), MinHeight = 44 };
            AutomationProperties.SetName(box, kind + " " + item.Excerpt);
            box.Click += (_, _) =>
            {
                _evidence = PublicRunEditor.Toggle(_evidence, item.EventId, box.IsChecked == true);
                box.IsChecked = _evidence.Contains(item.EventId);
                UpdateEvidenceBoxes();
                UpdateVerdict(copy, detail);
            };
            _evidenceBoxes.Add(box);
            evidence.Children.Add(box);
        }

        UpdateEvidenceBoxes();
        card.Children.Add(evidence);

        if (detail.HumanCorrection is { } correction)
        {
            var include = new CheckBox { Content = copy["publish_correction"], IsChecked = _includeCorrection, MinHeight = 44 };
            include.Click += (_, _) =>
            {
                _includeCorrection = include.IsChecked == true;
                Render();
            };
            card.Children.Add(include);
            if (_includeCorrection)
            {
                var well = new Border
                {
                    Background = Res<Brush>("TcSurfaceInsetBrush"),
                    CornerRadius = Res<CornerRadius>("TcCardCornerRadius"),
                    Padding = new Thickness(12),
                    Child = Selectable(correction, "TcCaptionTextStyle"),
                };
                card.Children.Add(well);
            }
        }

        // The reuse permission: nothing chosen until the person chooses.
        var permission = new StackPanel { Spacing = 6 };
        permission.Children.Add(Eyebrow(copy["reuse_permission"]));
        if (_permission is null)
        {
            permission.Children.Add(Caption(copy["choose_permission"], "TcInkSecondaryBrush"));
        }

        foreach (PublicRunReuseChoice choice in copy.ReusePermissions)
        {
            string words = choice.Label + " · " + choice.Explanation;
            var radio = new RadioButton
            {
                Content = new TextBlock { Text = words, TextWrapping = TextWrapping.Wrap },
                GroupName = "public-run-permission-" + Record.SubmissionId,
                IsChecked = _permission == choice.Permission,
                MinHeight = 44,
            };
            AutomationProperties.SetName(radio, words);
            radio.Checked += (_, _) =>
            {
                bool first = _permission is null;
                _permission = choice.Permission;
                if (first)
                {
                    Render();
                }
                else
                {
                    UpdateVerdict(copy, detail);
                }
            };
            permission.Children.Add(radio);
        }

        card.Children.Add(permission);

        var source = new StackPanel { Spacing = 6 };
        source.Children.Add(Eyebrow(copy["source_public_run"]));
        var sourceBox = new TextBox { Text = _source, PlaceholderText = copy["source_placeholder"] };
        AutomationProperties.SetName(sourceBox, copy["source_public_run"]);
        sourceBox.TextChanged += (_, _) =>
        {
            _source = sourceBox.Text;
            UpdateVerdict(copy, detail);
        };
        source.Children.Add(sourceBox);
        source.Children.Add(Caption(copy["source_help"], "TcInkSecondaryBrush"));
        card.Children.Add(source);

        _problemLine = Text(string.Empty, "TcCaptionTextStyle", "TcStatusOutsideTextBrush");
        card.Children.Add(_problemLine);

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        _reviewButton = ActionButton(copy["review_page"], primary: true);
        _reviewButton.Click += (_, _) =>
        {
            if (Verdict(detail) is ({ } draft, _))
            {
                _reviewDraft = draft;
                Render();
            }
        };
        actions.Children.Add(_reviewButton);
        if (detail.Publication is not null)
        {
            var cancel = new HyperlinkButton { Content = copy["cancel_edit"] };
            AutomationProperties.SetName(cancel, copy["cancel_edit"]);
            cancel.Click += (_, _) =>
            {
                _editingPublished = false;
                Render();
            };
            actions.Children.Add(cancel);
        }

        card.Children.Add(actions);
        UpdateVerdict(copy, detail);
    }

    private void Review(PublicRunCopy copy, StackPanel card, SessionDetail detail, PublicRunDraft draft)
    {
        TextBlock heading = Text(copy["exact_public_preview"], "TcTitleSectionTextStyle");
        AutomationProperties.SetHeadingLevel(heading, AutomationHeadingLevel.Level3);
        card.Children.Add(heading);
        card.Children.Add(Text(draft.Title, "TcTitleCardTextStyle"));
        card.Children.Add(PreviewField(copy["creator_report"], copy.TaskOutcomeLabel(detail.TaskSuccess) ?? copy["outcome_unavailable"]));
        card.Children.Add(PreviewField(copy["contributed_version"], detail.ContributedVersion));
        card.Children.Add(PreviewField(copy["public_outcome"], draft.OutcomeSummary));
        if (draft.CorrectionExcerpt is { } correction)
        {
            card.Children.Add(PreviewField(copy["decisive_correction"], correction));
        }

        var evidence = new StackPanel { Spacing = 4 };
        evidence.Children.Add(Eyebrow(copy["observed_evidence"]));
        foreach (SessionEvidence item in draft.Evidence)
        {
            evidence.Children.Add(Selectable(item.Excerpt, "TcBodyTextStyle"));
        }

        card.Children.Add(evidence);
        card.Children.Add(PreviewField(copy["use_workflow"], draft.Workflow));
        if (copy.ReuseChoice(draft.ReusePermission) is { } choice)
        {
            card.Children.Add(PreviewField(copy["reuse_permission"], choice.Label));
        }

        if (draft.SourceSlug is { } slug)
        {
            card.Children.Add(PreviewField(copy["source_public_run"], "/runs/" + slug));
        }

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        Button publish = ActionButton(
            _working ? copy["publishing"] : detail.Publication is null ? copy["publish_page"] : copy["update_page"],
            primary: true);
        publish.IsEnabled = !_working && !_loading;
        publish.Click += async (_, _) => await PublishAsync(draft);
        actions.Children.Add(publish);
        var edit = new HyperlinkButton { Content = copy["edit_draft"] };
        AutomationProperties.SetName(edit, copy["edit_draft"]);
        edit.Click += (_, _) =>
        {
            _reviewDraft = null;
            Render();
        };
        actions.Children.Add(edit);
        card.Children.Add(actions);
    }

    /// <summary>The core's verdict on what the editor holds now (SessionDetailView.swift:442-468).</summary>
    private (PublicRunDraft? Draft, string? Error)? Verdict(SessionDetail detail)
    {
        var evidence = detail.Evidence
            .Where(item => _evidence.Contains(item.EventId))
            .Select(item => (item.EventId, item.Excerpt))
            .ToList();
        return PublicRunEditor.Validate(new PublicRunEditorInput(
            _title, _outcome, _includeCorrection ? detail.HumanCorrection : null, _workflow, _permission, evidence, _source));
    }

    private void UpdateVerdict(PublicRunCopy copy, SessionDetail detail)
    {
        foreach ((TextBox box, (TextBlock counter, int limit)) in _counters)
        {
            counter.Text = PublicRunEditor.Counter(box.Text, limit);
        }

        (PublicRunDraft? Draft, string? Error)? verdict = Verdict(detail);
        string? problem = verdict is null ? copy["publication_unavailable"] : verdict.Value.Error;
        if (_problemLine is not null)
        {
            _problemLine.Text = problem ?? string.Empty;
            _problemLine.Visibility = string.IsNullOrEmpty(problem) ? Visibility.Collapsed : Visibility.Visible;
        }

        if (_reviewButton is not null)
        {
            _reviewButton.IsEnabled = verdict?.Draft is not null;
        }
    }

    private void UpdateEvidenceBoxes()
    {
        foreach (CheckBox box in _evidenceBoxes)
        {
            box.IsEnabled = PublicRunEditor.CanSelect(_evidence, (string)box.Tag);
        }
    }

    /// <summary>A published page back in the editor (SessionDetailView.swift:513-530).</summary>
    private void Populate(PublicRunPage page)
    {
        _title = page.Title;
        _outcome = page.OutcomeSummary;
        _workflow = page.Workflow;
        _includeCorrection = page.CorrectionExcerpt is not null;
        _permission = page.ReusePermission;
        _source = page.SourceSlug ?? CurrentDetail?.RetainedSourceSlug ?? string.Empty;
        _evidence = CurrentDetail is null
            ? new HashSet<string>(StringComparer.Ordinal)
            : PublicRunEditor.Restore(CurrentDetail.Evidence, page.Evidence);
        _reviewDraft = null;
    }

    // ---- Publish and unpublish (AppModel.swift:4260-4327) -------------------

    private async Task PublishAsync(PublicRunDraft draft)
    {
        if (_working || _loading || CurrentDetail is not { } detail
            || PublicRunEditor.PublishParams(Record.SubmissionId, draft, detail) is not { } request)
        {
            return;
        }

        _working = true;
        _state.PublicationStarted();
        _dismissedError = null;
        Render();
        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.PublishPublicRun, request)
            .ConfigureAwait(true);
        _working = false;
        if (!response.IsError && response.Result is JsonElement result && PublicRunJson.Page(result) is { } page)
        {
            _state.Published(page);
            _reviewDraft = null;
            _editingPublished = false;
        }
        else
        {
            // Refused: the record stays as it was, private if it was.
            _state.PublicationFailed(response.Error?.Message);
        }

        Render();
    }

    private async Task UnpublishAsync()
    {
        if (_working || _loading || CurrentDetail is null)
        {
            return;
        }

        _working = true;
        _state.PublicationStarted();
        _dismissedError = null;
        Render();
        DaemonResponse response = await _host
            .CallAsync(DaemonProtocol.Methods.UnpublishPublicRun, PublicRunEditor.SubmissionParams(Record.SubmissionId))
            .ConfigureAwait(true);
        _working = false;
        if (!response.IsError && response.Result is JsonElement result
            && PublicRunJson.Unpublished(result) is { } unpublished)
        {
            _state.Unpublished(unpublished.ExpectedPublicationVersion, unpublished.CredentialWarning);
            _editingPublished = false;
            _reviewDraft = null;
        }
        else
        {
            _state.PublicationFailed(response.Error?.Message);
        }

        Render();
    }

    // ---- Pieces -------------------------------------------------------------

    private static T Res<T>(string key) => (T)Application.Current.Resources[key];

    private static TextBlock Text(string text, string style, string? brush = null)
    {
        var block = new TextBlock { Text = text, Style = Res<Style>(style), TextWrapping = TextWrapping.Wrap };
        if (brush is not null)
        {
            block.Foreground = Res<Brush>(brush);
        }

        return block;
    }

    private static TextBlock Caption(string text, string brush) => Text(text, "TcCaptionTextStyle", brush);

    private static TextBlock Eyebrow(string text) => Text(text, "TcEyebrowTextStyle");

    private static TextBlock Selectable(string text, string style)
    {
        TextBlock block = Text(text, style);
        block.IsTextSelectionEnabled = true;
        return block;
    }

    /// <summary>A value, or its missing word in the secondary ink.</summary>
    private static TextBlock Value(string? text, string missing) =>
        text is null ? Text(missing, "TcBodyTextStyle", "TcInkSecondaryBrush") : Selectable(text, "TcBodyTextStyle");

    private static UIElement KeyValue(string key, string value, bool mono)
    {
        var grid = new Grid { ColumnSpacing = 12 };
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        grid.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        grid.Children.Add(Caption(key, "TcInkSecondaryBrush"));
        TextBlock v = Selectable(value, mono ? "TcMonoCodeTextStyle" : "TcCaptionTextStyle");
        Grid.SetColumn(v, 1);
        grid.Children.Add(v);
        return grid;
    }

    private static UIElement PreviewField(string label, string value)
    {
        var panel = new StackPanel { Spacing = 4 };
        panel.Children.Add(Eyebrow(label));
        panel.Children.Add(Selectable(value, "TcBodyTextStyle"));
        return panel;
    }

    private static Border Card() => new()
    {
        Background = Res<Brush>("TcSurfaceInsetBrush"),
        BorderBrush = Res<Brush>("TcHairlineBrush"),
        BorderThickness = new Thickness(1),
        CornerRadius = Res<CornerRadius>("TcCardCornerRadius"),
        Padding = new Thickness(14, 12, 14, 12),
    };

    private static Border Wrap(StackPanel panel)
    {
        Border card = Card();
        card.Child = panel;
        return card;
    }

    /// <summary>An eyebrowed card (<c>GlassEyebrowCard</c>), added to the page unless asked not to.</summary>
    private StackPanel EyebrowCard(string eyebrow, bool add = true)
    {
        var panel = new StackPanel { Spacing = 8 };
        TextBlock title = Eyebrow(eyebrow);
        AutomationProperties.SetHeadingLevel(title, AutomationHeadingLevel.Level3);
        panel.Children.Add(title);
        if (add)
        {
            _root.Children.Add(Wrap(panel));
        }

        return panel;
    }

    private static Border StatusTag(string word, StatusTone tone)
    {
        string brush = tone switch
        {
            StatusTone.On => "TcStatusOnTextBrush",
            StatusTone.Ask => "TcStatusAskTextBrush",
            _ => "TcInkSecondaryBrush",
        };
        return new Border
        {
            BorderBrush = Res<Brush>(brush),
            BorderThickness = new Thickness(1),
            CornerRadius = new CornerRadius(10),
            Padding = new Thickness(8, 2, 8, 2),
            HorizontalAlignment = HorizontalAlignment.Left,
            Child = Text(word, "TcCaptionSmallTextStyle", brush),
        };
    }

    private static Button ActionButton(string label, bool primary)
    {
        var button = new Button
        {
            Content = label,
            Style = Res<Style>(primary ? "TcPrimaryButtonStyle" : "TcSecondaryButtonStyle"),
            MinHeight = 44,
        };
        AutomationProperties.SetName(button, label);
        return button;
    }

    private UIElement Field(string label, string value, int limit, bool multiline, Action<string> set)
    {
        var panel = new StackPanel { Spacing = 4 };
        var header = new Grid();
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(Eyebrow(label));
        TextBlock counter = Text(PublicRunEditor.Counter(value, limit), "TcMonoChipTextStyle", "TcInkTertiaryBrush");
        AutomationProperties.SetAccessibilityView(counter, AccessibilityView.Raw);
        Grid.SetColumn(counter, 1);
        header.Children.Add(counter);
        panel.Children.Add(header);
        var box = new TextBox
        {
            Text = value,
            AcceptsReturn = multiline,
            TextWrapping = multiline ? TextWrapping.Wrap : TextWrapping.NoWrap,
            MinHeight = multiline ? (limit > PublicRunEditor.OutcomeLimit ? 128 : 88) : 0,
        };
        AutomationProperties.SetName(box, label);
        _counters[box] = (counter, limit);
        box.TextChanged += (_, _) =>
        {
            set(box.Text);
            if (Copy is { } copy && CurrentDetail is { } detail)
            {
                UpdateVerdict(copy, detail);
            }
        };
        panel.Children.Add(box);
        return panel;
    }
}
