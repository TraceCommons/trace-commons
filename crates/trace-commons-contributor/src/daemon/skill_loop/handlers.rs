//! INTEGRATION: adapts tested-skill workflow state to daemon IPC, credential
//! lifecycle cancellation, live evaluation, and guarded Codex installation.

use std::future::Future;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::{Arc, OnceLock};

use serde::Deserialize;
use uuid::Uuid;

use crate::daemon::ipc::{DaemonShared, ERR_BAD_PARAMS, ERR_UNAVAILABLE, Request, Response};
use crate::skill_loop::SkillDraft;

use crate::daemon::skill_loop::{
    ERR_CANDIDATE_UNKNOWN, ERR_EVALUATION_BUSY, ERR_EVALUATION_DID_NOT_PASS,
    ERR_EVALUATION_UNKNOWN, ERR_INSTALL_PLAN_UNKNOWN, ERR_INSTALL_UNKNOWN, ERR_REVIEW_UNKNOWN,
    EvaluationStart, HeldInstallPlan, InstallPlanStart, SkillLoopState,
};

pub(super) const CREDENTIAL_SNAPSHOT_ATTEMPTS: usize = 3;

#[cfg(test)]
#[derive(Clone)]
pub(super) struct TestSkillLoopRuntime {
    pub(super) evaluate: Arc<
        dyn Fn(&crate::skill_loop::SkillReview) -> crate::skill_loop::SkillEvaluationReport
            + Send
            + Sync,
    >,
    pub(super) codex_skills_root: PathBuf,
    pub(super) before_install_mutation: Option<Arc<dyn Fn() + Send + Sync>>,
}

#[cfg(test)]
pub(super) struct TestSkillLoopRuntimeGuard {
    store_dir: PathBuf,
}

#[cfg(test)]
impl Drop for TestSkillLoopRuntimeGuard {
    fn drop(&mut self) {
        match test_skill_loop_runtimes().lock() {
            Ok(mut runtimes) => {
                runtimes.remove(&self.store_dir);
            }
            Err(poisoned) => {
                poisoned.into_inner().remove(&self.store_dir);
            }
        }
    }
}

#[cfg(test)]
fn test_skill_loop_runtimes() -> &'static Mutex<HashMap<PathBuf, TestSkillLoopRuntime>> {
    static RUNTIMES: OnceLock<Mutex<HashMap<PathBuf, TestSkillLoopRuntime>>> = OnceLock::new();
    RUNTIMES.get_or_init(Default::default)
}

#[cfg(test)]
pub(super) fn install_test_skill_loop_runtime(
    shared: &DaemonShared,
    runtime: TestSkillLoopRuntime,
) -> TestSkillLoopRuntimeGuard {
    let store_dir = shared.store.dir().to_path_buf();
    let prior = match test_skill_loop_runtimes().lock() {
        Ok(mut runtimes) => runtimes.insert(store_dir.clone(), runtime),
        Err(poisoned) => poisoned.into_inner().insert(store_dir.clone(), runtime),
    };
    assert!(prior.is_none(), "test skill-loop runtime already installed");
    TestSkillLoopRuntimeGuard { store_dir }
}

#[cfg(test)]
fn test_skill_loop_runtime(shared: &DaemonShared) -> Option<TestSkillLoopRuntime> {
    match test_skill_loop_runtimes().lock() {
        Ok(runtimes) => runtimes.get(shared.store.dir()).cloned(),
        Err(poisoned) => poisoned.into_inner().get(shared.store.dir()).cloned(),
    }
}

/// Extends the installer's filesystem lock through the matching in-memory
/// phase update so status, commit, and rollback cannot publish stale state.
static INSTALL_TRANSACTIONS: Mutex<()> = Mutex::new(());

struct EvaluationAdmission<'a> {
    shared: &'a DaemonShared,
    review_id: Uuid,
    lease: crate::daemon::skill_loop::OwnerLease,
}

