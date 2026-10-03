//! The Flow 1 decisions: which step of the connect-and-forget onboarding
//! comes next, what stands between a contributor and the automatic grant,
//! and what a shell checks before it asks the daemon for that grant.
//!
//! K5 (#1173) moved these out of the Tauri shell -- the grant precondition
//! from its Rust, the step order and grant guards from its onboarding
//! (`frontend/src/features/onboarding/flow1.ts`) -- so the native macOS app
//! takes the same decisions from the core rather than writing its own. The C
//! ABI exports each of them (`tc_grant_precondition_text`, `tc_flow1_*`).
//!
//! # The order (connect-and-forget design, R7 and "The disclosure")
//!
//! Source roots are settled before connect; the scope picker runs right
//! after connect and before the path question; connecting inference (K12)
//! is optional and comes on every path before the disclosures, so the
//! witness screen shows a witness the contributor just connected; and the
//! automatic path reaches the grant only through both disclosure screens,
//! scrub first, then witness. Going back undoes the disclosures read. The
//! re-grant (K10) is the same screens again from the scope picker on, with
//! nothing carried over, so a re-grant is fresh consent under the settings
//! now in force.
//!
//! # A first line only
//!
//! None of this replaces the daemon's own refusals: `grant_automatic`
//! refuses unless the request carries `confirmed: true`
//! (`automatic-grant-confirmation-required`, the label
//! [`grant_precondition`] uses), without a recorded choice of a non-empty
//! scope list (`automatic-grant-scopes-not-chosen`), and when the witness
//! configured is not the one the disclosure screen showed
//! (`automatic-grant-witness-changed`). These checks keep a shell from
//! asking before the contributor has seen what they are agreeing to; a shell
//! sends `confirmed: true` only from the grant screen's button.

use serde::{Deserialize, Serialize};

use crate::config::ContributorConfig;

/// The grant screen's button was not pressed.
pub const GRANT_CONFIRMATION_REQUIRED: &str = "automatic-grant-confirmation-required";
/// This device is not enrolled.
pub const GRANT_NOT_ENROLLED: &str = "automatic-grant-not-enrolled";
/// No scope was chosen through the picker.
pub const GRANT_SCOPE_REQUIRED: &str = "automatic-grant-scope-required";
/// The contributor configuration could not be read, so nothing is asked.
pub const CONFIG_UNREADABLE: &str = "contributor-config-unreadable";

/// What a shell checks before asking the daemon for the Flow 1 grant.
///
/// Refuses when the contributor has not confirmed the grant screens, is not
/// enrolled, or never chose scopes through the picker. A saved scope list is
/// not a choice: `validate_scopes` always adds the floor scope, so an invite
/// enrollee holds one before the picker runs. The labels are fixed and carry
/// no content.
pub fn grant_precondition(
    confirmed: bool,
    config: Option<&ContributorConfig>,
) -> Result<(), &'static str> {
    if !confirmed {
        return Err(GRANT_CONFIRMATION_REQUIRED);
    }
    let Some(config) = config else {
        return Err(GRANT_NOT_ENROLLED);
    };
    if !config.consent_scopes_chosen || config.consent_scopes.is_empty() {
        return Err(GRANT_SCOPE_REQUIRED);
    }
    Ok(())
}

/// One scope the picker offers, as `consent_options` lists it. Fields the
/// picker does not decide on are ignored.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeOption {
    pub name: String,
    pub always_on: bool,
}

/// Whether the scope picker may continue, and which always-on scopes are
/// still unticked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopeChoice {
    pub can_continue: bool,
    pub missing_required: Vec<String>,
}

/// R7: the picker has no default -- it starts with nothing selected, the
/// floor scope included, so a grant never ships at a scope nobody chose --
/// and continues only with something chosen, every always-on scope ticked by
/// hand, and nothing the daemon did not offer.
#[must_use]
pub fn scope_choice(options: &[ScopeOption], selected: &[String]) -> ScopeChoice {
    let missing_required: Vec<String> = options
        .iter()
        .filter(|option| option.always_on && !selected.contains(&option.name))
        .map(|option| option.name.clone())
        .collect();
    let can_continue = !options.is_empty()
        && !selected.is_empty()
        && missing_required.is_empty()
        && selected
            .iter()
            .all(|name| options.iter().any(|option| &option.name == name));
    ScopeChoice {
        can_continue,
        missing_required,
    }
}

