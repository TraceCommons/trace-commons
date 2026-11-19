//! Customize's per-folder rules, as the Mac decides them
//! (`RulesScreenLayout` in `RulesScreen.swift`): the rule shown is the
//! person's answer, else the daemon's mode; the picker offers the modes the
//! daemon would set; a pick of the daemon's own mode is cleared, except Ask;
//! and Automatic picked before sign-in says it turns on after sign-in.
use super::{Rule, State};
use crate::model::Project;

impl Rule {
    /// The rule a `list_projects` `mode` names, or None for any other word.
    pub fn from_wire(mode: &str) -> Option<Self> {
        [Self::Ask, Self::Automatic, Self::Never]
            .into_iter()
            .find(|rule| rule.wire() == mode)
    }
}

/// The modes this folder's picker offers, in the order it shows them: Ask,
/// Automatic, Never (`ProjectRow.offerableModes`). The folder with no
/// usable name is never armed; a daemon that lists its own modes narrows
/// the list further.
pub fn offerable_modes(project: &Project) -> Vec<Rule> {
    let supported = if project.is_unresolved_bucket {
        vec![Rule::Ask, Rule::Never]
    } else {
        vec![Rule::Ask, Rule::Automatic, Rule::Never]
    };
    match &project.offerable_modes {
        None => supported,
        Some(listed) => supported
            .into_iter()
            .filter(|rule| listed.iter().any(|mode| mode == rule.wire()))
            .collect(),
    }
}

/// The rule the folder's picker shows: the person's answer, else the
/// daemon's mode (`RulesScreenLayout.rule`). Only a pick writes the state.
pub fn shown_rule(state: &State, project: &Project) -> Option<Rule> {
    state
        .rules
        .get(&project.project_id)
        .copied()
        .or_else(|| Rule::from_wire(&project.mode))
}

/// A pick on a folder's picker (`RulesScreenLayout.pick`). False when the
/// folder does not offer `wanted`; nothing is written then.
pub fn pick(state: &mut State, project: &Project, wanted: Rule) -> bool {
    let id = &project.project_id;
    if Rule::from_wire(&project.mode) == Some(wanted) && wanted != Rule::Ask {
        state.rules.remove(id);
        if wanted == Rule::Never {
            state.past_selections.remove(id);
        }
        return true;
    }
    if !offerable_modes(project).contains(&wanted) {
        return false;
    }
    state.rules.insert(id.clone(), wanted);
    // A Never folder contributes nothing, its past traces included.
    if wanted == Rule::Never {
        state.past_selections.remove(id);
    }
    true
}

/// A folder set to Automatic in this first run that cannot be armed yet,
/// for want of an enrollment (`RulesScreenLayout.armsAfterSignIn`): its row
/// says it turns on after sign-in, in Uses' words.
pub fn arms_after_sign_in(state: &State, project: &Project) -> bool {
    state.rules.get(&project.project_id) == Some(&Rule::Automatic) && !state.holds_enrollment()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::first_run::Account;

    fn project(id: &str, mode: &str) -> Project {
        serde_json::from_value(serde_json::json!({
            "project_id": id,
            "project_label": id,
            "mode": mode,
        }))
        .unwrap()
    }

    #[test]
    fn the_shown_rule_is_the_answer_else_the_daemons_mode() {
        let mut state = State::default();
        let p = project("p", "ignore");
        assert_eq!(shown_rule(&state, &p), Some(Rule::Never));
        state.rules.insert("p".into(), Rule::Ask);
        assert_eq!(shown_rule(&state, &p), Some(Rule::Ask));
        assert_eq!(shown_rule(&State::default(), &project("q", "?")), None);
    }

    #[test]
    fn the_picker_offers_what_the_daemon_would_set() {
        assert_eq!(
            offerable_modes(&project("p", "notify_only")),
            [Rule::Ask, Rule::Automatic, Rule::Never]
        );
        let mut bucket = project("b", "notify_only");
        bucket.is_unresolved_bucket = true;
        assert_eq!(offerable_modes(&bucket), [Rule::Ask, Rule::Never]);
        let mut narrowed = project("n", "notify_only");
        narrowed.offerable_modes = Some(vec!["ignore".into(), "notify_only".into()]);
        assert_eq!(offerable_modes(&narrowed), [Rule::Ask, Rule::Never]);
    }

    #[test]
    fn picking_the_daemons_own_mode_clears_the_answer_except_ask() {
        let mut state = State::default();
        let never = project("p", "ignore");
        state.rules.insert("p".into(), Rule::Ask);
        state.past_selections.insert("p".into(), vec!["s".into()]);
        assert!(pick(&mut state, &never, Rule::Never));
        assert!(
            !state.rules.contains_key("p"),
            "the daemon's Never is not sent again"
        );
        assert!(!state.past_selections.contains_key("p"));
        let auto = project("a", "auto_upload");
        state.rules.insert("a".into(), Rule::Never);
        assert!(pick(&mut state, &auto, Rule::Automatic));
        assert!(!state.rules.contains_key("a"));
        // Ask is kept even when the daemon already reads Ask (#1747).
        let ask = project("k", "notify_only");
        assert!(pick(&mut state, &ask, Rule::Ask));
        assert_eq!(state.rules.get("k"), Some(&Rule::Ask));
        // Never clears the folder's past selections.
        state.past_selections.insert("k".into(), vec!["s".into()]);
        assert!(pick(&mut state, &ask, Rule::Never));
        assert_eq!(state.rules.get("k"), Some(&Rule::Never));
        assert!(!state.past_selections.contains_key("k"));
    }

    #[test]
    fn a_mode_the_folder_does_not_offer_is_refused() {
        let mut state = State::default();
        let mut bucket = project("b", "notify_only");
        bucket.is_unresolved_bucket = true;
        assert!(!pick(&mut state, &bucket, Rule::Automatic));
        assert!(state.rules.is_empty());
    }

    #[test]
    fn automatic_before_sign_in_says_it_turns_on_after_sign_in() {
        let mut state = State::default();
        let p = project("p", "notify_only");
        assert!(pick(&mut state, &p, Rule::Automatic));
        assert!(arms_after_sign_in(&state, &p));
        state.account = Account::NearAi;
        state.near_ai_enrolled = true;
        assert!(
            !arms_after_sign_in(&state, &p),
            "an enrollment arms it at Start"
        );
        let daemon_auto = project("d", "auto_upload");
        assert!(
            !arms_after_sign_in(&State::default(), &daemon_auto),
            "only a pick here says so"
        );
    }
}