struct InstallAdmission<'a> {
    shared: &'a DaemonShared,
    plan_id: Uuid,
    lease: crate::daemon::skill_loop::OwnerLease,
}

struct PlanningAdmission<'a> {
    shared: &'a DaemonShared,
    planning_id: Uuid,
    lease: crate::daemon::skill_loop::OwnerLease,
}

impl Drop for EvaluationAdmission<'_> {
    fn drop(&mut self) {
        let mut state = state(self.shared);
        if state.owner_matches(&self.lease) {
            state.finish_failed_evaluation(self.review_id);
        }
    }
}

impl Drop for InstallAdmission<'_> {
    fn drop(&mut self) {
        let mut state = state(self.shared);
        if state.owner_matches(&self.lease) {
            state.finish_failed_install(self.plan_id);
        }
    }
}

impl Drop for PlanningAdmission<'_> {
    fn drop(&mut self) {
        let mut state = state(self.shared);
        if state.owner_matches(&self.lease) {
            state.finish_planning(self.planning_id);
        }
    }
}

#[derive(Deserialize)]
struct CandidateParams {
    submission_id: Uuid,
}

#[derive(Deserialize)]
pub(super) struct ReviewParams {
    candidate_id: Uuid,
    draft: SkillDraft,
    pub(super) replaces_review_id: Option<Uuid>,
}

#[derive(Deserialize)]
struct EvaluationParams {
    review_id: Uuid,
    skill_sha256: String,
}

#[derive(Deserialize)]
struct EvaluationIDParams {
    evaluation_id: Uuid,
}

#[derive(Deserialize)]
struct CommitParams {
    plan_id: Uuid,
    as_previewed_sha256: String,
    as_previewed_marker_sha256: String,
}

#[derive(Deserialize)]
struct InstallParams {
    install_id: Uuid,
    source_submission_id: Uuid,
}

#[derive(Deserialize)]
struct InstallStatusParams {
    source_submission_id: Uuid,
}

fn state(shared: &DaemonShared) -> MutexGuard<'_, SkillLoopState> {
    match shared.skill_loop.lock() {
        Ok(state) => state,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn bind_current_owner(
    shared: &DaemonShared,
    request_id: u64,
) -> Result<crate::daemon::skill_loop::OwnerLease, Box<Response>> {
    let owner_scope = crate::daemon::public_run::current_account_binding(shared, request_id)?;
    Ok(state(shared).bind_owner(&owner_scope))
}

fn refresh_owner_lease(
    shared: &DaemonShared,
    request_id: u64,
    lease: &crate::daemon::skill_loop::OwnerLease,
) -> Result<(), Box<Response>> {
    let owner_scope = crate::daemon::public_run::current_account_binding(shared, request_id)?;
    let current = state(shared).bind_owner(&owner_scope);
    if current == *lease {
        Ok(())
    } else {
        Err(Box::new(Response::err(
            request_id,
            ERR_UNAVAILABLE,
            "skill-owner-changed",
        )))
    }
}

fn with_owner_state<T>(
    shared: &DaemonShared,
    request_id: u64,
    lease: &crate::daemon::skill_loop::OwnerLease,
    operation: impl FnOnce(&mut SkillLoopState) -> T,
) -> Result<T, Box<Response>> {
    let mut state = state(shared);
    if !state.owner_matches(lease) {
        return Err(Box::new(Response::err(
            request_id,
            ERR_UNAVAILABLE,
            "skill-owner-changed",
        )));
    }
    Ok(operation(&mut state))
}

#[cfg(test)]
fn before_install_mutation(shared: &DaemonShared) {
    if let Some(hook) =
        test_skill_loop_runtime(shared).and_then(|runtime| runtime.before_install_mutation)
    {
        hook();
    }
}

#[cfg(not(test))]
fn before_install_mutation(_shared: &DaemonShared) {}

fn compensate_committed_install(
    shared: &DaemonShared,
    request_id: u64,
    lease: &crate::daemon::skill_loop::OwnerLease,
    identity: &crate::identity::DeviceIdentity,
    installed: &crate::skill_loop::CodexInstalledSkill,
    original_failure: Response,
) -> Response {
    match crate::skill_loop::rollback_codex_install(installed, identity, lease.scope()) {
        Ok(outcome) if outcome.removed => original_failure,
        Ok(_) | Err(_) => {
            // A failed compensating rollback means the signed package may
            // still be loadable. Retain its receipt when this owner remains
            // current; after an owner switch, status recovers it from the
            // signed marker when that owner returns.
            let _receipt_was_retained = with_owner_state(shared, request_id, lease, |state| {
                state.insert_install(installed.clone())
            })
            .is_ok();
            Response::err(
                request_id,
                ERR_UNAVAILABLE,
                "skill-install-rollback-incomplete",
            )
        }
    }
}

fn install_identity(
    shared: &DaemonShared,
    request_id: u64,
) -> Result<crate::identity::DeviceIdentity, Box<Response>> {
    let identity = match crate::identity::DeviceIdentity::load(&shared.store) {
        Ok(Some(identity)) => identity,
        Ok(None) | Err(_) => {
            return Err(Box::new(Response::err(
                request_id,
                ERR_UNAVAILABLE,
                "skill-install-identity-unavailable",
            )));
        }
    };
    let config_matches = shared
        .store
        .load_config()
        .ok()
        .flatten()
        .is_some_and(|config| config.device_key_id == identity.device_key_id);
    if !config_matches {
        return Err(Box::new(Response::err(
            request_id,
            ERR_UNAVAILABLE,
            "skill-install-identity-unavailable",
        )));
    }
    Ok(identity)
}

pub(super) fn install_transaction() -> MutexGuard<'static, ()> {
    match INSTALL_TRANSACTIONS.lock() {
        Ok(transaction) => transaction,
        Err(poisoned) => poisoned.into_inner(),
    }
}

