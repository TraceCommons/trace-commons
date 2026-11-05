//! What a bulk or group mode change does (Settings > Projects' Manage
//! sheet, #1952), decided once here so every shell asks the same question
//! before the same change.
//!
//! The rules, from the owner's decisions of 2026-11-04 on #1952:
//!
//! - A bulk change sets Ask or Never, never Automatic. Automatic is a
//!   grant (owner rulings 2026-10-02), made one project at a time through
//!   the arming confirmation, so `plan_project_modes` and
//!   `set_project_modes` both refuse it.
//! - A change that turns off one or more Automatic projects asks first,
//!   with the count.
//! - A Never that removes waiting traces asks first, with the count, as
//!   one project's Never does.
//! - Projects already in the mode, or unable to take it, are skipped and
//!   not counted.
//!
//! The counts are read from the daemon's live policy and queue when the
//! plan is asked for, and again when the change is made: a change whose
//! live counts grew past what the person agreed to is refused, and the
//! shell asks again.

use super::policy::{ProjectMode, ProjectPolicy};
use crate::project_copy::{BulkModeConfirmation, bulk_mode_confirmation, bulk_projects_name};

/// The modes a bulk or group change may set. Never Automatic.
pub const BULK_MODES: [ProjectMode; 2] = [ProjectMode::NotifyOnly, ProjectMode::Ignore];

/// Most projects one bulk call names. Far above any real folder count;
/// a bound so one request cannot make the daemon walk without end.
pub const MAX_BULK_PROJECTS: usize = 10_000;

/// One project a bulk change names, as the daemon holds it now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BulkProject {
    pub project_id: String,
    /// The `list_projects` label, used to name a change that reaches one.
    pub label: String,
    /// The mode in force (`ProjectPolicy::shown_mode`).
    pub mode: ProjectMode,
    /// Whether the project can take the mode being set
    /// (`ProjectPolicy::check_mode`).
    pub can_take: bool,
    /// Traces waiting in the project.
    pub pending: usize,
}

/// What a bulk change will do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum BulkOutcome {
    /// Nothing to change: every project is already there or cannot take it.
    Nothing,
    /// Change `project_ids` straight away.
    Apply,
    /// Ask first, in `confirmation`'s words.
    Confirm,
    /// The mode is not one a bulk change may set.
    Refused,
}

/// The plan for setting some projects to one mode.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct BulkPlan {
    pub outcome: BulkOutcome,
    /// The projects the change reaches, in the order named.
    pub project_ids: Vec<String>,
    /// How many of them are Automatic now.
    pub turns_off_automatic: usize,
    /// How many waiting traces a Never removes; 0 for Ask.
    pub pending: usize,
    /// Present when `outcome` is `confirm`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirmation: Option<BulkModeConfirmation>,
    /// [`BULK_MODES`], so a shell offers only what a bulk change may set.
    pub bulk_modes: [ProjectMode; 2],
}

/// Plan setting `projects` to `mode`.
#[must_use]
pub fn plan(projects: &[BulkProject], mode: ProjectMode) -> BulkPlan {
    let mut plan = BulkPlan {
        outcome: BulkOutcome::Refused,
        project_ids: Vec::new(),
        turns_off_automatic: 0,
        pending: 0,
        confirmation: None,
        bulk_modes: BULK_MODES,
    };
    if !BULK_MODES.contains(&mode) {
        return plan;
    }
    let targets: Vec<&BulkProject> = projects
        .iter()
        .filter(|p| p.mode != mode && p.can_take)
        .collect();
    if targets.is_empty() {
        plan.outcome = BulkOutcome::Nothing;
        return plan;
    }
    let never = mode == ProjectMode::Ignore;
    plan.project_ids = targets.iter().map(|p| p.project_id.clone()).collect();
    plan.turns_off_automatic = targets
        .iter()
        .filter(|p| p.mode == ProjectMode::AutoUpload)
        .count();
    plan.pending = if never {
        targets.iter().map(|p| p.pending).sum()
    } else {
        0
    };
    if plan.turns_off_automatic == 0 && plan.pending == 0 {
        plan.outcome = BulkOutcome::Apply;
        return plan;
    }
    let labels: Vec<&str> = targets.iter().map(|p| p.label.as_str()).collect();
    plan.outcome = BulkOutcome::Confirm;
    plan.confirmation = Some(bulk_mode_confirmation(
        &bulk_projects_name(&labels),
        never,
        plan.turns_off_automatic,
        plan.pending,
    ));
    plan
}

