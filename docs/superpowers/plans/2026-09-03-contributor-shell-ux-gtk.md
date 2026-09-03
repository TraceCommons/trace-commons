# Contributor Shell UX -- GTK (Linux) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bring the Linux contributor shell to the same folder-first queue
and scrubber transparency as the spec describes, matching the macOS shell's
behavior and wording.

**Architecture:** The GTK shell builds widgets imperatively in `render`
functions. Every decision this plan adds goes into a pure function with a
`#[test]` beside it, and the `render` functions call those -- the same
discipline the macOS plan uses, for a better reason here: `cargo test` on
this crate actually runs in CI, so a pure function is genuinely guarded.

**Tech Stack:** Rust 2024, GTK 4, libadwaita, `serde_json`.

**Spec:** [`docs/superpowers/specs/2026-09-03-contributor-shell-queue-ux-design.md`](../specs/2026-09-03-contributor-shell-queue-ux-design.md)

**Depends on:** [`2026-09-03-contributor-shell-ux-foundation.md`](2026-09-03-contributor-shell-ux-foundation.md)
(plan 1). **Do not start until plan 1 is merged.**

## What this shell already gets right

Two spec items need no work here, and both are worth knowing before you go
looking for them:

- **Recent searches (spec §3.3) are already correct.** `run_search(needle,
  remember)` takes the flag explicitly: `connect_search_changed` passes
  `false`, while `connect_activate` and the Search button pass `true`
  (`ui/preview.rs:650-663`). The prefix bug the reporter hit -- typing `xyz`
  recording `x`, `xy`, `xyz` -- is a macOS-only defect. **Do not "fix" this
  file.** If anything, the macOS plan copies this shape.
- **Opening the preview on a chosen tab already exists.**
  `preview::open_with_search(app, index, term, tab)` takes both a search term
  and a tab name, so Task 7's nothing-matched affordance is a call, not a new
  mechanism.

## Global Constraints

- **This crate is its own workspace with its own `Cargo.lock`.** Every
  command below needs
  `--manifest-path crates/trace-commons-contributor-gtk/Cargo.toml`; a bare
  `cargo test` at the repo root does not build this crate at all. If the
  lockfile changes, commit it -- it drifts from the root lockfile
  independently and only an `app-v*` tag catches the drift late.
- **No new dependencies.** If a task seems to need one, stop and ask.
- **CI runs build, `cargo test`, `cargo fmt --check`, and `cargo clippy` on
  this crate**, all with `--manifest-path`, plus a weston portal check that
  this plan does not touch. Clippy uses the repo allow-list; do not widen it.
- **All copy lives in `src/copy.rs`.** Do not inline a user-visible string in
  a widget. The three shells share wording, and `copy.rs` is what makes that
  checkable.
- **Design tokens only** -- `space::*`, `style::*`, `Tone`, and the CSS
  classes in `ui/css_contract.rs`. That module asserts the classes actually
  exist; a class name typed into a widget and never defined fails its test.
- **No emojis.**
- **`Look inside` keeps `suggested-action` / `tc-primary`.** Task 5 adds a
  second route; the button is not demoted.

---

### Task 1: Decode the new daemon fields

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/model.rs:104-180` (`QueueEntry`), `:182-200` (`PreviewSummary`), and `HistoryRecord`
- Test: same file, `#[cfg(test)] mod tests`

**Interfaces:**
- Consumes: plan 1 Tasks 5, 6, 7.
- Produces: `QueueEntry.project_path: String`, `QueueEntry.session_path: Option<String>`,
  `PreviewSummary.redactions_distinct: BTreeMap<String, u32>`,
  `HistoryRecord.project_id: String`, `ProjectRow.project_path: String`.

Every field on these structs already carries `#[serde(default)]`, so
tolerating an older daemon is free -- but assert it anyway, because that is
the property, not the attribute.

- [ ] **Step 1: Write the failing tests**

Add to `model.rs`'s test module:

```rust
    #[test]
    fn a_queue_entry_decodes_the_project_and_session_paths() {
        let e: QueueEntry = serde_json::from_value(serde_json::json!({
            "entry_id": "e1",
            "project_id": "proj_a",
            "project_label": "repo",
            "project_path": "~/code/repo",
            "session_path": "~/code/repo/crates/inner",
            "state": "pending"
        }))
        .unwrap();
        assert_eq!(e.project_path, "~/code/repo");
        assert_eq!(e.session_path.as_deref(), Some("~/code/repo/crates/inner"));
    }

    #[test]
    fn a_queue_entry_from_an_older_daemon_has_no_paths() {
        let e: QueueEntry = serde_json::from_value(serde_json::json!({
            "entry_id": "e1", "project_id": "proj_a",
            "project_label": "repo", "state": "pending"
        }))
        .unwrap();
        assert_eq!(e.project_path, "");
        assert_eq!(e.session_path, None);
    }

    #[test]
    fn a_preview_summary_decodes_distinct_redaction_counts() {
        let p: PreviewSummary = serde_json::from_value(serde_json::json!({
            "redactions": { "local_path": 185 },
            "redactions_distinct": { "local_path": 12 }
        }))
        .unwrap();
        assert_eq!(p.redactions.get("local_path"), Some(&185));
        assert_eq!(p.redactions_distinct.get("local_path"), Some(&12));
    }

    #[test]
    fn a_preview_summary_from_an_older_daemon_has_no_distinct_counts() {
        let p: PreviewSummary = serde_json::from_value(serde_json::json!({
            "redactions": { "local_path": 185 }
        }))
        .unwrap();
        assert!(p.redactions_distinct.is_empty());
    }

    #[test]
    fn a_history_record_decodes_its_project_id() {
        let r: HistoryRecord = serde_json::from_value(serde_json::json!({
            "submission_id": "s1",
            "project_id": "proj_a",
            "project_label": "repo",
            "status": "accepted"
        }))
        .unwrap();
        assert_eq!(r.project_id, "proj_a");
    }

    #[test]
    fn a_history_record_from_before_the_upgrade_has_no_project_id() {
        let r: HistoryRecord = serde_json::from_value(serde_json::json!({
            "submission_id": "s1", "project_label": "repo", "status": "accepted"
        }))
        .unwrap();
        assert_eq!(r.project_id, "");
    }
```

If a struct's other required fields make these literals fail to deserialize,
add only what the deserializer demands -- do not relax a field to
`#[serde(default)]` to make a test pass.

- [ ] **Step 2: Run the tests to verify they fail**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml model::tests
```

Expected: compile error, `no field project_path on type QueueEntry`.

- [ ] **Step 3: Add the fields**

In `model.rs`, in `QueueEntry` after `project_label`:

```rust
    /// The project's folder, `~`-abbreviated, for display only.
    ///
    /// The daemon relaxed its path rule in exactly one place to send this
    /// (`ipc::display_path`), because a label can keep two projects distinct
    /// but can never make them identifiable, and the folder rows are where
    /// that difference is decided. Never logged, never in a notification,
    /// never in a history record.
    #[serde(default)]
    pub project_path: String,
    /// Where this session actually ran, when that is not the project root.
    ///
    /// `None` both when the daemon predates the field and when the session
    /// ran at the root -- the daemon sends null rather than repeating
    /// `project_path`, so a row draws this line only when it says something.
    #[serde(default)]
    pub session_path: Option<String>,
