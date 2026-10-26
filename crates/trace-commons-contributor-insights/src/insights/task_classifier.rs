//! Infers a session's task type from its stored counts (spec section 5).
//!
//! The classifier reads only [`SessionShape`] counts. [`path_class`] runs
//! during a scan on a path the scan holds in memory; its result is counted
//! and the path is never stored.

use trace_commons_protocol::insights_usage_series::normalize_tool_path;

use super::analytics_constants::{
    CLASSIFIER_VERSION, CONFIG_EXTENSIONS, DOC_DIRS, DOC_EXTENSIONS, REFACTOR_MIN_FILES,
    REFACTOR_MIN_RENAMES, REFACTOR_NET_LINES, TEST_DIRS, TEST_NAME_MARKS, TEST_NAME_PREFIX,
};
use super::session_facts::{InferredTask, SessionFacts, SessionShape};

/// The extension class of one edited file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PathClass {
    Test,
    Doc,
    Source,
    Config,
}

/// Classifies a path, checking test, then doc, then config by extension;
/// anything else is source. Directory and file names compare
/// ASCII-case-insensitively on the normalized path.
pub fn path_class(path: &str) -> PathClass {
    let normalized = normalize_tool_path(path).to_ascii_lowercase();
    let mut segments: Vec<&str> = normalized.split('/').filter(|s| !s.is_empty()).collect();
    let name = segments.pop().unwrap_or("");
    let dirs = segments;
    let ext = name
        .rsplit_once('.')
        .filter(|(stem, _)| !stem.is_empty())
        .map(|(_, ext)| ext);

    let in_test_dir = dirs.iter().any(|dir| TEST_DIRS.contains(dir));
    let test_name = name.starts_with(TEST_NAME_PREFIX)
        || TEST_NAME_MARKS.iter().any(|mark| name.contains(mark));
    if in_test_dir || test_name {
        return PathClass::Test;
    }
    if let Some(ext) = ext {
        if DOC_EXTENSIONS.contains(&ext) {
            return PathClass::Doc;
        }
        if ext == "txt" && dirs.iter().any(|dir| DOC_DIRS.contains(dir)) {
            return PathClass::Doc;
        }
        if CONFIG_EXTENSIONS.contains(&ext) {
            return PathClass::Config;
        }
    }
    PathClass::Source
}

/// The seam a different classifier substitutes at.
pub trait TaskClassifier {
    fn classify(&self, shape: &SessionShape) -> InferredTask;
    fn version(&self) -> u32;
}

/// The local rules of spec section 5, at [`CLASSIFIER_VERSION`].
#[derive(Debug, Clone, Copy, Default)]
pub struct LocalRulesClassifier;

impl TaskClassifier for LocalRulesClassifier {
    fn classify(&self, shape: &SessionShape) -> InferredTask {
        classify(shape)
    }

    fn version(&self) -> u32 {
        CLASSIFIER_VERSION
    }
}

/// The local rules, first match wins: Docs, Tests, BugFix, Refactor, then
/// Other when any file was edited and NoEdits otherwise.
pub fn classify(shape: &SessionShape) -> InferredTask {
    let files = shape.edited_files;
    let total = u64::from(files.total());
    if total == 0 {
        return InferredTask::NoEdits;
    }
    if u64::from(files.doc) == total {
        return InferredTask::Docs;
    }
    if u64::from(files.test) * 2 > total {
        return InferredTask::Tests;
    }
    if shape.failed_before_first_edit_then_passed > 0 && u64::from(files.test) < total {
        return InferredTask::BugFix;
    }
    if shape.renamed_files >= REFACTOR_MIN_RENAMES
        || (total >= u64::from(REFACTOR_MIN_FILES) && lines_balanced(shape))
    {
        return InferredTask::Refactor;
    }
    InferredTask::Other
}

/// `|added - removed| <= REFACTOR_NET_LINES * (added + removed)`. False when
/// either count is unknown or no line changed.
fn lines_balanced(shape: &SessionShape) -> bool {
    let (Some(added), Some(removed)) = (shape.lines_added, shape.lines_removed) else {
        return false;
    };
    let changed = u128::from(added) + u128::from(removed);
    if changed == 0 {
        return false;
    }
    let net = u128::from(added.abs_diff(removed));
    net * u128::from(REFACTOR_NET_LINES.den) <= u128::from(REFACTOR_NET_LINES.num) * changed
}

/// The task the local rules give the facts now: as stored when they were
/// classified under this version, else re-run from the stored shape.
pub fn current_task(facts: &SessionFacts) -> InferredTask {
    let classifier = LocalRulesClassifier;
    if facts.classifier_version == classifier.version() {
        facts.task
    } else {
        classifier.classify(&facts.shape)
    }
}

