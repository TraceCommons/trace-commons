using System;
using System.Collections.Generic;
using System.ComponentModel;
using System.Text.Json;
using System.Text.Json.Nodes;
using System.Threading.Tasks;

namespace TraceCommons.Interop;

/// <summary>The operation a session is waiting on (<c>SkillLearningOperation</c>, SkillLearningStore.swift:6-13).</summary>
public enum SkillLearningOperation
{
    Learning,
    Reviewing,
    Evaluating,
    PlanningInstall,
    Installing,
    RollingBack,
}

/// <summary>Which case a <see cref="SkillInstallationState"/> is.</summary>
public enum SkillInstallationKind
{
    Unknown,
    Loading,
    Absent,
    Installed,
    Failed,
}

/// <summary>
/// What the app has established about the source session's local
/// installation (<c>SkillInstallationState</c>, SkillLearningStore.swift:15-59).
/// An unchecked path and a verified absence stay distinct, so status
/// loading is idempotent. Built only through its factories, so a case never
/// carries a field it does not have.
/// </summary>
public sealed record SkillInstallationState
{
    private readonly SkillLearningInstall? _skill;

    private SkillInstallationState(SkillInstallationKind kind, SkillLearningInstall? skill, string? message)
    {
        Kind = kind;
        _skill = skill;
        Message = message;
    }

    public SkillInstallationKind Kind { get; }

    /// <summary>The failure's sentence; only on <see cref="SkillInstallationKind.Failed"/>.</summary>
    public string? Message { get; }

    /// <summary><c>.unknown</c>: nothing has been read.</summary>
    public static SkillInstallationState Unknown { get; } = new(SkillInstallationKind.Unknown, null, null);

    /// <summary><c>.absent</c>: the daemon said nothing is installed.</summary>
    public static SkillInstallationState Absent { get; } = new(SkillInstallationKind.Absent, null, null);

    /// <summary><c>.loading(previous:)</c>.</summary>
    public static SkillInstallationState Loading(SkillLearningInstall? previous) => new(SkillInstallationKind.Loading, previous, null);

    /// <summary><c>.installed(_)</c>.</summary>
    public static SkillInstallationState Installed(SkillLearningInstall installed) => new(SkillInstallationKind.Installed, installed, null);

    /// <summary><c>.failed(message:previous:)</c>.</summary>
    public static SkillInstallationState Failed(string message, SkillLearningInstall? previous) =>
        new(SkillInstallationKind.Failed, previous, message);

    /// <summary>The installed skill, or the last one known while loading or after a failure (:25-34).</summary>
    public SkillLearningInstall? SkillLearningInstall => Kind is SkillInstallationKind.Unknown or SkillInstallationKind.Absent ? null : _skill;

    /// <summary>The failure, if this is one (:36-39).</summary>
    public string? Failure => Kind == SkillInstallationKind.Failed ? Message : null;

    /// <summary>(:41-44).</summary>
    public bool IsLoading => Kind == SkillInstallationKind.Loading;

    /// <summary>Unknown or failed: the status should be read (:46-53).</summary>
    public bool NeedsResolution => Kind is SkillInstallationKind.Unknown or SkillInstallationKind.Failed;

    /// <summary>(:55-58).</summary>
    public bool IsAbsent => Kind == SkillInstallationKind.Absent;
}

/// <summary>Which daemon refusal a label is (<c>SkillLearningDaemonFailure</c>, SkillLearningStore.swift:61-86).</summary>
public enum SkillLearningDaemonFailureKind
{
    CandidateUnknown,
    ReviewUnknown,
    EvaluationUnknown,
    InstallUnknown,
    Other,
}

/// <summary>A daemon refusal, typed (<c>SkillLearningDaemonFailure</c>, SkillLearningStore.swift:61-86).</summary>
public sealed record SkillLearningDaemonFailure(SkillLearningDaemonFailureKind Kind, string Label)
{
    /// <summary>The label, typed (<c>init(label:)</c>, :68-76).</summary>
    public static SkillLearningDaemonFailure FromLabel(string label) => new(
        label switch
        {
            "skill-candidate-unknown" => SkillLearningDaemonFailureKind.CandidateUnknown,
            "skill-review-unknown" => SkillLearningDaemonFailureKind.ReviewUnknown,
            "skill-evaluation-unknown" => SkillLearningDaemonFailureKind.EvaluationUnknown,
            "skill-install-unknown" => SkillLearningDaemonFailureKind.InstallUnknown,
            _ => SkillLearningDaemonFailureKind.Other,
        },
        label);

    /// <summary>
    /// The daemon no longer knows an opaque id the workflow holds, so the
    /// workflow restarts from idle with its draft kept (:78-85).
    /// </summary>
    public bool InvalidatesWorkflow => Kind is SkillLearningDaemonFailureKind.CandidateUnknown
        or SkillLearningDaemonFailureKind.ReviewUnknown
        or SkillLearningDaemonFailureKind.EvaluationUnknown;
}

/// <summary>What a review is sent with (<c>SkillReviewStart</c>, SkillLearningStore.swift:88-91).</summary>
public sealed record SkillReviewStart(SkillCandidate Candidate, string? ReplacesReviewId);

/// <summary>Which case a <see cref="SkillLearningPhase"/> is.</summary>
public enum SkillLearningPhaseKind
{
    Idle,
    Candidate,
    Reviewed,
    Evaluated,
    Planned,
}

/// <summary>
/// How far the workflow has come (<c>SkillLearningPhase</c>,
/// SkillLearningStore.swift:93-125). Each case carries everything before it;
/// built only through its factories, so a case never lacks what it carries.
/// </summary>
public sealed record SkillLearningPhase
{
    private SkillLearningPhase(
        SkillLearningPhaseKind kind,
        SkillCandidate? candidate,
        SkillReview? review,
        SkillEvaluationReport? report,
        SkillInstallPlan? plan)
    {
        Kind = kind;
        Candidate = candidate;
        Review = review;
        Report = report;
        Plan = plan;
    }