/// The path question's two answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContributionPath {
    /// Flow 1: the automatic grant, through both disclosures.
    Automatic,
    /// Flow 2: every session asked about first.
    AskFirst,
}

/// How far a contributor has come through the steps that stand between them
/// and the grant. Every field defaults to not done.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Flow1Progress {
    /// The daemon reports an enrollment. A shell sets this from the
    /// daemon's status every time, never from a step it remembers.
    pub connected: bool,
    /// Scopes the contributor chose and the daemon saved, this session.
    pub scopes_saved: Option<Vec<String>>,
    pub path: Option<ContributionPath>,
    pub scrub_disclosure_seen: bool,
    pub witness_disclosure_seen: bool,
    /// The signing address of the witness the witness screen showed, `None`
    /// for none. The daemon refuses the grant when the witness configured by
    /// then is a different one.
    pub witness_shown: Option<String>,
}

impl Flow1Progress {
    /// The same progress with both disclosures unread and no witness
    /// recorded: both screens, in order, stand before the grant again.
    #[must_use]
    fn with_disclosures_unread(mut self) -> Self {
        self.scrub_disclosure_seen = false;
        self.witness_disclosure_seen = false;
        self.witness_shown = None;
        self
    }
}

/// The onboarding's steps, in the order the design gives them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OnboardingStep {
    Welcome,
    Roots,
    Connect,
    Consent,
    Path,
    Privacy,
    Inference,
    DisclosureScrub,
    DisclosureWitness,
    Grant,
    Projects,
    Done,
}

/// One step of the grant still undone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GrantBlocker {
    Connect,
    Scope,
    Path,
    ScrubDisclosure,
    WitnessDisclosure,
}

/// Every step still standing between the contributor and the grant, in the
/// order the steps come.
#[must_use]
pub fn grant_blockers(progress: &Flow1Progress) -> Vec<GrantBlocker> {
    let mut blockers = Vec::new();
    if !progress.connected {
        blockers.push(GrantBlocker::Connect);
    }
    if progress.scopes_saved.as_ref().is_none_or(Vec::is_empty) {
        blockers.push(GrantBlocker::Scope);
    }
    if progress.path != Some(ContributionPath::Automatic) {
        blockers.push(GrantBlocker::Path);
    }
    if !progress.scrub_disclosure_seen {
        blockers.push(GrantBlocker::ScrubDisclosure);
    }
    if !progress.witness_disclosure_seen {
        blockers.push(GrantBlocker::WitnessDisclosure);
    }
    blockers
}

/// The only way a shell reaches `grant_automatic`: refused, with every step
/// still undone, until connect, a chosen scope, the automatic path and both
/// disclosure screens are done; then the witness the disclosure screen
/// showed, to pass as `witness_signing_address` (`None` for none).
pub fn grant_request(progress: &Flow1Progress) -> Result<Option<&str>, Vec<GrantBlocker>> {
    let blockers = grant_blockers(progress);
    if blockers.is_empty() {
        Ok(progress.witness_shown.as_deref())
    } else {
        Err(blockers)
    }
}

/// Where the onboarding is: the step on screen, the progress toward the
/// grant, and whether the NEAR AI privacy notice is among the steps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow1State {
    pub step: OnboardingStep,
    #[serde(default)]
    pub progress: Flow1Progress,
    #[serde(default)]
    pub privacy_included: bool,
}

/// Where the onboarding starts. A first run starts at the welcome; the
/// re-grant (K10), for a contributor already enrolled, starts at the scope
/// picker with nothing carried over.
#[must_use]
pub fn start(regrant: bool) -> Flow1State {
    Flow1State {
        step: if regrant {
            OnboardingStep::Consent
        } else {
            OnboardingStep::Welcome
        },
        progress: Flow1Progress::default(),
        privacy_included: false,
    }
}