/// Re-runs the local rules from the stored shape when the facts were
/// classified under another version.
pub fn reclassify(facts: &mut SessionFacts) {
    facts.task = current_task(facts);
    facts.classifier_version = LocalRulesClassifier.version();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::insights::TaskCategory;
    use crate::insights::analytics_constants::CLASSIFIER_VERSION;
    use crate::insights::session_facts::{
        EditMode, ExtensionClassCounts, InferredTask, ModelFacts, SessionFacts, SessionShape,
    };

    fn files(test: u32, doc: u32, source: u32, config: u32) -> SessionShape {
        SessionShape {
            edited_files: ExtensionClassCounts {
                test,
                doc,
                source,
                config,
            },
            renamed_files: 0,
            lines_added: Some(0),
            lines_removed: Some(0),
            failed_before_first_edit_then_passed: 0,
        }
    }

    #[test]
    fn all_doc_files_is_docs() {
        assert_eq!(classify(&files(0, 2, 0, 0)), InferredTask::Docs);
        // One non-doc file breaks the rule.
        assert_ne!(classify(&files(0, 2, 1, 0)), InferredTask::Docs);
    }

    #[test]
    fn a_txt_outside_docs_is_config_not_doc() {
        assert_eq!(path_class("CMakeLists.txt"), PathClass::Config);
        assert_eq!(path_class("docs/notes.txt"), PathClass::Doc);
        assert_eq!(path_class("doc/sub/notes.txt"), PathClass::Doc);
        assert_eq!(path_class("requirements.txt"), PathClass::Config);
        assert_eq!(path_class("src/notes.txt"), PathClass::Config);
        assert_eq!(path_class("README.md"), PathClass::Doc);
        assert_eq!(path_class("guide.adoc"), PathClass::Doc);
    }

    #[test]
    fn path_classes_check_test_then_doc_then_source_then_config() {
        assert_eq!(path_class("tests/fixture.json"), PathClass::Test);
        assert_eq!(path_class("src/__tests__/a.ts"), PathClass::Test);
        assert_eq!(path_class("spec/README.md"), PathClass::Test);
        assert_eq!(path_class("src/foo_test.go"), PathClass::Test);
        assert_eq!(path_class("src/foo.test.ts"), PathClass::Test);
        assert_eq!(path_class("src/foo.spec.js"), PathClass::Test);
        assert_eq!(path_class("test_parser.py"), PathClass::Test);
        assert_eq!(path_class("src\\tests\\a.rs"), PathClass::Test);
        assert_eq!(path_class("src/main.rs"), PathClass::Source);
        assert_eq!(path_class("Makefile"), PathClass::Source);
        assert_eq!(path_class("Cargo.toml"), PathClass::Config);
        assert_eq!(path_class("Cargo.lock"), PathClass::Config);
        assert_eq!(path_class("config/app.YAML"), PathClass::Config);
        // A directory named like a test file is not a test mark; only the
        // file name carries one.
        assert_eq!(path_class("test_utils/main.rs"), PathClass::Source);
        assert_eq!(path_class("/abs/latest/contest.rs"), PathClass::Source);
    }

    #[test]
    fn more_than_half_test_files_is_tests() {
        assert_eq!(classify(&files(2, 0, 1, 0)), InferredTask::Tests);
        assert_ne!(classify(&files(1, 0, 1, 0)), InferredTask::Tests);
    }

    #[test]
    fn a_command_failing_before_the_first_edit_then_passing_is_bug_fix() {
        let mut shape = files(1, 0, 1, 0);
        shape.failed_before_first_edit_then_passed = 1;
        assert_eq!(classify(&shape), InferredTask::BugFix);
    }

    #[test]
    fn tdd_a_test_written_then_made_to_pass_is_not_bug_fix() {
        // failed_before_first_edit_then_passed == 0: the failing run came
        // after the first edit.
        let tdd_shape = files(1, 0, 1, 0);
        assert_ne!(classify(&tdd_shape), InferredTask::BugFix);
        assert_eq!(classify(&tdd_shape), InferredTask::Other);
    }

    #[test]
    fn a_bug_fix_touching_only_test_files_is_tests() {
        let mut shape = files(2, 0, 0, 0);
        shape.failed_before_first_edit_then_passed = 1;
        assert_eq!(classify(&shape), InferredTask::Tests);
        // Exactly half test files: rule 2 misses, and the other half is
        // not test-class, so rule 3 matches.
        let mut half = files(1, 0, 1, 0);
        half.failed_before_first_edit_then_passed = 1;
        assert_eq!(classify(&half), InferredTask::BugFix);
    }

    #[test]
    fn renames_or_balanced_lines_over_five_files_is_refactor() {
        let mut renames = files(0, 0, 1, 0);
        renames.renamed_files = 3;
        assert_eq!(classify(&renames), InferredTask::Refactor);
        renames.renamed_files = 2;
        assert_eq!(classify(&renames), InferredTask::Other);

        let mut balanced = files(0, 0, 5, 0);
        balanced.lines_added = Some(100);
        balanced.lines_removed = Some(95);
        assert_eq!(classify(&balanced), InferredTask::Refactor);

        let mut unbalanced = files(0, 0, 5, 0);
        unbalanced.lines_added = Some(100);
        unbalanced.lines_removed = Some(80);
        assert_eq!(classify(&unbalanced), InferredTask::Other);

        // Exactly 10%: within.
        let mut edge = files(0, 0, 5, 0);
        edge.lines_added = Some(55);
        edge.lines_removed = Some(45);
        assert_eq!(classify(&edge), InferredTask::Refactor);

        // Four files, balanced: below the file floor.
        let mut few = files(0, 0, 4, 0);
        few.lines_added = Some(100);
        few.lines_removed = Some(100);
        assert_eq!(classify(&few), InferredTask::Other);

        // No line changed: the ratio is undefined and satisfies nothing.
        assert_eq!(classify(&files(0, 0, 5, 0)), InferredTask::Other);
    }

    #[test]
    fn unknown_lines_never_satisfy_the_line_clause() {
        let mut shape = files(0, 0, 5, 0);
        shape.lines_added = None;
        shape.lines_removed = None;
        assert_eq!(classify(&shape), InferredTask::Other);
        shape.lines_added = Some(100);
        assert_eq!(classify(&shape), InferredTask::Other);
    }

    #[test]
    fn no_edited_files_is_no_edits() {
        assert_eq!(classify(&files(0, 0, 0, 0)), InferredTask::NoEdits);
        // Renames and a passing command without an edited file are still
        // no edits.
        let mut shape = files(0, 0, 0, 0);
        shape.renamed_files = 4;
        shape.failed_before_first_edit_then_passed = 1;
        assert_eq!(classify(&shape), InferredTask::NoEdits);
    }

    #[test]
    fn inferred_tasks_map_to_shared_labels_without_mixed() {
        assert_eq!(
            InferredTask::BugFix.task_category(),
            TaskCategory::Debugging
        );
        assert_eq!(InferredTask::NoEdits.task_category(), TaskCategory::Other);
        assert_eq!(
            InferredTask::Refactor.task_category(),
            TaskCategory::Refactor
        );
        assert_eq!(InferredTask::Tests.task_category(), TaskCategory::Tests);
        assert_eq!(InferredTask::Docs.task_category(), TaskCategory::Docs);
        assert_eq!(InferredTask::Other.task_category(), TaskCategory::Other);
    }

    #[test]
    fn the_local_rules_classifier_is_the_versioned_classify() {
        let classifier = LocalRulesClassifier;
        assert_eq!(classifier.version(), CLASSIFIER_VERSION);
        assert_eq!(classifier.classify(&files(0, 3, 0, 0)), InferredTask::Docs);
    }

    fn facts(task: InferredTask, classifier_version: u32, shape: SessionShape) -> SessionFacts {
        SessionFacts {
            models: vec![ModelFacts {
                label_ix: 0,
                edits: Some(2),
                denied: None,
                undone: Some(0),
                interrupted: Some(1),
            }],
            shape,
            task,
            classifier_version,
            active_secs: Some(120),
            finished: None,
            user_turns: Some(3),
            edit_mode: EditMode::Unknown,
            capped: false,
            delegated_unread: false,
            reviewer: false,
        }
    }

    #[test]
    fn a_stale_classifier_version_reclassifies_from_the_stored_shape() {
        let mut stale = facts(InferredTask::Other, 0, files(0, 2, 0, 0));
        reclassify(&mut stale);
        assert_eq!(stale.task, InferredTask::Docs);
        assert_eq!(stale.classifier_version, CLASSIFIER_VERSION);

        // A current version is left as stored.
        let mut current = facts(InferredTask::Other, CLASSIFIER_VERSION, files(0, 2, 0, 0));
        reclassify(&mut current);
        assert_eq!(current.task, InferredTask::Other);
    }

    #[test]
    fn facts_round_trip_and_reject_unknown_fields() {
        let original = facts(InferredTask::BugFix, CLASSIFIER_VERSION, files(1, 0, 2, 1));
        let bytes = serde_json::to_vec(&original).expect("serialize");
        let back: SessionFacts = serde_json::from_slice(&bytes).expect("deserialize");
        assert_eq!(back, original);

        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("value");
        assert_eq!(value["task"], "bug_fix");
        assert_eq!(value["edit_mode"], "unknown");
        assert_eq!(value["models"][0]["denied"], serde_json::Value::Null);
        assert_eq!(
            serde_json::to_value(EditMode::NotPrompting).expect("mode"),
            "not_prompting"
        );
        assert_eq!(
            serde_json::to_value(InferredTask::NoEdits).expect("task"),
            "no_edits"
        );

        for (pointer, field) in [
            ("", "extra"),
            ("/models/0", "committed"),
            ("/shape", "extra"),
            ("/shape/edited_files", "other"),
        ] {
            let mut value = value.clone();
            value
                .pointer_mut(pointer)
                .and_then(serde_json::Value::as_object_mut)
                .expect("object")
                .insert(field.to_owned(), serde_json::Value::from(1));
            assert!(
                serde_json::from_value::<SessionFacts>(value).is_err(),
                "{pointer}/{field} must be rejected"
            );
        }
    }
}
