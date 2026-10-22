//! The judgement digest's scrub ignores the privacy-filter environment
//! (private-judgements addendum, "The scrub"): a digest built with
//! `TRACE_PRIVACY_FILTER_BACKEND=near-ai` set is byte-identical to one built
//! without it, so no network filter can run before consent.
//!
//! Its own test binary, with one test, so setting the variable races no other
//! test's environment read.

use trace_commons_contributor_insights::insights::analytics_constants::CLASSIFIER_VERSION;
use trace_commons_contributor_insights::insights::judgement_digest::{
    DigestInput, ScrubContext, build_judgement_digest,
};
use trace_commons_contributor_insights::insights::session_facts::{
    EditMode, ExtensionClassCounts, InferredTask, ModelFacts, SessionFacts, SessionShape,
};

fn facts() -> SessionFacts {
    SessionFacts {
        models: vec![ModelFacts {
            label_ix: 0,
            edits: Some(2),
            denied: Some(0),
            undone: Some(0),
            interrupted: Some(1),
        }],
        shape: SessionShape {
            edited_files: ExtensionClassCounts {
                test: 0,
                doc: 0,
                source: 1,
                config: 0,
            },
            renamed_files: 0,
            lines_added: Some(4),
            lines_removed: Some(1),
            failed_before_first_edit_then_passed: 0,
        },
        task: InferredTask::BugFix,
        classifier_version: CLASSIFIER_VERSION,
        active_secs: Some(60),
        finished: Some(true),
        user_turns: Some(2),
        edit_mode: EditMode::Prompting,
        capped: false,
        delegated_unread: false,
        reviewer: false,
    }
}

fn build(facts: &SessionFacts, prompts: &[String]) -> Vec<u8> {
    build_judgement_digest(&DigestInput {
        facts,
        series: None,
        prompts,
        final_message: Some("Updated the handler for Jane Doe in billing"),
        scrub: ScrubContext {
            home_dir: Some("/Users/alice"),
            working_dir: Some("/Users/alice/code/acme-billing"),
        },
        edited_extensions: &["rs"],
        events: &[],
    })
    .expect("digest builds")
    .bytes
}

#[test]
fn a_network_privacy_filter_in_the_environment_changes_nothing() {
    let facts = facts();
    let prompts = vec![
        "please fix the refund flow that Jane Doe reported".to_owned(),
        "now run the tests in /Users/alice/code/acme-billing".to_owned(),
    ];
    let without = build(&facts, &prompts);
    // SAFETY: this binary holds one test, so nothing reads the environment
    // concurrently.
    unsafe {
        std::env::set_var("TRACE_PRIVACY_FILTER_BACKEND", "near-ai");
    }
    let with = build(&facts, &prompts);
    unsafe {
        std::env::remove_var("TRACE_PRIVACY_FILTER_BACKEND");
    }
    assert_eq!(with, without);
    assert!(!String::from_utf8(with).unwrap().contains("Jane"));
}