/// What the contributor did on the step on screen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Flow1Event {
    /// The welcome's Continue.
    RootsStarted,
    /// The source roots are settled and the daemon is running.
    RootsReady,
    /// The daemon reports an enrollment (enrolled here, or already).
    Enrolled,
    /// The picker's Continue, after the daemon saved `scopes`.
    ScopesSaved { scopes: Vec<String> },
    /// R7's Decide later: nothing saved, no grant possible, Flow 2.
    DecideLater { show_privacy: bool },
    /// The path question answered.
    ChoosePath {
        path: ContributionPath,
        show_privacy: bool,
    },
    /// The NEAR AI privacy notice answered and saved.
    PrivacySaved,
    /// Connecting inference finished, connected or skipped.
    InferenceFinished,
    /// The scrub disclosure read.
    ScrubDisclosureRead,
    /// The witness disclosure read, and the witness it showed.
    WitnessDisclosureRead { signing_address: Option<String> },
    /// The daemon gave the grant.
    Granted,
    /// Left the grant screen without granting: Flow 2, not a half grant.
    SkipGrant,
    /// The projects step finished.
    ProjectsFinished,
    /// Back.
    Back,
}

/// After connecting inference, connected or skipped: the automatic path
/// goes to the disclosures, any other to the projects. Connecting inference
/// is never a grant blocker.
#[must_use]
pub fn after_inference(path: Option<ContributionPath>) -> OnboardingStep {
    if path == Some(ContributionPath::Automatic) {
        OnboardingStep::DisclosureScrub
    } else {
        OnboardingStep::Projects
    }
}

/// The step Back goes to.
#[must_use]
pub fn previous_step(
    current: OnboardingStep,
    privacy_included: bool,
    path: Option<ContributionPath>,
    scopes_chosen: bool,
) -> OnboardingStep {
    use OnboardingStep as S;
    match current {
        S::Roots => S::Welcome,
        S::Connect => S::Roots,
        S::Consent => S::Connect,
        S::Path => S::Consent,
        S::Privacy if scopes_chosen => S::Path,
        S::Privacy => S::Consent,
        S::Inference if privacy_included => S::Privacy,
        S::Inference if scopes_chosen && path.is_some() => S::Path,
        S::Inference => S::Consent,
        S::DisclosureScrub => S::Inference,
        S::DisclosureWitness => S::DisclosureScrub,
        S::Grant => S::DisclosureWitness,
        S::Projects => S::Inference,
        S::Welcome | S::Done => current,
    }
}

/// [`apply`] refused an event that is not one the step on screen offers.
pub const FLOW1_EVENT_NOT_FOR_STEP: &str = "flow1-event-not-for-step";

/// Whether `event` is something the contributor can do on `step`. Each
/// event belongs to the one screen that offers it; Back belongs to every
/// step that has a step before it, so not the welcome and not done.
///
/// Tauri's onboarding gets the same rule from its structure: each of its
/// handlers is reached only from its own screen's button.
#[must_use]
pub fn event_belongs_to(step: OnboardingStep, event: &Flow1Event) -> bool {
    use Flow1Event as E;
    use OnboardingStep as S;
    match event {
        E::RootsStarted => step == S::Welcome,
        E::RootsReady => step == S::Roots,
        E::Enrolled => step == S::Connect,
        E::ScopesSaved { .. } | E::DecideLater { .. } => step == S::Consent,
        E::ChoosePath { .. } => step == S::Path,
        E::PrivacySaved => step == S::Privacy,
        E::InferenceFinished => step == S::Inference,
        E::ScrubDisclosureRead => step == S::DisclosureScrub,
        E::WitnessDisclosureRead { .. } => step == S::DisclosureWitness,
        E::Granted | E::SkipGrant => step == S::Grant,
        E::ProjectsFinished => step == S::Projects,
        E::Back => !matches!(step, S::Welcome | S::Done),
    }
}