```

In `PreviewSummary` after `redactions`:

```rust
    /// Distinct values removed per label, beside `redactions`' occurrence
    /// counts. See `redaction_tally`.
    #[serde(default)]
    pub redactions_distinct: std::collections::BTreeMap<String, u32>,
```

In `HistoryRecord` before `project_label`, and in `ProjectRow` beside its
label, with the doc comments from the spec's §5 and §1.1 respectively.

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml model::tests
```

Expected: 6 new tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git add crates/trace-commons-contributor-gtk/src/model.rs
git commit -m "Decode the project path, session path, distinct counts, and history project id"
```

---

### Task 2: `redaction_tally` -- the removed-by-pattern figure

`removed_by_pattern` (`ui/queue.rs:1143`) reformats
`PreviewSummary::scrubbed_line()` by stripping a prefix and swapping
separators. It now has to fold in distinct counts, and string-surgery on a
sentence built elsewhere is the wrong place to do it.

**Files:**
- Create: `crates/trace-commons-contributor-gtk/src/redaction_tally.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/main.rs` or `lib.rs` (add `mod redaction_tally;`)
- Modify: `crates/trace-commons-contributor-gtk/src/ui/queue.rs:1143-1149`
- Test: `redaction_tally.rs`, inline

**Interfaces:**
- Consumes: `PreviewSummary.redactions`, `.redactions_distinct` (Task 1).
- Produces:
  - `pub fn line(occurrences: &BTreeMap<String, u32>, distinct: &BTreeMap<String, u32>) -> String`
  - `pub fn total(occurrences: &BTreeMap<String, u32>) -> u32`

- [ ] **Step 1: Write the failing tests**

Create `crates/trace-commons-contributor-gtk/src/redaction_tally.rs` with
the tests and a `todo!()`-free stub returning `String::new()`:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn map(pairs: &[(&str, u32)]) -> BTreeMap<String, u32> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn an_empty_tally_is_nothing_matched() {
        assert_eq!(line(&map(&[]), &map(&[])), crate::copy::NOTHING_MATCHED);
        assert_eq!(total(&map(&[])), 0);
    }

    #[test]
    fn labels_are_human_readable() {
        assert_eq!(line(&map(&[("local_path", 3)]), &map(&[])), "3 local path");
    }

    #[test]
    fn distinct_counts_are_shown_when_they_differ_from_occurrences() {
        assert_eq!(
            line(&map(&[("local_path", 185)]), &map(&[("local_path", 12)])),
            "185 local path (12 distinct)"
        );
    }

    #[test]
    fn distinct_is_omitted_when_every_occurrence_is_its_own_value() {
        // "3 secret (3 distinct)" says the same thing twice.
        assert_eq!(line(&map(&[("secret", 3)]), &map(&[("secret", 3)])), "3 secret");
    }

    #[test]
    fn distinct_is_omitted_when_the_daemon_did_not_report_it() {
        assert_eq!(line(&map(&[("secret", 3)]), &map(&[])), "3 secret");
    }

    #[test]
    fn a_distinct_count_above_its_occurrence_count_is_ignored() {
        // Impossible from a correct daemon; "3 secret (9 distinct)" would be
        // worse than saying nothing.
        assert_eq!(line(&map(&[("secret", 3)]), &map(&[("secret", 9)])), "3 secret");
    }

    #[test]
    fn the_biggest_count_leads_and_ties_break_on_label() {
        assert_eq!(
            line(&map(&[("secret", 3), ("local_path", 185), ("email", 3)]), &map(&[])),
            "185 local path \u{00b7} 3 email \u{00b7} 3 secret"
        );
    }

    #[test]
    fn total_sums_occurrences_not_distinct() {
        assert_eq!(total(&map(&[("a", 2), ("b", 3)])), 5);
    }

    /// `residual_secret_at:*` counts a secret that was DETECTED AND NOT
    /// REMOVED. It arrives in the same map as every genuine removal, and
    /// this line renders under the heading "Removed by pattern" -- so
    /// including it states the exact opposite of what happened, on the
    /// screen where someone is deciding whether to send the thing.
    #[test]
    fn a_residual_survivor_is_not_counted_as_removed() {
        let m = map(&[("local_path", 3), ("residual_secret_at:events.correction", 1)]);
        assert_eq!(line(&m, &map(&[])), "3 local path");
        assert_eq!(total(&m), 3);
    }

    /// A session whose only count is a survivor removed nothing.
    #[test]
    fn a_session_with_only_a_residual_matched_nothing() {
        let m = map(&[("residual_secret_at:events.x", 1)]);
        assert_eq!(line(&m, &map(&[])), crate::copy::NOTHING_MATCHED);
        assert_eq!(total(&m), 0);
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml redaction_tally
```

Expected: `cannot find function line in this scope`.

- [ ] **Step 3: Write the implementation**

At the top of `redaction_tally.rs`:

```rust
//! The card's "removed by pattern" figure.
//!
//! The daemon reports two maps. `redactions` counts OCCURRENCES -- how many
//! times a pattern fired. `redactions_distinct` counts VALUES, because the
//! redactor mints one placeholder per distinct value and reuses it wherever
//! that value recurs. One path referenced two hundred times is two hundred
//! occurrences and one value, and a card reporting only the first overstates
//! how much of the session was touched.
//!
//! A module rather than a method on `PreviewSummary` because it is the only
//! part of that strip with a right and a wrong answer, and because the macOS
//! and Windows shells carry the identical rules -- three copies of a
//! judgement need three test suites saying the same thing.

use std::collections::BTreeMap;

/// The prefix marking a secret that was DETECTED AND NOT REMOVED.
///
/// `note_residual_secret_location` increments this when a secret survives
/// redaction -- a credential inside a human correction, kept by design, or a
/// field the typed traversal never visits, which is a real gap. It rides in
/// the same map as every genuine removal, and everything here renders under
/// "Removed by pattern", so it is excluded from both figures.
pub const RESIDUAL_PREFIX: &str = "residual_secret_at";

/// The part of a label before its first `:`. The vocabulary is namespaced
/// and open -- `secret:{pattern}`, `privacy_filter:{label}`,
/// `tool_sensitive_field:{action}` are generated -- so a shell can only
/// reason about families, never a closed set of labels.
pub fn family(label: &str) -> &str {
    label.split_once(':').map_or(label, |(head, _)| head)
}

pub fn is_removal(label: &str) -> bool {
    family(label) != RESIDUAL_PREFIX
}

/// Total occurrences of things that were actually removed.
pub fn total(occurrences: &BTreeMap<String, u32>) -> u32 {
    occurrences
        .iter()
        .filter(|(label, _)| is_removal(label))
        .map(|(_, count)| count)
        .sum()
}

/// "185 local path (12 distinct) · 3 secret"
///
/// Ordered by count so the biggest number is first -- what a person scanning
/// a column of cards is looking for -- with ties broken on the label so the
/// order is stable between two renders.
pub fn line(
    occurrences: &BTreeMap<String, u32>,
    distinct: &BTreeMap<String, u32>,
) -> String {
    let mut parts: Vec<(&String, &u32)> = occurrences
        .iter()
        .filter(|(label, _)| is_removal(label))
        .collect();
    if parts.is_empty() {
        return crate::copy::NOTHING_MATCHED.to_string();
    }
    parts.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    parts
        .into_iter()
        .map(|(label, count)| {
            let words = label.replace('_', " ");
            // Only when it says something the occurrence count did not.
            match distinct.get(label) {
                Some(&values) if values > 0 && values < *count => {
                    format!("{count} {words} ({values} distinct)")
                }
                _ => format!("{count} {words}"),
            }
        })
        .collect::<Vec<_>>()
        .join(" \u{00b7} ")
}
```