    public SkillLearningPhaseKind Kind { get; }

    /// <summary>The candidate, in every case but idle (<c>candidate</c>, :109-119).</summary>
    public SkillCandidate? Candidate { get; }

    /// <summary>The review: reviewed, evaluated and planned.</summary>
    public SkillReview? Review { get; }

    /// <summary>The evaluation: evaluated and planned.</summary>
    public SkillEvaluationReport? Report { get; }

    /// <summary>The installation plan: planned only.</summary>
    public SkillInstallPlan? Plan { get; }

    /// <summary><c>.idle</c>.</summary>
    public static SkillLearningPhase Idle { get; } = new(SkillLearningPhaseKind.Idle, null, null, null, null);

    /// <summary><c>.candidate(_)</c>.</summary>
    public static SkillLearningPhase OfCandidate(SkillCandidate candidate) =>
        new(SkillLearningPhaseKind.Candidate, candidate, null, null, null);

    /// <summary><c>.reviewed(candidate:review:)</c>.</summary>
    public static SkillLearningPhase Reviewed(SkillCandidate candidate, SkillReview review) =>
        new(SkillLearningPhaseKind.Reviewed, candidate, review, null, null);

    /// <summary><c>.evaluated(candidate:review:report:)</c>.</summary>
    public static SkillLearningPhase Evaluated(SkillCandidate candidate, SkillReview review, SkillEvaluationReport report) =>
        new(SkillLearningPhaseKind.Evaluated, candidate, review, report, null);

    /// <summary><c>.planned(candidate:review:report:plan:)</c>.</summary>
    public static SkillLearningPhase Planned(
        SkillCandidate candidate, SkillReview review, SkillEvaluationReport report, SkillInstallPlan plan) =>
        new(SkillLearningPhaseKind.Planned, candidate, review, report, plan);

    /// <summary>(:121-124).</summary>
    public bool IsReviewedAwaitingEvaluation => Kind == SkillLearningPhaseKind.Reviewed;
}

/// <summary>
/// One session's tested-skill state (<c>SkillLearningSessionState</c>,
/// SkillLearningStore.swift:127-138). Immutable: the store replaces it, so a
/// state read earlier is a snapshot, as the Mac's value type is.
/// </summary>
public sealed record SkillLearningSessionState
{
    public SkillLearningPhase Phase { get; init; } = SkillLearningPhase.Idle;

    public SkillInstallationState Installation { get; init; } = SkillInstallationState.Unknown;

    public SkillLearningOperation? Operation { get; init; }

    public string? WorkflowFailure { get; init; }

    public SkillDraft? PreservedDraft { get; init; }

    public string? ReplacesReviewId { get; init; }

    /// <summary>(:135).</summary>
    public SkillLearningInstall? SkillLearningInstall => Installation.SkillLearningInstall;

    /// <summary>The workflow's failure, else the installation's (:136).</summary>
    public string? Failure => WorkflowFailure ?? Installation.Failure;

    /// <summary>An operation or a status read is in flight (:137).</summary>
    public bool IsWorking => Operation is not null || Installation.IsLoading;
}

/// <summary>
/// Every session's tested-skill state, and the only legal transitions
/// between them (<c>SkillLearningStore</c>, SkillLearningStore.swift:140-502).
/// Each <c>Begin</c> refuses unless its operation may start now; each
/// <c>Finish</c> refuses an answer that does not belong to the state it
/// would settle. At most 24 settled sessions are kept.
/// </summary>
public sealed class SkillLearningStore
{
    private const int MaximumRetainedSessions = 24;
    private readonly Dictionary<string, SkillLearningSessionState> _sessions = new(StringComparer.Ordinal);
    private readonly List<string> _recency = new();
    private static readonly SkillLearningSessionState Fresh = new();

    /// <summary>Raised with the session's id after every change to its state.</summary>
    public event Action<string>? Changed;

    /// <summary>(:146).</summary>
    public int RetainedSessionCount => _sessions.Count;

    /// <summary>The session's state; a session never seen is fresh (:148-150).</summary>
    public SkillLearningSessionState State(string submissionId) =>
        _sessions.TryGetValue(submissionId, out SkillLearningSessionState? state) ? state : Fresh;

    /// <summary>
    /// Every session goes, each announced through <see cref="Changed"/>, as
    /// the Mac's replacement with a fresh store (AppModel.swift:4260).
    /// </summary>
    public void Clear()
    {
        var removed = new List<string>(_sessions.Keys);
        _sessions.Clear();
        _recency.Clear();
        foreach (string id in removed)
        {
            Changed?.Invoke(id);
        }
    }

    /// <summary>(:152-158).</summary>
    public bool BeginLearning(string submissionId)
    {
        SkillLearningSessionState current = State(submissionId);
        if (!current.Installation.IsAbsent || current.Phase.Kind != SkillLearningPhaseKind.Idle)
        {
            return false;
        }

        return Begin(SkillLearningOperation.Learning, submissionId);
    }

