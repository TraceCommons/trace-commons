//! What is true of one coding tool on this machine, as a state rather than a
//! sentence.
//!
//! The words belong to `private_inference_copy`; this module carries only the
//! facts three shells must agree on -- whether a tool can be connected at all,
//! which of the three per-tool states it is in, and what a planned edit turned
//! out to be. Each of those is a branch, and a branch written three times in
//! three languages agrees today and drifts in silence tomorrow. That is why
//! they are here and exported across the C ABI rather than left to each shell.
//!
//! # Activity is attributed by protocol family, and often cannot be
//!
//! The proxy's ledger records a facade -- `anthropic`, `openai` -- not a tool
//! id. Claude Code speaks Anthropic and Codex speaks OpenAI, so today a call
//! separates them; two connected tools of the same family do not separate at
//! all. Nothing here invents an attribution the ledger cannot support: a call
//! whose family is shared by more than one connected tool is reported as
//! [`HarnessState::ActivityShared`], which says a call arrived without naming
//! the tool that made it, and a tool whose family this build does not know is
//! [`HarnessState::Unknown`] rather than quietly "no calls yet".

/// The state of one tool, as a surface needs to describe it.
///
/// The three-valued state the design asks for is [`Self::NotConnected`],
/// [`Self::ConnectedNoCalls`] and [`Self::Answering`]; the other two are the
/// honest answers for the cases where attribution is not available, and they
/// are separate values precisely so a shell cannot render them as one of the
/// three by accident.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessState {
    /// Its config does not send calls here.
    NotConnected,
    /// Its config sends calls here, and no call has arrived.
    ConnectedNoCalls,
    /// A call arrived, and only this tool could have made it.
    Answering,
    /// A call arrived in this tool's protocol family, but another connected
    /// tool speaks that family too, so it cannot be attributed to either.
    ActivityShared,
    /// Connected, and nothing here can say whether a call has arrived --
    /// no readable ledger, or a tool whose protocol family is not known.
    Unknown,
}

impl HarnessState {
    /// The wire label. Also what the C ABI maps back to a code.
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::NotConnected => "not_connected",
            Self::ConnectedNoCalls => "connected_no_calls",
            Self::Answering => "answering",
            Self::ActivityShared => "activity_shared",
            Self::Unknown => "unknown",
        }
    }

    /// A label back to a state, or `None` for one this build does not know.
    ///
    /// `None` rather than a defaulted [`Self::Unknown`]: a caller that
    /// received a label from a newer daemon must be able to tell "this build
    /// has never heard of that" from "the daemon said unknown". Both render
    /// the same way; only one is a reason to update.
    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "not_connected" => Some(Self::NotConnected),
            "connected_no_calls" => Some(Self::ConnectedNoCalls),
            "answering" => Some(Self::Answering),
            "activity_shared" => Some(Self::ActivityShared),
            "unknown" => Some(Self::Unknown),
            _ => None,
        }
    }
}

/// Everything the state derivation is allowed to look at.
///
/// A struct rather than five positional arguments because four of them are
/// booleans, and a caller that transposes two of them would turn "no call has
/// arrived" into "answering" with no compiler complaint.
#[derive(Debug, Clone, Copy)]
pub struct HarnessEvidence<'a> {
    /// `Tool.wired`: the config currently sends calls here.
    pub connected: bool,
    /// Whether the proxy's ledger answered at all. False means no evidence
    /// about any tool, which is not the same as evidence of no calls.
    pub activity_readable: bool,
    /// This tool's protocol family, when this build knows it.
    pub family: Option<&'a str>,
    /// Whether a call arrived in that family inside the window looked at.
    pub family_saw_call: bool,
    /// Whether more than one *connected* tool speaks that family.
    pub family_shared: bool,
}

/// The one place the per-tool state is decided.
#[must_use]
pub fn harness_state(evidence: HarnessEvidence<'_>) -> HarnessState {
    if !evidence.connected {
        // Ahead of every activity question on purpose. A tool that is not
        // connected cannot be the one that made a call, whatever the ledger
        // holds, and the ledger's rows are the other tools' rows.
        return HarnessState::NotConnected;
    }
    if !evidence.activity_readable || evidence.family.is_none() {
        return HarnessState::Unknown;
    }
    if !evidence.family_saw_call {
        return HarnessState::ConnectedNoCalls;
    }
    if evidence.family_shared {
        return HarnessState::ActivityShared;
    }
    HarnessState::Answering
}