Register the module beside the other `mod` declarations in the crate root.

- [ ] **Step 4: Point the view at it**

Replace `removed_by_pattern` in `ui/queue.rs` with a call:

```rust
fn removed_by_pattern(preview: &PreviewSummary) -> String {
    crate::redaction_tally::line(&preview.redactions, &preview.redactions_distinct)
}
```

- [ ] **Step 5: Run the tests**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

Expected: all pass. If an existing test asserted the old
`scrubbed: `-stripping wording, update its expectation to the new line --
`nothing` became `nothing matched`, which is the shared wording.

- [ ] **Step 6: Commit**

```bash
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git add crates/trace-commons-contributor-gtk/src/redaction_tally.rs \
        crates/trace-commons-contributor-gtk/src/ui/queue.rs \
        crates/trace-commons-contributor-gtk/src/main.rs
git commit -m "Move the removed-by-pattern figure into a tested module"
```

---

### Task 3: `queue_folders` -- grouping and navigation

`render` groups inline into a `Vec<(&str, &str, Vec<(usize, &QueueEntry)>)>`
(`ui/queue.rs:379-390`). The drill-in needs that grouping plus a location
that survives a folder emptying, and neither belongs in a 120-line render
function.

**Files:**
- Create: `crates/trace-commons-contributor-gtk/src/queue_folders.rs`
- Modify: crate root (add `mod queue_folders;`)
- Test: `queue_folders.rs`, inline