    /// <summary>(:160-181).</summary>
    public bool FinishLearning(string submissionId, SkillCandidate candidate)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation != SkillLearningOperation.Learning
            || !current.Installation.IsAbsent
            || candidate.SourceSubmissionId != submissionId)
        {
            return false;
        }

        SkillCandidate loaded = current.PreservedDraft is { } draft ? candidate.ReplacingDraft(draft) : candidate;
        Store(current with
        {
            Phase = SkillLearningPhase.OfCandidate(loaded),
            Operation = null,
            WorkflowFailure = null,
            PreservedDraft = null,
            ReplacesReviewId = candidate.ReplacesReviewId,
        }, submissionId);
        return true;
    }

    /// <summary>(:183-190).</summary>
    public bool BeginInstallStatusLoading(string submissionId)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation is not null || !current.Installation.NeedsResolution)
        {
            return false;
        }

        Store(current with
        {
            Installation = SkillInstallationState.Loading(current.SkillLearningInstall),
            WorkflowFailure = null,
        }, submissionId);
        return true;
    }

    /// <summary>(:192-205).</summary>
    public bool FinishInstallStatusLoading(string submissionId, SkillLearningInstall? installed)
    {
        SkillLearningSessionState current = State(submissionId);
        if (!current.Installation.IsLoading
            || (installed is not null && installed.SourceSubmissionId != submissionId))
        {
            return false;
        }

        Store(current with
        {
            Installation = installed is null ? SkillInstallationState.Absent : SkillInstallationState.Installed(installed),
            WorkflowFailure = null,
        }, submissionId);
        return true;
    }

    /// <summary>(:207-216).</summary>
    public bool FinishInstallStatusFailure(string submissionId, string message)
    {
        SkillLearningSessionState current = State(submissionId);
        if (!current.Installation.IsLoading)
        {
            return false;
        }

        Store(current with
        {
            Installation = SkillInstallationState.Failed(message, current.Installation.SkillLearningInstall),
        }, submissionId);
        return true;
    }

    /// <summary>(:218-220).</summary>
    public bool InstallStatusNeedsResolution(string submissionId) => State(submissionId).Installation.NeedsResolution;

    /// <summary>(:222-224).</summary>
    public bool InstallStatusIsAbsent(string submissionId) => State(submissionId).Installation.IsAbsent;

    /// <summary>(:226-244).</summary>
    public SkillReviewStart? BeginReview(string submissionId, SkillDraft? draft = null)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation is not null
            || !current.Installation.IsAbsent
            || current.Phase.Kind != SkillLearningPhaseKind.Candidate
            || current.Phase.Candidate is not { } candidate)
        {
            return null;
        }

        SkillCandidate reviewed = candidate.ReplacingDraft(draft ?? candidate.Draft);
        SkillLearningSessionState next = current with
        {
            Phase = SkillLearningPhase.OfCandidate(reviewed),
            Operation = SkillLearningOperation.Reviewing,
            WorkflowFailure = null,
        };
        Store(next, submissionId);
        return new SkillReviewStart(reviewed, next.ReplacesReviewId);
    }

    /// <summary>(:246-261).</summary>
    public bool FinishReview(string submissionId, SkillReview review)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation != SkillLearningOperation.Reviewing
            || current.Phase.Kind != SkillLearningPhaseKind.Candidate
            || current.Phase.Candidate is not { } candidate
            || review.CandidateId != candidate.CandidateId
            || review.SourceSubmissionId != submissionId
            || review.SourceSubmissionId != candidate.SourceSubmissionId)
        {
            return false;
        }

        Store(current with
        {
            Phase = SkillLearningPhase.Reviewed(candidate, review),
            Operation = null,
            WorkflowFailure = null,
            PreservedDraft = null,
            ReplacesReviewId = null,
        }, submissionId);
        return true;
    }

    /// <summary>(:263-270).</summary>
    public SkillReview? BeginEvaluation(string submissionId)
    {
        SkillLearningSessionState current = State(submissionId);
        if (!current.Installation.IsAbsent
            || current.Phase.Kind != SkillLearningPhaseKind.Reviewed
            || current.Phase.Review is not { } review
            || !Begin(SkillLearningOperation.Evaluating, submissionId))
        {
            return null;
        }

        return review;
    }

    /// <summary>(:272-284).</summary>
    public bool FinishEvaluation(string submissionId, SkillEvaluationReport report)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation != SkillLearningOperation.Evaluating
            || current.Phase.Kind != SkillLearningPhaseKind.Reviewed
            || current.Phase.Candidate is not { } candidate
            || current.Phase.Review is not { } review
            || report.ReviewId != review.ReviewId
            || report.SkillSha256 != review.SkillSha256)
        {
            return false;
        }

        Store(current with
        {
            Phase = SkillLearningPhase.Evaluated(candidate, review, report),
            Operation = null,
            WorkflowFailure = null,
        }, submissionId);
        return true;
    }

    /// <summary>
    /// (:286-304). A plan the target refused (occupied) is dropped back to
    /// its evaluation for a new preview; a failed gate starts nothing, and
    /// then nothing is changed.
    /// </summary>
    public SkillEvaluationReport? BeginInstallPlanning(string submissionId)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation is not null || !current.Installation.IsAbsent)
        {
            return null;
        }

        SkillLearningPhase phase = current.Phase;
        SkillEvaluationReport report;
        if (phase.Kind == SkillLearningPhaseKind.Evaluated && phase.Report is { } evaluated)
        {
            report = evaluated;
        }
        else if (phase is { Kind: SkillLearningPhaseKind.Planned, Plan.CanInstall: false }
                 && phase.Candidate is { } candidate && phase.Review is { } review && phase.Report is { } planned)
        {
            report = planned;
            phase = SkillLearningPhase.Evaluated(candidate, review, planned);
        }
        else
        {
            return null;
        }

        if (!report.InstallAllowed)
        {
            return null;
        }

        Store(current with
        {
            Phase = phase,
            Operation = SkillLearningOperation.PlanningInstall,
            WorkflowFailure = null,
        }, submissionId);
        return report;
    }

    /// <summary>(:306-326).</summary>
    public bool FinishInstallPlanning(string submissionId, SkillInstallPlan plan)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation != SkillLearningOperation.PlanningInstall
            || current.Phase.Kind != SkillLearningPhaseKind.Evaluated
            || current.Phase.Candidate is not { } candidate
            || current.Phase.Review is not { } review
            || current.Phase.Report is not { } report
            || plan.EvaluationId != report.EvaluationId
            || plan.SkillSha256 != report.SkillSha256)
        {
            return false;
        }

        Store(current with
        {
            Phase = SkillLearningPhase.Planned(candidate, review, report, plan),
            Operation = null,
            WorkflowFailure = null,
        }, submissionId);
        return true;
    }

    /// <summary>(:328-336).</summary>
    public SkillInstallPlan? BeginInstall(string submissionId)
    {
        SkillLearningSessionState current = State(submissionId);
        if (!current.Installation.IsAbsent
            || current.Phase.Kind != SkillLearningPhaseKind.Planned
            || current.Phase.Plan is not { CanInstall: true } plan
            || !Begin(SkillLearningOperation.Installing, submissionId))
        {
            return null;
        }

        return plan;
    }

    /// <summary>(:338-354). The installed session returns to its evaluation, so a rollback restores it.</summary>
    public bool FinishInstall(string submissionId, SkillLearningInstall installed)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation != SkillLearningOperation.Installing
            || current.Phase.Kind != SkillLearningPhaseKind.Planned
            || current.Phase.Candidate is not { } candidate
            || current.Phase.Review is not { } review
            || current.Phase.Report is not { } report
            || current.Phase.Plan is not { } plan
            || installed.EvaluationId != plan.EvaluationId
            || installed.SkillSha256 != plan.SkillSha256
            || installed.SourceSubmissionId != submissionId
            || installed.SourceSubmissionId != candidate.SourceSubmissionId
            || installed.SourceSubmissionId != review.SourceSubmissionId)
        {
            return false;
        }

        Store(current with
        {
            Phase = SkillLearningPhase.Evaluated(candidate, review, report),
            Installation = SkillInstallationState.Installed(installed),
            Operation = null,
            WorkflowFailure = null,
        }, submissionId);
        return true;
    }

    /// <summary>
    /// (:356-377). The commit's outcome could not be confirmed: keep the
    /// last receipt known, else absent when the status was read, else
    /// unknown-by-failure so the next look reads it again.
    /// </summary>
    public void FinishInstallReconciliationFailure(
        string submissionId,
        SkillLearningInstall? lastKnownInstalled,
        bool statusWasResolved,
        string message)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation != SkillLearningOperation.Installing
            || current.Phase.Kind != SkillLearningPhaseKind.Planned
            || current.Phase.Candidate is not { } candidate
            || current.Phase.Review is not { } review
            || current.Phase.Report is not { } report)
        {
            return;
        }

        SkillInstallationState installation = lastKnownInstalled is not null
            ? SkillInstallationState.Failed(message, lastKnownInstalled)
            : statusWasResolved
                ? SkillInstallationState.Absent
                : SkillInstallationState.Failed(message, null);
        Store(current with
        {
            Phase = SkillLearningPhase.Evaluated(candidate, review, report),
            Installation = installation,
            Operation = null,
            WorkflowFailure = message,
        }, submissionId);
    }

    /// <summary>(:379-384).</summary>
    public SkillLearningInstall? BeginRollback(string submissionId)
    {
        if (State(submissionId).SkillLearningInstall is not { } installed || !Begin(SkillLearningOperation.RollingBack, submissionId))
        {
            return null;
        }

        return installed;
    }

    /// <summary>(:386-404).</summary>
    public void FinishRollback(string submissionId, bool removed, string incompleteMessage)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation != SkillLearningOperation.RollingBack || current.SkillLearningInstall is not { } installed)
        {
            return;
        }

        Store(removed
            ? current with { Installation = SkillInstallationState.Absent, WorkflowFailure = null, Operation = null }
            : current with
            {
                Installation = SkillInstallationState.Failed(incompleteMessage, installed),
                WorkflowFailure = incompleteMessage,
                Operation = null,
            },
            submissionId);
    }

    /// <summary>(:406-421). A failed install returns to its evaluation; anything else keeps its phase.</summary>
    public void FinishFailure(string submissionId, SkillLearningOperation operation, string message)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation != operation)
        {
            return;
        }

        SkillLearningPhase phase = current.Phase;
        if (operation == SkillLearningOperation.Installing
            && phase.Kind == SkillLearningPhaseKind.Planned
            && phase.Candidate is { } candidate && phase.Review is { } review && phase.Report is { } report)
        {
            phase = SkillLearningPhase.Evaluated(candidate, review, report);
        }

        Store(current with { Phase = phase, Operation = null, WorkflowFailure = message }, submissionId);
    }

    /// <summary>(:423-444). Back to the candidate, with the reviewed draft, replacing that review.</summary>
    public bool Edit(string submissionId)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation is not null
            || !current.Installation.IsAbsent
            || current.Phase.Kind is not (SkillLearningPhaseKind.Reviewed or SkillLearningPhaseKind.Evaluated or SkillLearningPhaseKind.Planned)
            || current.Phase.Candidate is not { } value
            || current.Phase.Review is not { } review)
        {
            return false;
        }

        Store(current with
        {
            Phase = SkillLearningPhase.OfCandidate(value.ReplacingDraft(review.Draft)),
            Operation = null,
            WorkflowFailure = null,
            PreservedDraft = null,
            ReplacesReviewId = review.ReviewId,
        }, submissionId);
        return true;
    }

    /// <summary>(:446-459). The daemon forgot an id: back to idle, keeping the draft for the next learn.</summary>
    public void FinishExpiredWorkflow(string submissionId, SkillLearningOperation operation, string message)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation != operation)
        {
            return;
        }

        Store(current with
        {
            PreservedDraft = DraftFrom(current.Phase),
            Phase = SkillLearningPhase.Idle,
            Operation = null,
            WorkflowFailure = message,
            ReplacesReviewId = null,
        }, submissionId);
    }

    /// <summary>(:461-471).</summary>
    private bool Begin(SkillLearningOperation operation, string submissionId)
    {
        SkillLearningSessionState current = State(submissionId);
        if (current.Operation is not null)
        {
            return false;
        }

        Store(current with { Operation = operation, WorkflowFailure = null }, submissionId);
        return true;
    }

    /// <summary>(:473-488). Evicts the least recent settled session over the bound, never this one or a working one.</summary>
    private void Store(SkillLearningSessionState value, string submissionId)
    {
        _sessions[submissionId] = value;
        _recency.Remove(submissionId);
        _recency.Add(submissionId);
        while (_sessions.Count > MaximumRetainedSessions)
        {
            int index = _recency.FindIndex(candidate =>
                candidate != submissionId
                && _sessions.TryGetValue(candidate, out SkillLearningSessionState? state)
                && !state.IsWorking);
            if (index < 0)
            {
                break;
            }

            string evicted = _recency[index];
            _recency.RemoveAt(index);
            _sessions.Remove(evicted);
        }

        Changed?.Invoke(submissionId);
    }

    /// <summary>(:490-501).</summary>
    private static SkillDraft? DraftFrom(SkillLearningPhase phase) => phase.Kind switch
    {
        SkillLearningPhaseKind.Idle => null,
        SkillLearningPhaseKind.Candidate => phase.Candidate?.Draft,
        _ => phase.Review?.Draft,
    };
}

