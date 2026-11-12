# Insights model answers, daemon side (PR 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Collect per-model session facts in the counter pass (Claude and, through a new per-turn reader, Codex), classify each session's task type, compute the six Models answers from those facts and PR 0's pricing, and serve them through a new `insights_model_answers` IPC method.

**Architecture:** Pure scans and the answer engine live in `trace-commons-contributor-insights`: `session_facts` (types and the Claude facts scan), `codex_series` (the Codex per-turn reader and its facts), `task_classifier`, and `model_answers`. The daemon's streaming `scan_session` feeds the new scans the same lines it already reads, stores tokens and facts on each `CounterRow` (never a price), and re-reads each existing row once. `CounterPass::model_answers_value` joins rows to their route tallies at answer time and calls the engine with the production pricing catalog; `daemon::insights_model_answers` gates and dispatches it like `insights_sessions`.

**Tech Stack:** Rust; `trace-commons-protocol`, `trace-commons-contributor-insights`, `trace-commons-contributor` (all MIT OR Apache-2.0); `serde`, `serde_json`, `chrono`, `sha2` (existing).

**Spec:** `docs/superpowers/specs/2026-10-20-insights-model-answers-design.md` (sections 1, 2, 3's code-and-test list, 5, 6, 7, Testing; Delivery item 1). Review context: PR #1501, poldsam's review and the reply. Depends on PR 0: `docs/superpowers/plans/2026-10-20-insights-pricing-data.md`.

## Global Constraints

- Verification, run before every commit on the crates the task touches:
  - `cargo fmt --all -- --check`
  - `RUSTFLAGS="-D warnings" cargo clippy -p <crate> --all-targets -- -A clippy::type_complexity -A clippy::collapsible_if -A clippy::manual_option_as_slice -A clippy::useless_vec -A clippy::redundant_pattern_matching`
  - `RUSTFLAGS="-D warnings" cargo test -p <crate>`
  - `RUSTFLAGS="-D warnings" cargo check -p trace-commons-protocol --no-default-features` and the same for `trace-commons-contributor-insights` (the "permissive crates, standalone" CI job).
  - Known environmental failures to ignore, and only these: `an_unentitled_process_reports_one`, `daemon::ipc::tests::managed_sign_in_redeem_hosts_the_proxy_and_ends_it`, `daemon::managed::tests::opencode_starts_on_the_sign_in_without_a_model`. Capture the baseline failure list of all three crates before Task 1.
- Stored rows and logs are hash-only and label-only: counts, keyed digests under the counter key, fixed labels and declared model labels. No prompt or response text, path, file name, command text, commit hash or price is stored; old/new edit strings are held only as keyed digests and only during the scan. Each privacy claim is asserted over serialized bytes (`serde_json::to_vec` of the row or payload), as the counter-store tests do.
- Unknown is never zero. A fact that could not be read is `None` and stays out of every figure it would change; each answer counts what it left out, per reason.
- No new dependencies (the bootstrap PRNG is inline). All three crates stay MIT OR Apache-2.0; nothing may depend on an AGPL crate.
- No new user-facing copy in PR 1. The IPC carries fixed snake_case labels; the card's words are PR 2's, through `analytics_copy.rs` and `docs/ui-copy-guide.md`. No label or field name may contain "best", "better", "fastest", "benchmark" or "winner".
- Tests count operations (reads, polls, draws), never wall-clock time.
- Disclosure: the repository is public. Fixtures are synthetic or derived from a real transcript with every string replaced; no model label that is not printed on a vendor page appears in source, fixtures, commits or PR text. Use synthetic labels (`model-a`, `model-b`) in engine tests.
- Commits: short imperative subject, no `feat:`/`fix:` prefix, no emojis, body ending with `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.

### From PR 0 (consumed exactly as named there)

- `insights::session_pricing::{price_session, SessionPrice, ModelPrice, UnpricedReason, PricingTableRef, PRICING_BASIS_API_LIST, normalize_model_label, pricing_provider}` and `insights::pricing_catalog::production_pricing_catalog`.
- `price_session(catalog: &PricingCatalog, accounting: PricingAccounting, model_labels: &[String], turns: &[TurnRecord]) -> SessionPrice`; `SessionPrice.total_usd_micros: Option<u64>`, `.by_model: BTreeMap<String, ModelPrice>` keyed by normalized label, `.skipped_turns`, `.unlabelled_turns`, `.tables: Vec<PricingTableRef>`.
- Skip rule: a turn with `model_label_ix: None` whose usage is known and totals zero is skipped (the `<synthetic>` shape). Attribution here reuses it: a skipped turn counts toward no model and not toward the turn total.
- Codex turn mapping the reader must store: through `session_pricing::codex_turn_usage` and nothing else, which maps `uncached_input = input_tokens - cached_input_tokens - cache_write_input_tokens` with checked arithmetic (unknown, never wrapped, when cached and written exceed the input), `cache_read = cached_input_tokens`, `output = output_tokens`, `cache_write = CacheWrite::TotalOnly(cache_write_input_tokens)`. Storing `input - cached` would price every written token twice.
- `BillableTokenCategory::{CacheWriteFiveMinuteInput, CacheWriteOneHourInput}`.

## Review Focus

- A Codex transcript whose `token_count` events repeat (the owner's largest session has 16,534 events for 14,318 responses) must give one turn per increase of `total_token_usage.total_tokens`, never one per event, so tokens are not double-counted; a decrease makes that turn's usage unknown.
- A Claude session whose permission mode changes mid-session (`default` then `auto`) must have `denied` unknown for any model with an edit made under `auto`, and `edit_mode: unknown`, never a known zero.
- A row re-read when its file grows must recompute every fact from the new bytes; no count from the old row survives (a grown session can only gain edits, never keep a stale `finished`).
- Two models with identical figures and identical sample sizes must be ordered by label byte order and give the same answer on every run and every input order (the bootstrap seed is fixed and draws do not depend on map iteration order).
- A session in a week other than the one on screen, but inside the 13 weeks ending now, must count; one older than 13 weeks must not, even if its row is still stored.

Each line has its test in the owning task: line 1 Task 5, line 2 Task 4, line 3 Task 6, lines 4-5 Task 8.

---

## File Structure

- Modify `crates/trace-commons-protocol/src/insights_usage_series.rs`: `ToolKind::ApplyPatch`, Codex tool-name mapping, `ToolKind::is_edit`, the repo-independent `edit_text_key` digest.
- Create `crates/trace-commons-contributor-insights/src/insights/session_facts.rs`: `SessionFacts`, `ModelFacts`, `SessionShape`, `EditMode`, `InferredTask`, and `ClaudeFactsScan`.
- Create `crates/trace-commons-contributor-insights/src/insights/task_classifier.rs`: `TaskClassifier`, `LocalRulesClassifier`, `classify`.
- Create `crates/trace-commons-contributor-insights/src/insights/codex_series.rs`: `CodexTurnSeriesScan`, `CodexTurnSeriesEvidence`.
- Create `crates/trace-commons-contributor-insights/src/insights/model_answers.rs`: the engine and its wire types.
- Modify `crates/trace-commons-contributor-insights/src/insights.rs` (module list, lines 5-44).
- Modify `crates/trace-commons-contributor-insights/src/insights/analytics_constants.rs`: D11 and D5 flips, classifier and answer constants.
- Modify `crates/trace-commons-contributor-insights/src/insights/week_rollup.rs:128-160`: `SessionBody::Codex` gains `series`, `SessionInput` gains `facts`; every construction site in `goals.rs`, `week_glance.rs`, `week_patterns.rs`, `week_rollup.rs`, `session_drill.rs` and the daemon (18 `SessionInput {` sites, 14 `SessionBody::Codex` sites on `origin/main`).
- Modify `crates/trace-commons-contributor-insights/src/insights/session_drill.rs`: draw a Codex series when present.
- Modify `crates/trace-commons-contributor/src/daemon/insights_week.rs`: `RowBody::Codex`, `CounterRow`, `scan_session`, `read_body`, `pass_inner`'s unchanged test (about line 826), `turns()`/`tool_calls()`, `model_answers_value`.
- Create `crates/trace-commons-contributor/src/daemon/insights_model_answers.rs`; modify `crates/trace-commons-contributor/src/daemon/ipc.rs:438-440,542-544,3680-3682` and the daemon module list.
- Modify `docs/contributor-daemon-ipc-v1_1.md` (method table near line 611, a new section after "`insights_sessions` and `insights_session_drill`" near line 4749).
- Fixtures: `crates/trace-commons-contributor-insights/tests/fixtures/insights/` (or beside the existing turn-series fixtures; use whichever directory `turn_series.rs`'s `fixture()` reads).

---

### Task 1: Confirm the Codex markers and the reviewer rule

Research with a pinned outcome. Spec section 1 ("Codex markers are not confirmed") and section 2 ("Reviewer sessions").

**Files:**
- Modify: `crates/trace-commons-contributor-insights/src/insights/analytics_constants.rs`
- Create (only if a denial is found): a redacted fixture `codex_contributor_denial.jsonl`

**Interfaces:**
- Produces: `pub const CODEX_DENIAL_CONFIRMED: bool` and, when true, `pub const CODEX_CONTRIBUTOR_REVIEWER: &str` (the `approvals_reviewer` value naming the contributor) and the denial marker the Codex scan matches. `pub const CODEX_REVIEWER_MODEL: &str = "codex-auto-review";`.

- [ ] **Step 1: Search the owner's Codex transcripts** for sessions that could have put an edit to the contributor, and for any denial in them:

```bash
cd ~/.codex/sessions; c=$(mktemp)   # outside the repository
find . -name '*.jsonl' -print0 | xargs -0 grep -l '"type":"turn_context"' \
  | xargs grep -l -E '"approval_policy":"(on-request|untrusted|on-failure)"' \
  | xargs grep -L '"approvals_reviewer":"auto_review"' > "$c"
wc -l < "$c"
xargs grep -h -o -E '"type":"(exec_approval_request|apply_patch_approval_request)"|rejected by user[^"]{0,40}|"decision":"[a-z_]+"' < "$c" | sort | uniq -c
find . -name '*.jsonl' -print0 | xargs -0 grep -h '"type":"turn_context"' | jq -r '"\(.payload.approval_policy) \(.payload.approvals_reviewer // "absent")"' | sort | uniq -c
```

On 2026-10-20 the value set was `never`/`on-request`/`untrusted` x `auto_review`/`user`/absent, with `user` seen only under `never`. A denial counts as confirmed only when one approval request in a non-`never`, non-`auto_review` turn is followed by an event or output that records the contributor declining it, and its shape can be pinned (event type, field, value).

- [ ] **Step 2: Check the reviewer rule**: every file whose `turn_context` models include `codex-auto-review` has no other model.

```bash
find . -name '*.jsonl' -print0 | xargs -0 grep -l '"model":"codex-auto-review"' \
  | while read f; do grep -h '"type":"turn_context"' "$f" | jq -r .payload.model | sort -u | tr '\n' ' '; echo; done | sort | uniq -c
```

Expected: one line, `codex-auto-review` alone. If a file mixes, the rule stays "all turns declare it" (spec section 2) and the mixed count goes in the hand-off report.

- [ ] **Step 3: Write the failing test** in `analytics_constants.rs`:

```rust
#[test]
fn codex_denial_follows_the_confirmed_marker() {
    // false: no denial seen in a real transcript (2026-10-20 search); Codex `denied` is always None.
    const { assert!(!CODEX_DENIAL_CONFIRMED) };
    assert_eq!(CODEX_REVIEWER_MODEL, "codex-auto-review");
}
```

If Step 1 confirmed a denial, flip the assertion, pin `CODEX_CONTRIBUTOR_REVIEWER`, and build the fixture from that transcript with every string (ids, paths, text, cwd) replaced.

- [ ] **Step 4: Implement the constants**, each with a doc comment naming the search date and result (counts only, no labels).
- [ ] **Step 5: Run** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights analytics_constants` — Expected: PASS.
- [ ] **Step 6: Commit**: `git commit -m "Pin what the Codex transcripts show about contributor denials"` (with the trailer).

---

### Task 2: Codex tool kinds and the edit-text digest

**Files:**
- Modify: `crates/trace-commons-protocol/src/insights_usage_series.rs:123-167` (ToolKind), digest domains near `:362`

**Interfaces:**
- Produces:
  - `ToolKind::ApplyPatch` (serde `apply_patch`, domain byte 10). Existing variants and bytes unchanged.
  - `pub fn from_codex_tool_name(name: &str) -> ToolKind`: `apply_patch -> ApplyPatch`; `exec`, `exec_command`, `shell -> Bash`; everything else `Other`.
  - `pub fn is_edit(self) -> bool`: true for `Edit`, `Write`, `MultiEdit`, `NotebookEdit`, `ApplyPatch`.
  - `pub fn edit_text_key(key: &DigestKey, text: &str) -> KeyedDigest` under a new domain `trace-commons/insights/edit_text/v1\0`. Used only in memory during a scan (undone detection), never stored.

- [ ] **Step 1: Failing tests**

```rust
#[test] fn codex_tool_names_map_to_the_closed_list() {
    assert_eq!(ToolKind::from_codex_tool_name("apply_patch"), ToolKind::ApplyPatch);
    for shell in ["exec", "exec_command", "shell"] { assert_eq!(ToolKind::from_codex_tool_name(shell), ToolKind::Bash); }
    assert_eq!(ToolKind::from_codex_tool_name("Edit"), ToolKind::Other);
    assert_eq!(ToolKind::from_tool_name("apply_patch"), ToolKind::Other); // Claude names unchanged
}
#[test] fn the_edit_set_is_the_four_claude_edit_tools_and_apply_patch() {
    assert!(ToolKind::ApplyPatch.is_edit() && ToolKind::NotebookEdit.is_edit());
    assert!(!ToolKind::Bash.is_edit() && !ToolKind::Read.is_edit());
}
#[test] fn edit_text_keys_are_domain_separated_from_path_and_args_keys() { /* same input string, three different digests */ }
#[test] fn existing_tool_kind_domain_bytes_are_unchanged() { /* args_key of a fixed input under each old kind equals a pinned hex */ }
```

- [ ] **Step 2: Run** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-protocol insights_usage_series` — Expected: FAIL (no `ApplyPatch`).
- [ ] **Step 3: Implement** per the Interfaces block. `patterns.rs` `is_reread_edit`/`is_cycle_edit` stay Claude-only: Codex series do not reach the Patterns cards in this PR (Task 5).
- [ ] **Step 4: Run** protocol and insights tests — Expected: PASS.
- [ ] **Step 5: Commit**: `git commit -m "Name Codex apply_patch and shell calls in the tool list"` (with the trailer).

---

### Task 3: Session facts types and the task classifier

**Files:**
- Create: `crates/trace-commons-contributor-insights/src/insights/session_facts.rs` (types only in this task)
- Create: `crates/trace-commons-contributor-insights/src/insights/task_classifier.rs`
- Modify: `analytics_constants.rs`, `insights.rs`

**Interfaces:**
- Produces (all `Serialize`/`Deserialize`, `deny_unknown_fields`, serde snake_case):
  - `pub struct ModelFacts { pub label_ix: u16, pub edits: Option<u32>, pub denied: Option<u32>, pub undone: Option<u32>, pub interrupted: Option<u32> }`. PR 3 adds `committed`, `uncommitted`, `commit_unknown` with serde defaults; they are not added here.
  - `pub enum EditMode { Prompting, NotPrompting, Unknown }`.
  - `pub struct ExtensionClassCounts { pub test: u32, pub doc: u32, pub source: u32, pub config: u32 }` (distinct edited files per class).
  - `pub struct SessionShape { pub edited_files: ExtensionClassCounts, pub renamed_files: u32, pub lines_added: Option<u32>, pub lines_removed: Option<u32>, pub failed_before_first_edit_then_passed: u32 }`. Lines are `None` when any edit's line counts are unknown: every `Write` and `NotebookEdit` call makes both `None` (an overwrite's removed lines are not in the transcript).
  - `pub enum InferredTask { BugFix, Refactor, Tests, Docs, Other, NoEdits }` with `pub fn task_category(self) -> TaskCategory` (`BugFix -> Debugging`, `NoEdits -> Other`, the rest by name). `TaskCategory` is not changed.
  - `pub struct SessionFacts { pub models: Vec<ModelFacts>, pub shape: SessionShape, pub task: InferredTask, pub classifier_version: u32, pub active_secs: Option<u32>, pub finished: Option<bool>, pub user_turns: Option<u32>, pub edit_mode: EditMode, pub capped: bool, pub delegated_unread: bool, pub reviewer: bool }`. `capped`: the scan or series hit a cap. `delegated_unread`: set by the daemon from `group_member_count > 0`. `reviewer`: every turn declared `CODEX_REVIEWER_MODEL`.
  - `pub fn path_class(path: &str) -> PathClass` with `pub enum PathClass { Test, Doc, Source, Config }`, checked in that order, on the normalized path, computed during a scan and never stored.
  - `pub trait TaskClassifier { fn classify(&self, shape: &SessionShape) -> InferredTask; fn version(&self) -> u32; }`, `pub struct LocalRulesClassifier;`, `pub fn classify(shape: &SessionShape) -> InferredTask` (the local rules).
  - `pub fn reclassify(facts: &mut SessionFacts)`: re-runs the local rules when `facts.classifier_version != CLASSIFIER_VERSION`.
- Constants in `analytics_constants.rs` (spec section 5 values; the config list is a default taken):
  - `CLASSIFIER_VERSION: u32 = 1`, `SESSION_FACTS_VERSION: u32 = 1`.
  - `DOC_EXTENSIONS = ["md", "mdx", "rst", "adoc"]`, `DOC_DIRS = ["docs", "doc"]` (`.txt` is doc-class only under one of these).
  - `TEST_DIRS = ["tests", "test", "__tests__", "spec"]`, `TEST_NAME_MARKS = ["_test.", ".test.", ".spec."]`, `TEST_NAME_PREFIX = "test_"`.
  - `CONFIG_EXTENSIONS = ["txt", "json", "toml", "yaml", "yml", "ini", "cfg", "conf", "lock", "xml", "plist"]` (default taken; `CMakeLists.txt` and `requirements.txt` fall here by `txt`).
  - `REFACTOR_MIN_RENAMES = 3`, `REFACTOR_MIN_FILES = 5`, `REFACTOR_NET_LINES = Ratio::new(1, 10)`.

Rules, first match wins (spec section 5): Docs (every edited file doc-class, at least one); Tests (`test * 2 > total`); BugFix (`failed_before_first_edit_then_passed > 0` and at least one non-test edited file); Refactor (`renamed_files >= 3`, or `total >= 5` and lines known and `|added - removed| * 10 <= added + removed`); Other when any file edited; NoEdits otherwise.

- [ ] **Step 1: Failing tests** in `task_classifier.rs`, one per rule plus the edges:

```rust
#[test] fn all_doc_files_is_docs() { assert_eq!(classify(&shape(doc: 2)), InferredTask::Docs); }
#[test] fn a_txt_outside_docs_is_config_not_doc() {
    assert_eq!(path_class("CMakeLists.txt"), PathClass::Config);
    assert_eq!(path_class("docs/notes.txt"), PathClass::Doc);
    assert_eq!(path_class("requirements.txt"), PathClass::Config);
}
#[test] fn more_than_half_test_files_is_tests() { /* 2 of 3 -> Tests; 1 of 2 -> not Tests */ }
#[test] fn a_command_failing_before_the_first_edit_then_passing_is_bug_fix() { assert_eq!(.., InferredTask::BugFix); }
#[test] fn tdd_a_test_written_then_made_to_pass_is_not_bug_fix() {
    // failed_before_first_edit_then_passed == 0: the failing run came after the first edit.
    assert_ne!(classify(&tdd_shape()), InferredTask::BugFix);
}
#[test] fn a_bug_fix_touching_only_test_files_is_tests() { /* rule 2 matches first */ }
#[test] fn renames_or_balanced_lines_over_five_files_is_refactor() { /* 3 renames; 5 files with 100 added 95 removed; 5 files 100/80 is Other */ }
#[test] fn unknown_lines_never_satisfy_the_line_clause() { /* 5 files, lines None, 0 renames -> Other */ }
#[test] fn no_edited_files_is_no_edits() {}
#[test] fn inferred_tasks_map_to_shared_labels_without_mixed() { assert_eq!(InferredTask::BugFix.task_category(), TaskCategory::Debugging); assert_eq!(InferredTask::NoEdits.task_category(), TaskCategory::Other); }
#[test] fn a_stale_classifier_version_reclassifies_from_the_stored_shape() { /* facts with version 0 and task Other, shape says Docs -> Docs, version CLASSIFIER_VERSION */ }
#[test] fn facts_round_trip_and_reject_unknown_fields() {}
```

- [ ] **Step 2: Run** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights task_classifier` — Expected: FAIL (modules missing).
- [ ] **Step 3: Implement** the types, `path_class`, `classify`, `reclassify` and constants. Add the constant values to the `analytics_constants` pin tests.
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit**: `git commit -m "Infer a session's task type from counts with local rules"` (with the trailer).