pub(in crate::daemon) async fn handle_candidate(shared: &DaemonShared, req: &Request) -> Response {
    let params: CandidateParams = match serde_json::from_value(req.params.clone()) {
        Ok(params) => params,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "submission_id-invalid"),
    };
    let (detail, owner_scope) = match crate::daemon::public_run::load_detail_with_binding(
        shared,
        req.id,
        params.submission_id,
    )
    .await
    {
        Ok(detail) => detail,
        Err(response) => return *response,
    };
    let candidate = match crate::skill_loop::propose_candidate(params.submission_id, &detail) {
        Ok(candidate) => candidate,
        Err(label) => return Response::err(req.id, ERR_BAD_PARAMS, label),
    };
    let lease = {
        let mut state = state(shared);
        state.bind_owner(&owner_scope)
    };
    let held = match with_owner_state(shared, req.id, &lease, |state| {
        state.hold_candidate(candidate)
    }) {
        Ok(held) => held,
        Err(response) => return *response,
    };
    let Some(candidate) = held else {
        return Response::err(req.id, ERR_UNAVAILABLE, "skill-workflow-capacity");
    };
    let response = match serde_json::to_value(&candidate) {
        Ok(value) => Response::ok(req.id, value),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "skill-candidate-unavailable"),
    };
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    response
}

pub(in crate::daemon) fn handle_review(shared: &DaemonShared, req: &Request) -> Response {
    let lease = match bind_current_owner(shared, req.id) {
        Ok(lease) => lease,
        Err(response) => return *response,
    };
    let params: ReviewParams = match serde_json::from_value(req.params.clone()) {
        Ok(params) => params,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "skill-review-invalid"),
    };
    let candidate = match with_owner_state(shared, req.id, &lease, |state| {
        state.candidate(params.candidate_id)
    }) {
        Ok(candidate) => candidate,
        Err(response) => return *response,
    };
    let Some(candidate) = candidate else {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_CANDIDATE_UNKNOWN);
    };
    let proposed = match crate::skill_loop::review_candidate(&candidate, params.draft) {
        Ok(review) => review,
        Err(error) => return Response::err(req.id, ERR_BAD_PARAMS, error.label()),
    };
    let review = match with_owner_state(shared, req.id, &lease, |state| {
        state.hold_review(proposed, params.replaces_review_id)
    }) {
        Ok(review) => review,
        Err(response) => return *response,
    };
    let Some(review) = review else {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_CANDIDATE_UNKNOWN);
    };
    let response = match serde_json::to_value(&review) {
        Ok(value) => Response::ok(req.id, value),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "skill-review-unavailable"),
    };
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    response
}

