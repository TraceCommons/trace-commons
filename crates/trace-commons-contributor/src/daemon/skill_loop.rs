//! INTEGRATION: binds the tested-skill flow to real account-owned records,
//! live NEAR AI evaluation, held reviews, and one guarded Codex installation.

use std::collections::{BTreeMap, VecDeque};
use std::time::{Duration, Instant};

use uuid::Uuid;

#[cfg(test)]
use crate::skill_loop::InstalledSkill;
#[cfg(test)]
use crate::skill_loop::SkillDraft;
use crate::skill_loop::{
    CodexInstallPlan, CodexInstalledSkill, SkillCandidate, SkillEvaluationReport,
    SkillInstallError, SkillInstallPlan, SkillReview,
};

mod handlers;

#[cfg(test)]
use handlers::{
    CREDENTIAL_SNAPSHOT_ATTEMPTS, ReviewParams, TestSkillLoopRuntime,
    consistent_credential_snapshot, install_test_skill_loop_runtime, install_transaction,
    run_until_credential_change,
};
pub(super) use handlers::{
    handle_candidate, handle_evaluate, handle_install_commit, handle_install_plan,
    handle_install_status, handle_review, handle_rollback,
};

const MAX_HELD_ITEMS: usize = 12;
const INSTALL_PLAN_TTL: Duration = Duration::from_secs(10 * 60);

pub const ERR_CANDIDATE_UNKNOWN: &str = "skill-candidate-unknown";
pub const ERR_REVIEW_UNKNOWN: &str = "skill-review-unknown";
pub const ERR_EVALUATION_UNKNOWN: &str = "skill-evaluation-unknown";
pub const ERR_EVALUATION_DID_NOT_PASS: &str = "skill-evaluation-did-not-pass";
pub const ERR_EVALUATION_BUSY: &str = "skill-evaluation-busy";
pub const ERR_INSTALL_PLAN_UNKNOWN: &str = "skill-install-plan-unknown";
pub const ERR_INSTALL_UNKNOWN: &str = "skill-install-unknown";

#[derive(Default)]
pub(crate) struct SkillLoopState {
    owner_scope: Option<String>,
    owner_generation: Uuid,
    workflows: VecDeque<SkillWorkflow>,
    active_evaluation: Option<Uuid>,
    /// Installation ownership is orthogonal to candidate progress. Commit and
    /// first trusted recovery receipts remain authoritative for this daemon's
    /// lifetime, even when bounded workflow state is evicted.
    install_receipts: BTreeMap<Uuid, CodexInstalledSkill>,
}

#[derive(Clone, PartialEq, Eq)]
pub(super) struct OwnerLease {
    scope: String,
    generation: Uuid,
}

impl OwnerLease {
    pub(super) fn scope(&self) -> &str {
        &self.scope
    }
}

struct SkillWorkflow {
    candidate: SkillCandidate,
    phase: SkillWorkflowPhase,
}

enum SkillWorkflowPhase {
    Candidate,
    Reviewed(SkillReview),
    Evaluating(SkillReview),
    Evaluated(Box<EvaluatedSkill>),
    Planning {
        evaluated: Box<EvaluatedSkill>,
        planning_id: Uuid,
    },
    Planned {
        evaluated: Box<EvaluatedSkill>,
        held: HeldInstallPlan,
    },
    Installing {
        evaluated: Box<EvaluatedSkill>,
        plan_id: Uuid,
    },
}

struct EvaluatedSkill {
    review: SkillReview,
    report: SkillEvaluationReport,
}

struct HeldInstallPlan {
    plan: CodexInstallPlan,
    minted: Instant,
}

impl SkillWorkflow {
    fn candidate_for_client(&self) -> SkillCandidate {
        let mut candidate = self.candidate.clone();
        let review = match &self.phase {
            SkillWorkflowPhase::Reviewed(review) | SkillWorkflowPhase::Evaluating(review) => {
                Some(review)
            }
            SkillWorkflowPhase::Evaluated(evaluated)
            | SkillWorkflowPhase::Planning { evaluated, .. }
            | SkillWorkflowPhase::Planned { evaluated, .. }
            | SkillWorkflowPhase::Installing { evaluated, .. } => Some(&evaluated.review),
            SkillWorkflowPhase::Candidate => None,
        };
        if let Some(review) = review {
            candidate.draft = review.draft.clone();
            candidate.replaces_review_id = Some(review.review_id);
        }
        candidate
    }
}