---

### Task 4: The Claude facts scan

**Files:**
- Modify: `crates/trace-commons-contributor-insights/src/insights/session_facts.rs`
- Fixtures: synthetic Claude JSONL files, one per case below

**Interfaces:**
- Consumes: Task 2 (`ToolKind::is_edit`, `edit_text_key`, `path_key`), Task 3 (types, `path_class`, `classify`).
- Produces:
  - `pub struct ClaudeFactsScan<'k>`; `pub fn new(key: &'k DigestKey) -> Self`; `pub fn observe(&mut self, row: &Value) -> Result<()>` (same record contract as `ClaudeTurnSeriesScan::observe`); `pub fn finish(self, model_labels: &[String]) -> SessionFacts` (labels from the series evidence; a model whose label is not in them is dropped from `models` and the session's facts are `capped`).
  - `pub const CLAUDE_DENIAL_TEXT: &str = "The user doesn't want to proceed with this tool use";`
  - `pub const CLAUDE_INTERRUPT_MARKERS: [&str; 2] = ["[Request interrupted by user]", "[Request interrupted by user for tool use]"];`
  - `pub const CLAUDE_PROMPTING_MODE: &str = "default";` `pub const CLAUDE_NOT_PROMPTING_MODES: [&str; 4] = ["auto", "acceptEdits", "bypassPermissions", "dontAsk"];`

Rules the scan implements (spec section 1; the implementer verifies each field spelling against a real transcript before writing its fixture):
- Model of a call or turn: the `message.model` of its assistant record (through `safe_model`), keyed by message id as `turn_series.rs` does.
- `edits`: tool_use blocks whose `ToolKind::is_edit()`. Known (possibly 0) for every model with at least one turn.
- Mode of an edit call: the latest `permissionMode` read on any line before the call; none seen yet is an unknown mode. `denied` for a model is known only when every one of its edit calls was made under `default`. A declined call: a `tool_result` block whose text (string content or any text part) contains `CLAUDE_DENIAL_TEXT`, counted once per `tool_use_id`, and only when that id is an edit call. The repeated text in `toolUseResult` is never matched.
- `edit_mode`: `Prompting` when every edit call was under `default`; `NotPrompting` when every one was under a `CLAUDE_NOT_PROMPTING_MODES` value; otherwise (mixed, unknown, or no edits) `Unknown`.
- Contributor prompt: a `user` record not `isMeta: true` with a text part that is not an interrupt marker, and no `tool_result` block. `user_turns` counts them.
- `interrupted`: a user text starting with a marker (trailing newline allowed), credited to the model of the latest assistant turn; the "for tool use" marker on the first user record after a denied call's result is not counted.
- `undone`: per `path_key`, each `Edit` and each `MultiEdit` sub-edit records `(edit_text_key(old), edit_text_key(new), model, barrier_count)`; a later `Edit`/sub-edit on the same path with `old == earlier.new` and `new == earlier.old`, with `barrier_count` advanced since (a contributor prompt, a denial or an interrupt), adds one to the earlier edit's model. Held entries are capped at `MAX_SERIES_TOOL_CALLS`; past it the facts are `capped`.
- Shape: distinct edited `path_key`s per `path_class`; `renamed_files` += the number of path operands of a `Bash` command whose first word is `mv` or `git mv` (operands minus one; the command text is dropped after counting); lines from `Edit`/`MultiEdit` old/new line counts; per `Bash` `args_key`, failure (`is_error: true`) before the first edit call and a later success.
- Spans and `active_secs`: per spec section 1 "Active time": from each contributor prompt's `timestamp` to the last model event (assistant record or tool result) before the next prompt, minus each `AskUserQuestion` call's and each denied call's use-to-result time. Any missing or unparsable timestamp in a span makes `active_secs` `None`.
- `finished`: at least one edit; the last assistant record has text and no unpaired `tool_use`; no denial, undo or interrupt after the last contributor prompt.

- [ ] **Step 1: Failing tests**, each on its own fixture:

```rust
#[test] fn a_denial_counts_once_though_its_text_appears_twice() { assert_eq!(m.denied, Some(1)); }
#[test] fn a_declined_bash_or_ask_user_question_is_not_a_denied_edit() { assert_eq!(m.denied, Some(0)); }
#[test] fn the_interrupt_after_a_denial_is_not_counted_again() { assert_eq!(m.interrupted, Some(0)); }
#[test] fn both_interrupt_spellings_count_with_or_without_a_newline() { assert_eq!(m.interrupted, Some(4)); }
#[test] fn denied_is_unknown_under_each_non_prompting_mode() { for mode in ["auto","acceptEdits","bypassPermissions","dontAsk","someNewMode"] { assert_eq!(m.denied, None); } }
#[test] fn a_mode_change_mid_session_makes_denied_and_edit_mode_unknown() { assert_eq!(m.denied, None); assert_eq!(f.edit_mode, EditMode::Unknown); }
#[test] fn a_restore_after_a_contributor_prompt_is_undone() { assert_eq!(m.undone, Some(1)); }
#[test] fn the_same_restore_with_nothing_between_is_not_undone() { assert_eq!(m.undone, Some(0)); }
#[test] fn a_multiedit_sub_edit_restore_counts_and_a_write_overwrite_does_not() {}
#[test] fn undone_goes_to_the_model_that_made_the_edit() { /* model-a edits, model-b restores after a prompt -> a.undone 1, b.undone 0 */ }
#[test] fn a_long_build_inside_a_span_counts_and_the_idle_tail_does_not() { assert_eq!(f.active_secs, Some(EXPECTED)); }
#[test] fn ask_user_question_and_denied_call_waits_are_subtracted() {}
#[test] fn a_missing_timestamp_in_a_span_makes_active_time_unknown() { assert_eq!(f.active_secs, None); }
#[test] fn finished_needs_an_edit_a_text_ending_and_no_rejection_after_the_last_prompt() { /* four fixtures */ }
#[test] fn meta_lines_and_tool_results_are_not_contributor_prompts() { assert_eq!(f.user_turns, Some(2)); }
#[test] fn renames_count_operands_and_keep_no_command_text() {}
#[test] fn the_tdd_fixture_has_no_failure_before_the_first_edit() { assert_eq!(f.shape.failed_before_first_edit_then_passed, 0); }
#[test] fn serialized_facts_hold_no_path_text_or_command() {
    let bytes = serde_json::to_vec(&facts).unwrap();
    for secret in ["PRIVATE_PATH", "PRIVATE_TEXT", "PRIVATE_CMD", "src/", ".rs"] { assert!(!contains(&bytes, secret)); }
}
#[test] fn a_model_with_turns_but_no_edits_has_zero_edits_not_unknown() { assert_eq!(m.edits, Some(0)); }
```

- [ ] **Step 2: Run** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights session_facts` — Expected: FAIL.
- [ ] **Step 3: Implement `ClaudeFactsScan`** per the rules. Memory bound: maps keyed by message id and tool-use id, each at most the series caps; a test counts the held entries after a fixture past the caps, as `the_scan_holds_no_more_than_the_caps` does in `turn_series.rs`.
- [ ] **Step 4: Run** — Expected: PASS (19 tests plus the bound test).
- [ ] **Step 5: Commit**: `git commit -m "Read denials, undos, interrupts and active time from Claude transcripts"` (with the trailer).

---

### Task 5: The Codex per-turn reader

**Files:**
- Create: `crates/trace-commons-contributor-insights/src/insights/codex_series.rs`
- Modify: `week_rollup.rs:128-141` (`SessionBody::Codex`), every `SessionBody::Codex { observed }` site, `session_drill.rs` (Codex branch and the test at about line 505), `analytics_constants.rs:117-120,290`
- Fixtures: synthetic Codex JSONL

**Interfaces:**
- Consumes: Task 1 constants, Task 2 (`from_codex_tool_name`, `is_edit`, `edit_text_key`), Task 3 types, PR 0's Codex turn mapping.
- Produces:
  - `pub struct CodexTurnSeriesEvidence { pub model_labels: Vec<String>, pub series: UsageSeries }` (`Serialize`, `Deserialize`, `deny_unknown_fields`).
  - `pub struct CodexTurnSeriesScan<'k>`; `new(key: &'k DigestKey) -> Self`; `observe(&mut self, row: &Value) -> Result<()>`; `finish(self) -> Result<(CodexTurnSeriesEvidence, SessionFacts)>`.
  - `SessionBody::Codex { observed: Option<CodexObserved>, series: Option<CodexTurnSeriesEvidence> }`. Weekly token figures, goals, glance and Patterns keep reading `observed` only (decision: PR 1 changes no weekly figure); `session_drill` draws `series` when present.
  - `CODEX_PER_TURN_SERIES = true`; its doc says D11 is settled (owner, 2026-10-20) and names which views read the series (the drill and the Models answers).

Reader rules (spec section 2; verify spellings on a real file before writing fixtures):
- Turn label: `turn_context.payload.model`, through `safe_model`, carried until the next `turn_context`.
- One `TurnRecord` per `event_msg` `token_count` with non-null `info` whose `info.total_token_usage.total_tokens` is greater than the previous such event's; a repeat (equal) is skipped; a decrease makes that turn's usage unknown and resets the baseline. Counters from `info.last_token_usage` by PR 0's mapping; `at` from the line's top-level `timestamp`. (Measured 2026-10-20 on six large sessions: the increase-only sum equals the final total exactly.)
- Tool calls: `response_item` `custom_tool_call` or `function_call`, kind by `from_codex_tool_name(name)`, `args_key` over the input, `path_key`/`path_ext` from each `apply_patch` file header; outputs (`*_call_output`) pair by `call_id`.
- Facts: `edits` = `apply_patch` file hunks for the turn's model; a model with shell calls and no `apply_patch` call has `edits: None`. `denied`: always `None` while `CODEX_DENIAL_CONFIRMED` is false. `interrupted`: `event_msg` `turn_aborted` with reason `interrupted`, or a call output starting "aborted by user". `undone`: each `apply_patch` update hunk's removed and added text, by the Task 4 rule. `user_turns`: `task_started` events. Spans: `task_started` to the last model event before the next one, minus `request_user_input` / `request_user_input_async` call-to-output time. `finished`: an edit, last turn ended in `task_complete`, no undo or interrupt after the last `task_started`. `edit_mode`: `NotPrompting` when every turn with an edit has `approval_policy: never` or `approvals_reviewer: auto_review`; `Unknown` otherwise (no Codex turn is `Prompting` while `CODEX_DENIAL_CONFIRMED` is false). `reviewer`: every labelled turn is `CODEX_REVIEWER_MODEL`. Shape: as Task 4, from `apply_patch` (add/delete/update/move headers: a move counts as a rename) and shell commands.

- [ ] **Step 1: Failing tests**

```rust
#[test] fn repeated_token_counts_make_one_turn_per_increase() { assert_eq!(e.series.turns.len(), 3); assert_eq!(sum_total(&e), final_total); }
#[test] fn a_decreasing_total_makes_that_turn_unknown_never_negative() {}
#[test] fn each_turn_carries_the_model_of_the_turn_context_before_it() {}
#[test] fn counters_follow_the_pricing_mapping() { let u = codex_turn_usage(input, cached, w, output).unwrap(); assert_eq!(t.uncached_input, Some(u.uncached_input)); assert_eq!(t.uncached_input, Some(input - cached - w)); assert_eq!(t.cache_write, Some(CacheWrite::TotalOnly(w))); }
#[test] fn shell_only_sessions_have_edits_unknown_not_zero() { assert_eq!(m.edits, None); }
#[test] fn apply_patch_hunks_count_edits_and_feed_undone() {}
#[test] fn codex_denied_is_unknown_in_every_mode() { for policy in ["never","on-request","untrusted"] { assert_eq!(m.denied, None); } }
#[test] fn turn_aborted_and_exec_aborted_by_user_are_interrupts() {}
#[test] fn request_user_input_waits_are_subtracted() {}
#[test] fn an_all_reviewer_session_is_marked_reviewer() { assert!(f.reviewer); }
#[test] fn past_the_turn_cap_the_series_is_truncated_and_facts_capped() {}
#[test] fn the_codex_series_and_facts_hold_no_readable_text() { /* serialized bytes, as in turn_series.rs */ }
#[test] fn a_codex_drill_with_a_series_draws_it() { assert!(drill.series.is_some()); assert_eq!(drill.series_unavailable, None); }
#[test] fn a_codex_drill_without_a_series_still_reads_not_recorded() { /* the existing test, with series: None */ }
#[test] fn weekly_codex_figures_ignore_the_series() { /* same rollup, patterns and glance with series Some and None */ }
```

Update `held_and_default_decisions` to `const { assert!(CODEX_PER_TURN_SERIES) };`.

- [ ] **Step 2: Run** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights` — Expected: FAIL to compile on the new field and module.
- [ ] **Step 3: Implement** the reader; add `series: None` at every existing `SessionBody::Codex` site, the daemon's `session_input()` (`insights_week.rs:291`) and daemon tests included, so both crates build at this commit (the real daemon wiring is Task 6).
- [ ] **Step 4: Run** the insights and contributor suites — Expected: PASS against the baseline.
- [ ] **Step 5: Commit**: `git commit -m "Read Codex sessions turn by turn"` (with the trailer).