/// <summary>
/// A tested-skill call that did not produce its answer. <see cref="Label"/>
/// is the daemon's fixed refusal label (its error frame's message, as the
/// Mac's <c>DaemonClient.Failure.message</c>), or null when the call failed
/// any other way: a reply that did not decode, or a transport that threw.
/// </summary>
public sealed class SkillLearningCallException : Exception
{
    public SkillLearningCallException(string? label, Exception? inner = null)
        : base(label, inner)
    {
        Label = label;
    }

    public string? Label { get; }
}

/// <summary>
/// The tested-skill daemon calls with their exact parameters
/// (DaemonClient+SkillLearning.swift:6-91), over the shell's call function.
/// Each answer decodes as strictly as the Mac's, or the call fails.
/// </summary>
public sealed class SkillLearningClient
{
    public const string CandidateMethod = "skill_candidate";
    public const string ReviewMethod = "skill_review";
    public const string EvaluateMethod = "skill_evaluate";
    public const string InstallPlanMethod = "skill_install_plan";
    public const string InstallCommitMethod = "skill_install_commit";
    public const string InstallStatusMethod = "skill_install_status";
    public const string RollbackMethod = "skill_install_rollback";

    private readonly Func<string, string, Task<DaemonResponse>> _call;

    public SkillLearningClient(Func<string, string, Task<DaemonResponse>> call) => _call = call;

