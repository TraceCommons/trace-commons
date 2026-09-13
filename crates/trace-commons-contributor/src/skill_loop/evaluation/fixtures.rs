//! INTEGRATION: fixes the client-shipped public repository evidence used by
//! every arm of the generated-source-repair comparison.

use crate::skill_loop::{SkillSourceTaskFingerprint, source_task_fingerprint};

/// Raw excerpts copied from an immutable public Trace Commons commit. They are
/// shipped in the client so every arm receives identical evidence without a
/// network fetch. They are public benchmark inputs, not adversarial secrets.
#[derive(Clone, Copy)]
pub(super) struct RepositoryEvidence {
    pub(super) path: &'static str,
    pub(super) contents: &'static str,
}

#[derive(Clone, Copy)]
pub(super) struct EvaluationFixture {
    pub(super) id: &'static str,
    pub(super) cluster: &'static str,
    pub(super) source_url: &'static str,
    pub(super) snapshot_commit: &'static str,
    pub(super) task: &'static str,
    pub(super) evidence: &'static [RepositoryEvidence],
    /// Evaluator-only oracle fields. `fixture_prompt` must never serialize
    /// these arrays as an answer key.
    pub(super) required_edit_paths: &'static [&'static str],
    pub(super) generated_output_paths: &'static [&'static str],
    pub(super) regeneration_steps: &'static [&'static [&'static str]],
    pub(super) verification_steps: &'static [&'static [&'static str]],
}

#[derive(Clone, Copy)]
pub(super) struct ApplicabilityFixture {
    pub(super) id: &'static str,
    pub(super) task: &'static str,
    pub(super) guidance_should_apply: bool,
}

mod catalog;

use catalog::REPOSITORY_URL;
pub(super) use catalog::{FIXTURES, PLAN_TASK_COUNT};
#[cfg(test)]
use catalog::{FLATPAK_COMMIT, FLATPAK_DEPENDENCY, MARK_COMMIT};

pub(super) struct HeldOutFixtureSelection {
    pub(super) selected: Vec<EvaluationFixture>,
    pub(super) source_overlap_ids: Vec<String>,
    pub(super) reserve_ids: Vec<String>,
}

pub(super) fn select_held_out_fixtures(
    source: &SkillSourceTaskFingerprint,
) -> Option<HeldOutFixtureSelection> {
    if !source.is_valid() {
        return None;
    }
    let mut eligible = Vec::with_capacity(FIXTURES.len());
    let mut source_overlap_ids = Vec::new();
    for fixture in FIXTURES {
        let fixture_fingerprint = source_task_fingerprint(fixture.task)?;
        if source.is_exact_or_near_duplicate(&fixture_fingerprint) {
            source_overlap_ids.push(fixture.id.to_string());
        } else {
            eligible.push(*fixture);
        }
    }
    if eligible.len() < PLAN_TASK_COUNT {
        return None;
    }
    let reserve_ids = eligible[PLAN_TASK_COUNT..]
        .iter()
        .map(|fixture| fixture.id.to_string())
        .collect();
    eligible.truncate(PLAN_TASK_COUNT);
    Some(HeldOutFixtureSelection {
        selected: eligible,
        source_overlap_ids,
        reserve_ids,
    })
}

pub(super) const APPLICABILITY_FIXTURES: &[ApplicabilityFixture] = &[
    ApplicabilityFixture {
        id: "discovery-positive-generated-source",
        task: "A checked-in Windows package image is generated from shared Rust source; change its background and regenerate every scale variant.",
        guidance_should_apply: true,
    },
    ApplicabilityFixture {
        id: "discovery-negative-handwritten-docs",
        task: "Correct punctuation in a hand-written README paragraph. The file is not generated and no derived output changes.",
        guidance_should_apply: false,
    },
];

pub(super) const APPLICABILITY_SOURCE_URL: &str = REPOSITORY_URL;

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;

    #[test]
    fn fixtures_are_one_task_per_distinct_verified_incident() {
        assert_eq!(FIXTURES.len(), PLAN_TASK_COUNT + 1);
        assert_eq!(PLAN_TASK_COUNT % 3, 0);
        assert_eq!(
            FIXTURES
                .iter()
                .map(|fixture| fixture.id)
                .collect::<BTreeSet<_>>()
                .len(),
            FIXTURES.len()
        );
        assert_eq!(
            FIXTURES
                .iter()
                .map(|fixture| fixture.cluster)
                .collect::<BTreeSet<_>>()
                .len(),
            FIXTURES.len()
        );
        assert_eq!(
            FIXTURES
                .iter()
                .map(|fixture| fixture.source_url)
                .collect::<BTreeSet<_>>()
                .len(),
            FIXTURES.len()
        );
        for fixture in FIXTURES {
            assert_eq!(fixture.snapshot_commit.len(), 40);
            assert!(
                fixture
                    .snapshot_commit
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
            );
            assert!(fixture.source_url.ends_with(fixture.snapshot_commit));
        }
    }

    #[test]
    fn public_evidence_is_client_shipped_nonempty_and_bounded() {
        for fixture in FIXTURES {
            assert!(!fixture.evidence.is_empty());
            let bytes = fixture
                .evidence
                .iter()
                .map(|item| item.path.len() + item.contents.len())
                .sum::<usize>();
            assert!(
                bytes <= 16 * 1024,
                "{} evidence is {bytes} bytes",
                fixture.id
            );
            assert!(
                fixture
                    .evidence
                    .iter()
                    .all(|item| !item.path.is_empty() && !item.contents.is_empty())
            );
        }
    }

    #[test]
    fn corrected_mark_and_flatpak_oracles_name_real_history() {
        assert_eq!(MARK_COMMIT, "b6722426bb4b83d90425494b664ac468d67943b5");
        assert_eq!(FLATPAK_COMMIT, "e89a628bf4be8b1e31dd278793b00391e9781f1c");
        assert!(FLATPAK_DEPENDENCY.contains("git = \"https://github.com/nearai/ironwire\""));
    }

    #[test]
    fn applicability_set_has_one_positive_and_one_unrelated_negative() {
        assert_eq!(APPLICABILITY_FIXTURES.len(), 2);
        assert!(APPLICABILITY_FIXTURES[0].guidance_should_apply);
        assert!(!APPLICABILITY_FIXTURES[1].guidance_should_apply);
        assert!(APPLICABILITY_FIXTURES[1].task.contains("not generated"));
    }

    #[test]
    fn held_out_selection_excludes_the_source_incident_and_stays_balanced() {
        let source = source_task_fingerprint(FIXTURES[0].task).expect("source fingerprint");
        let selection = select_held_out_fixtures(&source).expect("six held-out fixtures remain");
        assert_eq!(selection.selected.len(), PLAN_TASK_COUNT);
        assert_eq!(selection.selected.len() % 3, 0);
        assert_eq!(selection.source_overlap_ids, vec![FIXTURES[0].id]);
        assert!(selection.reserve_ids.is_empty());
        assert!(
            selection
                .selected
                .iter()
                .all(|fixture| fixture.id != FIXTURES[0].id)
        );
    }

    #[test]
    fn held_out_selection_discloses_an_unused_reserve_without_overlap() {
        let source = source_task_fingerprint(
            "Repair a generated GraphQL client from its schema and run its drift check.",
        )
        .expect("source fingerprint");
        let selection = select_held_out_fixtures(&source).expect("selection");
        assert_eq!(selection.selected.len(), PLAN_TASK_COUNT);
        assert!(selection.source_overlap_ids.is_empty());
        assert_eq!(selection.reserve_ids, vec![FIXTURES[PLAN_TASK_COUNT].id]);
    }

    #[test]
    fn distinct_fixture_tasks_remain_outside_the_near_duplicate_cutoff() {
        for (index, left) in FIXTURES.iter().enumerate() {
            let left = source_task_fingerprint(left.task).expect("left fingerprint");
            for right in FIXTURES.iter().skip(index + 1) {
                let right_fingerprint =
                    source_task_fingerprint(right.task).expect("right fingerprint");
                assert!(
                    !left.is_exact_or_near_duplicate(&right_fingerprint),
                    "fixture {} overlaps fixture {}",
                    FIXTURES[index].id,
                    right.id
                );
            }
        }
    }
}