---

### Task 6: Facts and Codex series on counter rows

**Files:**
- Modify: `crates/trace-commons-contributor/src/daemon/insights_week.rs`: module doc (lines 22-26, the D11 sentence), `RowBody::Codex` (168-170), `CounterRow` (176-241), `turns()`/`tool_calls()` (259-271), `session_input()` (273-302), `pass_inner` unchanged test (about 826-838) and row construction (about 885-901), `read_body` (1562-1611), `ScannedBody`/`scan_session` (1623-1755)
- Modify: `week_rollup.rs` `SessionInput` (146-160) and its construction sites

**Interfaces:**
- Consumes: Tasks 3-5.
- Produces:
  - `SessionInput.facts: Option<SessionFacts>` (`None` for saved snapshots and for counter rows without facts).
  - `RowBody::Codex { observed: Option<CodexObserved>, #[serde(default)] series: Option<CodexTurnSeriesEvidence> }`.
  - `CounterRow` gains `#[serde(default)] facts: Option<SessionFacts>` and `#[serde(default)] facts_version: u32`. Every row the pass makes is written with `facts_version = SESSION_FACTS_VERSION`, including rows whose facts scan failed (`facts: None`) and `RowBody::Unknown` rows, so a session that cannot be read is not read on every poll. A row is unchanged only when, beyond today's conditions, `facts_version == SESSION_FACTS_VERSION`; so every existing row is read once more, within the per-poll budget (1,921 Codex rows on the owner's machine take about 120 polls at 16 reads each). A row whose `facts.classifier_version` is stale is re-classified at answer time with `reclassify`, not re-read.
  - `scan_session` runs `ClaudeFactsScan` beside `ClaudeTurnSeriesScan`, and `CodexTurnSeriesScan` beside `CodexUsageScan`, on the same records; a scan that fails leaves the facts `None` and the rest unchanged. `read_body` sets `facts.delegated_unread = candidate.group_member_count > 0` and `facts.capped |= series.truncated` (before the delegated mark sets `truncated`).
  - A Codex row's series has its own allowance (owner decision, 2026-10-20): it does not count toward the shared caps (`COUNTER_STORE_MAX_TURNS`, `..._TOOL_CALLS`, Claude Code only), but toward `COUNTER_STORE_MAX_CODEX_SERIES_TURNS` and `..._TOOL_CALLS` (196608 each). Past that budget the oldest Codex rows lose only their series (`series_trimmed`), keeping the row, `observed`, dates and facts; a trimmed row is not evicted and not re-read while unchanged, the Sessions answers draw it as a pre-reader row, and the Models answers count it in `unknown_facts`.

