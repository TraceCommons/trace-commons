//! Pure first-run navigation and ordered commits, shared by the GTK coordinator.
//! This module performs no IO. A successful transport call is not enough for
//! actions with an acknowledgement contract: the coordinator validates the
//! daemon's response before resolving a ticket as successful.
use std::collections::{BTreeMap, VecDeque};

use serde_json::{Value, json};

pub type Roots = BTreeMap<String, Value>;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Step {
    #[default]
    Welcome,
    Tools,
    PrivateAi,
    Traces,
    Uses,
    Terms,
    Join,
}
impl Step {
    pub const ALL: [Self; 7] = [
        Self::Welcome,
        Self::Tools,
        Self::PrivateAi,
        Self::Traces,
        Self::Uses,
        Self::Terms,
        Self::Join,
    ];
    pub fn next(self) -> Self {
        Self::ALL[(self.index() + 1).min(Self::ALL.len() - 1)]
    }
    pub fn previous(self) -> Self {
        Self::ALL[self.index().saturating_sub(1)]
    }
    fn index(self) -> usize {
        Self::ALL
            .iter()
            .position(|step| *step == self)
            .expect("known step")
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Account {
    #[default]
    None,
    WatchOnly,
    NearAi,
    Enrolled,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Sharing {
    #[default]
    Ask,
    Automatic,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Rule {
    Ask,
    Automatic,
    Never,
}
impl Rule {
    pub fn wire(self) -> &'static str {
        match self {
            Self::Ask => "notify_only",
            Self::Automatic => "auto_upload",
            Self::Never => "ignore",
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Commit {
    Roots,
    SignIn,
    Start,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    StartDaemon(Roots),
    SetSourceSettings(Roots),
    LookupInvite(String),
    Enroll(String),
    SignInNearAi,
    NearAiLogin,
    EnrollNearAi,
    SetAskEachTime,
    SetConsentScopes(Vec<String>),
    SetProjectMode {
        project_id: String,
        mode: Rule,
    },
    IncludePastSessions {
        project_id: String,
        session_ids: Vec<String>,
    },
    SetPrivateAi(bool),
    GrantAutomatic {
        witness: Option<String>,
    },
    MarkComplete,
    MarkWatchOnlyComplete,
}
impl Action {
    /// Only direct IPC mappings. Startup, browser ceremonies, enrollment
    /// completion and watch-only markers require the coordinator's adapters.
    pub fn ipc(&self) -> Option<(&'static str, Value)> {
        Some(match self {
            Self::SetSourceSettings(roots) => ("set_settings", json!(roots)),
            Self::LookupInvite(invite) => ("invite_lookup", json!({"code":invite})),
            Self::Enroll(invite) => ("enroll", json!({"invite":invite})),
            Self::SetAskEachTime => (
                "withdraw_automatic_grant",
                json!({"return_folders_to_ask":true}),
            ),
            Self::SetConsentScopes(scopes) => ("set_consent_scopes", json!({"scopes":scopes})),
            Self::SetProjectMode { project_id, mode } => (
                "set_project_mode",
                json!({"project_id":project_id,"mode":mode.wire()}),
            ),
            Self::IncludePastSessions {
                project_id,
                session_ids,
            } => (
                "include_past_sessions",
                json!({"project_id":project_id,"session_ids":session_ids}),
            ),
            Self::SetPrivateAi(on) => (
                "set_settings",
                json!({"private_inference":on,"private_inference_offer_seen":true}),
            ),
            Self::GrantAutomatic { witness } => (
                "grant_automatic_including_existing",
                json!({"confirmed":true,"witness_signing_address":witness,"include_existing":true,"keep_folder_asks":true}),
            ),
            _ => return None,
        })
    }
}

/// Validate the daemon's action-specific acknowledgement before advancing.
/// Authentication and startup use separate coordinator adapters and cannot be
/// acknowledged by an arbitrary JSON success response.
pub fn acknowledged(action: &Action, value: &Value) -> bool {
    match action {
        Action::SetSourceSettings(roots) => {
            !roots.is_empty()
                && roots
                    .iter()
                    .all(|(key, expected)| value.get(key) == Some(expected))
        }
        Action::SetAskEachTime => value.get("withdrawn").and_then(Value::as_bool).is_some(),
        Action::SetConsentScopes(requested) => {
            let Ok(expected) = trace_commons_contributor::consent::validate_scopes(requested)
            else {
                return false;
            };
            let Some(actual) = value.get("consent_scopes").and_then(Value::as_array) else {
                return false;
            };
            let Some(mut actual) = actual.iter().map(Value::as_str).collect::<Option<Vec<_>>>()
            else {
                return false;
            };
            actual.sort_unstable();
            actual.dedup();
            let mut expected: Vec<_> = expected.iter().map(String::as_str).collect();
            expected.sort_unstable();
            actual == expected
        }
        Action::SetProjectMode { mode, .. } => {
            // This endpoint returns a commit result, not a project/mode echo.
            value.get("ok").and_then(Value::as_bool) == Some(true)
                && value
                    .get("overridden_by")
                    .is_some_and(|v| v.is_null() || v.as_str() == Some(mode.wire()))
        }
        Action::IncludePastSessions { session_ids, .. } => {
            let unique: std::collections::BTreeSet<_> = session_ids.iter().collect();
            let Some(approved) = value.get("approved").and_then(Value::as_u64) else {
                return false;
            };
            let Some(skipped) = value.get("skipped").and_then(Value::as_array) else {
                return false;
            };
            let Some(entries) = value.get("approved_entry_ids").and_then(Value::as_array) else {
                return false;
            };
            !unique.is_empty()
                && approved == unique.len() as u64
                && skipped.is_empty()
                && entries.len() == unique.len()
                && entries.iter().all(|id| {
                    id.as_str()
                        .is_some_and(|s| uuid::Uuid::parse_str(s).is_ok())
                })
                && entries
                    .iter()
                    .filter_map(Value::as_str)
                    .collect::<std::collections::BTreeSet<_>>()
                    .len()
                    == entries.len()
        }
        Action::SetPrivateAi(on) => {
            value.get("private_inference").and_then(Value::as_bool) == Some(*on)
                && value
                    .get("private_inference_offer_seen")
                    .and_then(Value::as_bool)
                    == Some(true)
        }
        Action::GrantAutomatic { .. } => {
            value.get("granted").and_then(Value::as_bool) == Some(true)
                && value.get("include_existing").and_then(Value::as_bool) == Some(true)
        }
        Action::MarkComplete | Action::MarkWatchOnlyComplete => {
            value.get("complete").and_then(Value::as_bool) == Some(true)
        }
        _ => false,
    }
}

#[derive(Clone, Debug, Default)]
pub struct State {
    pub step: Step,
    pub account: Account,
    pub sharing: Sharing,
    /// Only validated explicit source declarations are placed here by Tools.
    pub roots: Roots,
    pub committed_roots: Option<Roots>,
    pub daemon_started: bool,
    /// Last fresh daemon status, independent of the currently selected answer.
    pub enrollment_held: bool,
    /// The answer has been signed out of, but the daemon may retain enrollment
    /// until its explicit unenroll operation has succeeded.
    pub signed_out_of_enrollment: bool,
    pub invite: String,
    pub enrolled_invite: Option<String>,
    pub signed_in: bool,
    pub near_ai_enrolled: bool,
    pub scopes: Vec<String>,
    pub rules: BTreeMap<String, Rule>,
    pub past_selections: BTreeMap<String, Vec<String>>,
    pub private_ai: bool,
    pub grant_ready: bool,
    pub witness: Option<String>,
    pub completed: bool,
}
impl State {
    pub fn next(&mut self) {
        self.step = self.step.next();
    }
    pub fn back(&mut self) {
        self.step = self.step.previous();
        self.grant_ready = false;
    }
    pub fn back_to(&mut self, step: Step) {
        if step.index() < self.step.index() {
            self.step = step;
            self.grant_ready = false;
        }
    }
    pub fn holds_enrollment(&self) -> bool {
        !self.signed_out_of_enrollment
            && match self.account {
                Account::Enrolled => self.enrollment_held,
                Account::NearAi => {
                    self.enrollment_held || self.enrolled_invite.is_some() || self.near_ai_enrolled
                }
                Account::None | Account::WatchOnly => false,
            }
    }
    pub fn daemon_holds_enrollment(&self) -> bool {
        self.enrollment_held
            || self.signed_out_of_enrollment
            || self.enrolled_invite.is_some()
            || self.near_ai_enrolled
    }
    pub fn plan(&self, commit: Commit) -> Vec<Action> {
        match commit {
            Commit::Roots => self.root_calls(),
            Commit::SignIn => {
                if self.roots.is_empty() {
                    return vec![];
                }
                let mut calls = self.root_calls();
                let invite = self.invite.trim();
                let joins_invite = self.account != Account::WatchOnly && !invite.is_empty();
                if joins_invite && self.enrolled_invite.as_deref() != Some(invite) {
                    calls.push(Action::LookupInvite(invite.into()));
                    calls.push(Action::Enroll(invite.into()));
                }
                if self.account == Account::NearAi {
                    if joins_invite || self.enrolled_invite.is_some() {
                        if !self.signed_in {
                            calls.push(Action::SignInNearAi);
                        }
                    } else if !self.near_ai_enrolled {
                        calls.extend([Action::NearAiLogin, Action::EnrollNearAi]);
                    }
                }
                calls
            }
            Commit::Start => self.start_calls(),
        }
    }
    fn root_calls(&self) -> Vec<Action> {
        if self.roots.is_empty() {
            return vec![];
        }
        if !self.daemon_started {
            return vec![Action::StartDaemon(self.roots.clone())];
        }
        let mut changes: Roots = self
            .roots
            .iter()
            .filter(|(key, value)| {
                self.committed_roots.as_ref().and_then(|old| old.get(*key)) != Some(*value)
            })
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        if let Some(before) = &self.committed_roots {
            for key in before.keys().filter(|key| {
                !self.roots.contains_key(*key)
                    && before[*key].get("mode").and_then(Value::as_str) != Some("off")
            }) {
                changes.insert(key.clone(), json!({"mode":"off"}));
            }
        }
        if changes.is_empty() {
            vec![]
        } else {
            vec![Action::SetSourceSettings(changes)]
        }
    }
    fn start_calls(&self) -> Vec<Action> {
        let watching = self.account == Account::WatchOnly;
        if (watching && self.daemon_holds_enrollment()) || (!watching && !self.holds_enrollment()) {
            return vec![];
        }
        let mut calls = Vec::new();
        if !watching {
            if self.sharing == Sharing::Ask {
                calls.push(Action::SetAskEachTime);
            }
            let mut scopes = self.scopes.clone();
            scopes.sort();
            scopes.dedup();
            calls.push(Action::SetConsentScopes(scopes));
        }
        let grants =
            self.sharing == Sharing::Automatic && self.grant_ready && self.holds_enrollment();
        let after = |id: &str| grants && self.rules.get(id) == Some(&Rule::Ask);
        let mut later = Vec::new();
        let mut later_includes = Vec::new();
        for (project_id, mode) in &self.rules {
            if *mode == Rule::Automatic && !self.holds_enrollment() {
                continue;
            }
            let action = Action::SetProjectMode {
                project_id: project_id.clone(),
                mode: *mode,
            };
            if after(project_id) {
                later.push(action);
            } else {
                calls.push(action);
            }
        }
        for (project_id, sessions) in &self.past_selections {
            if self.rules.get(project_id) == Some(&Rule::Never) || sessions.is_empty() {
                continue;
            }
            let mut session_ids = sessions.clone();
            session_ids.sort();
            session_ids.dedup();
            let action = Action::IncludePastSessions {
                project_id: project_id.clone(),
                session_ids,
            };
            if after(project_id) {
                later_includes.push(action);
            } else {
                calls.push(action);
            }
        }
        if self.private_ai {
            calls.push(Action::SetPrivateAi(true));
        }
        if grants {
            calls.push(Action::GrantAutomatic {
                witness: self.witness.clone(),
            });
        }
        calls.extend(later);
        calls.extend(later_includes);
        calls.push(if watching {
            Action::MarkWatchOnlyComplete
        } else {
            Action::MarkComplete
        });
        calls
    }
    /// Called only for a confirmed successful action from the current ticket.
    pub fn apply_success(&mut self, action: &Action) {
        match action {
            Action::StartDaemon(roots) => {
                self.daemon_started = true;
                self.committed_roots = Some(roots.clone());
            }
            Action::SetSourceSettings(changes) => {
                let mut roots = self.committed_roots.clone().unwrap_or_default();
                roots.extend(changes.clone());
                self.committed_roots = Some(roots);
            }
            Action::Enroll(invite) => {
                self.enrolled_invite = Some(invite.clone());
                self.enrollment_held = true;
                self.signed_out_of_enrollment = false;
            }
            Action::SignInNearAi => self.signed_in = true,
            Action::EnrollNearAi => {
                self.near_ai_enrolled = true;
                self.signed_in = true;
                self.enrollment_held = true;
                self.signed_out_of_enrollment = false;
            }
            Action::MarkComplete | Action::MarkWatchOnlyComplete => self.completed = true,
            _ => {}
        }
    }
}

/// The sequence number is part of the ticket: a duplicate callback for an
/// earlier action in this same commit cannot complete the following action.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ticket {
    pub generation: u64,
    pub sequence: usize,
    pub action: Action,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Success,
    Failure(String),
    Cancelled,
    GrantRefused(String),
}
#[derive(Default)]
pub struct Runner {
    generation: u64,
    sequence: usize,
    queue: VecDeque<Action>,
    current: Option<Ticket>,
    pub failure: Option<String>,
    pub refused_grant: Option<String>,
}
impl Runner {
    pub fn busy(&self) -> bool {
        self.current.is_some()
    }
    pub fn begin(&mut self, state: &State, commit: Commit) -> Option<Ticket> {
        if self.busy() {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.sequence = 0;
        self.failure = None;
        self.queue = state.plan(commit).into();
        self.advance()
    }
    fn advance(&mut self) -> Option<Ticket> {
        self.current = self.queue.pop_front().map(|action| Ticket {
            generation: self.generation,
            sequence: self.sequence,
            action,
        });
        self.sequence += 1;
        self.current.clone()
    }
    pub fn cancel(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.current = None;
        self.queue.clear();
    }
    pub fn resolve(
        &mut self,
        ticket: &Ticket,
        state: &mut State,
        outcome: Outcome,
    ) -> Option<Ticket> {
        if self.current.as_ref() != Some(ticket) {
            return None;
        }
        match outcome {
            Outcome::Success => {
                state.apply_success(&ticket.action);
                if matches!(ticket.action, Action::GrantAutomatic { .. }) {
                    self.refused_grant = None;
                }
            }
            Outcome::GrantRefused(label)
                if matches!(ticket.action, Action::GrantAutomatic { .. }) =>
            {
                self.refused_grant = Some(label);
                state.sharing = Sharing::Ask;
                state.grant_ready = false;
                // Refusal is not completion until the fallback Ask is confirmed.
                self.queue.push_front(Action::SetAskEachTime);
            }
            Outcome::Failure(label) | Outcome::GrantRefused(label) => {
                self.failure = Some(label);
                match ticket.action {
                    Action::LookupInvite(_) => state.step = Step::Join,
                    Action::SignInNearAi | Action::NearAiLogin | Action::EnrollNearAi => {
                        state.step = Step::Join;
                        if state.account == Account::NearAi {
                            state.account = Account::None;
                        }
                    }
                    _ => {}
                }
                self.current = None;
                self.queue.clear();
                return None;
            }
            Outcome::Cancelled => {
                self.cancel();
                return None;
            }
        }
        self.advance()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acknowledgements_reject_generic_success_and_partial_results() {
        let action = Action::IncludePastSessions {
            project_id: "p".into(),
            session_ids: vec!["s".into()],
        };
        let good = json!({"approved":1,"skipped":[],"approved_entry_ids":["00000000-0000-0000-0000-000000000001"],"hold_until":null});
        assert!(acknowledged(&action, &good));
        assert!(!acknowledged(
            &action,
            &json!({"approved":0,"skipped":[{"session_id":"s","label":"session-still-active"}],"approved_entry_ids":[]})
        ));
        let grant = Action::GrantAutomatic { witness: None };
        assert!(acknowledged(
            &grant,
            &json!({"granted":true,"include_existing":true})
        ));
        assert!(!acknowledged(
            &grant,
            &json!({"granted":true,"include_existing":false})
        ));
        assert!(!acknowledged(&grant, &json!({"ok":true})));
        assert!(acknowledged(
            &Action::SetAskEachTime,
            &json!({"withdrawn":false})
        ));
        assert!(!acknowledged(&Action::SetAskEachTime, &json!({"ok":true})));
        let project = Action::SetProjectMode {
            project_id: "p".into(),
            mode: Rule::Ask,
        };
        assert!(acknowledged(
            &project,
            &json!({"ok":true,"overridden_by":null})
        ));
        assert!(!acknowledged(
            &project,
            &json!({"ok":true,"overridden_by":"auto_upload"})
        ));
        assert!(!acknowledged(
            &project,
            &json!({"ok":false,"overridden_by":null})
        ));
        assert!(acknowledged(
            &Action::SetPrivateAi(true),
            &json!({"private_inference":true,"private_inference_offer_seen":true})
        ));
        assert!(!acknowledged(
            &Action::SetPrivateAi(true),
            &json!({"private_inference":false,"private_inference_offer_seen":true})
        ));
    }

    #[test]
    fn scope_acknowledgement_allows_floor_but_not_unrequested_uses() {
        let action = Action::SetConsentScopes(vec!["benchmark_only".into()]);
        assert!(acknowledged(
            &action,
            &json!({"consent_scopes":["benchmark_only","debugging_evaluation"]})
        ));
        assert!(!acknowledged(
            &action,
            &json!({"consent_scopes":["debugging_evaluation"]})
        ));
        assert!(!acknowledged(
            &action,
            &json!({"consent_scopes":["benchmark_only","debugging_evaluation","model_training"]})
        ));
        assert!(!acknowledged(&action, &json!({"consent_scopes":[true]})));
    }

    fn ready() -> State {
        State {
            roots: BTreeMap::from([(
                "codex_source".into(),
                json!({"mode":"watch","path":"/fixtures/codex"}),
            )]),
            daemon_started: true,
            enrollment_held: true,
            account: Account::Enrolled,
            scopes: vec!["training".into(), "research".into()],
            ..Default::default()
        }
    }
    #[test]
    fn successful_regrant_clears_prior_refusal() {
        let mut state = ready();
        state.sharing = Sharing::Automatic;
        state.grant_ready = true;
        let mut runner = Runner {
            refused_grant: Some("old-refusal".into()),
            ..Runner::default()
        };
        let mut ticket = runner.begin(&state, Commit::Start);
        while let Some(current) = ticket {
            ticket = runner.resolve(&current, &mut state, Outcome::Success);
        }
        assert!(state.completed);
        assert!(runner.refused_grant.is_none());
    }

    #[test]
    fn action_wire_matches_daemon_and_keeps_witness_explicit() {
        assert_eq!(
            Action::SetProjectMode {
                project_id: "p".into(),
                mode: Rule::Ask
            }
            .ipc(),
            Some((
                "set_project_mode",
                json!({"project_id":"p","mode":"notify_only"})
            ))
        );
        assert_eq!(
            Action::SetPrivateAi(true).ipc(),
            Some((
                "set_settings",
                json!({"private_inference":true,"private_inference_offer_seen":true})
            ))
        );
    }

    #[test]
    fn navigation_retains_answers_but_clears_readiness() {
        let mut state = ready();
        state.step = Step::Join;
        state.grant_ready = true;
        state.back();
        assert_eq!(state.step, Step::Terms);
        assert!(!state.grant_ready);
        assert_eq!(state.scopes.len(), 2);
        state.next();
        assert_eq!(state.step, Step::Join);
        state.back_to(Step::Tools);
        assert_eq!(state.step, Step::Tools);
        state.back_to(Step::Join);
        assert_eq!(state.step, Step::Tools);
    }
    #[test]
    fn roots_send_only_changes_and_explicitly_disable_withdrawn_sources() {
        let mut state = ready();
        state.committed_roots = Some(state.roots.clone());
        assert!(state.plan(Commit::Roots).is_empty());
        state.committed_roots.as_mut().unwrap().insert(
            "claude_source".into(),
            json!({"mode":"watch","path":"/old"}),
        );
        assert_eq!(
            state.plan(Commit::Roots),
            vec![Action::SetSourceSettings(BTreeMap::from([(
                "claude_source".into(),
                json!({"mode":"off"})
            )]))]
        );
    }
    #[test]
    fn sign_in_retry_does_not_restart_or_enroll_again() {
        let mut state = State {
            account: Account::NearAi,
            invite: " invite ".into(),
            roots: ready().roots,
            ..Default::default()
        };
        let actions = state.plan(Commit::SignIn);
        assert!(matches!(
            actions.as_slice(),
            [
                Action::StartDaemon(_),
                Action::LookupInvite(_),
                Action::Enroll(_),
                Action::SignInNearAi
            ]
        ));
        for action in &actions[..3] {
            state.apply_success(action);
        }
        assert_eq!(state.plan(Commit::SignIn), vec![Action::SignInNearAi]);
    }
    #[test]
    fn automatic_keeps_explicit_ask_after_grant_and_never_excludes_history() {
        let mut state = ready();
        state.sharing = Sharing::Automatic;
        state.grant_ready = true;
        state.rules = BTreeMap::from([
            ("a".into(), Rule::Ask),
            ("b".into(), Rule::Never),
            ("c".into(), Rule::Automatic),
        ]);
        for key in ["a", "b", "c"] {
            state
                .past_selections
                .insert(key.into(), vec!["session".into()]);
        }
        let actions = state.plan(Commit::Start);
        let grant = actions
            .iter()
            .position(|a| matches!(a, Action::GrantAutomatic { .. }))
            .unwrap();
        let ask = actions.iter().position(|a| matches!(a, Action::SetProjectMode {project_id, mode:Rule::Ask} if project_id == "a")).unwrap();
        assert!(ask > grant);
        assert!(!actions.iter().any(
            |a| matches!(a, Action::IncludePastSessions {project_id,..} if project_id == "b")
        ));
        assert!(matches!(actions.last(), Some(Action::MarkComplete)));
        let (_, params) = actions[grant].ipc().unwrap();
        assert!(params["witness_signing_address"].is_null());
        assert_eq!(params["keep_folder_asks"], true);
    }
    #[test]
    fn watch_only_never_arms_and_refuses_retained_enrollment() {
        let mut state = ready();
        state.account = Account::WatchOnly;
        assert!(state.plan(Commit::Start).is_empty());
        state.enrollment_held = false;
        state.sharing = Sharing::Automatic;
        state.grant_ready = true;
        state.rules.insert("a".into(), Rule::Automatic);
        assert_eq!(
            state.plan(Commit::Start),
            vec![Action::MarkWatchOnlyComplete]
        );
        state.enrolled_invite = Some("retained".into());
        assert!(state.plan(Commit::Start).is_empty());
    }
    #[test]
    fn selecting_near_ai_is_not_an_enrollment() {
        let state = State {
            account: Account::NearAi,
            grant_ready: true,
            sharing: Sharing::Automatic,
            ..Default::default()
        };
        assert!(state.plan(Commit::Start).is_empty());
    }
    #[test]
    fn every_failed_start_action_stops_before_completion_and_retries() {
        let mut prototype = ready();
        prototype.private_ai = true;
        prototype.rules.insert("a".into(), Rule::Ask);
        prototype
            .past_selections
            .insert("a".into(), vec!["old".into()]);
        let actions = prototype.plan(Commit::Start);
        for failed_at in 0..actions.len() {
            let mut state = prototype.clone();
            let mut runner = Runner::default();
            let mut ticket = runner.begin(&state, Commit::Start).unwrap();
            for (index, action) in actions.iter().enumerate().take(failed_at + 1) {
                assert_eq!(&ticket.action, action);
                if index == failed_at {
                    assert!(
                        runner
                            .resolve(&ticket, &mut state, Outcome::Failure("refused".into()))
                            .is_none()
                    );
                } else {
                    ticket = runner
                        .resolve(&ticket, &mut state, Outcome::Success)
                        .unwrap();
                }
            }
            assert!(!state.completed);
            assert!(!runner.busy());
            assert_eq!(
                runner.begin(&state, Commit::Start).unwrap().action,
                actions[0]
            );
        }
    }
    #[test]
    fn stale_and_duplicate_callbacks_cannot_advance_the_queue() {
        let mut state = ready();
        let mut runner = Runner::default();
        let first = runner.begin(&state, Commit::Start).unwrap();
        let second = runner
            .resolve(&first, &mut state, Outcome::Success)
            .unwrap();
        assert!(
            runner
                .resolve(&first, &mut state, Outcome::Success)
                .is_none()
        );
        assert!(runner.busy());
        runner.cancel();
        assert!(
            runner
                .resolve(&second, &mut state, Outcome::Success)
                .is_none()
        );
        assert!(!state.completed);
    }
    #[test]
    fn refused_grant_requires_confirmed_ask_before_marker() {
        let mut state = ready();
        state.sharing = Sharing::Automatic;
        state.grant_ready = true;
        let mut runner = Runner::default();
        let mut ticket = runner.begin(&state, Commit::Start).unwrap();
        while !matches!(ticket.action, Action::GrantAutomatic { .. }) {
            ticket = runner
                .resolve(&ticket, &mut state, Outcome::Success)
                .unwrap();
        }
        let fallback = runner
            .resolve(
                &ticket,
                &mut state,
                Outcome::GrantRefused("witness-unavailable".into()),
            )
            .unwrap();
        assert_eq!(fallback.action, Action::SetAskEachTime);
        assert_eq!(state.sharing, Sharing::Ask);
        assert!(
            runner
                .resolve(&fallback, &mut state, Outcome::Failure("ask-failed".into()))
                .is_none()
        );
        assert!(!state.completed);
        assert!(
            !state
                .plan(Commit::Start)
                .iter()
                .any(|a| matches!(a, Action::GrantAutomatic { .. }))
        );
    }
}
