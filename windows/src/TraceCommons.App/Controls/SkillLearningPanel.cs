using System;
using System.Collections.Generic;
using System.Threading.Tasks;
using Microsoft.UI.Dispatching;
using Microsoft.UI.Xaml;
using Microsoft.UI.Xaml.Automation;
using Microsoft.UI.Xaml.Automation.Peers;
using Microsoft.UI.Xaml.Controls;
using Microsoft.UI.Xaml.Controls.Primitives;
using Microsoft.UI.Xaml.Media;
using TraceCommons.Interop;
using TraceCommons.Interop.Shell;

namespace TraceCommons.App.Controls;

/// <summary>
/// The tested Agent Skill flow for one sent trace (W20-D2, taken (a)): learn
/// a skill from its correction, review the exact package, test it on NEAR
/// AI, review where it will be written, install it into the coding tool's
/// folder, and roll it back (<c>SkillLearningView</c>,
/// macos/Sources/TraceCommonsApp/Views/SkillLearningView.swift). Every word is
/// the core's (<see cref="SkillLearningCopy"/>, <see cref="PublicRunCopy"/>);
/// every step is the daemon's, through <see cref="SkillLearningModel"/>.
/// Built in code, as the inspector around it is.
/// </summary>
public sealed class SkillLearningPanel : UserControl
{
    private readonly SkillLearningModel _model;
    private readonly string _id;
    private readonly SkillLearningCopy _copy;
    private readonly StackPanel _stage = new() { Spacing = 16 };
    private readonly Grid _failure = new() { ColumnSpacing = 12 };
    private readonly DispatcherQueueTimer _validate;

    // The editor's draft (SkillLearningView.swift:15-24).
    private SkillDraft _draft = new(string.Empty, string.Empty, string.Empty);
    private string? _configuredCandidateId;
    private SkillDraft? _configuredDraft;
    private SkillDraftValidation? _validation;
    private bool _showsEvidence;
    private bool _showsRuns;
    private readonly HashSet<string> _openOutputs = new(StringComparer.Ordinal);
    private string? _dismissedFailure;
    private string? _lastFailure;
    private bool _lastWorking;
    private TextBlock? _applicabilityCount;
    private TextBlock? _procedureCount;
    private Button? _reviewButton;
    private StackPanel? _reviewProblem;

    /// <summary>The sent trace this panel is about.</summary>
    public string SubmissionId => _id;