- [ ] **Step 1: Failing tests** in `insights_week.rs` tests:

```rust
#[test] fn a_claude_row_stores_facts_and_a_codex_row_its_series() {}
#[test] fn a_row_without_facts_is_read_once_more_then_not_again() { /* reads counted: 1 then 0 on the next poll */ }
#[test] fn a_row_with_an_old_facts_version_is_read_once_more_within_the_budget() { /* 20 stale rows: 16 then 4 reads */ }
#[test] fn a_grown_session_recomputes_every_fact() { /* finished true -> grown with an interrupt -> finished false, edits grown */ }
#[test] fn a_stale_classifier_version_reclassifies_without_a_read() { /* reads counted 0 */ }
#[test] fn delegated_and_capped_are_told_apart() { assert!(f.delegated_unread && !f.capped); }
#[test] fn codex_series_turns_count_toward_the_store_caps() {}
#[test] fn a_failed_facts_scan_keeps_the_counters() { /* facts None, series Some, facts_version current; the next poll reads 0 files */ }
#[test] fn an_unknown_row_is_not_read_on_every_poll() { /* RowBody::Unknown, facts None, facts_version current: 0 reads on the next poll */ }
#[test] fn the_stored_row_holds_no_path_text_command_or_price() {
    // serialized store bytes: no PRIVATE_* marker, no "cost", "price", "usd", no repo path, no file name
}
#[test] fn rows_without_facts_keep_todays_week_answer() {
    // equivalence: week_value over rows with facts stripped equals week_value over the same rows on origin/main's shape
}
```

