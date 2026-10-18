//! Source assertions that need the daemon, kept here because the adapters
//! moved to `trace-commons-contributor-sources`, which must not depend on
//! this crate. Only public APIs are used.

use super::{SessionRef, TraceSource, claude_code::ClaudeCodeSource, session_hash};
use crate::daemon::queue::entry_id_for;

/// A minimal top-level Claude Code session record, the shape
/// `claude_code`'s own group tests write.
fn record(session: &str, text: &str) -> String {
    format!(
        "{{\"type\":\"user\",\"cwd\":\"/Users/testuser/code/myproj\",\
         \"sessionId\":\"{session}\",\
         \"message\":{{\"role\":\"user\",\"content\":\"{text}\"}}}}\n"
    )
}

/// Moved from `claude_code::tests`: a parent session with a delegated
/// transcript is one group, and that group must queue under a different
/// entry id than the parent file would alone. The adapter crate keeps the
/// hash half; this is the queue half.
#[test]
fn a_group_gets_a_different_entry_id_than_its_parent_file_alone() {
    let session = "22222222-2222-2222-2222-222222222222";
    let root = tempfile::tempdir().unwrap();
    let project_dir = root.path().join("-Users-testuser-code-myproj");
    let subagents = project_dir.join(session).join("subagents");
    std::fs::create_dir_all(&subagents).unwrap();
    let parent = project_dir.join(format!("{session}.jsonl"));
    std::fs::write(&parent, record(session, "parent turn")).unwrap();
    std::fs::write(subagents.join("agent-a.jsonl"), record(session, "a")).unwrap();

    let source = ClaudeCodeSource::new(root.path().to_path_buf());
    let found: Vec<SessionRef> = source.discover().unwrap();
    assert_eq!(found.len(), 1, "one group, not one ref per file: {found:?}");
    let group = source.load(&found[0]).unwrap().session_hash;
    assert_eq!(
        found[0].group_member_count, 1,
        "the delegated transcript is in the group"
    );
    let parent_only = session_hash(&std::fs::read(&parent).unwrap());

    assert_ne!(group, parent_only);
    assert_ne!(entry_id_for(&group), entry_id_for(&parent_only));
}