    /// <summary><c>skill_candidate</c>'s parameters (:7-13).</summary>
    public static string CandidateParams(string submissionId) =>
        new JsonObject { ["submission_id"] = submissionId }.ToJsonString();

    /// <summary><c>skill_review</c>'s parameters; no <c>replaces_review_id</c> unless there is one (:15-36).</summary>
    public static string ReviewParams(string candidateId, SkillDraft draft, string? replacesReviewId)
    {
        var parameters = new JsonObject
        {
            ["candidate_id"] = candidateId,
            ["draft"] = draft.ToNode(),
        };
        if (replacesReviewId is not null)
        {
            parameters["replaces_review_id"] = replacesReviewId;
        }

        return parameters.ToJsonString();
    }

    /// <summary><c>skill_evaluate</c>'s parameters (:38-44).</summary>
    public static string EvaluateParams(string reviewId, string skillSha256) =>
        new JsonObject { ["review_id"] = reviewId, ["skill_sha256"] = skillSha256 }.ToJsonString();

    /// <summary><c>skill_install_plan</c>'s parameters (:46-52).</summary>
    public static string InstallPlanParams(string evaluationId) =>
        new JsonObject { ["evaluation_id"] = evaluationId }.ToJsonString();

    /// <summary><c>skill_install_commit</c>'s parameters: the plan and the exact digests previewed (:54-68).</summary>
    public static string InstallCommitParams(string planId, string asPreviewedSha256, string asPreviewedMarkerSha256) =>
        new JsonObject
        {
            ["plan_id"] = planId,
            ["as_previewed_sha256"] = asPreviewedSha256,
            ["as_previewed_marker_sha256"] = asPreviewedMarkerSha256,
        }.ToJsonString();

    /// <summary><c>skill_install_status</c>'s parameters (:70-76).</summary>
    public static string InstallStatusParams(string sourceSubmissionId) =>
        new JsonObject { ["source_submission_id"] = sourceSubmissionId }.ToJsonString();

    /// <summary><c>skill_install_rollback</c>'s parameters (:78-90).</summary>
    public static string RollbackParams(string installId, string sourceSubmissionId) =>
        new JsonObject { ["install_id"] = installId, ["source_submission_id"] = sourceSubmissionId }.ToJsonString();

    /// <summary>(:7-13).</summary>
    public Task<SkillCandidate> SkillCandidateAsync(string submissionId) =>
        CallAsync<SkillCandidate>(CandidateMethod, CandidateParams(submissionId), SkillCandidate.Parse);

    /// <summary>(:15-36).</summary>
    public Task<SkillReview> ReviewSkillAsync(string candidateId, SkillDraft draft, string? replacesReviewId = null) =>
        CallAsync<SkillReview>(ReviewMethod, ReviewParams(candidateId, draft, replacesReviewId), SkillReview.Parse);

    /// <summary>(:38-44).</summary>
    public Task<SkillEvaluationReport> EvaluateSkillAsync(string reviewId, string skillSha256) =>
        CallAsync<SkillEvaluationReport>(EvaluateMethod, EvaluateParams(reviewId, skillSha256), SkillEvaluationReport.Parse);

    /// <summary>(:46-52).</summary>
    public Task<SkillInstallPlan> SkillInstallPlanAsync(string evaluationId) =>
        CallAsync<SkillInstallPlan>(InstallPlanMethod, InstallPlanParams(evaluationId), SkillInstallPlan.Parse);