    public SkillLearningPanel(SkillLearningModel model, string submissionId, SkillLearningCopy copy)
    {
        _model = model ?? throw new ArgumentNullException(nameof(model));
        _id = submissionId ?? throw new ArgumentNullException(nameof(submissionId));
        _copy = copy ?? throw new ArgumentNullException(nameof(copy));
        _validate = DispatcherQueue.GetForCurrentThread().CreateTimer();
        _validate.IsRepeating = false;
        _validate.Interval = TimeSpan.FromMilliseconds(150);
        _validate.Tick += async (_, _) => await ValidateAsync();

        var panel = new StackPanel { Spacing = 8 };
        TextBlock heading = Text(copy.Heading, "TcEyebrowTextStyle", null);
        AutomationProperties.SetHeadingLevel(heading, AutomationHeadingLevel.Level3);
        panel.Children.Add(heading);
        var body = new StackPanel { Spacing = 16 };
        body.Children.Add(Caption(copy.Promise));
        body.Children.Add(_stage);
        body.Children.Add(_failure);
        panel.Children.Add(body);
        Content = Card(panel);
        _failure.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        _failure.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });

        Loaded += OnLoaded;
    }

    private async void OnLoaded(object sender, RoutedEventArgs e)
    {
        Refresh();
        await _model.EnsureLocalInstalledSkillStatusAsync(_id);
    }

    /// <summary>
    /// The session's state changed: take a new candidate's draft and redraw.
    /// The inspector that holds this panel calls it; the panel subscribes to
    /// nothing, so a detach and re-add never leaves it deaf.
    /// </summary>
    public void Refresh()
    {
        ConfigureDraft();
        Render();
    }

    /// <summary>A new candidate, or its draft changed: the editor takes it (<c>configureDraft</c>, :256-265).</summary>
    private void ConfigureDraft()
    {
        if (_model.State(_id).Phase.Candidate is not { } candidate
            || (_configuredCandidateId == candidate.CandidateId && _configuredDraft == candidate.Draft))
        {
            return;
        }

        _configuredCandidateId = candidate.CandidateId;
        _configuredDraft = candidate.Draft;
        _draft = candidate.Draft;
        _validation = null;
        _validate.Start();
    }

    private void Render()
    {
        SkillLearningSessionState state = _model.State(_id);

        // A failure that arrives clears the dismissal, and so does a new
        // attempt (:56-58).
        if (state.Failure != _lastFailure || (state.IsWorking && !_lastWorking))
        {
            _dismissedFailure = null;
        }

        _lastFailure = state.Failure;
        _lastWorking = state.IsWorking;

        _stage.Children.Clear();
        _applicabilityCount = _procedureCount = null;
        _reviewButton = null;
        _reviewProblem = null;
        Stage(state);

        _failure.Children.Clear();
        if (state.Failure is { } message && message != _dismissedFailure)
        {
            TextBlock line = Text(message, "TcBodyTextStyle", "TcStatusOutsideTextBrush");
            AutomationProperties.SetLiveSetting(line, AutomationLiveSetting.Assertive);
            _failure.Children.Add(line);
            var dismiss = new HyperlinkButton { Content = MonitorWords.Dismiss, VerticalAlignment = VerticalAlignment.Top };
            Grid.SetColumn(dismiss, 1);
            dismiss.Click += (_, _) =>
            {
                _dismissedFailure = message;
                Render();
            };
            _failure.Children.Add(dismiss);
        }
    }

    // ---- Stages (:63-110) ----------------------------------------------------

    private void Stage(SkillLearningSessionState state)
    {
        bool working = state.IsWorking;
        if (state.SkillLearningInstall is { } installed)
        {
            InstalledPanel(installed, working);
            return;
        }

        SkillLearningPhase phase = state.Phase;
        switch (phase.Kind)
        {
            case SkillLearningPhaseKind.Idle:
                _stage.Children.Add(Caption(_copy.SupportedFamily));
                _stage.Children.Add(Action(working ? _copy.Learning : _copy.LearnAction, primary: true, !working,
                    () => _model.LearnSkillAsync(_id)));
                break;
            case SkillLearningPhaseKind.Candidate when phase.Candidate is { } candidate:
                CandidateEditor(candidate, working);
                break;
            case SkillLearningPhaseKind.Reviewed when phase.Candidate is { } candidate && phase.Review is { } review:
                ReviewPreview(candidate, review, working);
                break;
            case SkillLearningPhaseKind.Evaluated when phase.Report is { } report:
                EvaluationResults(report, working);
                break;
            case SkillLearningPhaseKind.Planned when phase.Plan is { } plan:
                InstallPreview(plan, working);
                break;
        }
    }

    private void CandidateEditor(SkillCandidate candidate, bool working)
    {
        var title = new Grid();
        title.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        title.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        title.Children.Add(Strong(_copy.CandidateHeading));
        Border tag = Pill(_copy.GeneratedSource, "TcInkSecondaryBrush");
        Grid.SetColumn(tag, 1);
        title.Children.Add(tag);
        _stage.Children.Add(title);

        var name = new TextBox { Header = _copy.Name, Text = _draft.Name, MinHeight = 44 };
        AutomationProperties.SetName(name, _copy.Name);
        name.TextChanged += (_, _) => DraftChanged(_draft with { Name = name.Text });
        _stage.Children.Add(name);

        (UIElement applicability, _applicabilityCount) = Area(_copy.Applicability, _draft.Description, 96,
            text => DraftChanged(_draft with { Description = text }));
        _stage.Children.Add(applicability);
        (UIElement procedure, _procedureCount) = Area(_copy.Procedure, _draft.Procedure, 240,
            text => DraftChanged(_draft with { Procedure = text }));
        _stage.Children.Add(procedure);

        _stage.Children.Add(Expander(_copy.SourceEvidence, _showsEvidence, open =>
        {
            _showsEvidence = open;
            Render();
        }));
        if (_showsEvidence)
        {
            var evidence = new StackPanel { Spacing = 8 };
            evidence.Children.Add(Selectable(candidate.SourceCorrection, "TcBodyTextStyle", "TcInkPrimaryBrush"));
            foreach (SkillSourceEvidence item in candidate.SourceEvidence)
            {
                var row = new StackPanel { Spacing = 2 };
                row.Children.Add(FieldLabel(PublicRunCopy.Current?.EvidenceKindLabel(item.Kind) ?? string.Empty));
                row.Children.Add(Selectable(item.Excerpt, "TcCaptionTextStyle", "TcInkSecondaryBrush"));
                evidence.Children.Add(row);
            }

            _stage.Children.Add(evidence);
        }

        _stage.Children.Add(EvaluationContract(candidate));

        var review = new StackPanel { Spacing = 8 };
        _reviewButton = Action(working ? _copy.Reviewing : _copy.ReviewAction, primary: true, enabled: false,
            () => _model.ReviewSkillAsync(_id, _draft));
        review.Children.Add(_reviewButton);
        _reviewProblem = new StackPanel();
        review.Children.Add(_reviewProblem);
        _stage.Children.Add(review);
        UpdateValidation(working);
    }

    /// <summary>The contract the test runs under (<c>evaluationContract</c>, :206-243).</summary>
    private UIElement EvaluationContract(SkillCandidate candidate)
    {
        SkillEvaluationContract contract = candidate.EvaluationContract;
        var panel = new StackPanel { Spacing = 8 };
        panel.Children.Add(FieldLabel(_copy.TestContract));
        panel.Children.Add(Caption(SkillLearningCopy.Format(_copy.ContractSummaryFormat, contract.TaskCount,
            contract.ClusterCount, contract.TotalRequests, contract.OutputTokenLimit, contract.RequestTimeoutSeconds)));
        panel.Children.Add(Caption(contract.FixtureScope));
        var manual = new StackPanel { Spacing = 2 };
        manual.Children.Add(FieldLabel(_copy.ManualInstruction));
        manual.Children.Add(Selectable(candidate.ManualControlInstruction, "TcCaptionTextStyle", "TcInkPrimaryBrush"));
        panel.Children.Add(manual);
        return Well(panel);
    }

    private void ReviewPreview(SkillCandidate candidate, SkillReview review, bool working)
    {
        _stage.Children.Add(Strong(_copy.ExactPackage));
        _stage.Children.Add(CodeBlock(review.SkillMd));
        // The digest wraps beside its label rather than run past the pane.
        var digest = new Grid { ColumnSpacing = 12 };
        digest.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        digest.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        digest.Children.Add(FieldLabel(_copy.Digest));
        TextBlock sha = Selectable(review.SkillSha256, "TcMonoCodeTextStyle", "TcInkPrimaryBrush");
        sha.TextWrapping = TextWrapping.Wrap;
        Grid.SetColumn(sha, 1);
        digest.Children.Add(sha);
        _stage.Children.Add(digest);
        _stage.Children.Add(Caption(_copy.EvaluationDisclosure));
        SkillEvaluationContract contract = candidate.EvaluationContract;
        _stage.Children.Add(Text(SkillLearningCopy.Format(_copy.ReviewBudgetFormat, contract.TotalRequests,
            contract.OutputTokenLimit, contract.RequiredModelOwner), "TcMonoCodeTextStyle", "TcInkSecondaryBrush"));

        // The action first, Edit skill after it as a link (:308-320).
        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        actions.Children.Add(Action(working ? _copy.Testing : _copy.ApproveAndTest, primary: true, !working,
            () => _model.TestSkillAsync(_id)));
        actions.Children.Add(Link(_copy.EditSkill, !working, () => _model.EditSkill(_id)));
        _stage.Children.Add(actions);
    }

    private void EvaluationResults(SkillEvaluationReport report, bool working)
    {
        var title = new Grid();
        title.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        title.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        title.Children.Add(Strong(_copy.Results));
        Border gate = report.InstallAllowed
            ? Pill(_copy.PassedGate, "TcStatusOnTextBrush")
            : Pill(_copy.FailedGate, "TcStatusOutsideTextBrush");
        Grid.SetColumn(gate, 1);
        title.Children.Add(gate);
        _stage.Children.Add(title);

        _stage.Children.Add(SummaryGroup(_copy.RepositoryPlans, report.PlanSummaries));
        _stage.Children.Add(SummaryGroup(_copy.SkillApplicability, report.ApplicabilitySummaries));

        var model = new StackPanel { Spacing = 2 };
        model.Children.Add(FieldLabel(_copy.ModelAndBudget));
        model.Children.Add(Selectable(report.ServedModel, "TcMonoCodeTextStyle", "TcInkPrimaryBrush"));
        model.Children.Add(Caption(SkillLearningCopy.Format(_copy.ModelBudgetFormat, report.ModelOwnedBy,
            report.TaskCount, report.OutputTokenLimit)));
        _stage.Children.Add(model);

        var regressions = new StackPanel { Spacing = 4 };
        regressions.Children.Add(FieldLabel(_copy.Regressions));
        if (report.Regressions.Count == 0)
        {
            regressions.Children.Add(Caption(_copy.NoRegressions));
        }
        else
        {
            foreach (string task in report.Regressions)
            {
                regressions.Children.Add(Text(task, "TcMonoCodeTextStyle", "TcStatusOutsideTextBrush"));
            }
        }

        _stage.Children.Add(regressions);

        _stage.Children.Add(Expander(_copy.InspectRuns, _showsRuns, open =>
        {
            _showsRuns = open;
            Render();
        }));
        if (_showsRuns)
        {
            var runs = new StackPanel { Spacing = 16 };
            foreach (SkillTrialResult trial in report.Trials)
            {
                runs.Children.Add(TrialRow(trial));
            }

            _stage.Children.Add(runs);
        }

        var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
        if (report.InstallAllowed)
        {
            actions.Children.Add(Action(working ? _copy.Preparing : _copy.ReviewInstall, primary: true, !working,
                () => _model.ReviewSkillInstallAsync(_id)));
        }

        actions.Children.Add(Link(_copy.EditSkill, !working, () => _model.EditSkill(_id)));
        _stage.Children.Add(actions);
    }

    private UIElement SummaryGroup(string label, IReadOnlyList<SkillArmSummary> summaries)
    {
        var panel = new StackPanel { Spacing = 4 };
        panel.Children.Add(FieldLabel(label));
        foreach (SkillArmSummary summary in summaries)
        {
            var row = new Grid();
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
            row.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
            row.Children.Add(Text(summary.Label, "TcBodyTextStyle", "TcInkPrimaryBrush"));
            TextBlock figure = Text($"{summary.Passed}/{summary.Total}", "TcMonoCodeTextStyle", "TcInkPrimaryBrush");
            Grid.SetColumn(figure, 1);
            row.Children.Add(figure);
            panel.Children.Add(row);
        }

        return Well(panel);
    }

    /// <summary>One trial (<c>SkillTrialRow</c>, :424-497).</summary>
    private UIElement TrialRow(SkillTrialResult trial)
    {
        var panel = new StackPanel { Spacing = 4 };
        var title = new Grid();
        title.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        title.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        title.Children.Add(Text(trial.TaskId, "TcTitleCardTextStyle", "TcInkPrimaryBrush"));
        string arm = trial.Arm switch
        {
            SkillEvaluationArm.Baseline => _copy.Baseline,
            SkillEvaluationArm.ManualInstruction => _copy.SimpleInstruction,
            _ => _copy.CandidateSkill,
        };
        Border tag = Pill(arm + " · " + (trial.Passed ? _copy.Passed : _copy.Failed),
            trial.Passed ? "TcStatusOnTextBrush" : "TcStatusOutsideTextBrush");
        Grid.SetColumn(tag, 1);
        title.Children.Add(tag);
        panel.Children.Add(title);
        panel.Children.Add(Caption(trial.Task));
        var source = new HyperlinkButton { Content = _copy.OpenFixtureSource, NavigateUri = trial.SourceUrl, MinHeight = 44 };
        AutomationProperties.SetName(source, _copy.OpenFixtureSource + ": " + trial.TaskId);
        panel.Children.Add(source);
        foreach (string failure in trial.FailureReasons)
        {
            panel.Children.Add(Text(failure, "TcCaptionTextStyle", "TcStatusOutsideTextBrush"));
        }

        if (trial.Answer is { } answer)
        {
            panel.Children.Add(Text(answer.Diagnosis, "TcCaptionTextStyle", "TcInkPrimaryBrush"));
            if (answer.EditPaths.Count > 0)
            {
                panel.Children.Add(Selectable(_copy.Edits + ": " + string.Join(", ", answer.EditPaths), "TcMonoCodeTextStyle", "TcInkSecondaryBrush"));
            }

            if (answer.Commands.Count > 0)
            {
                panel.Children.Add(Selectable(_copy.Commands + ": " + string.Join("\n", answer.Commands), "TcMonoCodeTextStyle", "TcInkSecondaryBrush"));
            }

            if (answer.Verification.Count > 0)
            {
                panel.Children.Add(Selectable(_copy.Checks + ": " + string.Join("\n", answer.Verification), "TcMonoCodeTextStyle", "TcInkSecondaryBrush"));
            }
        }

        bool open = _openOutputs.Contains(trial.Id);
        panel.Children.Add(Expander(_copy.ModelOutput, open, show =>
        {
            if (show)
            {
                _openOutputs.Add(trial.Id);
            }
            else
            {
                _openOutputs.Remove(trial.Id);
            }

            Render();
        }));
        if (open)
        {
            panel.Children.Add(CodeBlock(trial.RawOutput));
        }

        return panel;
    }

    /// <summary>Where the skill will be written, exactly (<c>SkillInstallPreview</c>, :500-572).</summary>
    private void InstallPreview(SkillInstallPlan plan, bool working)
    {
        _stage.Children.Add(Strong(_copy.InstallPreview));
        _stage.Children.Add(Caption(_copy.InstallDisclosure));
        _stage.Children.Add(FieldLabel(_copy.PersistentWrites));
        _stage.Children.Add(Labeled(_copy.TargetPath, plan.TargetLocation));
        _stage.Children.Add(Labeled(_copy.CodingTool, plan.Tool));
        _stage.Children.Add(Labeled(_copy.SkillFile, plan.SkillLocation));
        _stage.Children.Add(Labeled(_copy.Digest, plan.SkillSha256));
        _stage.Children.Add(FieldLabel(_copy.ExactPackage));
        _stage.Children.Add(CodeBlock(plan.SkillMd));
        _stage.Children.Add(Labeled(_copy.OwnershipMarker, plan.MarkerLocation));
        _stage.Children.Add(Labeled(_copy.MarkerDigest, plan.MarkerFileSha256));
        _stage.Children.Add(FieldLabel(_copy.ExactMarker));
        _stage.Children.Add(CodeBlock(plan.MarkerJson));
        if (!plan.CanInstall)
        {
            // Retry first, Edit skill after it as a link, then why (:542-562).
            var refused = new StackPanel { Spacing = 8 };
            var actions = new StackPanel { Orientation = Orientation.Horizontal, Spacing = 12 };
            actions.Children.Add(Action(working ? _copy.Preparing : _copy.RetryInstall, primary: true, !working,
                () => _model.ReviewSkillInstallAsync(_id)));
            actions.Children.Add(Link(_copy.EditSkill, !working, () => _model.EditSkill(_id)));
            refused.Children.Add(actions);
            string why = plan.Occupied
                ? SkillLearningNative.ErrorLine("skill-install-occupied") ?? _copy.Unavailable
                : _copy.Unavailable;
            refused.Children.Add(Text(why, "TcBodyTextStyle", "TcStatusOutsideTextBrush"));
            _stage.Children.Add(refused);
        }
        else
        {
            _stage.Children.Add(Action(working ? _copy.Installing : _copy.InstallAction, primary: true, !working,
                () => _model.InstallSkillAsync(_id)));
        }
    }

    /// <summary>The installed skill and its rollback, which asks first (<c>InstalledSkillPanel</c>, :575-646).</summary>
    private void InstalledPanel(SkillLearningInstall installed, bool working)
    {
        _stage.Children.Add(Pill(_copy.Installed, "TcStatusOnTextBrush"));
        _stage.Children.Add(Text(installed.Name, "TcTitleSectionTextStyle", "TcInkPrimaryBrush"));
        _stage.Children.Add(Labeled(_copy.TargetPath, installed.TargetLocation));
        _stage.Children.Add(Labeled(_copy.Digest, installed.SkillSha256));
        _stage.Children.Add(Labeled(_copy.MarkerDigest, installed.MarkerSha256));
        _stage.Children.Add(Caption(_copy.RollbackDisclosure));
        _stage.Children.Add(Action(working ? _copy.RollingBack : _copy.Rollback, primary: false, !working, ConfirmRollbackAsync));
    }

    private async Task ConfirmRollbackAsync()
    {
        if (XamlRoot is null)
        {
            return;
        }

        var dialog = new ContentDialog
        {
            XamlRoot = XamlRoot,
            Title = _copy.RollbackConfirmTitle,
            Content = new TextBlock { Text = _copy.RollbackConfirmBody, TextWrapping = TextWrapping.Wrap },
            CloseButtonText = _copy.RollbackKeep,
            PrimaryButtonText = _copy.Rollback,
            DefaultButton = ContentDialogButton.Close,
        };

        // Only an explicit Roll back removes what the coding tool reads; a
        // dialog that could not be shown is Keep.
        if (await DialogGuard.ShowOnceAsync(dialog) == ContentDialogResult.Primary)
        {
            await _model.RollbackSkillAsync(_id);
        }
    }

    // ---- The draft's validation (:178-198) ----------------------------------

    private void DraftChanged(SkillDraft draft)
    {
        _draft = draft;
        _validate.Stop();
        _validate.Start();
    }

    private async Task ValidateAsync()
    {
        SkillDraft draft = _draft;
        SkillDraftValidation? validated = await Task.Run(() => SkillLearningNative.ValidateDraft(draft));
        if (draft != _draft)
        {
            return;
        }

        _validation = validated;
        UpdateValidation(_model.State(_id).IsWorking);
    }

    private void UpdateValidation(bool working)
    {
        SkillDraftValidation? validation = _validation;
        if (_applicabilityCount is not null)
        {
            _applicabilityCount.Text = Count(validation?.DescriptionChars, validation?.DescriptionMaxChars);
        }

        if (_procedureCount is not null)
        {
            _procedureCount.Text = Count(validation?.ProcedureChars, validation?.ProcedureMaxChars);
        }

        if (_reviewButton is not null)
        {
            _reviewButton.IsEnabled = !working && validation?.Valid == true;
        }

        if (_reviewProblem is not null)
        {
            _reviewProblem.Children.Clear();
            if (validation?.Error is { } error && SkillLearningNative.ErrorLine(error) is { } message)
            {
                _reviewProblem.Children.Add(Text(message, "TcBodyTextStyle", "TcStatusOutsideTextBrush"));
            }
        }
    }

    private static string Count(int? count, int? maximum) =>
        count is int c && maximum is int m ? $"{c}/{m}" : "—";

    // ---- Pieces ------------------------------------------------------------

    private static T Res<T>(string key) => (T)Application.Current.Resources[key];

    private static TextBlock Text(string text, string style, string? brush)
    {
        var block = new TextBlock { Text = text, Style = Res<Style>(style), TextWrapping = TextWrapping.Wrap };
        if (brush is not null)
        {
            block.Foreground = Res<Brush>(brush);
        }

        return block;
    }

    private static TextBlock Caption(string text) => Text(text, "TcCaptionTextStyle", "TcInkSecondaryBrush");

    private static TextBlock Strong(string text) => Text(text, "TcTitleCardTextStyle", "TcInkPrimaryBrush");

    /// <summary>A field's eyebrow label (<c>skillFieldLabel</c>, :651-655).</summary>
    private static TextBlock FieldLabel(string text) => Text(text, "TcEyebrowTextStyle", "TcInkTertiaryBrush");

    private static TextBlock Selectable(string text, string style, string brush)
    {
        TextBlock block = Text(text, style, brush);
        block.IsTextSelectionEnabled = true;
        return block;
    }

    private static UIElement Labeled(string label, string value)
    {
        var panel = new StackPanel { Spacing = 2 };
        panel.Children.Add(FieldLabel(label));
        panel.Children.Add(Selectable(value, "TcMonoCodeTextStyle", "TcInkPrimaryBrush"));
        return panel;
    }

    /// <summary>A package, marker or model output shown verbatim in an inset well (<c>skillCodeBlock</c>, :711-721).</summary>
    private static UIElement CodeBlock(string text) => Well(Selectable(text, "TcMonoCodeTextStyle", "TcInkPrimaryBrush"));

    private static Border Well(UIElement child) => new()
    {
        Background = Res<Brush>("TcSurfaceInsetBrush"),
        BorderBrush = Res<Brush>("TcHairlineBrush"),
        BorderThickness = new Thickness(1),
        CornerRadius = Res<CornerRadius>("TcCardCornerRadius"),
        Padding = new Thickness(12),
        Child = child,
    };

    private static Border Card(UIElement child) => new()
    {
        Background = Res<Brush>("TcSurfaceInsetBrush"),
        BorderBrush = Res<Brush>("TcHairlineBrush"),
        BorderThickness = new Thickness(1),
        CornerRadius = Res<CornerRadius>("TcCardCornerRadius"),
        Padding = new Thickness(14, 12, 14, 12),
        Child = child,
    };

    private static Border Pill(string word, string brush) => new()
    {
        BorderBrush = Res<Brush>(brush),
        BorderThickness = new Thickness(1),
        CornerRadius = new CornerRadius(GlassTokens.Radius.Tile),
        Padding = new Thickness(8, 2, 8, 2),
        HorizontalAlignment = HorizontalAlignment.Left,
        VerticalAlignment = VerticalAlignment.Center,
        Child = Text(word, "TcCaptionSmallTextStyle", brush),
    };

    private static (UIElement Field, TextBlock Count) Area(string label, string text, double height, Action<string> changed)
    {
        var panel = new StackPanel { Spacing = 4 };
        var header = new Grid();
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = new GridLength(1, GridUnitType.Star) });
        header.ColumnDefinitions.Add(new ColumnDefinition { Width = GridLength.Auto });
        header.Children.Add(FieldLabel(label));
        TextBlock count = Text("—", "TcMonoChipTextStyle", "TcInkTertiaryBrush");
        AutomationProperties.SetAccessibilityView(count, AccessibilityView.Raw);
        Grid.SetColumn(count, 1);
        header.Children.Add(count);
        panel.Children.Add(header);
        // AcceptsReturn before Text: a single-line box keeps only the first
        // line of what it is given.
        var box = new TextBox { AcceptsReturn = true, TextWrapping = TextWrapping.Wrap, MinHeight = height };
        box.Text = text;
        AutomationProperties.SetName(box, label);
        // The box breaks lines with \r; the draft is sent with \n, as the Mac's is.
        box.TextChanged += (_, _) => changed(box.Text.Replace("\r\n", "\n", StringComparison.Ordinal).Replace('\r', '\n'));
        panel.Children.Add(box);
        return (panel, count);
    }

    private static Button Action(string label, bool primary, bool enabled, Func<Task> act)
    {
        var button = new Button
        {
            Content = label,
            Style = Res<Style>(primary ? "TcPrimaryButtonStyle" : "TcSecondaryButtonStyle"),
            MinHeight = 44,
            IsEnabled = enabled,
        };
        AutomationProperties.SetName(button, label);
        button.Click += async (_, _) => await act();
        return button;
    }

    private static HyperlinkButton Link(string label, bool enabled, Action act)
    {
        var link = new HyperlinkButton { Content = label, MinHeight = 44, IsEnabled = enabled };
        AutomationProperties.SetName(link, label);
        link.Click += (_, _) => act();
        return link;
    }

    private static ToggleButton Expander(string label, bool open, Action<bool> toggled)
    {
        var toggle = new ToggleButton
        {
            Content = label,
            IsChecked = open,
            MinHeight = 44,
        };
        AutomationProperties.SetName(toggle, label);
        toggle.Click += (_, _) => toggled(toggle.IsChecked == true);
        return toggle;
    }
}