pub(in crate::daemon) async fn handle_evaluate(shared: &DaemonShared, req: &Request) -> Response {
    let lease = match bind_current_owner(shared, req.id) {
        Ok(lease) => lease,
        Err(response) => return *response,
    };
    let params: EvaluationParams = match serde_json::from_value(req.params.clone()) {
        Ok(params) => params,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "skill-evaluation-invalid"),
    };
    let review = match with_owner_state(shared, req.id, &lease, |state| {
        state.review(params.review_id)
    }) {
        Ok(review) => review,
        Err(response) => return *response,
    };
    let Some(review) = review else {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_REVIEW_UNKNOWN);
    };
    if review.skill_sha256 != params.skill_sha256 {
        return Response::err(req.id, ERR_BAD_PARAMS, "skill-review-changed");
    }
    let start = match with_owner_state(shared, req.id, &lease, |state| {
        state.begin_evaluation(review.review_id)
    }) {
        Ok(start) => start,
        Err(response) => return *response,
    };
    match start {
        EvaluationStart::Existing(report) => {
            let response = match serde_json::to_value(report) {
                Ok(value) => Response::ok(req.id, value),
                Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "skill-evaluation-unavailable"),
            };
            if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
                return *response;
            }
            return response;
        }
        EvaluationStart::InProgress => {
            return Response::err(req.id, ERR_UNAVAILABLE, "skill-evaluation-in-progress");
        }
        EvaluationStart::Busy => {
            return Response::err(req.id, ERR_UNAVAILABLE, ERR_EVALUATION_BUSY);
        }
        EvaluationStart::Started => {}
    }
    let _admission = EvaluationAdmission {
        shared,
        review_id: review.review_id,
        lease: lease.clone(),
    };
    let report = match evaluate_review(shared, &review).await {
        Ok(report) => report,
        Err(label) => return Response::err(req.id, ERR_UNAVAILABLE, label),
    };
    if report.review_id != review.review_id || report.skill_sha256 != review.skill_sha256 {
        return Response::err(
            req.id,
            ERR_UNAVAILABLE,
            crate::skill_loop::SkillEvaluationError::Internal.label(),
        );
    }
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    let response = match serde_json::to_value(&report) {
        Ok(value) => Response::ok(req.id, value),
        Err(_) => {
            return Response::err(req.id, ERR_UNAVAILABLE, "skill-evaluation-unavailable");
        }
    };
    if let Err(response) = with_owner_state(shared, req.id, &lease, |state| {
        state.insert_evaluation(report)
    }) {
        return *response;
    }
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    response
}

async fn evaluate_review(
    shared: &DaemonShared,
    review: &crate::skill_loop::SkillReview,
) -> Result<crate::skill_loop::SkillEvaluationReport, &'static str> {
    #[cfg(test)]
    if let Some(runtime) = test_skill_loop_runtime(shared) {
        return Ok((runtime.evaluate)(review));
    }

    let credential_dir = shared.store.dir().to_path_buf();
    let snapshot = consistent_credential_snapshot(
        || crate::daemon::nearai_credential::ceremony::change_count(&credential_dir),
        || shared.reconcile_private_inference(),
        || shared.absorbed_near_ai_credential_change_count(),
        || {
            let settings = match shared.settings.lock() {
                Ok(settings) => settings,
                Err(poisoned) => poisoned.into_inner(),
            };
            settings
                .near_ai_inference
                .as_ref()
                .map(|credential| credential.key.clone())
        },
    )
    .await;
    let Some((credential_change, api_key)) = snapshot else {
        return Err("skill-evaluation-credential-changed");
    };
    let Some(api_key) = api_key else {
        return Err(crate::skill_loop::SkillEvaluationError::CredentialRequired.label());
    };
    let evaluation = crate::skill_loop::evaluate_skill(api_key, review);
    let Some(result) =
        run_until_credential_change(credential_dir, credential_change, evaluation).await
    else {
        return Err("skill-evaluation-credential-changed");
    };
    result.map_err(crate::skill_loop::SkillEvaluationError::label)
}