Keep the existing `nothing_identifying_crosses` (about line 3457, forbids "cost") and the fold test at about line 4153 unchanged; they must still pass.

- [ ] **Step 2: Run** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor daemon::insights_week` — Expected: FAIL.
- [ ] **Step 3: Implement**; update the module doc: rows carry per-model facts and a Codex per-turn series (D11 settled), still no price.
- [ ] **Step 4: Run** the daemon and insights suites — Expected: PASS against the baseline, `the_streamed_read_matches_the_whole_file_read` included.
- [ ] **Step 5: Commit**: `git commit -m "Store session facts and Codex turns on counter rows"` (with the trailer).

---

### Task 7: Model answers, per task type and cost

**Files:**
- Create: `crates/trace-commons-contributor-insights/src/insights/model_answers.rs`
- Modify: `analytics_constants.rs` (answer constants, D5 flip)

**Interfaces:**
- Consumes: Task 3 types, Task 5/6 `SessionBody` and `SessionInput.facts`, PR 0's `price_session`, `normalize_model_label`, `PRICING_BASIS_API_LIST`, `PricingTableRef`.
- Produces:
  - `pub enum ModelRoute { Unobserved, Unrecorded, Outside, Mixed, CheckFailed, RoutedVerified, RoutedUnverified }`: the engine's public mirror of the daemon's `pub(crate) RouteCategory`. Compared: `Unobserved`, `Outside`, `RoutedVerified`; Q4 compares `Unobserved` only (spec section 3: every session with a tally is left out of Q4).
  - `pub fn model_answers(sessions: &[SessionInput], routes: &BTreeMap<String, ModelRoute>, catalog: &PricingCatalog, now: DateTime<Utc>) -> ModelAnswers`. `routes` is keyed by `session_ref`; absent means `Unobserved`. The catalog is a parameter so tests use a synthetic one.
  - `pub enum ModelQuestion { BugFixRejected, RefactorRejected, TestsRejected, DocsCost, RejectedAcrossTasks, FinishTime }` (serde snake_case; the IPC's `question`).
  - `pub enum AnswerState { NotEnoughSessions, NotPriced, NoClearDifference, Answered }`; `pub enum RejectionBasis { Rejected, Undone }`.
  - `pub enum Figure { RatePermille { value: u32 }, UsdMicros { value: u64 }, FinishTime { median_active_secs: u32, median_user_turns: u32 } }` (serde tag `kind`).
  - `pub struct ModelFigure { pub label: String, pub sessions: u32, pub edits: Option<u32>, pub figure: Figure, pub interrupts_per_session_permille: Option<u32> }`.
  - `pub struct LeftOut { pub unknown_facts: u32, pub delegated: u32, pub capped: u32, pub routed: u32, pub reviewer: u32, pub no_majority: u32, pub unpriced: u32, pub too_few_edits: u32, pub mix_too_different: u32 }`.
  - `pub struct ModelAnswerRow { pub question: ModelQuestion, pub state: AnswerState, pub basis: Option<RejectionBasis>, pub edit_mode: Option<EditMode>, pub leader: Option<ModelFigure>, pub next: Option<ModelFigure>, pub sessions_by_model: Vec<(String, u32)>, pub left_out: LeftOut }`.
  - `pub struct ModelAnswers { pub window_start: DateTime<Utc>, pub window_end: DateTime<Utc>, pub rows: Vec<ModelAnswerRow>, pub pricing_tables: Vec<PricingTableRef> }` (rows always six, in `ModelQuestion` order).
  - Constants: `MODEL_ANSWER_MIN_SESSIONS = 5`, `MODEL_ANSWER_MIN_EDITS = 30`, `MODEL_ANSWER_MIN_MIX_COVERAGE = Ratio::new(4, 5)`, `MODEL_ANSWER_BOOTSTRAP_DRAWS = 1_000`, `MODEL_ANSWER_LEAD_SHARE = Ratio::new(95, 100)`, `MODEL_ANSWER_BOOTSTRAP_SEED: u64` (any fixed value, pinned by test), `MODEL_ANSWER_WINDOW_WEEKS = COUNTER_STORE_KEEP_WEEKS`.
  - `LEDGER_MONEY_IN_INSIGHTS = true`; its doc: D5 settled 2026-10-20, Insights shows money labelled (ledger `cost_usd` as a charge, everything else at API list prices from reviewed tables, no runtime fetch); its const assert at `analytics_constants.rs:292` flips.

Engine rules this task implements (spec section 6):
- Window: sessions whose latest dated turn (or `placed_at`) is in `[now - 13 weeks, now]`.
- A session enters no figure when: facts `None` (`unknown_facts`); `reviewer`; `task == NoEdits`; route not compared (`routed`); no model has a strict majority of its counted turns, after PR 0's skip rule (`no_majority`). Labels are normalized with `normalize_model_label`.
- Partial rows, per spec section 1's table: `delegated_unread` is out of rejection and cost, kept for time; `capped` is out of all three; each counted in `left_out`.
- Per (task, model) cell: sessions; rejection numerator and edits from the majority model's own `ModelFacts`; sessions with `edits: None` out of rejection figures; interrupts per session; median priced cost (session total from `price_session`, `None` totals counted as `unpriced`); finished sessions' median active time and turns.
- Qualifies: `>= 5` sessions, and for rejection `>= 30` edits (`too_few_edits` otherwise). A question needs two qualifying models.
- Basis: `Rejected` when `denied` is known in every session of the compared cells, else `Undone` for every model in that question.
- Q1-Q3: the cells of BugFix, Refactor, Tests. Q4: Docs cells, cost over priced sessions; state `NotPriced` when two models have `>= 5` Docs sessions but fewer than two have `>= 5` priced ones.
- States in order: `NotEnoughSessions`, `NotPriced` (Q4 only), `NoClearDifference` (Task 8), `Answered`. In this task, any two qualifying models are `Answered` by the order rule; Task 8 inserts the bootstrap test in front.

- [ ] **Step 1: Failing tests** (synthetic labels `model-a`, `model-b`, `model-c`; a synthetic catalog):

```rust
#[test] fn four_sessions_are_not_enough_and_five_are() {}
#[test] fn twenty_nine_edits_are_too_few_for_a_rejection_figure() { assert_eq!(row.left_out.too_few_edits, ..); }
#[test] fn one_session_with_unknown_denied_puts_the_whole_question_on_undone() { assert_eq!(row.basis, Some(RejectionBasis::Undone)); }
#[test] fn fifty_fifty_and_three_way_sessions_count_for_no_model() { assert_eq!(row.left_out.no_majority, 2); }
#[test] fn rates_use_only_the_majority_models_own_edits() {}
#[test] fn session_cost_sums_each_models_tokens_at_its_own_rate() {}
#[test] fn synthetic_and_dated_labels_normalize_into_one_model() { /* "Model-A-20261001" and "model-a" -> one cell; zero-usage unlabelled turns do not block a majority */ }
#[test] fn delegated_rows_leave_rejection_and_cost_but_keep_time() {}
#[test] fn capped_rows_leave_every_figure() {}
#[test] fn compared_and_left_out_route_categories() { /* Unobserved, Outside, RoutedVerified compared; four others counted in routed */ }
#[test] fn routed_sessions_never_enter_the_cost_question() {}
#[test] fn an_unpriced_model_reads_not_priced_never_not_enough_or_zero() { assert_eq!(row.state, AnswerState::NotPriced); }
#[test] fn unknown_facts_are_never_counted_as_zero() { /* a model whose only sessions have facts None has no cell, not a 0% cell */ }
#[test] fn reviewer_and_no_edit_sessions_are_left_out() {}
#[test] fn saved_snapshots_without_facts_are_left_out() {}
#[test] fn the_answers_always_carry_six_rows_in_order() {}
#[test] fn ledger_money_is_settled() { const { assert!(LEDGER_MONEY_IN_INSIGHTS) }; }
```

- [ ] **Step 2: Run** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights model_answers` — Expected: FAIL.
- [ ] **Step 3: Implement**. Rates as exact fractions until the wire figure (`permille = num * 1000 / den`, rounded half-up once).
- [ ] **Step 4: Run** — Expected: PASS.
- [ ] **Step 5: Commit**: `git commit -m "Compute per-task model answers from session facts"` (with the trailer).