    /// <summary>(:54-68).</summary>
    public Task<SkillLearningInstall> InstallSkillAsync(string planId, string asPreviewedSha256, string asPreviewedMarkerSha256) =>
        CallAsync<SkillLearningInstall>(InstallCommitMethod, InstallCommitParams(planId, asPreviewedSha256, asPreviewedMarkerSha256), SkillLearningInstall.Parse);

    /// <summary>(:70-76).</summary>
    public Task<SkillInstallStatus> SkillInstallStatusAsync(string sourceSubmissionId) =>
        CallAsync<SkillInstallStatus>(InstallStatusMethod, InstallStatusParams(sourceSubmissionId), SkillInstallStatus.Parse);

    /// <summary>(:78-90).</summary>
    public Task<SkillRollbackResult> RollbackSkillAsync(string installId, string sourceSubmissionId) =>
        CallAsync<SkillRollbackResult>(RollbackMethod, RollbackParams(installId, sourceSubmissionId), SkillRollbackResult.Parse);

    /// <summary>
    /// One call: an error frame fails with its label, a frame with no result
    /// fails as <c>missing-result</c> (DaemonClient.swift:1189-1191), and a
    /// result that does not decode, or a transport that throws, fails with
    /// no label.
    /// </summary>
    private async Task<T> CallAsync<T>(string method, string parameters, Func<JsonElement, T?> parse)
        where T : class
    {
        DaemonResponse response;
        try
        {
            response = await _call(method, parameters).ConfigureAwait(true);
        }
        catch (Exception exception)
        {
            throw new SkillLearningCallException(null, exception);
        }

        if (response.Error is { } error)
        {
            throw new SkillLearningCallException(error.Message);
        }

        if (response.Result is not { } result)
        {
            throw new SkillLearningCallException("missing-result");
        }

        return parse(result) ?? throw new SkillLearningCallException(null);
    }
}

/// <summary>
/// The tested-skill workflow's actions over every session
/// (AppModel+SkillLearning.swift:7-350): learn, review, test, preview an
/// install, install, roll back and edit, each settling the session's state
/// in <see cref="SkillLearningStore"/> against the daemon's answer. Every
/// sentence a failure shows is the core's: the error line for the daemon's
/// label, else the copy's <c>unavailable</c> (:310-312).
/// </summary>
/// <remarks>
/// Each action starts synchronously (its <c>Begin</c> runs before the first
/// await) and its task completes once the session has settled, so a caller
/// may fire it and forget, or await it.
/// </remarks>
public sealed class SkillLearningModel : INotifyPropertyChanged
{
    private const string LineageInvalid = "skill-source-lineage-invalid";

    private readonly SkillLearningClient? _client;
    private readonly Func<string, string?> _errorLine;
    private readonly SkillLearningStore _store = new();

    /// <param name="call">The daemon call, or null when there is no daemon (the Mac's nil <c>skillLearningClient</c>).</param>
    /// <param name="copy">The core's words, for the rollback-incomplete and unavailable sentences.</param>
    /// <param name="errorLine">The core's sentence for a refusal label (<see cref="SkillLearningNative.ErrorLine"/>).</param>
    public SkillLearningModel(
        Func<string, string, Task<DaemonResponse>>? call,
        SkillLearningCopy? copy,
        Func<string, string?> errorLine)
    {
        _client = call is null ? null : new SkillLearningClient(call);
        Copy = copy;
        _errorLine = errorLine;
        _store.Changed += id =>
        {
            SessionChanged?.Invoke(id);
            PropertyChanged?.Invoke(this, new PropertyChangedEventArgs(string.Empty));
        };
    }

    /// <summary>The model over the core's own words and error lines.</summary>
    public static SkillLearningModel WithCore(Func<string, string, Task<DaemonResponse>>? call) =>
        new(call, SkillLearningCopy.Current, SkillLearningNative.ErrorLine);

    public event PropertyChangedEventHandler? PropertyChanged;

    /// <summary>Raised with a session's id after its state changes.</summary>
    public event Action<string>? SessionChanged;

    /// <summary>The core's words, or null when they could not be read (the surface draws nothing then).</summary>
    public SkillLearningCopy? Copy { get; }

    /// <summary>Whether there is a daemon to ask.</summary>
    public bool Available => _client is not null;

    /// <summary>A session's state (<c>skillLearningState(for:)</c>, :306-308).</summary>
    public SkillLearningSessionState State(string submissionId) => _store.State(submissionId);

    /// <summary>How many sessions the store holds (<see cref="SkillLearningStore.RetainedSessionCount"/>).</summary>
    public int RetainedSessionCount => _store.RetainedSessionCount;

    /// <summary>
    /// Another account, or none, owns the window now: every session's state
    /// goes (<c>skillLearningStore = SkillLearningStore()</c>,
    /// AppModel.swift:4255-4261). An answer still in flight settles nothing,
    /// since its session no longer waits on it.
    /// </summary>
    public void ClearAccountOwnedContent() => _store.Clear();

    /// <summary>
    /// Learn a skill from the session's correction (<c>learnSkill</c>,
    /// :8-52). An unread installation status is read first, and learning
    /// continues only when nothing is installed.
    /// </summary>
    public async Task LearnSkillAsync(string submissionId)
    {
        if (_client is null)
        {
            return;
        }

        if (_store.InstallStatusNeedsResolution(submissionId))
        {
            await EnsureInstalledSkillStatusAsync(submissionId, continueLearningWhenAbsent: true).ConfigureAwait(true);
            return;
        }

        if (!_store.InstallStatusIsAbsent(submissionId) || !_store.BeginLearning(submissionId))
        {
            return;
        }

        SkillCandidate candidate;
        try
        {
            candidate = await _client.SkillCandidateAsync(submissionId).ConfigureAwait(true);
            if (candidate.SourceSubmissionId != submissionId)
            {
                throw new SkillLearningCallException(LineageInvalid);
            }
        }
        catch (SkillLearningCallException failure)
        {
            _store.FinishFailure(submissionId, SkillLearningOperation.Learning, ErrorLine(failure.Label ?? string.Empty));
            return;
        }

        if (!_store.FinishLearning(submissionId, candidate))
        {
            _store.FinishFailure(submissionId, SkillLearningOperation.Learning, ErrorLine(LineageInvalid));
        }
    }