/// The state after `event`, or [`FLOW1_EVENT_NOT_FOR_STEP`] when the step
/// on screen does not offer it ([`event_belongs_to`]); a refused event moves
/// nothing, so a disclosure is never marked read from another screen.
///
/// Every event that changes the path, the scopes or the step order before
/// the disclosures leaves both disclosures unread, and so does Back: the
/// grant is given only under screens read in order, for the choices on
/// screen now.
///
/// `Enrolled` moves the step on and does not set `progress.connected`. That
/// is deliberate: `connected` is the daemon's answer, which a shell sets
/// from its status before each call, as Tauri's onboarding derives it from
/// `daemon.logged_in` on every render. A step machine that remembered it
/// would keep saying connected after the daemon stopped saying so.
///
/// Back from the grant screen lands on the witness screen with both
/// disclosures unread, so reading the witness screen alone leaves the scrub
/// disclosure as a blocker; the contributor goes Back once more to read it.
/// That fails closed, and it is what Tauri's `goBack` in `flow1.ts` does,
/// which D1 leaves as it is; the two are kept the same rather than fixed in
/// one.
pub fn apply(state: &Flow1State, event: Flow1Event) -> Result<Flow1State, &'static str> {
    use OnboardingStep as S;
    if !event_belongs_to(state.step, &event) {
        return Err(FLOW1_EVENT_NOT_FOR_STEP);
    }
    let mut next = state.clone();
    match event {
        Flow1Event::RootsStarted => next.step = S::Roots,
        Flow1Event::RootsReady => next.step = S::Connect,
        Flow1Event::Enrolled => next.step = S::Consent,
        Flow1Event::ScopesSaved { scopes } => {
            next.progress = Flow1Progress {
                scopes_saved: Some(scopes),
                path: None,
                ..next.progress
            }
            .with_disclosures_unread();
            next.step = S::Path;
        }
        Flow1Event::DecideLater { show_privacy } => {
            next.progress = Flow1Progress {
                path: Some(ContributionPath::AskFirst),
                ..Flow1Progress::default()
            };
            next.privacy_included = show_privacy;
            next.step = if show_privacy {
                S::Privacy
            } else {
                S::Inference
            };
        }
        Flow1Event::ChoosePath { path, show_privacy } => {
            next.progress = Flow1Progress {
                path: Some(path),
                ..next.progress
            }
            .with_disclosures_unread();
            next.privacy_included = show_privacy;
            next.step = if show_privacy {
                S::Privacy
            } else {
                S::Inference
            };
        }
        Flow1Event::PrivacySaved => next.step = S::Inference,
        Flow1Event::InferenceFinished => next.step = after_inference(next.progress.path),
        Flow1Event::ScrubDisclosureRead => {
            next.progress.scrub_disclosure_seen = true;
            next.step = S::DisclosureWitness;
        }
        Flow1Event::WitnessDisclosureRead { signing_address } => {
            next.progress.witness_disclosure_seen = true;
            next.progress.witness_shown = signing_address;
            next.step = S::Grant;
        }
        Flow1Event::Granted | Flow1Event::ProjectsFinished => next.step = S::Done,
        Flow1Event::SkipGrant => {
            next.progress.path = Some(ContributionPath::AskFirst);
            next.step = S::Projects;
        }
        Flow1Event::Back => {
            next.step = previous_step(
                state.step,
                state.privacy_included,
                state.progress.path,
                state.progress.scopes_saved.is_some(),
            );
            next.progress = next.progress.with_disclosures_unread();
        }
    }
    Ok(next)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(scopes: &[&str], chosen: bool) -> ContributorConfig {
        serde_json::from_value(serde_json::json!({
            "schema_version": crate::config::CONTRIBUTOR_CONFIG_SCHEMA_VERSION,
            "issuer_url": "https://issuer.invalid",
            "ingest_url": "https://ingest.invalid",
            "audience": "aud",
            "tenant_id": "tenant-1",
            "instance_id": "instance-1",
            "user_subject": "alice",
            "device_key_id": "sha256:aa",
            "consent_scopes": scopes,
            "consent_scopes_chosen": chosen,
        }))
        .expect("a contributor config")
    }

    /// What an invite enrollment saves: the floor scope `validate_scopes`
    /// adds, with nobody having picked it.
    fn invite_enrolled() -> ContributorConfig {
        let floor = crate::consent::validate_scopes(&[]).expect("the floor scope");
        let floor: Vec<&str> = floor.iter().map(String::as_str).collect();
        config(&floor, false)
    }

    #[test]
    fn the_grant_needs_confirmation_enrollment_and_a_chosen_scope() {
        let chosen = config(&["debugging_evaluation"], true);
        assert_eq!(
            grant_precondition(false, Some(&chosen)),
            Err(GRANT_CONFIRMATION_REQUIRED)
        );
        assert_eq!(grant_precondition(true, None), Err(GRANT_NOT_ENROLLED));
        // An invite enrollee holds a saved scope before the picker runs.
        let enrolled = invite_enrolled();
        assert!(!enrolled.consent_scopes.is_empty());
        assert_eq!(
            grant_precondition(true, Some(&enrolled)),
            Err(GRANT_SCOPE_REQUIRED)
        );
        // A choice recorded over an empty list is no scope either.
        assert_eq!(
            grant_precondition(true, Some(&config(&[], true))),
            Err(GRANT_SCOPE_REQUIRED)
        );
        assert_eq!(grant_precondition(true, Some(&chosen)), Ok(()));
        // The labels are the ones the Tauri shell has always refused with.
        assert_eq!(
            GRANT_CONFIRMATION_REQUIRED,
            "automatic-grant-confirmation-required"
        );
        assert_eq!(GRANT_NOT_ENROLLED, "automatic-grant-not-enrolled");
        assert_eq!(GRANT_SCOPE_REQUIRED, "automatic-grant-scope-required");
    }

    fn options() -> Vec<ScopeOption> {
        [
            ("debugging_evaluation", true),
            ("benchmark_only", false),
            ("public_attribution", false),
        ]
        .into_iter()
        .map(|(name, always_on)| ScopeOption {
            name: name.to_owned(),
            always_on,
        })
        .collect()
    }

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn the_scope_picker_continues_only_with_the_always_on_scope_ticked_by_hand() {
        let none = scope_choice(&options(), &[]);
        assert!(!none.can_continue);
        assert_eq!(none.missing_required, names(&["debugging_evaluation"]));
        assert!(!scope_choice(&options(), &names(&["benchmark_only"])).can_continue);
        assert!(scope_choice(&options(), &names(&["debugging_evaluation"])).can_continue);
        assert!(
            scope_choice(
                &options(),
                &names(&["debugging_evaluation", "public_attribution"])
            )
            .can_continue
        );
        // No options, and a scope the daemon did not offer, never continue.
        assert!(!scope_choice(&[], &[]).can_continue);
        assert!(!scope_choice(&[], &names(&["anything"])).can_continue);
        assert!(
            !scope_choice(&options(), &names(&["debugging_evaluation", "invented"])).can_continue
        );
    }

    fn complete() -> Flow1Progress {
        Flow1Progress {
            connected: true,
            scopes_saved: Some(names(&["debugging_evaluation"])),
            path: Some(ContributionPath::Automatic),
            scrub_disclosure_seen: true,
            witness_disclosure_seen: true,
            witness_shown: None,
        }
    }

    #[test]
    fn the_grant_is_refused_before_every_step_is_done() {
        use GrantBlocker as B;
        assert_eq!(
            grant_blockers(&Flow1Progress::default()),
            [
                B::Connect,
                B::Scope,
                B::Path,
                B::ScrubDisclosure,
                B::WitnessDisclosure
            ]
        );
        let missing: [(B, Flow1Progress); 7] = [
            (
                B::Connect,
                Flow1Progress {
                    connected: false,
                    ..complete()
                },
            ),
            (
                B::Scope,
                Flow1Progress {
                    scopes_saved: None,
                    ..complete()
                },
            ),
            (
                B::Scope,
                Flow1Progress {
                    scopes_saved: Some(Vec::new()),
                    ..complete()
                },
            ),
            (
                B::Path,
                Flow1Progress {
                    path: None,
                    ..complete()
                },
            ),
            (
                B::Path,
                Flow1Progress {
                    path: Some(ContributionPath::AskFirst),
                    ..complete()
                },
            ),
            (
                B::ScrubDisclosure,
                Flow1Progress {
                    scrub_disclosure_seen: false,
                    ..complete()
                },
            ),
            (
                B::WitnessDisclosure,
                Flow1Progress {
                    witness_disclosure_seen: false,
                    ..complete()
                },
            ),
        ];
        for (blocker, progress) in missing {
            assert_eq!(grant_blockers(&progress), [blocker], "{blocker:?}");
            assert_eq!(grant_request(&progress), Err(vec![blocker]));
        }
        assert!(grant_blockers(&complete()).is_empty());
    }

    #[test]
    fn the_grant_is_asked_under_the_witness_the_disclosure_screen_showed() {
        assert_eq!(grant_request(&complete()), Ok(None));
        let shown = Flow1Progress {
            witness_shown: Some("0xshown".to_owned()),
            ..complete()
        };
        assert_eq!(grant_request(&shown), Ok(Some("0xshown")));
    }

    #[test]
    fn the_labels_on_the_wire_are_the_onboardings() {
        let blockers = serde_json::to_value(grant_blockers(&Flow1Progress::default())).unwrap();
        assert_eq!(
            blockers,
            serde_json::json!([
                "connect",
                "scope",
                "path",
                "scrub_disclosure",
                "witness_disclosure"
            ])
        );
        assert_eq!(
            serde_json::to_value(OnboardingStep::DisclosureWitness).unwrap(),
            "disclosure_witness"
        );
        assert_eq!(
            serde_json::to_value(ContributionPath::AskFirst).unwrap(),
            "ask_first"
        );
        // A progress object a shell sends with fields missing reads as those
        // steps not done, never as done.
        let partial: Flow1Progress =
            serde_json::from_value(serde_json::json!({"connected": true})).unwrap();
        assert_eq!(grant_blockers(&partial).len(), 4);
    }

    /// Walk the automatic path from the welcome to the grant.
    fn walk_to_grant(show_privacy: bool) -> Flow1State {
        let mut state = start(false);
        for event in [
            Flow1Event::RootsStarted,
            Flow1Event::RootsReady,
            Flow1Event::Enrolled,
            Flow1Event::ScopesSaved {
                scopes: names(&["debugging_evaluation"]),
            },
            Flow1Event::ChoosePath {
                path: ContributionPath::Automatic,
                show_privacy,
            },
        ] {
            state = go(&state, event);
        }
        if show_privacy {
            assert_eq!(state.step, OnboardingStep::Privacy);
            state = go(&state, Flow1Event::PrivacySaved);
        }
        assert_eq!(state.step, OnboardingStep::Inference);
        state = go(&state, Flow1Event::InferenceFinished);
        assert_eq!(state.step, OnboardingStep::DisclosureScrub);
        state = go(&state, Flow1Event::ScrubDisclosureRead);
        assert_eq!(state.step, OnboardingStep::DisclosureWitness);
        state = go(
            &state,
            Flow1Event::WitnessDisclosureRead {
                signing_address: Some("0xabc".to_owned()),
            },
        );
        assert_eq!(state.step, OnboardingStep::Grant);
        // `Enrolled` does not set it: a shell sets `connected` from the
        // daemon's status before each call, as this does for one that says
        // enrolled.
        state.progress.connected = true;
        state
    }

    /// `apply` for an event the step on screen offers.
    fn go(state: &Flow1State, event: Flow1Event) -> Flow1State {
        apply(state, event).expect("an event the step on screen offers")
    }

    #[test]
    fn the_automatic_path_reaches_the_grant_only_through_both_disclosures() {
        for show_privacy in [true, false] {
            let state = walk_to_grant(show_privacy);
            assert_eq!(grant_request(&state.progress), Ok(Some("0xabc")));
            assert_eq!(go(&state, Flow1Event::Granted).step, OnboardingStep::Done);
        }
        // The ask-first path never reaches the disclosures.
        assert_eq!(after_inference(None), OnboardingStep::Projects);
        assert_eq!(
            after_inference(Some(ContributionPath::AskFirst)),
            OnboardingStep::Projects
        );
        assert_eq!(
            after_inference(Some(ContributionPath::Automatic)),
            OnboardingStep::DisclosureScrub
        );
    }

    #[test]
    fn back_undoes_the_disclosures_read() {
        let at_grant = walk_to_grant(false);
        let back = go(&at_grant, Flow1Event::Back);
        assert_eq!(back.step, OnboardingStep::DisclosureWitness);
        assert!(!back.progress.scrub_disclosure_seen);
        assert!(!back.progress.witness_disclosure_seen);
        assert_eq!(back.progress.witness_shown, None);
        assert_eq!(
            grant_blockers(&back.progress),
            [
                GrantBlocker::ScrubDisclosure,
                GrantBlocker::WitnessDisclosure
            ]
        );
        // The choices made before the disclosures survive Back.
        assert_eq!(back.progress.path, Some(ContributionPath::Automatic));
        assert!(back.progress.scopes_saved.is_some());
    }

    #[test]
    fn changing_the_path_or_the_scopes_undoes_the_disclosures_read() {
        // Both disclosures read, then on the path question and on the scope
        // picker. Back would have unread them already, so these states are
        // built by hand: they pin that a new choice unreads them on its own.
        let read = walk_to_grant(false);
        let on = |step: OnboardingStep| Flow1State {
            step,
            ..read.clone()
        };
        assert!(on(OnboardingStep::Path).progress.scrub_disclosure_seen);
        let rechosen = go(
            &on(OnboardingStep::Path),
            Flow1Event::ChoosePath {
                path: ContributionPath::Automatic,
                show_privacy: false,
            },
        );
        assert!(!rechosen.progress.scrub_disclosure_seen);
        assert!(!rechosen.progress.witness_disclosure_seen);
        assert_eq!(rechosen.progress.witness_shown, None);
        let rescoped = go(
            &on(OnboardingStep::Consent),
            Flow1Event::ScopesSaved {
                scopes: names(&["debugging_evaluation", "benchmark_only"]),
            },
        );
        assert_eq!(rescoped.step, OnboardingStep::Path);
        assert_eq!(rescoped.progress.path, None);
        assert!(!rescoped.progress.scrub_disclosure_seen);
        assert!(!rescoped.progress.witness_disclosure_seen);
    }

    #[test]
    fn back_follows_the_steps_the_contributor_took() {
        use OnboardingStep as S;
        let auto = Some(ContributionPath::Automatic);
        assert_eq!(previous_step(S::Roots, false, None, false), S::Welcome);
        assert_eq!(previous_step(S::Connect, false, None, false), S::Roots);
        assert_eq!(previous_step(S::Consent, false, None, false), S::Connect);
        assert_eq!(previous_step(S::Path, false, None, false), S::Consent);
        assert_eq!(previous_step(S::Privacy, true, auto, true), S::Path);
        // Decide later went to the privacy notice without a scope.
        assert_eq!(previous_step(S::Privacy, true, None, false), S::Consent);
        assert_eq!(previous_step(S::Inference, true, auto, true), S::Privacy);
        assert_eq!(previous_step(S::Inference, false, auto, true), S::Path);
        assert_eq!(previous_step(S::Inference, false, None, false), S::Consent);
        assert_eq!(
            previous_step(S::DisclosureScrub, false, auto, true),
            S::Inference
        );
        assert_eq!(
            previous_step(S::DisclosureWitness, false, auto, true),
            S::DisclosureScrub
        );
        assert_eq!(
            previous_step(S::Grant, false, auto, true),
            S::DisclosureWitness
        );
        assert_eq!(previous_step(S::Projects, false, None, false), S::Inference);
        assert_eq!(previous_step(S::Welcome, false, None, false), S::Welcome);
        assert_eq!(previous_step(S::Done, false, None, false), S::Done);
    }

    #[test]
    fn decide_later_saves_no_scope_and_gives_no_grant() {
        for show_privacy in [true, false] {
            let connect = Flow1State {
                step: OnboardingStep::Connect,
                ..start(false)
            };
            let at_picker = go(&connect, Flow1Event::Enrolled);
            let later = go(&at_picker, Flow1Event::DecideLater { show_privacy });
            assert_eq!(later.progress.scopes_saved, None);
            assert_eq!(later.progress.path, Some(ContributionPath::AskFirst));
            assert_eq!(later.privacy_included, show_privacy);
            assert_eq!(
                later.step,
                if show_privacy {
                    OnboardingStep::Privacy
                } else {
                    OnboardingStep::Inference
                }
            );
            // Even a connected contributor who read everything gets no grant.
            let progress = Flow1Progress {
                connected: true,
                scrub_disclosure_seen: true,
                witness_disclosure_seen: true,
                ..later.progress
            };
            assert!(grant_blockers(&progress).contains(&GrantBlocker::Scope));
            assert!(grant_request(&progress).is_err());
        }
    }

    #[test]
    fn leaving_the_grant_screen_is_flow_2_not_a_half_grant() {
        let skipped = go(&walk_to_grant(false), Flow1Event::SkipGrant);
        assert_eq!(skipped.step, OnboardingStep::Projects);
        assert_eq!(skipped.progress.path, Some(ContributionPath::AskFirst));
        assert_eq!(grant_blockers(&skipped.progress), [GrantBlocker::Path]);
        assert_eq!(
            go(&skipped, Flow1Event::ProjectsFinished).step,
            OnboardingStep::Done
        );
    }

    /// K10: the re-grant opens the same screens from the scope picker on,
    /// with nothing carried over, so every step is taken again.
    #[test]
    fn the_regrant_starts_at_the_scope_picker_with_nothing_carried_over() {
        let regrant = start(true);
        assert_eq!(regrant.step, OnboardingStep::Consent);
        assert_eq!(regrant.progress, Flow1Progress::default());
        assert!(!regrant.privacy_included);
        assert_eq!(start(false).step, OnboardingStep::Welcome);
        let connected = Flow1Progress {
            connected: true,
            ..regrant.progress
        };
        assert_eq!(
            grant_blockers(&connected),
            [
                GrantBlocker::Scope,
                GrantBlocker::Path,
                GrantBlocker::ScrubDisclosure,
                GrantBlocker::WitnessDisclosure
            ]
        );
    }

    /// An event is what the contributor did on the step on screen, so one
    /// sent from any other step is refused and the state is not moved: a
    /// scrub disclosure "read" from the welcome must not mark it read.
    #[test]
    fn an_event_from_another_step_is_refused() {
        use OnboardingStep as S;
        let event_for = |step: S| -> Flow1Event {
            match step {
                S::Welcome => Flow1Event::RootsStarted,
                S::Roots => Flow1Event::RootsReady,
                S::Connect => Flow1Event::Enrolled,
                S::Consent => Flow1Event::ScopesSaved {
                    scopes: names(&["debugging_evaluation"]),
                },
                S::Path => Flow1Event::ChoosePath {
                    path: ContributionPath::Automatic,
                    show_privacy: false,
                },
                S::Privacy => Flow1Event::PrivacySaved,
                S::Inference => Flow1Event::InferenceFinished,
                S::DisclosureScrub => Flow1Event::ScrubDisclosureRead,
                S::DisclosureWitness => Flow1Event::WitnessDisclosureRead {
                    signing_address: None,
                },
                S::Grant => Flow1Event::Granted,
                S::Projects => Flow1Event::ProjectsFinished,
                S::Done => Flow1Event::Back,
            }
        };
        let steps = [
            S::Welcome,
            S::Roots,
            S::Connect,
            S::Consent,
            S::Path,
            S::Privacy,
            S::Inference,
            S::DisclosureScrub,
            S::DisclosureWitness,
            S::Grant,
            S::Projects,
        ];
        for on in steps {
            let state = Flow1State {
                step: on,
                progress: Flow1Progress::default(),
                privacy_included: false,
            };
            for from in steps {
                let result = apply(&state, event_for(from));
                if on == from {
                    assert!(result.is_ok(), "{from:?}'s event on {on:?}");
                } else {
                    assert_eq!(
                        result,
                        Err(FLOW1_EVENT_NOT_FOR_STEP),
                        "{from:?}'s event on {on:?}"
                    );
                }
            }
        }
        // The scrub disclosure read from the welcome marks nothing read.
        assert_eq!(
            apply(&start(false), Flow1Event::ScrubDisclosureRead),
            Err(FLOW1_EVENT_NOT_FOR_STEP)
        );
        // Back has nowhere to go from the welcome or once done.
        assert!(apply(&start(false), Flow1Event::Back).is_err());
        let done = Flow1State {
            step: S::Done,
            ..start(false)
        };
        assert!(apply(&done, Flow1Event::Back).is_err());
        assert!(apply(&done, Flow1Event::Granted).is_err());
        assert_eq!(FLOW1_EVENT_NOT_FOR_STEP, "flow1-event-not-for-step");
    }

    #[test]
    fn events_and_states_read_from_the_wire() {
        let state: Flow1State =
            serde_json::from_value(serde_json::json!({"step": "consent"})).unwrap();
        assert_eq!(state, start(true));
        let event: Flow1Event = serde_json::from_value(serde_json::json!({
            "event": "witness_disclosure_read",
            "signing_address": "0xabc",
        }))
        .unwrap();
        assert_eq!(
            event,
            Flow1Event::WitnessDisclosureRead {
                signing_address: Some("0xabc".to_owned())
            }
        );
        let event: Flow1Event =
            serde_json::from_value(serde_json::json!({"event": "back"})).unwrap();
        assert_eq!(event, Flow1Event::Back);
        assert!(
            serde_json::from_value::<Flow1Event>(serde_json::json!({"event": "grant_anyway"}))
                .is_err()
        );
    }
}