---

### Task 8: Across task types, noise and order

**Files:**
- Modify: `crates/trace-commons-contributor-insights/src/insights/model_answers.rs`

**Interfaces:**
- Consumes: Task 7.
- Produces: Q5 and Q6 rows; `NoClearDifference`; the total order. `fn split_mix64(state: &mut u64) -> u64` (inline PRNG, private).

Rules (spec section 6):
- Mix weighting for Q5 and Q6: per model, its qualifying task types; each type weighted by the contributor's session count in it across all compared models; weighted mean over the model's qualifying types with weights renormalized; compared only when its types carry `>= 80%` of the total weight (`mix_too_different` otherwise).
- Q6: sessions of one `edit_mode`; `NotPrompting` when two models qualify there, else `Prompting`; `Unknown` never. Figure: median active seconds of finished sessions, with median contributor turns beside it.
- Bootstrap: for each draw, resample sessions with replacement within each (task, model) cell, recompute each model's figure, and count draws where the leader's figure is strictly below the next model's. Named only at `>= 95%` of `MODEL_ANSWER_BOOTSTRAP_DRAWS`; else `NoClearDifference` with both figures. One PRNG seeded with `MODEL_ANSWER_BOOTSTRAP_SEED` per question, cells visited in the total order, sessions within a cell in `session_ref` order, so the result is independent of input order.
- Total order: figure ascending; then more edits (Q4: more priced sessions; Q6: more finished sessions); then more sessions; then label byte order.