pub(in crate::daemon) fn handle_install_plan(shared: &DaemonShared, req: &Request) -> Response {
    let lease = match bind_current_owner(shared, req.id) {
        Ok(lease) => lease,
        Err(response) => return *response,
    };
    let identity = match install_identity(shared, req.id) {
        Ok(identity) => identity,
        Err(response) => return *response,
    };
    let params: EvaluationIDParams = match serde_json::from_value(req.params.clone()) {
        Ok(params) => params,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "evaluation_id-invalid"),
    };
    let _transaction = install_transaction();
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    let start = match with_owner_state(shared, req.id, &lease, |state| {
        state.begin_install_plan(params.evaluation_id)
    }) {
        Ok(start) => start,
        Err(response) => return *response,
    };
    let (evaluation, review, planning_id) = match start {
        InstallPlanStart::Existing(plan) => {
            let response = match serde_json::to_value(plan) {
                Ok(value) => Response::ok(req.id, value),
                Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "skill-install-plan-unavailable"),
            };
            if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
                return *response;
            }
            return response;
        }
        InstallPlanStart::Ready(values) => *values,
        InstallPlanStart::Unknown => {
            return Response::err(req.id, ERR_BAD_PARAMS, ERR_EVALUATION_UNKNOWN);
        }
    };
    let _admission = PlanningAdmission {
        shared,
        planning_id,
        lease: lease.clone(),
    };
    if !evaluation.install_allowed || evaluation.skill_sha256 != review.skill_sha256 {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_EVALUATION_DID_NOT_PASS);
    }
    let root = match codex_skills_root(shared) {
        Ok(root) => root,
        Err(error) => return Response::err(req.id, ERR_UNAVAILABLE, error.label()),
    };
    let plan = match crate::skill_loop::plan_codex_install(
        &review,
        evaluation.evaluation_id,
        &root,
        &identity,
        lease.scope(),
    ) {
        Ok(plan) => plan,
        Err(error) => return Response::err(req.id, ERR_UNAVAILABLE, error.label()),
    };
    let preview = if plan.preview().can_install {
        if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
            return *response;
        }
        match with_owner_state(shared, req.id, &lease, |state| {
            state.hold_install_plan(
                evaluation.evaluation_id,
                planning_id,
                HeldInstallPlan {
                    plan,
                    minted: Instant::now(),
                },
            )
        }) {
            Err(response) => return *response,
            Ok(Some(plan)) => plan,
            Ok(None) => return Response::err(req.id, ERR_UNAVAILABLE, ERR_EVALUATION_UNKNOWN),
        }
    } else {
        plan.preview().clone()
    };
    let response = match serde_json::to_value(&preview) {
        Ok(value) => Response::ok(req.id, value),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "skill-install-plan-unavailable"),
    };
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    response
}