**Interfaces:**
- Consumes: `QueueEntry` (Task 1).
- Produces:
  - `pub struct Folder { pub project_id: String, pub project_label: String, pub project_path: String, pub bytes: u64, pub members: Vec<(usize, QueueEntry)> }`
  - `pub fn group(pending: &[&QueueEntry]) -> Vec<Folder>`
  - `pub enum Location { Root, Project(String) }`
  - `pub fn resolve(location: &Location, folders: &[Folder]) -> Location`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str, project: &str, label: &str, bytes: u64) -> QueueEntry {
        QueueEntry {
            entry_id: id.to_string(),
            project_id: project.to_string(),
            project_label: label.to_string(),
            project_path: format!("~/code/{label}"),
            size_bytes: bytes,
            state: "pending".to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn an_empty_queue_has_no_folders() {
        assert!(group(&[]).is_empty());
    }

    #[test]
    fn folders_keep_first_seen_order() {
        let a = entry("1", "p2", "two", 1);
        let b = entry("2", "p1", "one", 1);
        let c = entry("3", "p2", "two", 1);
        let folders = group(&[&a, &b, &c]);
        assert_eq!(
            folders.iter().map(|f| f.project_id.as_str()).collect::<Vec<_>>(),
            ["p2", "p1"]
        );
    }

    /// The index each entry had in the flat pending list must survive
    /// grouping: `Look inside` opens the preview sheet BY THAT INDEX, and
    /// the sheet re-derives its own copy of the pending list with the same
    /// filter. A folder that renumbered its members would open the wrong
    /// session's transcript.
    #[test]
    fn members_keep_their_flat_pending_index() {
        let a = entry("1", "p2", "two", 1);
        let b = entry("2", "p1", "one", 1);
        let c = entry("3", "p2", "two", 1);
        let folders = group(&[&a, &b, &c]);
        assert_eq!(
            folders[0].members.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            [0, 2]
        );
        assert_eq!(
            folders[1].members.iter().map(|(i, _)| *i).collect::<Vec<_>>(),
            [1]
        );
    }

    #[test]
    fn a_folder_sums_its_members_bytes() {
        let a = entry("1", "p1", "one", 30);
        let b = entry("2", "p1", "one", 12);
        assert_eq!(group(&[&a, &b])[0].bytes, 42);
    }

    #[test]
    fn a_folder_takes_the_path_of_its_first_member() {
        let a = entry("1", "p1", "one", 1);
        assert_eq!(group(&[&a])[0].project_path, "~/code/one");
    }

    #[test]
    fn two_projects_sharing_a_label_stay_separate() {
        let a = entry("1", "p1", "api", 1);
        let b = entry("2", "p2", "api", 1);
        assert_eq!(group(&[&a, &b]).len(), 2, "a label is not an identity");
    }

    #[test]
    fn root_stays_root() {
        assert!(matches!(resolve(&Location::Root, &[]), Location::Root));
    }

    #[test]
    fn a_folder_that_still_exists_is_kept() {
        let a = entry("1", "p1", "one", 1);
        let folders = group(&[&a]);
        assert!(matches!(
            resolve(&Location::Project("p1".into()), &folders),
            Location::Project(ref id) if id == "p1"
        ));
    }

    /// Submit all inside a folder empties it. Standing there would show a
    /// blank pane with a back button and no account of where it went.
    #[test]
    fn a_folder_that_emptied_falls_back_to_root() {
        let a = entry("1", "p2", "two", 1);
        let folders = group(&[&a]);
        assert!(matches!(
            resolve(&Location::Project("p1".into()), &folders),
            Location::Root
        ));
        assert!(matches!(resolve(&Location::Project("p1".into()), &[]), Location::Root));
    }
}
```

If `QueueEntry` has no `Default`, add `#[derive(Default)]` to it in
`model.rs` -- every field is already `#[serde(default)]`, so a `Default` impl
is consistent with how it deserializes, and it keeps these tests from being
twenty lines of irrelevant field initializers each.

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml queue_folders
```

Expected: `cannot find function group in this scope`.

- [ ] **Step 3: Write the implementation**

```rust
//! Grouping the waiting queue into folders, and remembering which one is
//! open.
//!
//! Both halves used to be absent: `render` grouped inline and there was only
//! one level to be on. The drill-in adds a second level that can be pulled
//! out from under the person standing on it -- approving a folder's last
//! session removes the folder, and so does an upload finishing in the
//! background.

use crate::model::QueueEntry;

/// One project's slice of the waiting queue.
pub struct Folder {
    /// `project_id`, and the id `submit_project` acts on. Never the label:
    /// a label is a display name, is not unique across two projects, and
    /// grouping on it would put one `Submit all` over another project's
    /// sessions.
    pub project_id: String,
    pub project_label: String,
    /// Taken from the first member. Every member of one project reports the
    /// same path, and a later entry with a stale one does not rename the
    /// folder out from under its buttons.
    pub project_path: String,
    pub bytes: u64,
    /// Each member paired with the index it had in the FLAT pending list.
    ///
    /// That index is load-bearing: `Look inside` opens the preview sheet by
    /// it, and the sheet re-derives its own copy of the pending list with an
    /// identical filter. Renumbering members inside a folder would open the
    /// wrong transcript.
    pub members: Vec<(usize, QueueEntry)>,
}

/// Which level of the queue is showing.
pub enum Location {
    Root,
    /// One folder, by `project_id`.
    Project(String),
}

pub fn group(pending: &[&QueueEntry]) -> Vec<Folder> {
    let mut folders: Vec<Folder> = Vec::new();
    for (index, entry) in pending.iter().enumerate() {
        match folders.iter_mut().find(|f| f.project_id == entry.project_id) {
            Some(folder) => {
                folder.bytes += entry.size_bytes;
                folder.members.push((index, (*entry).clone()));
            }
            None => folders.push(Folder {
                project_id: entry.project_id.clone(),
                project_label: entry.project_label.clone(),
                project_path: entry.project_path.clone(),
                bytes: entry.size_bytes,
                members: vec![(index, (*entry).clone())],
            }),
        }
    }
    folders
}

/// The location that is actually valid, given what the queue now holds.
///
/// A pure function of the location and the folders rather than a mutation,
/// so `render` can call it every time and never hold a stale location.
pub fn resolve(location: &Location, folders: &[Folder]) -> Location {
    match location {
        Location::Root => Location::Root,
        Location::Project(id) => {
            if folders.iter().any(|f| &f.project_id == id) {
                Location::Project(id.clone())
            } else {
                Location::Root
            }
        }
    }
}
```

- [ ] **Step 4: Run the tests to verify they pass**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml queue_folders
```

Expected: 9 tests pass.

- [ ] **Step 5: Commit**

```bash
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git add crates/trace-commons-contributor-gtk/src/queue_folders.rs \
        crates/trace-commons-contributor-gtk/src/model.rs \
        crates/trace-commons-contributor-gtk/src/main.rs
git commit -m "Group the queue into folders, with a location that survives one emptying"
```

---

### Task 4: Draw the folder list and the folder detail

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/ui/queue.rs:355-470` (`render`), `:905-1010` (`project_header` becomes `folder_row`)
- Modify: `crates/trace-commons-contributor-gtk/src/copy.rs` (new strings)
- Modify: `crates/trace-commons-contributor-gtk/src/app.rs` (hold the location)
- Test: `copy.rs` inline, for the new strings

**Interfaces:**
- Consumes: `queue_folders::{group, resolve, Folder, Location}` (Task 3).
- Produces: `copy::ALL_FOLDERS`, `copy::folder_summary(sessions, bytes)`;
  `App.queue_location: RefCell<Location>`.

- [ ] **Step 1: Add the copy, with tests**

In `copy.rs`:

```rust
/// The back control at the head of a folder's sessions.
pub const ALL_FOLDERS: &str = "All folders";

/// A folder row's right-hand figures: how much is waiting, and how big.
pub fn folder_summary(sessions: usize, bytes: u64) -> String {
    let unit = if sessions == 1 { "session" } else { "sessions" };
    format!("{sessions} {unit}  \u{00b7}  {}", crate::format::bytes(bytes))
}
```

with tests beside the other `copy.rs` tests:

```rust
    #[test]
    fn a_folder_summary_inflects_its_session_count() {
        assert!(folder_summary(1, 1024).starts_with("1 session "));
        assert!(folder_summary(2, 1024).starts_with("2 sessions "));
    }
```

Use whatever the crate's existing byte formatter is called rather than
`crate::format::bytes` if that is not its name -- `manifest_for` already
formats byte figures, so follow it.

- [ ] **Step 2: Hold the location on `App`**

In `app.rs`, beside the other `RefCell` state:

```rust
    /// Which level of the queue is showing. Resolved against the live
    /// folders on every render (`queue_folders::resolve`), so a folder that
    /// empties while it is open returns to the list.
    pub queue_location: RefCell<crate::queue_folders::Location>,
```

initialised to `Location::Root`.

- [ ] **Step 3: Rewrite `render`'s grouping section**

Replace the inline grouping loop and the `for (project_id, project_label,
members)` loop with:

```rust
    let folders = crate::queue_folders::group(&pending);
    let here = crate::queue_folders::resolve(&app.queue_location.borrow(), &folders);
    *app.queue_location.borrow_mut() = match &here {
        crate::queue_folders::Location::Root => crate::queue_folders::Location::Root,
        crate::queue_folders::Location::Project(id) => {
            crate::queue_folders::Location::Project(id.clone())
        }
    };

    match &here {
        crate::queue_folders::Location::Root => {
            for folder in &folders {
                view.list.append(&folder_row(app, folder));
            }
        }
        crate::queue_folders::Location::Project(id) => {
            if let Some(folder) = folders.iter().find(|f| &f.project_id == id) {
                view.list.append(&folder_heading(app, folder));
                for (index, entry) in &folder.members {
                    let widget = row(app, entry, *index);
                    app.card_widgets
                        .borrow_mut()
                        .insert(entry.entry_id.clone(), widget.clone());
                    view.list.append(&widget);
                }
            }
        }
    }
```

Keep `app.set_queue_count(pending.len())` and the heading above this
untouched: the count is the whole queue, not the open folder.

- [ ] **Step 4: Turn `project_header` into `folder_row`**

Rename it and change its shape: it takes a `&Folder`, its label becomes the
row's largest text with the path beneath it (`tc-card-title` over
`tc-meta`), the figures from `copy::folder_summary` sit at the trailing
edge, and the whole row is wrapped in a `gtk::Button` with the
`flat` class whose click sets
`*app.queue_location.borrow_mut() = Location::Project(id)` and re-renders.

**Show `Submit all` at every count, including one.** Delete the `if waiting >
1` guard. Add a comment where the guard was:

```rust
    // Shown at every count, including one. The old rule hid it at one
    // because the row's own `Submit` was on the same screen and did the same
    // thing. Under drill-in that row is a level down, so hiding this would
    // mean opening a folder to do the thing the folder is offering. The rule
    // expired with the layout it was written for.
```

Keep `Submit all as...` and `Ignore project` exactly as they are, including
`Ignore project` never carrying `suggested-action`.

- [ ] **Step 5: Add `folder_heading`**

A small function drawing the back control (`copy::ALL_FOLDERS`, a flat
button that sets `Location::Root` and re-renders), the folder label, and its
path.

- [ ] **Step 6: Show where each session ran**

In `row`, beside the project label, draw `entry.session_path` when it is
`Some`, in `tc-meta`, ellipsized at the start so the tail of the path
survives truncation.

- [ ] **Step 7: Build and test**

```bash
cargo build --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --bin trace-commons-shell
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

Expected: builds, all pass.

- [ ] **Step 8: Commit**

```bash
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git add crates/trace-commons-contributor-gtk/src/
git commit -m "Show folders first in the queue, with sessions one level in"
```

---

### Task 5: The card opens the preview

**Files:**
- Modify: `crates/trace-commons-contributor-gtk/src/ui/queue.rs:653-728` (`row`)

- [ ] **Step 1: Add a gesture to the card**

In `row`, after the card is built, attach a click gesture that calls the
same handler `Look inside` uses:

```rust
    // A second route to `Look inside`, never a replacement for it. The
    // button keeps its emphasis -- one-click submit added AVAILABILITY, and
    // primary styling is a RECOMMENDATION. What this adds is that the
    // obvious gesture on a card does the obvious thing.
    let click = gtk::GestureClick::new();
    let app_for_click = Rc::clone(app);
    click.connect_released(move |gesture, n_press, _, _| {
        if n_press != 1 {
            return;
        }
        // Claimed so the gesture does not also reach the card's own
        // buttons' parents.
        gesture.set_state(gtk::EventSequenceState::Claimed);
        crate::ui::preview::open(&app_for_click, index);
    });
    card.add_controller(click);
```

The footer buttons are `gtk::Button`s, which handle their own clicks and do
not propagate, so `Not this one`, `Submit`, and `Look inside` keep working.
Confirm that by hand in Step 2.

- [ ] **Step 2: Build and check by hand**

```bash
cargo build --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --bin trace-commons-shell
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

Then run the shell and confirm the card body opens the preview while each
footer button still does its own job. Record what you saw in the commit
message.

- [ ] **Step 3: Commit**

```bash
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git add crates/trace-commons-contributor-gtk/src/ui/queue.rs
git commit -m "Open the preview from anywhere on the card"
```

---

### Task 6: Name the chips this shell already draws

The spec's 3.1 is a correction: **all three shells already mark the
redactor's tokens**. This one does it in `transcript_paging::marker_spans`,
which `ui/preview.rs` walks to apply a gold text tag per marker. That scan is
deliberately shared with the chunker so a marker is never cut in half, and
`every_fixed_token_the_pipeline_emits_is_matched` is the guard that keeps it
covering all five fixed tokens. A shell must not add a second marker pass,
restyle the existing chips, or bypass the chunk-boundary contract.

What is missing is the *naming*: every chip today draws as the same
anonymous token. This task adds one pure function that turns a matched token
into words, and calls it from the one place the tag is already applied.

**Files:**
- Create: `crates/trace-commons-contributor-gtk/src/marker_names.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/lib.rs` (`pub mod marker_names;`)
- Modify: `crates/trace-commons-contributor-gtk/src/ui/preview.rs:1287-1298` (the existing `marker_spans` loop)
- Test: `marker_names.rs`, inline

**Interfaces:**
- Consumes: `transcript_paging::marker_spans` -- the existing scan,
  unchanged. No new scan, no new pattern, no new constant.
- Produces:
  - `pub struct MarkerName { pub text: String, pub ordinal: Option<u32> }`
  - `pub fn name_of(token: &str) -> MarkerName`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_numbered_form_carries_a_label_and_an_ordinal() {
        let n = name_of("<PRIVATE_LOCAL_PATH_3>");
        assert_eq!(n.text, "local path");
        assert_eq!(n.ordinal, Some(3));
    }

    /// `apply_placeholder_regex` mints the numbered form for exactly two
    /// labels: `local_path` and `private_email`.
    #[test]
    fn the_other_numbered_label_is_named_too() {
        assert_eq!(name_of("<PRIVATE_PRIVATE_EMAIL_1>").text, "private email");
    }

    /// The ordinal is the last underscore-delimited run of digits, so a
    /// label that itself ends in a number must not steal it.
    #[test]
    fn a_label_containing_digits_is_parsed_correctly() {
        let n = name_of("<PRIVATE_SHA256_KEY_7>");
        assert_eq!(n.text, "sha256 key");
        assert_eq!(n.ordinal, Some(7));
    }

    /// The five fixed tokens, from the same sources as
    /// `every_fixed_token_the_pipeline_emits_is_matched`. None carries an
    /// ordinal -- there is no second number to report, and inventing one
    /// would claim a distinctness the token does not have.
    #[test]
    fn every_fixed_token_is_named_and_carries_no_ordinal() {
        for (token, expected) in [
            ("[REDACTED]", "something removed"),
            ("[REDACTED:aws_secret_key]", "aws secret key"),
            ("[REDACTED:person_name]", "person name"),
            ("[REDACTED_PATH]", "URL path"),
            ("<REDACTED_PRIVATE_KEY>", "private key"),
        ] {
            let n = name_of(token);
            assert_eq!(n.text, expected, "{token}");
            assert_eq!(n.ordinal, None, "{token} carries no ordinal");
        }
    }

    /// Labels are an open, namespaced vocabulary. A token this build has no
    /// words for must still say that something left, never nothing.
    #[test]
    fn an_unrecognized_token_still_says_something_left() {
        let n = name_of("[REDACTED:some_future_detector]");
        assert_eq!(n.text, "some future detector");
        assert_eq!(n.ordinal, None);
    }

    /// Drives the naming from the shared scan rather than from a second
    /// list that could drift away from it.
    #[test]
    fn every_token_the_scan_finds_is_named() {
        let body = "<PRIVATE_LOCAL_PATH_1> [REDACTED] [REDACTED:aws_secret_key] \
                    [REDACTED_PATH] <REDACTED_PRIVATE_KEY>";
        let spans = crate::transcript_paging::marker_spans(body);
        assert_eq!(spans.len(), 5);
        for span in spans {
            assert!(!name_of(&body[span]).text.is_empty());
        }
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml marker_names
```

Expected: `cannot find function name_of in this scope`.

If `every_token_the_scan_finds_is_named` reports fewer than 5 spans, this
checkout predates the widened pattern -- `<REDACTED_PRIVATE_KEY>` was
unmatched until it was added. Rebase rather than working around it here; the
pattern is the chunker's too.

- [ ] **Step 3: Write the implementation**

`marker_names.rs` takes the matched token text -- nothing else -- and
returns words. No regex: this crate has none and the shapes are all
prefix/suffix work.

- `<PRIVATE_<LABEL>_<n>>`: the label lowercased with `_` as a space, and `n`
  as the ordinal. The redactor mints one token per distinct value and reuses
  it, so two chips with the same ordinal are the same original string, which
  is the fact worth surfacing.
- `<REDACTED_PRIVATE_KEY>`: `"private key"`, no ordinal.
- `[REDACTED:<label>]`: the label lowercased with `_` as a space, no ordinal.
- `[REDACTED_PATH]`: `"URL path"` -- it replaces a URL's path component, not
  a local one.
- `[REDACTED]` and anything else the scan matched: `"something removed"`.
  Never empty, and never a guess at a category.

The module doc records the two things it must not be read as saying: a
region with no chip is not a region with nothing sensitive in it -- the
detector scans every leaf while the rewriter reaches only typed fields, and
`copy::residual_risk_line` is the sentence that says so -- and a name is not
a distinct count. Only `local_path` and `private_email` mint placeholders,
so only those can report "the same value twice".

- [ ] **Step 4: Put the name on the chip**

In `ui/preview.rs`, the same forward pass over `marker_spans` that already
applies the gold `tag`, with the same tag and the same character-offset
carry. Only the chip's *text* changes: before applying the tag, replace the
span's text in the buffer with `name_of(&text[span]).text`, suffixed with
` #<ordinal>` when there is one, and tag the replacement's range instead.

Replace the text through the buffer's own delete/insert so the running
`chars` offset is corrected by the length difference -- the pass is
one-directional and a substitution that changed the length without
adjusting the counter would drag every later tag off its marker.

Two consequences to carry rather than discover:

1. The transcript's on-screen text is no longer byte-identical to the body.
   The copy-all path is unaffected: it copies the body string, not the
   buffer. Check `copy_all`'s handler before relying on that.
2. Chunk residency and the row estimate are computed from the body's bytes,
   not from what the buffer holds, so this changes neither.

Keep the residual-risk sentence visible on the same tab as the marks.

- [ ] **Step 5: Run the tests**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
```

Expected: all pass.

- [ ] **Step 6: Commit**

```bash
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git add crates/trace-commons-contributor-gtk/src/marker_names.rs \
        crates/trace-commons-contributor-gtk/src/lib.rs \
        crates/trace-commons-contributor-gtk/src/ui/preview.rs
git commit -m "Say what each redaction chip stood for"
```

---

### Task 6b: The removed-summary panel

Marking placeholders answers *where*. It does not answer "so I can right away
see what doesn't go", because collecting the marks means scrolling the whole
transcript. This is the panel that answers it -- and the surface where the
`residual_secret_at` defect gets stated correctly rather than backwards.

**Files:**
- Create: `crates/trace-commons-contributor-gtk/src/redaction_summary.rs`
- Modify: crate root (`mod redaction_summary;`), `src/copy.rs` (the descriptions), `src/ui/preview.rs` (the scrubbing page)
- Test: `redaction_summary.rs`, inline

**Interfaces:**
- Consumes: `redaction_tally::{family, is_removal, RESIDUAL_PREFIX}` (Task 2).
- Produces:
  - `pub struct Row { pub family: String, pub display: String, pub description: &'static str, pub occurrences: u32, pub distinct: u32, pub detail: Vec<String> }`
  - `pub fn rows(occurrences: &BTreeMap<String, u32>, distinct: &BTreeMap<String, u32>) -> (Vec<Row>, Vec<Row>)` -- `(removed, still_present)`

The contract is the one in the spec's §3.1b, and it is identical in all three
shells: group by family, keep an unrecognised family with a neutral
description, never drop one, and put `residual_secret_at` in the second list
rather than the first.

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn map(pairs: &[(&str, u32)]) -> BTreeMap<String, u32> {
        pairs.iter().map(|(k, v)| (k.to_string(), *v)).collect()
    }

    #[test]
    fn an_empty_map_produces_no_rows() {
        let (removed, still) = rows(&map(&[]), &map(&[]));
        assert!(removed.is_empty());
        assert!(still.is_empty());
    }

    #[test]
    fn one_family_becomes_one_row() {
        let (removed, _) = rows(&map(&[("local_path", 185)]), &map(&[("local_path", 12)]));
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].family, "local_path");
        assert_eq!(removed[0].display, "local path");
        assert_eq!(removed[0].occurrences, 185);
        assert_eq!(removed[0].distinct, 12);
        assert!(!removed[0].description.is_empty());
    }

    /// Nine secret patterns are one `secret` row, not nine rows.
    #[test]
    fn sub_labels_collapse_into_their_family() {
        let (removed, _) = rows(
            &map(&[("secret:contextual_entropy", 3), ("secret:pem_private_key", 1), ("secret", 2)]),
            &map(&[("secret:contextual_entropy", 2), ("secret:pem_private_key", 1), ("secret", 2)]),
        );
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].occurrences, 6);
        assert_eq!(removed[0].distinct, 5);
        assert_eq!(removed[0].detail, ["contextual entropy", "pem private key"]);
    }

    /// A secret DETECTED AND NOT REMOVED. Putting it in `removed` would
    /// state the exact opposite of what happened.
    #[test]
    fn a_residual_survivor_is_reported_as_still_present() {
        let (removed, still) = rows(
            &map(&[("local_path", 3), ("residual_secret_at:events.correction", 1)]),
            &map(&[]),
        );
        assert_eq!(removed.iter().map(|r| r.family.as_str()).collect::<Vec<_>>(), ["local_path"]);
        assert_eq!(still.iter().map(|r| r.family.as_str()).collect::<Vec<_>>(), ["residual_secret_at"]);
        assert_eq!(still[0].detail, ["events.correction"]);
    }

    /// Hiding a category this build has no words for would understate what
    /// happened, which is the one direction this panel must not fail in.
    #[test]
    fn an_unknown_family_is_kept_with_a_neutral_description() {
        let (removed, _) = rows(&map(&[("future_category", 4)]), &map(&[]));
        assert_eq!(removed.len(), 1);
        assert_eq!(removed[0].description, crate::copy::REDACTION_CATEGORY_UNKNOWN);
    }

    #[test]
    fn rows_are_ordered_by_occurrences_then_family() {
        let (removed, _) = rows(
            &map(&[("secret", 3), ("local_path", 185), ("email", 3)]),
            &map(&[]),
        );
        assert_eq!(
            removed.iter().map(|r| r.family.as_str()).collect::<Vec<_>>(),
            ["local_path", "email", "secret"]
        );
    }

    /// The panel names kinds, never values.
    #[test]
    fn a_row_carries_no_matched_text() {
        let (removed, _) = rows(&map(&[("secret", 1)]), &map(&[]));
        assert!(removed[0].detail.is_empty());
    }
}
```

- [ ] **Step 2: Run to verify it fails**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml redaction_summary
```