- [ ] **Step 1: Failing tests**

```rust
#[test] fn a_docs_heavy_model_does_not_win_across_types_on_mix_alone() {
    // model-a: 40 Docs at 2%, 10 BugFix at 20%; model-b: 10 Docs at 3%, 40 BugFix at 15%.
    // Pooled, model-a leads; weighted by the shared mix, model-b leads.
}
#[test] fn a_model_covering_under_eighty_percent_of_the_mix_is_not_compared() {}
#[test] fn finish_time_uses_not_prompting_when_two_models_qualify_there() {}
#[test] fn finish_time_falls_back_to_prompting_and_never_mixes_modes() {}
#[test] fn the_same_rows_give_the_same_answer_in_any_input_order() { /* shuffle inputs 5 ways: identical ModelAnswers */ }
#[test] fn a_gap_within_noise_reads_no_clear_difference_with_both_figures() {}
#[test] fn a_clear_gap_names_the_leader() {}
#[test] fn ties_break_by_edits_then_sessions_then_label_bytes() { /* identical figures, edits and sessions: "model-a" before "model-b" */ }
#[test] fn the_window_is_thirteen_weeks_ending_now_whatever_week_is_on_screen() { /* session 12 weeks back counts; 14 weeks back does not */ }
#[test] fn the_bootstrap_runs_exactly_the_configured_draws() { /* a counting seam: draws == 1_000 per answered question */ }
#[test] fn the_seed_is_pinned() { assert_eq!(MODEL_ANSWER_BOOTSTRAP_SEED, PINNED); }
```