pub(in crate::daemon) fn handle_install_commit(shared: &DaemonShared, req: &Request) -> Response {
    let lease = match bind_current_owner(shared, req.id) {
        Ok(lease) => lease,
        Err(response) => return *response,
    };
    let identity = match install_identity(shared, req.id) {
        Ok(identity) => identity,
        Err(response) => return *response,
    };
    let params: CommitParams = match serde_json::from_value(req.params.clone()) {
        Ok(params) => params,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "skill-install-commit-invalid"),
    };
    let _transaction = install_transaction();
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    let held = match with_owner_state(shared, req.id, &lease, |state| {
        state.take_plan(params.plan_id)
    }) {
        Ok(held) => held,
        Err(response) => return *response,
    };
    let Some((plan, review)) = held else {
        return Response::err(req.id, ERR_BAD_PARAMS, ERR_INSTALL_PLAN_UNKNOWN);
    };
    let _admission = InstallAdmission {
        shared,
        plan_id: plan.preview().plan_id,
        lease: lease.clone(),
    };
    before_install_mutation(shared);
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    let installed = match crate::skill_loop::commit_codex_install(
        &plan,
        &review,
        &identity,
        lease.scope(),
        &params.as_previewed_sha256,
        &params.as_previewed_marker_sha256,
    ) {
        Ok(installed) => installed,
        Err(error) => return Response::err(req.id, ERR_UNAVAILABLE, error.label()),
    };
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return compensate_committed_install(
            shared, req.id, &lease, &identity, &installed, *response,
        );
    }
    let response = match serde_json::to_value(installed.receipt()) {
        Ok(value) => Response::ok(req.id, value),
        Err(_) => {
            return compensate_committed_install(
                shared,
                req.id,
                &lease,
                &identity,
                &installed,
                Response::err(req.id, ERR_UNAVAILABLE, "skill-install-commit-unavailable"),
            );
        }
    };
    if let Err(response) = with_owner_state(shared, req.id, &lease, |state| {
        state.insert_install(installed.clone())
    }) {
        return compensate_committed_install(
            shared, req.id, &lease, &identity, &installed, *response,
        );
    }
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return compensate_committed_install(
            shared, req.id, &lease, &identity, &installed, *response,
        );
    }
    response
}

pub(in crate::daemon) fn handle_install_status(shared: &DaemonShared, req: &Request) -> Response {
    let lease = match bind_current_owner(shared, req.id) {
        Ok(lease) => lease,
        Err(response) => return *response,
    };
    let identity = match install_identity(shared, req.id) {
        Ok(identity) => identity,
        Err(response) => return *response,
    };
    let params: InstallStatusParams = match serde_json::from_value(req.params.clone()) {
        Ok(params) => params,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "skill-install-status-invalid"),
    };
    let _transaction = install_transaction();
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    let root = match codex_skills_root(shared) {
        Ok(root) => root,
        Err(error) => return Response::err(req.id, ERR_UNAVAILABLE, error.label()),
    };
    let observed = match crate::skill_loop::codex_install_status_for_submission(
        params.source_submission_id,
        &root,
        &identity,
        lease.scope(),
    ) {
        Ok(observed) => observed,
        Err(error) => return Response::err(req.id, ERR_UNAVAILABLE, error.label()),
    };
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    let installed = match with_owner_state(shared, req.id, &lease, |state| {
        state.reconcile_install_status(params.source_submission_id, observed)
    }) {
        Ok(result) => match result {
            Ok(installed) => installed,
            Err(error) => return Response::err(req.id, ERR_UNAVAILABLE, error.label()),
        },
        Err(response) => return *response,
    };
    let response = match installed {
        Some(installed) => match serde_json::to_value(installed.receipt()) {
            Ok(value) => Response::ok(
                req.id,
                serde_json::json!({ "installed": true, "skill": value }),
            ),
            Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "skill-install-status-unavailable"),
        },
        None => Response::ok(
            req.id,
            serde_json::json!({ "installed": false, "skill": null }),
        ),
    };
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    response
}