- [ ] **Step 3: Add the descriptions to `copy.rs`**

```rust
/// What each redaction family IS, in words -- the panel's actual value to a
/// reader who has never seen these labels.
///
/// Deliberately not exhaustive. The vocabulary is generated and open, which
/// is why `describe` falls back rather than panicking.
pub const REDACTION_CATEGORY_LOCAL_PATH: &str = "File paths from this machine.";
pub const REDACTION_CATEGORY_SECRET: &str =
    "API keys, tokens, private keys, and high-entropy strings found next to credential words.";
pub const REDACTION_CATEGORY_PRIVACY_FILTER: &str =
    "Names, emails, and other personal details found in prose.";
pub const REDACTION_CATEGORY_SENSITIVE_FIELD: &str =
    "Fields whose name marks them sensitive, like password or authorization.";
pub const REDACTION_CATEGORY_TOOL_SENSITIVE_FIELD: &str =
    "Tool-call arguments whose name marks them sensitive.";
pub const REDACTION_CATEGORY_RESIDUAL: &str =
    "Found, and still in what would be sent. Either a credential inside a correction \
     you wrote, which is kept on purpose, or a field scrubbing does not reach.";

/// The neutral description for a family this build has no words for. It must
/// still appear: dropping an unrecognised category would understate what
/// happened.
pub const REDACTION_CATEGORY_UNKNOWN: &str =
    "Removed by a pattern this version has no description for.";
```