/// The two things a contributor can ask for, one tool at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HarnessAction {
    Connect,
    Disconnect,
}

impl HarnessAction {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Connect => "connect",
            Self::Disconnect => "disconnect",
        }
    }

    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "connect" => Some(Self::Connect),
            "disconnect" => Some(Self::Disconnect),
            _ => None,
        }
    }
}

/// Whether an action may be offered for a tool in this state.
///
/// Two rules, and the second is the one worth centralising:
///
/// - A tool that is not installed cannot be connected. Writing a config file
///   for a tool that is not on the machine invents a setting for nobody.
/// - A tool that IS connected can always be disconnected, installed or not.
///   Uninstalling a tool does not remove the line we put in its config, and
///   the rule "remove only what we put there" is worth nothing if the surface
///   hides the button that does the removing.
#[must_use]
pub fn action_available(action: HarnessAction, installed: bool, connected: bool) -> bool {
    match action {
        HarnessAction::Connect => installed && !connected,
        HarnessAction::Disconnect => connected,
    }
}

/// What planning an edit turned out to be, before anything is written.
///
/// `occupied` is deliberately NOT one of these. A plan can carry changes and
/// occupied slots at once -- IronWire fills the empty slots and reports the
/// full one in the same pass -- so flattening the two into one outcome would
/// lose whichever half came second. Occupied slots ride alongside as their own
/// list, and are reported whatever the outcome.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanOutcome {
    /// There is an edit to make, and it is described in `changes`.
    Changes,
    /// Nothing to do: the file already says what the action wanted it to say.
    Noop,
    /// The file could not be parsed, so it was refused rather than rewritten.
    /// Distinct from [`Self::Noop`] on purpose: nothing was decided, and the
    /// file needs a human.
    Unparseable,
    /// The tool is not on this machine, so there is nothing to connect.
    NotInstalled,
    /// The catalog entry describing this tool did not survive validation.
    EntryUnusable,
    /// This build could not work out where the tool keeps its config.
    NoConfigPath,
}

impl PlanOutcome {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::Changes => "changes",
            Self::Noop => "noop",
            Self::Unparseable => "unparseable",
            Self::NotInstalled => "not_installed",
            Self::EntryUnusable => "entry_unusable",
            Self::NoConfigPath => "no_config_path",
        }
    }

    #[must_use]
    pub fn from_label(label: &str) -> Option<Self> {
        match label {
            "changes" => Some(Self::Changes),
            "noop" => Some(Self::Noop),
            "unparseable" => Some(Self::Unparseable),
            "not_installed" => Some(Self::NotInstalled),
            "entry_unusable" => Some(Self::EntryUnusable),
            "no_config_path" => Some(Self::NoConfigPath),
            _ => None,
        }
    }

    /// Whether this outcome has an edit waiting to be committed.
    ///
    /// Only [`Self::Changes`] does. The daemon mints a plan id for exactly
    /// this case and for no other, so a shell that reads this and a shell that
    /// reads "is there a plan id" get the same answer.
    #[must_use]
    pub fn is_committable(self) -> bool {
        matches!(self, Self::Changes)
    }
}