pub(in crate::daemon) fn handle_rollback(shared: &DaemonShared, req: &Request) -> Response {
    let lease = match bind_current_owner(shared, req.id) {
        Ok(lease) => lease,
        Err(response) => return *response,
    };
    let identity = match install_identity(shared, req.id) {
        Ok(identity) => identity,
        Err(response) => return *response,
    };
    let params: InstallParams = match serde_json::from_value(req.params.clone()) {
        Ok(params) => params,
        Err(_) => return Response::err(req.id, ERR_BAD_PARAMS, "install_id-invalid"),
    };
    let _transaction = install_transaction();
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    let root = match codex_skills_root(shared) {
        Ok(root) => root,
        Err(error) => return Response::err(req.id, ERR_UNAVAILABLE, error.label()),
    };
    let observed = match crate::skill_loop::codex_install_status_for_submission(
        params.source_submission_id,
        &root,
        &identity,
        lease.scope(),
    ) {
        Ok(installed) => installed,
        Err(error) => return Response::err(req.id, ERR_UNAVAILABLE, error.label()),
    };
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    let reconciled = match with_owner_state(shared, req.id, &lease, |state| {
        state.reconcile_install_status(params.source_submission_id, observed)
    }) {
        Ok(reconciled) => reconciled,
        Err(response) => return *response,
    };
    let installed = match reconciled {
        Ok(Some(installed)) if installed.receipt().install_id == params.install_id => installed,
        Ok(Some(_)) | Ok(None) => {
            return Response::err(req.id, ERR_BAD_PARAMS, ERR_INSTALL_UNKNOWN);
        }
        Err(error) => return Response::err(req.id, ERR_UNAVAILABLE, error.label()),
    };
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    let result =
        match crate::skill_loop::rollback_codex_install(&installed, &identity, lease.scope()) {
            Ok(result) => result,
            Err(error) => return Response::err(req.id, ERR_UNAVAILABLE, error.label()),
        };
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    if let Err(response) = with_owner_state(shared, req.id, &lease, |state| {
        state.restore_after_rollback(installed.receipt().source_submission_id)
    }) {
        return *response;
    }
    let response = match serde_json::to_value(result) {
        Ok(value) => Response::ok(req.id, value),
        Err(_) => Response::err(req.id, ERR_UNAVAILABLE, "skill-rollback-unavailable"),
    };
    if let Err(response) = refresh_owner_lease(shared, req.id, &lease) {
        return *response;
    }
    response
}

fn codex_skills_root(
    _shared: &DaemonShared,
) -> Result<PathBuf, crate::skill_loop::SkillInstallError> {
    #[cfg(test)]
    if let Some(runtime) = test_skill_loop_runtime(_shared) {
        return Ok(runtime.codex_skills_root);
    }

    crate::skill_loop::codex_skills_root()
}

async fn wait_for_credential_change(dir: PathBuf, observed: u64) {
    while crate::daemon::nearai_credential::ceremony::change_count(&dir) == observed {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(super) async fn consistent_credential_snapshot<
    Count,
    Reconcile,
    ReconcileFuture,
    Absorbed,
    ReadKey,
>(
    mut change_count: Count,
    mut reconcile: Reconcile,
    mut absorbed_count: Absorbed,
    mut read_key: ReadKey,
) -> Option<(u64, Option<String>)>
where
    Count: FnMut() -> u64,
    Reconcile: FnMut() -> ReconcileFuture,
    ReconcileFuture: Future<Output = ()>,
    Absorbed: FnMut() -> u64,
    ReadKey: FnMut() -> Option<String>,
{
    for _ in 0..CREDENTIAL_SNAPSHOT_ATTEMPTS {
        let before = change_count();
        reconcile().await;
        let absorbed = absorbed_count();
        let key = read_key();
        let after = change_count();
        if before == after && absorbed == after {
            return Some((after, key));
        }
    }
    None
}

pub(super) async fn run_until_credential_change<F>(
    dir: PathBuf,
    observed: u64,
    operation: F,
) -> Option<F::Output>
where
    F: Future,
{
    tokio::pin!(operation);
    let credential_changed = wait_for_credential_change(dir.clone(), observed);
    tokio::pin!(credential_changed);
    tokio::select! {
        biased;
        () = &mut credential_changed => None,
        result = &mut operation => {
            (crate::daemon::nearai_credential::ceremony::change_count(&dir) == observed)
                .then_some(result)
        },
    }
}
