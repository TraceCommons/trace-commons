//! Uses' decisions, as the Mac makes them (`UsesScreenLayout`). Every word
//! is the core's; these choose and order.
use super::{Sharing, State};

/// One `consent_options` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScopeOption {
    pub name: String,
    pub title: String,
    pub description: String,
    pub always_on: bool,
    pub grants_data_use: bool,
}
impl ScopeOption {
    /// A `consent_options` row, or None when a field the screen needs is
    /// missing. `grants_data_use` absent reads as a data use.
    pub fn from_wire(v: &serde_json::Value) -> Option<Self> {
        Some(Self {
            name: v["name"].as_str()?.to_owned(),
            title: v["title"].as_str()?.to_owned(),
            description: v["description"].as_str().unwrap_or_default().to_owned(),
            always_on: v["always_on"].as_bool()?,
            grants_data_use: v["grants_data_use"].as_bool().unwrap_or(true),
        })
    }
}

/// The floor scope, shown as the "Always on" line and always included.
pub fn required_scope(options: &[ScopeOption]) -> Option<&ScopeOption> {
    options.iter().find(|o| o.always_on)
}

/// The optional data uses, the one that earns credit first, the rest in
/// the daemon's order (owner, 2026-10-27).
pub fn shown_optional_scopes<'a>(
    options: &'a [ScopeOption],
    credit_scope: &str,
) -> Vec<&'a ScopeOption> {
    let optional = options.iter().filter(|o| !o.always_on && o.grants_data_use);
    optional
        .clone()
        .filter(|o| o.name == credit_scope)
        .chain(optional.filter(|o| o.name != credit_scope))
        .collect()
}

/// Whether the person chose anything on Uses: an optional use ticked, a
/// Sharing choice, or under Customize a folder rule or a past trace. The
/// action reads Skip until then (Ron, 2026-11-05).
pub fn has_selection(state: &State, options: &[ScopeOption]) -> bool {
    state.sharing_chosen
        || options
            .iter()
            .filter(|o| !o.always_on && o.grants_data_use)
            .any(|o| state.scopes.contains(&o.name))
        || !state.rules.is_empty()
        || state.past_selections.values().any(|v| !v.is_empty())
}

/// The Sharing radios' selection: none until the person picks one.
pub fn sharing_selection(state: &State) -> Option<Sharing> {
    state.sharing_chosen.then_some(state.sharing)
}

/// The person's pick of a Sharing choice. A new choice needs a new answer
/// from the core, so readiness is cleared.
pub fn choose_sharing(state: &mut State, sharing: Sharing) {
    state.sharing = sharing;
    state.sharing_chosen = true;
    state.grant_ready = false;
}

/// The required use among the scopes, as its locked line shows.
pub fn include_required(state: &mut State, required: Option<&str>) {
    if let Some(name) = required {
        if !state.scopes.iter().any(|s| s == name) {
            state.scopes.push(name.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn option(name: &str, always_on: bool, grants: bool) -> ScopeOption {
        ScopeOption {
            name: name.into(),
            title: name.to_uppercase(),
            description: String::new(),
            always_on,
            grants_data_use: grants,
        }
    }

    fn options() -> Vec<ScopeOption> {
        vec![
            option("benchmark_only", false, true),
            option("debugging_evaluation", true, true),
            option("model_training", false, true),
            option("public_handle", false, false),
        ]
    }

    #[test]
    fn neither_sharing_option_is_chosen_until_the_person_picks() {
        let mut state = State::default();
        assert_eq!(sharing_selection(&state), None);
        // Unchosen still commits as Ask, the default.
        assert_eq!(state.sharing, Sharing::Ask);
        state.grant_ready = true;
        choose_sharing(&mut state, Sharing::Automatic);
        assert_eq!(sharing_selection(&state), Some(Sharing::Automatic));
        assert!(!state.grant_ready);
    }

    #[test]
    fn the_action_reads_skip_until_something_is_chosen() {
        let opts = options();
        let mut state = State::default();
        include_required(&mut state, Some("debugging_evaluation"));
        assert!(
            !has_selection(&state, &opts),
            "the required use is nobody's choice"
        );
        state.scopes.push("model_training".into());
        assert!(has_selection(&state, &opts));
        state.scopes.retain(|s| s != "model_training");
        choose_sharing(&mut state, Sharing::Ask);
        assert!(has_selection(&state, &opts));
        let mut rules = State::default();
        rules.rules.insert("p".into(), super::super::Rule::Never);
        assert!(has_selection(&rules, &opts));
        let mut past = State::default();
        past.past_selections.insert("p".into(), vec![]);
        assert!(!has_selection(&past, &opts));
        past.past_selections.insert("p".into(), vec!["s".into()]);
        assert!(has_selection(&past, &opts));
    }

    #[test]
    fn credit_scope_comes_first_and_the_handle_is_not_a_data_use() {
        let opts = options();
        let shown: Vec<_> = shown_optional_scopes(&opts, "model_training")
            .iter()
            .map(|o| o.name.as_str())
            .collect();
        assert_eq!(shown, ["model_training", "benchmark_only"]);
        assert_eq!(
            required_scope(&opts).map(|o| o.name.as_str()),
            Some("debugging_evaluation")
        );
    }

    #[test]
    fn the_required_use_is_included_once() {
        let mut state = State::default();
        include_required(&mut state, Some("floor"));
        include_required(&mut state, Some("floor"));
        assert_eq!(state.scopes, ["floor"]);
        include_required(&mut state, None);
        assert_eq!(state.scopes, ["floor"]);
    }
}