- [ ] **Step 2: Run** — Expected: FAIL.
- [ ] **Step 3: Implement**.
- [ ] **Step 4: Run** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor-insights` — Expected: PASS against the baseline.
- [ ] **Step 5: Commit**: `git commit -m "Compare models across task types without letting the mix decide"` (with the trailer).

---

### Task 9: The `insights_model_answers` IPC method

**Files:**
- Create: `crates/trace-commons-contributor/src/daemon/insights_model_answers.rs`
- Modify: `insights_week.rs` (`CounterPass::model_answers_value`), `ipc.rs:438-440` (`METHODS`), `:542-544` (`DEV_DRY_RUN_LOCAL_METHODS`), `:3680-3682` (dispatch), the daemon module list
- Modify: `docs/contributor-daemon-ipc-v1_1.md` (table near line 611; a new `### insights_model_answers` section after the one at about line 4749)

**Interfaces:**
- Consumes: Task 7/8 `model_answers`, `ModelRoute`; PR 0's `production_pricing_catalog`, `PRICING_BASIS_API_LIST`; `insights_route_tally::route_category`.
- Produces:
  - `pub(crate) fn model_answers_value(&self, enabled: bool, never_project_keys: &NeverFolders, ledger_feed: bool, ledger_present: bool, now: DateTime<Utc>) -> serde_json::Value` on `CounterPass`: rows from `visible_rows`, each `session_input()` with facts re-classified by `reclassify`; routes from `store.routing` by `harness_session` through `route_category`, mapped to `ModelRoute`; every route `Unobserved` when the ledger feed is off or no ledger is declared.
  - `pub fn handle_model_answers(shared: &DaemonShared, req: &Request) -> Response`, gated exactly as `insights_sessions`' `gate`: off answers `{enabled: false, feed: "counter_pass"}` without opening the store or the key; an unreadable store, key or folder policy answers `readable: false` with its fixed reason.
  - Payload while enabled and readable: `{enabled, feed, readable, updated_at, window_start, window_end, pricing: {basis: "api_list", tables: [{table_id, version}]}, rows: [ModelAnswerRow...]}`. Money appears only as `figure.value` under `kind: "usd_micros"` on `docs_cost` rows. `table_id` is the content digest of a shipped price table, the one digest allowed on this wire.

- [ ] **Step 1: Failing tests** in `insights_model_answers.rs`:

```rust
#[test] fn off_by_default_without_opening_the_store_or_making_a_key() {}
#[test] fn an_unreadable_policy_store_or_key_answers_readable_false() {}
#[test] fn the_method_is_advertised_dispatched_and_local() { /* METHODS, DEV_DRY_RUN_LOCAL_METHODS, handle_request */ }
#[test] fn never_folder_sessions_enter_no_answer() {}
#[test] fn with_the_feed_off_every_session_is_unobserved() {}
#[test] fn nothing_identifying_crosses() {
    // serialized payload: no PRIVATE_* marker, no session_ref ("t1"), no digest other than the pinned table ids,
    // no path, file name, backend name or commit-hash-shaped string.
}
#[test] fn money_crosses_only_as_usd_micros_on_the_docs_cost_row() { /* walk the JSON: every key containing "usd" is figure.value under kind usd_micros, question docs_cost */ }
#[test] fn no_label_reads_as_a_ranking_claim() {
    // every string value and key in the payload, lowercased: none contains "best", "better", "fastest", "benchmark", "winner"
}
#[test] fn insights_week_still_carries_no_price() { /* the existing forbidden-"cost" assertions still hold with facts stored */ }
```

- [ ] **Step 2: Run** `RUSTFLAGS="-D warnings" cargo test -p trace-commons-contributor daemon::insights_model_answers` — Expected: FAIL.
- [ ] **Step 3: Implement** the handler and `model_answers_value`. Document the method in `docs/contributor-daemon-ipc-v1_1.md`: the table row (`additive; read-only, no network, dry-run local; gated on insights_counter_pass`), and a section listing every field, the money fields exactly, the left-out reasons, that `insights_week` keeps its no-price contract, and that labels are declared labels, not verified serving identities.
- [ ] **Step 4: Run** all three suites, fmt, clippy and the `--no-default-features` checks — Expected: green against the baseline.
- [ ] **Step 5: Commit**: `git commit -m "Serve the Models answers over a new local IPC method"` (with the trailer).

### Addendum (2026-10-20): automated sessions out of the shared limit

Owner decision, option (b): automated review sessions are kept out of the
Insights store's shared 2,048-session limit and given their own small
allowance. Landed on PR #1510:

- Marker: a Codex session is automated when its first `session_meta` names
  a `source` object holding a `subagent` key (Codex's automatic approvals
  reviewer, or a thread an agent spawned). Only the shape is read. Neither
  `approvals_reviewer: auto_review` (a session policy, set on sessions the
  person started too) nor the `codex-auto-review` model label (only the
  reviewer, not spawned threads) is used. No `session_meta`/`source`,
  `source: "unknown"`, a spawned thread with a later non-spawned
  `session_meta`, and every Claude Code session are unknown; unknown counts
  as the person's. `exec` and `mcp` count as the person's.
- Store: a row field `automation` (`unknown`/`person`/`automated`) with
  `automation_read`; old Codex rows are read once more within the per-poll
  budget. Automated rows are outside the shared session, turn and tool-call
  caps, under `COUNTER_STORE_MAX_AUTOMATED_SESSIONS` (1024), oldest first.
  Eviction marks record which allowance evicted them; only the shared caps'
  marks bound unread sessions; marks from before the allowance are dropped.
- Downstream: the Models answers leave them out under `left_out.automated`
  (every row); the Patterns cards and the Sessions feed leave them out with
  no switch (none existed); weekly token figures keep them, and
  `insights_week` adds `automated_stored`. UI copy is a later PR.

---

## Self-review notes

- Spec coverage: section 1 facts (Tasks 4-6; `committed`/`uncommitted`/`commit_unknown` are PR 3's and are not added here); section 2 Codex reader (Tasks 1, 2, 5); section 3 code-and-test list (`LEDGER_MONEY_IN_INSIGHTS` and its assert, Task 7; `insights_week.rs:71`, `:2008`, `:3488`, `:4153` unchanged and re-asserted, Tasks 6 and 9; the new method's money fields, Task 9; route tally and glance wording untouched); section 5 (Task 3); section 6 (Tasks 7-8); section 7 (Task 9); Testing list (spread as above; the Swift tests and card copy are PR 2).
- Interfaces from PR 0 are used as named there; nothing here pins a rate or a table id.