enum EvaluationStart {
    Existing(Box<SkillEvaluationReport>),
    InProgress,
    Busy,
    Started,
}

enum InstallPlanStart {
    Existing(Box<SkillInstallPlan>),
    Ready(Box<(SkillEvaluationReport, SkillReview, Uuid)>),
    Unknown,
}

impl SkillLoopState {
    fn bind_owner(&mut self, owner_scope: &str) -> OwnerLease {
        if self.owner_scope.as_deref() == Some(owner_scope) {
            return OwnerLease {
                scope: owner_scope.to_string(),
                generation: self.owner_generation,
            };
        }
        self.workflows.clear();
        self.install_receipts.clear();
        self.active_evaluation = None;
        self.owner_scope = Some(owner_scope.to_string());
        self.owner_generation = Uuid::new_v4();
        OwnerLease {
            scope: owner_scope.to_string(),
            generation: self.owner_generation,
        }
    }

    fn owner_matches(&self, lease: &OwnerLease) -> bool {
        self.owner_scope.as_deref() == Some(lease.scope())
            && self.owner_generation == lease.generation
    }

    fn hold_candidate(&mut self, value: SkillCandidate) -> Option<SkillCandidate> {
        if let Some(existing) = self
            .workflows
            .iter()
            .find(|workflow| workflow.candidate.source_submission_id == value.source_submission_id)
        {
            return Some(existing.candidate_for_client());
        }
        if self.workflows.len() >= MAX_HELD_ITEMS {
            for workflow in &mut self.workflows {
                let expired = matches!(
                    &workflow.phase,
                    SkillWorkflowPhase::Planned { held, .. }
                        if held.minted.elapsed() > INSTALL_PLAN_TTL
                );
                if expired {
                    let phase =
                        std::mem::replace(&mut workflow.phase, SkillWorkflowPhase::Candidate);
                    if let SkillWorkflowPhase::Planned { evaluated, .. } = phase {
                        workflow.phase = SkillWorkflowPhase::Evaluated(evaluated);
                    }
                }
            }
            let removable = self
                .workflows
                .iter()
                .enumerate()
                .filter_map(|(position, workflow)| {
                    let priority = match &workflow.phase {
                        SkillWorkflowPhase::Candidate => Some(0_u8),
                        SkillWorkflowPhase::Reviewed(_) => Some(1),
                        SkillWorkflowPhase::Evaluated(_) => Some(2),
                        SkillWorkflowPhase::Evaluating(_)
                        | SkillWorkflowPhase::Planning { .. }
                        | SkillWorkflowPhase::Installing { .. }
                        | SkillWorkflowPhase::Planned { .. } => None,
                    }?;
                    Some((priority, position))
                })
                .min()
                .map(|(_, position)| position);
            let position = removable?;
            self.workflows.remove(position);
        }
        self.workflows.push_back(SkillWorkflow {
            candidate: value.clone(),
            phase: SkillWorkflowPhase::Candidate,
        });
        Some(value)
    }

    fn candidate(&self, id: Uuid) -> Option<SkillCandidate> {
        self.workflows
            .iter()
            .find(|workflow| workflow.candidate.candidate_id == id)
            .map(|workflow| workflow.candidate.clone())
    }

    fn review(&self, id: Uuid) -> Option<SkillReview> {
        self.workflows
            .iter()
            .find_map(|workflow| match &workflow.phase {
                SkillWorkflowPhase::Reviewed(review) | SkillWorkflowPhase::Evaluating(review)
                    if review.review_id == id =>
                {
                    Some(review.clone())
                }
                SkillWorkflowPhase::Evaluated(evaluated)
                | SkillWorkflowPhase::Planning { evaluated, .. }
                | SkillWorkflowPhase::Planned { evaluated, .. }
                | SkillWorkflowPhase::Installing { evaluated, .. }
                    if evaluated.review.review_id == id =>
                {
                    Some(evaluated.review.clone())
                }
                _ => None,
            })
    }