- [ ] **Step 4: Write `redaction_summary.rs`**

Group by `redaction_tally::family`, summing occurrences and distinct counts
and collecting humanised sub-labels; sort by occurrences descending with ties
on family; split on `redaction_tally::is_removal`. Sub-labels are safe to
render -- they are schema-shaped identifiers by construction, which is the
same property `log_residual_secret_locations` relies on.

- [ ] **Step 5: Draw the panel**

On the preview sheet's scrubbing page, above the transcript marks: a
"Removed" section of rows (display name and counts in `tc-card-title`,
description in `tc-meta`, detail in `tc-meta` dimmed), and -- only when
non-empty -- a "Found, and still in what would be sent" section in the
attention tone with the warning glyph, listing the schema paths.

Keep the scrubbing caveat below both. A panel that enumerates categories makes
the app look more thorough than it is, which is when that sentence earns its
place.

- [ ] **Step 6: Run the tests and commit**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git add crates/trace-commons-contributor-gtk/src/
git commit -m "Summarise what scrubbing removed, and what it left in"
```

---

### Task 7: Tell "never there" apart from "removed", and give the chip a job

**Files:**
- Create: `crates/trace-commons-contributor-gtk/src/original_search.rs`
- Modify: crate root, `src/backend.rs` (the new call), `src/ui/preview.rs` (`run_search`), `src/ui/queue.rs` (the nothing-matched chip), `src/copy.rs`
- Test: `original_search.rs`, inline

**Interfaces:**
- Consumes: the `search_original` IPC method (plan 1 Task 8);
  `Backend::call` (existing, `backend.rs:192`).
- Produces:
  - `Backend::search_original(&self, entry_id: &str, needle: &str) -> Option<u32>`
  - `pub enum Outcome { Absent, AllRemoved(u32), SomeRemain { remaining: u32, total: u32 }, Unknown }`
  - `pub fn classify(remaining: u32, original: Option<u32>) -> Outcome`
  - `pub fn sentence(outcome: &Outcome) -> String`, `pub fn is_alarming(outcome: &Outcome) -> bool`

- [ ] **Step 1: Write the failing tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nowhere_in_either_text_is_absent() {
        assert!(matches!(classify(0, Some(0)), Outcome::Absent));
    }

    #[test]
    fn present_originally_and_gone_now_is_all_removed() {
        assert!(matches!(classify(0, Some(3)), Outcome::AllRemoved(3)));
    }

    #[test]
    fn still_present_is_some_remain() {
        assert!(matches!(
            classify(2, Some(5)),
            Outcome::SomeRemain { remaining: 2, total: 5 }
        ));
    }

    /// Reporting "not in this session" because a call failed would be the
    /// single most dangerous wrong answer this tab can give.
    #[test]
    fn a_failed_original_search_is_unknown_not_absent() {
        assert!(matches!(classify(0, None), Outcome::Unknown));
        assert!(matches!(
            classify(2, None),
            Outcome::SomeRemain { remaining: 2, total: 2 }
        ));
    }

    #[test]
    fn an_original_count_below_the_remaining_count_falls_back_to_what_is_certain() {
        assert!(matches!(
            classify(2, Some(1)),
            Outcome::SomeRemain { remaining: 2, total: 2 }
        ));
    }

    #[test]
    fn the_sentences_say_which_case_it_is() {
        assert_eq!(sentence(&Outcome::Absent), "0 matches \u{2014} not in this session");
        assert_eq!(sentence(&Outcome::AllRemoved(3)), "3 matches \u{2014} all 3 were removed");
        assert_eq!(
            sentence(&Outcome::SomeRemain { remaining: 2, total: 5 }),
            "5 matches \u{2014} 2 would still be sent"
        );
    }

    #[test]
    fn only_a_remaining_match_is_alarming() {
        assert!(is_alarming(&Outcome::SomeRemain { remaining: 1, total: 1 }));
        assert!(!is_alarming(&Outcome::AllRemoved(3)));
        assert!(!is_alarming(&Outcome::Absent));
        assert!(!is_alarming(&Outcome::Unknown));
    }
}
```