    /// <summary>
    /// Read whether this session's skill is installed, unless that is known
    /// (<c>ensureLocalInstalledSkillStatus(for:)</c>, :54-56). The surface
    /// calls this when it appears.
    /// </summary>
    public Task EnsureLocalInstalledSkillStatusAsync(string submissionId) =>
        EnsureInstalledSkillStatusAsync(submissionId, continueLearningWhenAbsent: false);

    /// <summary>Send the draft for review (<c>reviewSkill</c>, :99-127).</summary>
    public async Task ReviewSkillAsync(string submissionId, SkillDraft draft)
    {
        if (_client is null || _store.BeginReview(submissionId, draft) is not { } start)
        {
            return;
        }

        SkillReview review;
        try
        {
            review = await _client.ReviewSkillAsync(start.Candidate.CandidateId, start.Candidate.Draft, start.ReplacesReviewId)
                .ConfigureAwait(true);
        }
        catch (SkillLearningCallException failure)
        {
            FinishWorkflowFailure(submissionId, SkillLearningOperation.Reviewing, failure.Label ?? string.Empty);
            return;
        }

        if (!_store.FinishReview(submissionId, review))
        {
            _store.FinishFailure(submissionId, SkillLearningOperation.Reviewing, ErrorLine(LineageInvalid));
        }
    }

    /// <summary>Test the reviewed skill (<c>testSkill</c>, :129-156).</summary>
    public async Task TestSkillAsync(string submissionId)
    {
        if (_client is null || _store.BeginEvaluation(submissionId) is not { } review)
        {
            return;
        }

        SkillEvaluationReport report;
        try
        {
            report = await _client.EvaluateSkillAsync(review.ReviewId, review.SkillSha256).ConfigureAwait(true);
        }
        catch (SkillLearningCallException failure)
        {
            FinishWorkflowFailure(submissionId, SkillLearningOperation.Evaluating, failure.Label ?? string.Empty);
            return;
        }

        if (!_store.FinishEvaluation(submissionId, report))
        {
            _store.FinishFailure(submissionId, SkillLearningOperation.Evaluating, ErrorLine(LineageInvalid));
        }
    }

    /// <summary>
    /// Preview the installation of a skill that passed its gate
    /// (<c>reviewSkillInstall</c>, :158-190). An unread status is read
    /// instead, and nothing else happens this time.
    /// </summary>
    public async Task ReviewSkillInstallAsync(string submissionId)
    {
        if (_client is null)
        {
            return;
        }

        if (_store.InstallStatusNeedsResolution(submissionId))
        {
            await EnsureLocalInstalledSkillStatusAsync(submissionId).ConfigureAwait(true);
            return;
        }

        if (_store.BeginInstallPlanning(submissionId) is not { } evaluation)
        {
            return;
        }

        SkillInstallPlan plan;
        try
        {
            plan = await _client.SkillInstallPlanAsync(evaluation.EvaluationId).ConfigureAwait(true);
        }
        catch (SkillLearningCallException failure)
        {
            FinishWorkflowFailure(submissionId, SkillLearningOperation.PlanningInstall, failure.Label ?? string.Empty);
            return;
        }

        if (!_store.FinishInstallPlanning(submissionId, plan))
        {
            _store.FinishFailure(submissionId, SkillLearningOperation.PlanningInstall, ErrorLine(LineageInvalid));
        }
    }

    /// <summary>
    /// Commit the previewed installation (<c>installSkill</c>, :192-252).
    /// The status is read after the commit whatever the commit answered,
    /// and the status, not the commit's reply, settles what is installed: a
    /// lost receipt is recovered, and a refused commit leaves nothing.
    /// </summary>
    public async Task InstallSkillAsync(string submissionId)
    {
        if (_client is null || _store.BeginInstall(submissionId) is not { } plan)
        {
            return;
        }

        SkillLearningInstall? commitReceipt = null;
        SkillLearningCallException? commitFailure = null;
        try
        {
            SkillLearningInstall installed = await _client
                .InstallSkillAsync(plan.PlanId, plan.SkillSha256, plan.MarkerFileSha256)
                .ConfigureAwait(true);
            if (installed.SourceSubmissionId == submissionId)
            {
                commitReceipt = installed;
            }
        }
        catch (SkillLearningCallException failure)
        {
            commitFailure = failure;
        }

        SkillLearningInstall? statusInstalled;
        try
        {
            statusInstalled = ValidatedInstalledSkill(
                await _client.SkillInstallStatusAsync(submissionId).ConfigureAwait(true),
                submissionId);
        }
        catch (SkillLearningCallException failure)
        {
            _store.FinishInstallReconciliationFailure(
                submissionId,
                lastKnownInstalled: commitReceipt,
                statusWasResolved: false,
                message: ErrorLine(failure.Label ?? string.Empty));
            return;
        }

        if (statusInstalled is not null)
        {
            if (!_store.FinishInstall(submissionId, statusInstalled))
            {
                _store.FinishInstallReconciliationFailure(
                    submissionId,
                    lastKnownInstalled: statusInstalled,
                    statusWasResolved: true,
                    message: ErrorLine(LineageInvalid));
            }

            return;
        }

        string label = commitFailure is not null ? commitFailure.Label ?? string.Empty : LineageInvalid;
        _store.FinishInstallReconciliationFailure(
            submissionId,
            lastKnownInstalled: null,
            statusWasResolved: true,
            message: ErrorLine(label));
    }