    fn hold_review(
        &mut self,
        proposed: SkillReview,
        replaces_review_id: Option<Uuid>,
    ) -> Option<SkillReview> {
        let install_locked = self
            .install_receipts
            .contains_key(&proposed.source_submission_id);
        let workflow = self
            .workflows
            .iter_mut()
            .find(|workflow| workflow.candidate.candidate_id == proposed.candidate_id)?;
        let (existing, replacement_allowed) = match &workflow.phase {
            SkillWorkflowPhase::Candidate if replaces_review_id.is_none() => (None, true),
            SkillWorkflowPhase::Reviewed(review) => (Some(review.clone()), true),
            SkillWorkflowPhase::Evaluating(review) => (Some(review.clone()), false),
            SkillWorkflowPhase::Evaluated(evaluated)
            | SkillWorkflowPhase::Planned { evaluated, .. } => {
                (Some(evaluated.review.clone()), true)
            }
            SkillWorkflowPhase::Planning { evaluated, .. }
            | SkillWorkflowPhase::Installing { evaluated, .. } => {
                (Some(evaluated.review.clone()), false)
            }
            SkillWorkflowPhase::Candidate => return None,
        };
        if let Some(existing) = existing {
            if existing.skill_sha256 == proposed.skill_sha256 {
                return Some(existing);
            }
            if install_locked
                || !replacement_allowed
                || replaces_review_id != Some(existing.review_id)
            {
                return None;
            }
        }
        workflow.phase = SkillWorkflowPhase::Reviewed(proposed.clone());
        Some(proposed)
    }

    fn insert_evaluation(&mut self, value: SkillEvaluationReport) {
        let review_id = value.review_id;
        let Some(workflow) = self.workflows.iter_mut().find(|workflow| {
            matches!(
                &workflow.phase,
                SkillWorkflowPhase::Evaluating(review) if review.review_id == review_id
            )
        }) else {
            return;
        };
        let phase = std::mem::replace(&mut workflow.phase, SkillWorkflowPhase::Candidate);
        if let SkillWorkflowPhase::Evaluating(review) = phase {
            workflow.phase = SkillWorkflowPhase::Evaluated(Box::new(EvaluatedSkill {
                review,
                report: value,
            }));
        }
        if self.active_evaluation == Some(review_id) {
            self.active_evaluation = None;
        }
    }

    fn begin_evaluation(&mut self, review_id: Uuid) -> EvaluationStart {
        let Some(position) = self.workflows.iter().position(|workflow| {
            matches!(
                &workflow.phase,
                SkillWorkflowPhase::Reviewed(review)
                    | SkillWorkflowPhase::Evaluating(review)
                    if review.review_id == review_id
            ) || matches!(
                &workflow.phase,
                SkillWorkflowPhase::Evaluated(evaluated)
                    | SkillWorkflowPhase::Planning { evaluated, .. }
                    | SkillWorkflowPhase::Planned { evaluated, .. }
                    | SkillWorkflowPhase::Installing { evaluated, .. }
                    if evaluated.review.review_id == review_id
            )
        }) else {
            return EvaluationStart::InProgress;
        };
        match &self.workflows[position].phase {
            SkillWorkflowPhase::Evaluated(evaluated)
            | SkillWorkflowPhase::Planning { evaluated, .. }
            | SkillWorkflowPhase::Planned { evaluated, .. }
            | SkillWorkflowPhase::Installing { evaluated, .. } => {
                return EvaluationStart::Existing(Box::new(evaluated.report.clone()));
            }
            SkillWorkflowPhase::Evaluating(_) => return EvaluationStart::InProgress,
            SkillWorkflowPhase::Reviewed(_) => {}
            SkillWorkflowPhase::Candidate => return EvaluationStart::InProgress,
        }
        if self.active_evaluation.is_some() {
            return EvaluationStart::Busy;
        }
        let phase = std::mem::replace(
            &mut self.workflows[position].phase,
            SkillWorkflowPhase::Candidate,
        );
        if let SkillWorkflowPhase::Reviewed(review) = phase {
            self.workflows[position].phase = SkillWorkflowPhase::Evaluating(review);
            self.active_evaluation = Some(review_id);
            EvaluationStart::Started
        } else {
            EvaluationStart::InProgress
        }
    }