- [ ] **Step 2: Run to verify it fails, then implement**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml original_search
```

Write `original_search.rs` with the module doc explaining that
`tc_preview_search`'s equivalent here scans the REDACTED body, so a removed
value returns zero matches -- indistinguishable from a value that was never
present -- and that `Unknown` exists so a failed call never renders as a
clean result. `classify` mirrors the macOS plan's:

```rust
pub fn classify(remaining: u32, original: Option<u32>) -> Outcome {
    let Some(original) = original else {
        // Fail toward what is certain. The redacted body is in hand, so
        // matches in it are known; the absence of a check is not a clean
        // result and must never render as one.
        return if remaining > 0 {
            Outcome::SomeRemain { remaining, total: remaining }
        } else {
            Outcome::Unknown
        };
    };
    if remaining > 0 {
        return Outcome::SomeRemain { remaining, total: original.max(remaining) };
    }
    if original > 0 { Outcome::AllRemoved(original) } else { Outcome::Absent }
}
```

Put the four sentences in `copy.rs` as format functions and have `sentence`
call them, so the wording sits with the rest of the wording.

- [ ] **Step 3: Add the backend call**

In `backend.rs`, beside the `preview` call at `:226`:

```rust
    /// How many times `needle` appears in an entry's PRE-redaction session
    /// text. `None` on any failure.
    ///
    /// A COUNT, never content -- that is the whole bound of the daemon call
    /// behind this, and the reason it is allowed to read unredacted bytes at
    /// all.
    pub fn search_original(&self, entry_id: &str, needle: &str) -> Option<u32> {
        let value = self
            .call(
                "search_original",
                serde_json::json!({ "entry_id": entry_id, "needle": needle }),
            )
            .ok()?;
        value["matches"].as_u64().and_then(|n| u32::try_from(n).ok())
    }
```

- [ ] **Step 4: Wire it into `run_search`**

In `ui/preview.rs`'s `run_search`, after the redacted-body hits are counted,
call `search_original` for the same needle and set `search_summary` from
`original_search::sentence`, with the tone from `is_alarming`. Keep the
existing `remember` flag behavior exactly as it is -- see "What this shell
already gets right".

- [ ] **Step 5: Make the nothing-matched chip a control**

In `ui/queue.rs`, wrap the `copy::NOTHING_MATCHED` chip in a flat
`gtk::Button` whose click calls
`preview::open_with_search(app, index, None, Some("search".into()))` -- the
tab name is the one registered at `ui/preview.rs:142`. Extend
`copy::SCRUBBING_CAVEAT`'s zero-redaction sentence with the clause pointing
at search, matching the macOS wording exactly, and assert the two agree in a
`copy.rs` test:

```rust
    #[test]
    fn the_nothing_matched_line_offers_a_next_step() {
        assert!(
            scrubbing_caveat_line(0).to_lowercase().contains("search"),
            "the line must point at the thing to do about it"
        );
    }
```

- [ ] **Step 6: Run the tests and commit**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git add crates/trace-commons-contributor-gtk/src/
git commit -m "Say whether a searched value was removed, and give the chip a job"
```

---

### Task 8: Shield state, and history by folder

Folded into one task: both are small, both are the same pattern as Tasks 2
and 3, and neither is separately rejectable.

**Files:**
- Create: `crates/trace-commons-contributor-gtk/src/shield.rs`
- Modify: `crates/trace-commons-contributor-gtk/src/ui/mod.rs` (the sidebar count), `src/ui/history.rs:189-260` (`render`)
- Test: `shield.rs` inline; `ui/history.rs` inline for the grouping

**Interfaces:**
- Consumes: `queue_folders::{group, resolve}` (Task 3), `HistoryRecord.project_id` (Task 1).
- Produces:
  - `pub enum Shield { Clear, Waiting, Attention }`, `pub fn state(waiting: usize, nothing_matched: usize, trimmed: usize) -> Shield`
  - `pub fn history_folders(records: &[HistoryRecord]) -> Vec<(String, String, Vec<HistoryRecord>)>` in `ui/history.rs`