    /// <summary>
    /// Remove the installed skill (<c>rollbackSkill</c>, :254-300). A daemon
    /// that no longer knows the installation is asked for the status: gone
    /// counts as removed, still there keeps the refusal.
    /// </summary>
    public async Task RollbackSkillAsync(string submissionId)
    {
        if (_client is null || _store.BeginRollback(submissionId) is not { } installed)
        {
            return;
        }

        SkillRollbackResult rollback;
        try
        {
            try
            {
                rollback = await _client.RollbackSkillAsync(installed.InstallId, installed.SourceSubmissionId)
                    .ConfigureAwait(true);
            }
            catch (SkillLearningCallException failure) when (
                SkillLearningDaemonFailure.FromLabel(failure.Label ?? string.Empty).Kind
                == SkillLearningDaemonFailureKind.InstallUnknown)
            {
                SkillInstallStatus status = await _client.SkillInstallStatusAsync(installed.SourceSubmissionId)
                    .ConfigureAwait(true);
                if (ValidatedInstalledSkill(status, installed.SourceSubmissionId) is not null)
                {
                    throw;
                }

                rollback = new SkillRollbackResult(Removed: true, RetainedDirectory: false);
            }
        }
        catch (SkillLearningCallException failure)
        {
            _store.FinishFailure(submissionId, SkillLearningOperation.RollingBack, ErrorLine(failure.Label ?? string.Empty));
            return;
        }

        _store.FinishRollback(submissionId, rollback.Removed, Copy?.RollbackIncomplete ?? string.Empty);
    }

    /// <summary>Back to the draft (<c>editSkill</c>, :302-304).</summary>
    public void EditSkill(string submissionId) => _store.Edit(submissionId);

    /// <summary>(:58-97).</summary>
    private async Task EnsureInstalledSkillStatusAsync(string submissionId, bool continueLearningWhenAbsent)
    {
        if (_client is null || !_store.BeginInstallStatusLoading(submissionId))
        {
            return;
        }

        SkillLearningInstall? installed;
        try
        {
            installed = ValidatedInstalledSkill(
                await _client.SkillInstallStatusAsync(submissionId).ConfigureAwait(true),
                submissionId);
        }
        catch (SkillLearningCallException failure)
        {
            _store.FinishInstallStatusFailure(submissionId, ErrorLine(failure.Label ?? string.Empty));
            return;
        }

        if (!_store.FinishInstallStatusLoading(submissionId, installed))
        {
            _store.FinishInstallStatusFailure(submissionId, ErrorLine(LineageInvalid));
        }
        else if (installed is null && continueLearningWhenAbsent)
        {
            await LearnSkillAsync(submissionId).ConfigureAwait(true);
        }
    }

    /// <summary>
    /// The status's skill, refusing a contradictory shape or another
    /// session's installation (<c>validatedInstalledSkill</c>, :314-328).
    /// </summary>
    private static SkillLearningInstall? ValidatedInstalledSkill(SkillInstallStatus status, string sourceSubmissionId)
    {
        if (status.Installed != (status.Skill is not null)
            || (status.Skill is { } skill && skill.SourceSubmissionId != sourceSubmissionId))
        {
            throw new SkillLearningCallException(LineageInvalid);
        }

        return status.Skill;
    }

    /// <summary>(:310-312).</summary>
    private string ErrorLine(string label) => _errorLine(label) ?? Copy?.Unavailable ?? string.Empty;

    /// <summary>An expired opaque id restarts the workflow; anything else keeps its phase (:330-349).</summary>
    private void FinishWorkflowFailure(string submissionId, SkillLearningOperation operation, string label)
    {
        string message = ErrorLine(label);
        if (SkillLearningDaemonFailure.FromLabel(label).InvalidatesWorkflow)
        {
            _store.FinishExpiredWorkflow(submissionId, operation, message);
        }
        else
        {
            _store.FinishFailure(submissionId, operation, message);
        }
    }
}

/// <summary>
/// When a contribution offers the Skills flow, one rule for every surface
/// that shows it (<c>SkillLearningGate</c>, SkillLearningView.swift:661-721).
/// Its withdrawal and activity halves are <see cref="ContributionGate"/>'s
/// (<c>withdrawalCompleted</c>, <c>contributionIsWithdrawn</c>,
/// <c>contributionIsActive</c>, :666-680), which this reuses.
/// </summary>
public static class SkillLearningGate
{
    /// <summary>
    /// Learn, review, test and install: an accepted contribution with a
    /// known task outcome, not withdrawn or closed, carrying a human
    /// correction. An absent detail offers nothing (:682-687).
    /// </summary>
    public static bool OffersLearning(SessionDetail? detail, string recordStatus, bool withdrawalCompleted) =>
        detail is not null
        && ContributionGate.IsActive(detail, recordStatus, withdrawalCompleted)
        && detail.HumanCorrection is not null;

    /// <summary>
    /// Whether the installed-skill surface (and its status retry) may show:
    /// whenever the regular flow is not the one on screen, including before
    /// the detail answers (:689-698).
    /// </summary>
    public static bool ShowsInstalledSurface(SessionDetail? detail, bool withdrawalCompleted)
    {
        bool regularSurfaceIsVisible = detail is { Accepted: true, HumanCorrection: not null }
            && !withdrawalCompleted
            && !ContributionGate.IsTerminal(detail.ContributionStatus);
        return !regularSurfaceIsVisible;
    }

    /// <summary>
    /// The installed-skill status read failed on a closed or unaccepted
    /// contribution with nothing installed: offer to read it again (:700-708).
    /// </summary>
    public static bool OffersInstallStatusRetry(SkillLearningSessionState state, SessionDetail? detail, string recordStatus)
    {
        if (state.SkillLearningInstall is not null || state.Phase.Kind != SkillLearningPhaseKind.Idle || state.Failure is null)
        {
            return false;
        }

        return ContributionGate.IsTerminal(recordStatus) || detail is { Accepted: false };
    }
}