    fn finish_failed_evaluation(&mut self, review_id: Uuid) {
        if let Some(workflow) = self.workflows.iter_mut().find(|workflow| {
            matches!(
                &workflow.phase,
                SkillWorkflowPhase::Evaluating(review) if review.review_id == review_id
            )
        }) {
            let phase = std::mem::replace(&mut workflow.phase, SkillWorkflowPhase::Candidate);
            if let SkillWorkflowPhase::Evaluating(review) = phase {
                workflow.phase = SkillWorkflowPhase::Reviewed(review);
            }
        }
        if self.active_evaluation == Some(review_id) {
            self.active_evaluation = None;
        }
    }

    fn begin_install_plan(&mut self, evaluation_id: Uuid) -> InstallPlanStart {
        let Some(position) = self.workflows.iter().position(|workflow| {
            matches!(
                &workflow.phase,
                SkillWorkflowPhase::Evaluated(evaluated)
                    | SkillWorkflowPhase::Planning { evaluated, .. }
                    | SkillWorkflowPhase::Planned { evaluated, .. }
                    if evaluated.report.evaluation_id == evaluation_id
            )
        }) else {
            return InstallPlanStart::Unknown;
        };
        let expired = matches!(
            &self.workflows[position].phase,
            SkillWorkflowPhase::Planned { held, .. }
                if held.minted.elapsed() > INSTALL_PLAN_TTL
        );
        if expired {
            let phase = std::mem::replace(
                &mut self.workflows[position].phase,
                SkillWorkflowPhase::Candidate,
            );
            if let SkillWorkflowPhase::Planned { evaluated, .. } = phase {
                self.workflows[position].phase = SkillWorkflowPhase::Evaluated(evaluated);
            }
        }
        match &self.workflows[position].phase {
            SkillWorkflowPhase::Planned { held, .. } => {
                InstallPlanStart::Existing(Box::new(held.plan.preview().clone()))
            }
            SkillWorkflowPhase::Planning { .. } => InstallPlanStart::Unknown,
            SkillWorkflowPhase::Evaluated(_) => {
                let phase = std::mem::replace(
                    &mut self.workflows[position].phase,
                    SkillWorkflowPhase::Candidate,
                );
                let SkillWorkflowPhase::Evaluated(evaluated) = phase else {
                    return InstallPlanStart::Unknown;
                };
                let planning_id = Uuid::new_v4();
                let start = (
                    evaluated.report.clone(),
                    evaluated.review.clone(),
                    planning_id,
                );
                self.workflows[position].phase = SkillWorkflowPhase::Planning {
                    evaluated,
                    planning_id,
                };
                InstallPlanStart::Ready(Box::new(start))
            }
            _ => InstallPlanStart::Unknown,
        }
    }

    fn hold_install_plan(
        &mut self,
        evaluation_id: Uuid,
        planning_id: Uuid,
        held: HeldInstallPlan,
    ) -> Option<SkillInstallPlan> {
        let workflow = self.workflows.iter_mut().find(|workflow| {
            matches!(
                &workflow.phase,
                SkillWorkflowPhase::Planning {
                    evaluated,
                    planning_id: active_planning_id,
                } if evaluated.report.evaluation_id == evaluation_id
                    && *active_planning_id == planning_id
            )
        })?;
        let phase = std::mem::replace(&mut workflow.phase, SkillWorkflowPhase::Candidate);
        match phase {
            SkillWorkflowPhase::Planning { evaluated, .. } => {
                let plan = held.plan.preview().clone();
                workflow.phase = SkillWorkflowPhase::Planned { evaluated, held };
                Some(plan)
            }
            other => {
                workflow.phase = other;
                None
            }
        }
    }

    fn finish_planning(&mut self, planning_id: Uuid) {
        if let Some(workflow) = self.workflows.iter_mut().find(|workflow| {
            matches!(
                &workflow.phase,
                SkillWorkflowPhase::Planning {
                    planning_id: active_planning_id,
                    ..
                } if *active_planning_id == planning_id
            )
        }) {
            let phase = std::mem::replace(&mut workflow.phase, SkillWorkflowPhase::Candidate);
            if let SkillWorkflowPhase::Planning { evaluated, .. } = phase {
                workflow.phase = SkillWorkflowPhase::Evaluated(evaluated);
            }
        }
    }