/// What the person agreed to when they confirmed: the counts the
/// confirmation named.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Agreed {
    pub turns_off_automatic: usize,
    pub pending: usize,
}

/// Whether `plan`, read live when the change is made, is still covered by
/// what the person `agreed` to: a plan that needs no confirmation always
/// is; one that does is covered only when neither count grew. A missing
/// agreement covers nothing that asks.
#[must_use]
pub fn agreed_covers(plan: &BulkPlan, agreed: Option<Agreed>) -> bool {
    match plan.outcome {
        BulkOutcome::Nothing | BulkOutcome::Apply => true,
        BulkOutcome::Refused => false,
        BulkOutcome::Confirm => agreed.is_some_and(|a| {
            plan.turns_off_automatic <= a.turns_off_automatic && plan.pending <= a.pending
        }),
    }
}

/// [`BulkProject::can_take`] for a key, by the daemon's own refusal.
#[must_use]
pub fn can_take(project_key: &str, mode: ProjectMode) -> bool {
    ProjectPolicy::check_mode(project_key, mode).is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str, mode: ProjectMode, pending: usize) -> BulkProject {
        BulkProject {
            project_id: id.to_string(),
            label: format!("label-{id}"),
            mode,
            can_take: true,
            pending,
        }
    }

    use ProjectMode::{AutoUpload as Auto, Ignore as Never, NotifyOnly as Ask};

    #[test]
    fn automatic_is_never_set_in_bulk() {
        let rows = [project("a", Ask, 0), project("b", Never, 0)];
        let plan = plan(&rows, Auto);
        assert_eq!(plan.outcome, BulkOutcome::Refused);
        assert!(plan.project_ids.is_empty());
        assert_eq!(plan.bulk_modes, [Ask, Never]);
        assert!(!agreed_covers(&plan, Some(Agreed::default())));
    }

    /// Kristi's first case: 10 selected, 7 already Never. Three change,
    /// and the title counts three, not ten.
    #[test]
    fn projects_already_in_the_mode_are_skipped_and_not_counted() {
        let mut rows: Vec<BulkProject> = (0..7)
            .map(|i| project(&format!("n{i}"), Never, 0))
            .collect();
        rows.extend((0..3).map(|i| project(&format!("a{i}"), Ask, 1)));
        let plan = plan(&rows, Never);
        assert_eq!(plan.project_ids, ["a0", "a1", "a2"]);
        assert_eq!(plan.pending, 3);
        assert_eq!(plan.outcome, BulkOutcome::Confirm);
        let copy = plan.confirmation.unwrap();
        assert_eq!(copy.title, "Set 3 projects to Never?");
        assert!(copy.destructive);
    }

    /// Kristi's second case: 40 selected, 12 Automatic, set to Never with
    /// nothing waiting. It asks, naming the 12.
    #[test]
    fn turning_off_automatic_asks_with_the_count() {
        let mut rows: Vec<BulkProject> = (0..12)
            .map(|i| project(&format!("auto{i}"), Auto, 0))
            .collect();
        rows.extend((0..28).map(|i| project(&format!("ask{i}"), Ask, 0)));
        let plan = plan(&rows, Never);
        assert_eq!(plan.outcome, BulkOutcome::Confirm);
        assert_eq!(plan.turns_off_automatic, 12);
        assert_eq!(plan.pending, 0);
        assert_eq!(plan.project_ids.len(), 40);
        let copy = plan.confirmation.unwrap();
        assert_eq!(copy.title, "Set 40 projects to Never?");
        assert_eq!(copy.body, "This turns off Automatic for 12 projects.");
        assert_eq!(copy.button, "Never");
        assert_eq!(copy.keep, "Keep");
        assert_eq!(copy.help, None, "nothing waiting is removed");
    }

    /// Ask asks too when it turns off Automatic, and never counts waiting
    /// traces: Ask removes none.
    #[test]
    fn ask_over_automatic_asks_and_counts_no_pending() {
        let rows = [project("a", Auto, 5), project("b", Never, 2)];
        let plan = plan(&rows, Ask);
        assert_eq!(plan.outcome, BulkOutcome::Confirm);
        assert_eq!(plan.turns_off_automatic, 1);
        assert_eq!(plan.pending, 0);
        let copy = plan.confirmation.unwrap();
        assert_eq!(copy.title, "Set 2 projects to Ask?");
        assert_eq!(copy.body, "This turns off Automatic for 1 project.");
        assert_eq!(copy.button, "Ask");
        assert!(!copy.destructive);
    }

    /// Ask onto Ask and Never rows changes the Never ones silently.
    #[test]
    fn a_change_that_turns_nothing_off_and_removes_nothing_applies() {
        let rows = [project("a", Ask, 4), project("b", Never, 0)];
        let plan = plan(&rows, Ask);
        assert_eq!(plan.outcome, BulkOutcome::Apply);
        assert_eq!(plan.project_ids, ["b"]);
        assert_eq!(plan.confirmation, None);
        assert!(agreed_covers(&plan, None));
    }

    /// Pending is summed over the projects that change only.
    #[test]
    fn pending_counts_only_the_projects_that_change() {
        let rows = [
            project("a", Never, 9),
            project("b", Ask, 2),
            project("c", Auto, 3),
        ];
        let plan = plan(&rows, Never);
        assert_eq!(plan.pending, 5);
        assert_eq!(plan.turns_off_automatic, 1);
        let copy = plan.confirmation.unwrap();
        assert_eq!(
            copy.body,
            "This turns off Automatic for 1 project. \
             This removes 5 waiting traces. Traces already sent aren't affected."
        );
        assert!(copy.help.is_some());
    }

    /// One project reached is named by its label.
    #[test]
    fn one_project_is_named_by_its_label() {
        let rows = [project("a", Never, 0), project("b", Ask, 1)];
        let copy = plan(&rows, Never).confirmation.unwrap();
        assert_eq!(copy.title, "Set label-b to Never?");
    }

    #[test]
    fn nothing_to_change_is_nothing() {
        assert_eq!(plan(&[], Never).outcome, BulkOutcome::Nothing);
        let mut bucket = project("u", Ask, 0);
        bucket.can_take = false;
        assert_eq!(plan(&[bucket], Never).outcome, BulkOutcome::Nothing);
        assert_eq!(
            plan(&[project("a", Never, 1)], Never).outcome,
            BulkOutcome::Nothing
        );
    }

    /// A change whose live counts grew past what was agreed is not
    /// covered; equal or fewer is.
    #[test]
    fn an_agreement_covers_only_counts_that_did_not_grow() {
        let rows = [project("a", Auto, 0), project("b", Ask, 2)];
        let plan = plan(&rows, Never);
        assert_eq!((plan.turns_off_automatic, plan.pending), (1, 2));
        assert!(!agreed_covers(&plan, None));
        let exact = Agreed {
            turns_off_automatic: 1,
            pending: 2,
        };
        assert!(agreed_covers(&plan, Some(exact)));
        assert!(agreed_covers(
            &plan,
            Some(Agreed {
                turns_off_automatic: 3,
                pending: 9
            })
        ));
        assert!(!agreed_covers(
            &plan,
            Some(Agreed {
                turns_off_automatic: 1,
                pending: 1
            })
        ));
        assert!(!agreed_covers(
            &plan,
            Some(Agreed {
                turns_off_automatic: 0,
                pending: 2
            })
        ));
    }

    #[test]
    fn the_unknown_bucket_cannot_take_automatic() {
        use super::super::policy::UNKNOWN_PROJECT_KEY;
        assert!(!can_take(UNKNOWN_PROJECT_KEY, Auto));
        assert!(can_take(UNKNOWN_PROJECT_KEY, Never));
        assert!(can_take("/tmp/x", Ask));
    }
}