/// The protocol family a built-in tool speaks, or `None`.
///
/// The two ids IronWire ships knowing about, mapped onto the facade its
/// ledger records for a call -- which is what makes activity attributable at
/// all. A catalog-described tool answers `None`: the catalog says which key
/// in which file to set, not which facade the resulting calls land on, so
/// claiming a family for one would be a guess, and a guess here reads to a
/// contributor as proof their tool is working.
#[must_use]
pub fn built_in_family(id: &str) -> Option<&'static str> {
    match id {
        "claude" => Some("anthropic"),
        "codex" => Some("openai"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn evidence() -> HarnessEvidence<'static> {
        HarnessEvidence {
            connected: true,
            activity_readable: true,
            family: Some("anthropic"),
            family_saw_call: false,
            family_shared: false,
        }
    }

    #[test]
    fn an_unconnected_tool_is_never_answering() {
        let state = harness_state(HarnessEvidence {
            connected: false,
            family_saw_call: true,
            ..evidence()
        });
        assert_eq!(state, HarnessState::NotConnected);
    }

    #[test]
    fn connected_with_no_call_is_its_own_state() {
        assert_eq!(harness_state(evidence()), HarnessState::ConnectedNoCalls);
    }

    #[test]
    fn a_call_in_an_unshared_family_is_answering() {
        let state = harness_state(HarnessEvidence {
            family_saw_call: true,
            ..evidence()
        });
        assert_eq!(state, HarnessState::Answering);
    }

    /// The rule the design is emphatic about: two tools of one family are
    /// indistinguishable, and the surface must not pick one.
    #[test]
    fn a_call_in_a_shared_family_names_no_tool() {
        let state = harness_state(HarnessEvidence {
            family_saw_call: true,
            family_shared: true,
            ..evidence()
        });
        assert_eq!(state, HarnessState::ActivityShared);
    }

    #[test]
    fn no_readable_ledger_is_unknown_not_no_calls() {
        let state = harness_state(HarnessEvidence {
            activity_readable: false,
            ..evidence()
        });
        assert_eq!(state, HarnessState::Unknown);
    }

    #[test]
    fn a_tool_of_unknown_family_is_unknown_not_no_calls() {
        let state = harness_state(HarnessEvidence {
            family: None,
            ..evidence()
        });
        assert_eq!(state, HarnessState::Unknown);
    }

    #[test]
    fn every_state_label_round_trips() {
        for state in [
            HarnessState::NotConnected,
            HarnessState::ConnectedNoCalls,
            HarnessState::Answering,
            HarnessState::ActivityShared,
            HarnessState::Unknown,
        ] {
            assert_eq!(HarnessState::from_label(state.label()), Some(state));
        }
        assert_eq!(HarnessState::from_label("hopeful"), None);
    }

    #[test]
    fn a_tool_that_is_not_installed_cannot_be_connected() {
        assert!(!action_available(HarnessAction::Connect, false, false));
        assert!(action_available(HarnessAction::Connect, true, false));
    }

    #[test]
    fn an_already_connected_tool_is_not_offered_a_connect() {
        assert!(!action_available(HarnessAction::Connect, true, true));
    }

    /// Removing what we put there must stay possible after the tool is gone.
    #[test]
    fn a_connected_tool_can_be_disconnected_even_uninstalled() {
        assert!(action_available(HarnessAction::Disconnect, false, true));
        assert!(!action_available(HarnessAction::Disconnect, true, false));
    }

    #[test]
    fn only_a_changes_outcome_is_committable() {
        assert!(PlanOutcome::Changes.is_committable());
        for outcome in [
            PlanOutcome::Noop,
            PlanOutcome::Unparseable,
            PlanOutcome::NotInstalled,
            PlanOutcome::EntryUnusable,
            PlanOutcome::NoConfigPath,
        ] {
            assert!(!outcome.is_committable(), "{outcome:?}");
        }
    }

    #[test]
    fn unparseable_and_noop_are_distinguishable_labels() {
        assert_ne!(PlanOutcome::Unparseable.label(), PlanOutcome::Noop.label());
        for outcome in [
            PlanOutcome::Changes,
            PlanOutcome::Noop,
            PlanOutcome::Unparseable,
            PlanOutcome::NotInstalled,
            PlanOutcome::EntryUnusable,
            PlanOutcome::NoConfigPath,
        ] {
            assert_eq!(PlanOutcome::from_label(outcome.label()), Some(outcome));
        }
        assert_eq!(PlanOutcome::from_label("fine"), None);
    }

    #[test]
    fn the_two_built_in_families_are_the_two_facades_the_ledger_records() {
        assert_eq!(built_in_family("claude"), Some("anthropic"));
        assert_eq!(built_in_family("codex"), Some("openai"));
        assert_eq!(built_in_family("something-from-a-catalog"), None);
    }

    #[test]
    fn action_labels_round_trip() {
        for action in [HarnessAction::Connect, HarnessAction::Disconnect] {
            assert_eq!(HarnessAction::from_label(action.label()), Some(action));
        }
        assert_eq!(HarnessAction::from_label("rewire"), None);
    }
}