    fn take_plan(&mut self, id: Uuid) -> Option<(CodexInstallPlan, SkillReview)> {
        let position = self.workflows.iter().position(|workflow| {
            matches!(
                &workflow.phase,
                SkillWorkflowPhase::Planned { held, .. } if held.plan.preview().plan_id == id
            )
        })?;
        let phase = std::mem::replace(
            &mut self.workflows[position].phase,
            SkillWorkflowPhase::Candidate,
        );
        let SkillWorkflowPhase::Planned { evaluated, held } = phase else {
            return None;
        };
        if held.minted.elapsed() > INSTALL_PLAN_TTL {
            self.workflows[position].phase = SkillWorkflowPhase::Evaluated(evaluated);
            return None;
        }
        let review = evaluated.review.clone();
        let plan = held.plan;
        self.workflows[position].phase = SkillWorkflowPhase::Installing {
            evaluated,
            plan_id: plan.preview().plan_id,
        };
        Some((plan, review))
    }

    fn finish_failed_install(&mut self, plan_id: Uuid) {
        if let Some(workflow) = self.workflows.iter_mut().find(|workflow| {
            matches!(
                &workflow.phase,
                SkillWorkflowPhase::Installing {
                    plan_id: active_plan_id,
                    ..
                } if *active_plan_id == plan_id
            )
        }) {
            let phase = std::mem::replace(&mut workflow.phase, SkillWorkflowPhase::Candidate);
            if let SkillWorkflowPhase::Installing { evaluated, .. } = phase {
                workflow.phase = SkillWorkflowPhase::Evaluated(evaluated);
            }
        }
    }

    fn insert_install(&mut self, value: CodexInstalledSkill) {
        let receipt = value.receipt();
        if let Some(position) = self.workflows.iter().position(|workflow| {
            workflow.candidate.source_submission_id == receipt.source_submission_id
        }) {
            let phase = std::mem::replace(
                &mut self.workflows[position].phase,
                SkillWorkflowPhase::Candidate,
            );
            self.workflows[position].phase = match phase {
                SkillWorkflowPhase::Installing { evaluated, .. }
                | SkillWorkflowPhase::Planned { evaluated, .. } => {
                    SkillWorkflowPhase::Evaluated(evaluated)
                }
                other => other,
            };
        }
        self.install_receipts
            .entry(receipt.source_submission_id)
            .or_insert(value);
    }

    fn install_receipt(&self, source_submission_id: Uuid) -> Option<CodexInstalledSkill> {
        self.install_receipts.get(&source_submission_id).cloned()
    }

    /// Binds status recovery to the first complete receipt this daemon saw.
    /// A later self-consistent marker is still a different installation and
    /// cannot replace the receipt that commit or an earlier recovery held.
    fn reconcile_install_status(
        &mut self,
        source_submission_id: Uuid,
        observed: Option<CodexInstalledSkill>,
    ) -> Result<Option<CodexInstalledSkill>, SkillInstallError> {
        let Some(observed) = observed else {
            self.clear_missing_install(source_submission_id);
            return Ok(None);
        };
        if observed.receipt().source_submission_id != source_submission_id {
            return Err(SkillInstallError::InstallNotOwned);
        }
        if let Some(receipt) = self.install_receipt(source_submission_id) {
            return (receipt == observed)
                .then_some(Some(receipt))
                .ok_or(SkillInstallError::InstallNotOwned);
        }
        // Active review/evaluation phases deliberately remain authoritative.
        // In settled phases this records the restart-recovered receipt; in an
        // active phase the caller may use this observation once without
        // discarding paid or in-flight work.
        self.insert_install(observed.clone());
        Ok(Some(observed))
    }

    fn restore_after_rollback(&mut self, source_submission_id: Uuid) {
        self.install_receipts.remove(&source_submission_id);
    }

    fn clear_missing_install(&mut self, source_submission_id: Uuid) {
        self.restore_after_rollback(source_submission_id);
    }
}

#[cfg(test)]
#[path = "skill_loop/tests.rs"]
mod tests;