- [ ] **Step 1: Write the failing shield tests**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_queue_is_clear() {
        assert!(matches!(state(0, 0, 0), Shield::Clear));
    }

    #[test]
    fn an_ordinary_queue_is_waiting() {
        assert!(matches!(state(12, 0, 0), Shield::Waiting));
    }

    #[test]
    fn a_session_where_nothing_matched_raises_attention() {
        assert!(matches!(state(12, 1, 0), Shield::Attention));
    }

    #[test]
    fn a_trimmed_session_raises_attention() {
        assert!(matches!(state(12, 0, 1), Shield::Attention));
    }

    #[test]
    fn an_empty_queue_is_clear_even_with_stale_flags() {
        assert!(matches!(state(0, 3, 2), Shield::Clear));
    }
}
```

- [ ] **Step 2: Write the failing history-grouping tests**

In `ui/history.rs`'s test module:

```rust
    fn record(id: &str, project: &str, label: &str) -> HistoryRecord {
        HistoryRecord {
            submission_id: id.to_string(),
            project_id: project.to_string(),
            project_label: label.to_string(),
            ..Default::default()
        }
    }

    #[test]
    fn history_groups_by_project_id() {
        let groups = history_folders(&[
            record("1", "proj_a", "api"),
            record("2", "proj_b", "web"),
            record("3", "proj_a", "api"),
        ]);
        assert_eq!(groups.len(), 2);
        assert_eq!(groups[0].2.len(), 2);
    }

    #[test]
    fn two_projects_sharing_a_label_stay_separate() {
        let groups = history_folders(&[
            record("1", "proj_a", "api"),
            record("2", "proj_b", "api"),
        ]);
        assert_eq!(groups.len(), 2, "a label is not an identity");
    }

    /// Records submitted before project keys were normalized carry no id.
    /// Grouping them all under "" would put unrelated repositories in one
    /// row.
    #[test]
    fn records_with_no_project_id_group_by_label_instead() {
        let groups = history_folders(&[
            record("1", "", "api"),
            record("2", "", "web"),
            record("3", "", "api"),
        ]);
        assert_eq!(groups.len(), 2);
    }

    /// Same label, one resolvable and one not. Claiming they are the same
    /// folder is a guess; two rows is the honest answer.
    #[test]
    fn an_identified_and_an_unidentified_record_do_not_merge() {
        let groups = history_folders(&[record("1", "proj_a", "api"), record("2", "", "api")]);
        assert_eq!(groups.len(), 2);
    }
```

- [ ] **Step 3: Implement both**

`shield.rs` mirrors the macOS `QueueShieldState`, including the doc comment
recording that the shield is **added to** the numeric count rather than
replacing it: the ask was to swap the count for an icon, and at 149 waiting
sessions the count is the signal a contributor is actually reading.

`history_folders` groups on `project_id`, falling back to a
`"label:"`-prefixed synthetic key when the id is empty -- a real id always
starts with `proj_`, so the two key spaces cannot collide.

- [ ] **Step 4: Wire both into their views**

Sidebar: derive the shield from the pending entries in `queue::render` (the
`nothing_matched` count is entries whose preview reported an empty
`redactions` map; `trimmed` is those with `subagents_dropped > 0`) and pass
it to whatever draws the queue count, keeping the number.

History: same two-level shape as the queue, with its own `Location` on
`App`, folder rows resolved to paths by matching `project_id` against the
loaded project rows, and any group whose key starts with `label:` rendering
its label alone.

- [ ] **Step 5: Run the tests and commit**

```bash
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
git add crates/trace-commons-contributor-gtk/src/
git commit -m "Add the queue shield, and group history by folder"
```

---

### Task 9: Full verification

- [ ] **Step 1: Everything CI runs on this crate**

```bash
cargo fmt --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml -- --check
cargo build --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --bin trace-commons-shell
cargo test --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml
cargo clippy --manifest-path crates/trace-commons-contributor-gtk/Cargo.toml --all-targets -- \
  -A clippy::type_complexity -A clippy::collapsible_if \
  -A clippy::manual_option_as_slice -A clippy::useless_vec \
  -A clippy::redundant_pattern_matching
```

Expected: all clean. Paste the test summary into the PR body.

- [ ] **Step 2: Check the lockfile**

```bash
git status --short crates/trace-commons-contributor-gtk/Cargo.lock
```

This crate's lockfile is separate from the root one. If it moved, commit it;
if it moved and you added no dependency, find out why before continuing.

- [ ] **Step 3: Run the shell and check what tests cannot see**

Confirm by hand, and report in the PR body:

1. The queue opens on a folder list; folder names are the largest text.
2. Clicking a folder opens its sessions; `All folders` returns.
3. `Submit all` on a one-session folder works without opening it.
4. Approving a folder's last session returns you to the folder list.
5. `Look inside` opens the **right** session's transcript from inside a
   folder -- this is what the flat-index test in Task 3 is guarding, and it
   is the thing most likely to be silently wrong.
6. Clicking a card body opens the preview; footer buttons still work.
7. Redactions are marked in the transcript, no transcript text renders as
   broken markup, and the scrubbing page lists what was removed with a
   description per category.
8. Searching a value you know was redacted says it was removed.
9. The nothing-matched chip opens the search tab.
10. History is grouped by folder.
11. On a session with a `residual_secret_at` count, the panel reports it
    under "still in what would be sent" and NOT as removed.

- [ ] **Step 4: Open the PR**

```bash
git push -u origin shell-ux-gtk
gh pr create --repo zmanian/trace-commons-server \
  --title "Folder-first queue and scrubber transparency, GTK" \
  --body "Implements docs/superpowers/plans/2026-09-03-contributor-shell-ux-gtk.md.

Depends on the daemon and FFI foundation PR.

Spec: docs/superpowers/specs/2026-09-03-contributor-shell-queue-ux-design.md"
```

---

## Self-Review

**Spec coverage.**

| Spec section | Task |
|---|---|
| §1.1 `project_path` consumed | Task 1 |
| §2.1 folder list, name prominence | Tasks 3, 4 |
| §2.2 folder detail, `session_path` | Task 4 |
| §2.3 `Submit all` at n = 1 | Task 4 Step 4 |
| §2.4 card click | Task 5 |
| §3.1 chips named (already marked) | Task 6 |
| §3.1b removed-summary panel | Task 6b |
| §3.1b `residual_secret_at` excluded from the card figure | Task 2 |
| §3.1 distinct counts | Task 2 |
| §3.2 original search | Task 7 |
| §3.3 recent-search prefixes | **No work** -- already correct here, see "What this shell already gets right" |
| §3.4 nothing-matched affordance | Task 7 Step 5 |
| §4 shield plus count | Task 8 |
| §5 history grouping | Task 8 |

**Placeholder scan:** no TBDs. Four steps say "match the existing name"
(Task 4 Step 1's byte formatter, Task 8 Step 4's sidebar count function,
Task 6 Step 4's markup call, Task 7 Step 4's summary label) -- each names
what to look for.

**Type consistency check.** `redaction_tally::{line, total}` defined in Task
2, called in Task 2 Step 4. `queue_folders::{Folder, Location, group,
resolve}` defined in Task 3, used in Tasks 4 and 8. `marker_names::{name_of,
MarkerName}` defined and used in Task 6; the scan it names is the existing
`transcript_paging::marker_spans`. `original_search::{Outcome,
classify, sentence, is_alarming}` defined in Task 7 Steps 1-2 and used in
Step 4; `Backend::search_original` defined in Step 3 and called in Step 4
with `(&str, &str) -> Option<u32>` in both. `shield::{Shield, state}` defined
and used in Task 8.

**Two differences from the macOS plan, both deliberate.** This shell needs
no recent-search fix, and it reaches `search_original` over the IPC socket
(`Backend::call`) rather than through the C ABI -- macOS is the only shell
that links the FFI.

**The riskiest thing in this plan** is the flat pending index. `Look inside`
opens the preview sheet by position in the flat pending list, and the sheet
re-derives that list itself; grouping into folders must not renumber
anything. Task 3 has a dedicated test, and Task 9 Step 3 item 5 checks it by
hand, because an off-by-one here shows a contributor the wrong transcript
before they approve it.
