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
#[path = "fixtures_tests.rs"]
mod tests;
