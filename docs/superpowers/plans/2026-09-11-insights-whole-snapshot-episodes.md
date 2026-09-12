# User-confirmed episodes from whole saved snapshots

This is an implementation plan, not a shipped capability. It follows the native
evidence layer in PR #894 and advances the [Insights program](2026-09-11-trace-insights-program.md)
through explicit local grouping. It builds on the
[model and outcome evidence contract](2026-09-11-insights-model-outcome-evidence.md)
and [native lifecycle contract](2026-09-11-insights-native-evidence.md).

## Product scope

A user selects one or more whole saved snapshots and confirms that they belong
to an episode. The group can span trace files, source formats, and declared
models. The user can inspect its members, replace the membership, independently
assess the group, or remove it without deleting its snapshots or original files.

Call this scope **user-selected whole snapshots**. It does not establish a task's
true boundaries: a snapshot may contain unrelated work, and different episodes
may reuse it. There is no partial-session selection, automatic grouping, inferred
start/end time, causal model attribution, episode cost, time-saved estimate, or
independent-task metric in this slice. The program's richer task-episode model
remains a later design, rather than a claim made by these groups.

Keep the existing saved-snapshot summary unchanged. Episode assessments do not
alter snapshot assessments, known-outcome metrics, task-mix totals, or rankings.
Member Git objects and imported reports retain their existing authority; their
presence cannot establish an episode's success or identify which model did work.

## Current implementation seams

- `crates/trace-commons-contributor/src/insights.rs` owns store schema v3,
  path-hash aliases, content-addressed reports, validation, the stable lock file,
  and atomic private writes. `import` prunes reports whose final alias is replaced;
  `delete` removes a report and all its aliases. Both must invalidate episodes
  within that same locked index update.
- `insights/service.rs` is the synchronous local operation/response contract.
  `list_saved` short-circuits an absent store without creating it. Native callers
  schedule this service off their UI thread; it does not resolve enrollment state.
- `src/bin/contributor_cli/insights.rs` exposes the current flat Insights command
  family. Some snapshot operations still call the store directly; cleanup must
  therefore live in the store, not only in service dispatch.
- `crates/trace-commons-contributor-ffi/src/insights.rs` exposes the generic JSON
  `tc_insights_call`. Episodes need new typed operations and compatibility tests,
  not another native ABI function or a separate backend.

## Types and authority

Add `insights/episodes.rs` to the permissively licensed contributor crate, using
the already available UUID, time, serialization, and hashing dependencies.

| Type | Fields and invariant |
| --- | --- |
| `LocalEpisode` | `schema_version: 1`, stable UUID `id`, `revision: u64`, `membership_revision: u64`, `created_at`, `updated_at`, `provenance: UserSelectedWholeSnapshots`, sorted unique `members`, optional `manual_assessment` |
| `EpisodeMember` | Exact `snapshot_id` and `source_digest`; both must match an existing validated saved report |
| `EpisodeAssessment` | Existing category/outcome enums, `provenance: UserReported`, `recorded_at`, the assessed `membership_revision`, and a canonical `members_digest` |
| `EpisodeListEntry` | Episode metadata and members, plus distinct overlapping episode IDs; no expanded member reports or aggregate metrics |
| `EpisodeDetail` | Episode, current resolved member reports, per-member overlapping episode IDs, and local `resolved_at`; all obtained from one validated index read |

The UUID identifies the group across edits; it is not derived from membership.
Creating two groups with identical members creates two distinct user records and
exposes their overlap. No free-text title, inferred project identity, transcript
body, source path, or copied Git/test artifact is stored in the episode.

Define `members_digest` over a documented canonical encoding of the sorted
`(snapshot_id, source_digest)` pairs, with unambiguous field boundaries. The
assessment's membership revision and digest must match the current membership.
This is consistency/provenance binding within a local cache, not authentication
or independently verified task success. Unknown is an explicit user assessment;
an absent assessment remains unassessed.

## Revisions and edits

Initialize both revisions to 1. Every edit and episode deletion supplies the
last observed `expected_revision`. Check it under the store lock before deciding
whether the request is an effective change. A mismatch returns
`insights_episode_revision_conflict` and writes nothing; the caller refreshes and
lets the user review the new state. Never silently retry a destructive edit.

- Replacing members with a different canonical set increments both revisions,
  clears the episode assessment, and updates `updated_at` in one transaction.
  It does not modify any member snapshot.
- Annotating or clearing an assessment increments only `revision` and updates
  `updated_at`. An annotation records the unchanged `membership_revision` and
  member digest. It never copies or combines member assessments.
- A replacement with the same canonical set is a no-op. An identical assessment
  or clearing an absent assessment is also a no-op. These requests still require
  a matching expected revision; they preserve timestamps and revisions.
- Revision increments are checked for overflow and fail without writing. Validate
  revision values and timestamp ordering on reads; use nondecreasing local edit
  timestamps if the wall clock moves backward. These dates are edit times, not
  inferred task activity times.
- A missing episode returns `insights_episode_not_found`, including stale edits
  after another client deleted it. Deletion is not reported as a successful edit
  of a different or already absent group.

Changing a member's snapshot assessment, Git link, or imported-report link does
**not** edit episode membership, increment either episode revision, or change its
assessment. Explain resolves that member's current evidence on the next read.
The episode assessment remains a user statement about the selected whole
snapshots; it is not silently reinterpreted as a verdict on newly linked evidence.
Show episode edit time separately from member analysis/link/inspection times.

## Store v4 and atomic invalidation

Extend `Index` with an `episodes` map keyed by UUID and bump its version to 4.
Default the field to empty when reading v1/v2/v3; reject nonempty episode fields
claimed by a legacy version. Legacy reads do not rewrite the index. The next
successful mutation writes v4, preserving all existing report, alias, annotation,
and evidence validation. Older clients must refuse v4 rather than downgrade it;
clients sharing this store need a coordinated upgrade.

Validate episode keys/IDs, revisions, timestamps, scope, caps, canonical unique
members, source digest bindings, assessment bindings, and all referenced reports
before returning any record. Corrupt/dangling episode data fails closed; do not
silently repair it during a read. All episode writes use the existing stable lock
and one atomic index replacement. No second store or partially committed file is
introduced.

For saved import and snapshot deletion, compute the reports removed by the
existing alias/report update, then remove every episode containing any removed
report before writing the index. This deliberately deletes the **entire affected
group and its assessment**; it does not shrink membership or transfer it to a new
digest. The grouping is lost, including references to members that still exist.
Users may create a new group from surviving snapshots. No tombstone, automatic
undo, or resurrection on later import is included in this slice.

| Snapshot action | Episode effect |
| --- | --- |
| Reimport identical bytes in the same format | Preserve membership, UUID, both revisions, and episode assessment; newly resolved member metadata may change |
| Add an identical-content alias | Preserve groups; the alias is not another episode member |
| Replace one alias while another still retains the old report | Preserve groups referencing that old report; do not add the replacement |
| Replace the last alias of a report | Atomically remove all episodes referencing that report |
| Explicitly delete a saved snapshot | Remove all its aliases and all affected episodes atomically; original files survive |
| Remove/change a source file outside Insights | No immediate effect: there is no watcher or source reread; later explicit reimport or deletion determines cleanup |
| Delete an episode | Remove only its group and assessment; preserve reports, aliases, member links, and overlapping groups |

Store mutation results must include sorted `invalidated_episode_ids`. Route all
CLI/service saved-import and snapshot-delete paths through this implementation;
keep any compatibility wrappers delegating to it so they cannot bypass cleanup.
Expose an additive mutation-effects field on service Analyze/Delete responses
(empty for ephemeral analysis). Native and CLI presentation must explicitly say
when groups and their assessments were removed, including on replacement imports.

Preserve the existing CLI JSON insight shape: saved analysis can serialize its
existing insight fields plus a top-level `mutation_effects` field, without adding
those effects to the persisted `LocalInsight`. Snapshot-delete JSON adds the same
effects beside its existing result. Pin both shapes with compatibility tests.
Human output names the removed episode IDs and explains that grouping was lost;
an empty effects list is quiet. Report effects only after the atomic write
succeeds. A validation, lock, revision, or write failure exits nonzero and must
not claim deletion or cleanup succeeded; original index bytes remain usable.

No episode operation can bootstrap useful state without saved members. On an
absent store, list returns empty and explain/edit/delete return not found without
creating a directory or lock file. Create on an absent store returns missing
members without creating state. Preserve the existing symlink/private-directory
checks and distinguish permission/corruption errors from empty history.

## Overlap, bounded reads, and limits

Allow overlapping groups. Derive overlap from the same validated index used for
the response; do not cache it independently. List exposes distinct other episode
IDs, while explain identifies the shared member IDs and their other groups.
Native views surface this overlap beside membership. An episode count means only
saved user groups, never independent tasks or an eligible comparison population.

Initial implementation bounds are 256 episodes per store and 64 distinct members
per episode, with at least one member required. The former keeps flat local
selection/list and overlap work bounded; the latter supports multi-session work
while limiting one detail response. These are product/resource bounds, not
statistical sufficiency thresholds. Bound requests before allocating/locking;
reject duplicate member IDs and oversized sets rather than silently truncating.

Retain the current 16 MiB serialized index bound. List must not expand all member
reports repeatedly. Explain includes each selected report once, with a separate
bounded overlap structure. Check serialized service responses against a 16 MiB
bound too and return a defined size error rather than a partial evidence view.
Test worst-case overlap and large existing member reports. Adjusting a bound
later requires explicit compatibility/performance evidence, not a new dependency.

## Shared operations and CLI

Add flat operation variants to `LocalInsightsOperation` and corresponding typed
responses, with all logic in the shared local service/store:

| Operation | CLI |
| --- | --- |
| `EpisodeCreate { snapshot_ids }` | `insights episode-create --snapshot ID [--snapshot ID ...]` |
| `EpisodeList {}` | `insights episode-list` |
| `EpisodeExplain { id }` | `insights episode-explain EPISODE_ID` |
| `EpisodeReplaceMembers { id, expected_revision, snapshot_ids }` | `insights episode-replace-members EPISODE_ID --expected-revision N --snapshot ID [...]` |
| `EpisodeAnnotate { id, expected_revision, category, outcome }` | `insights episode-annotate EPISODE_ID --expected-revision N --category tests --outcome accepted` |
| `EpisodeClearAssessment { id, expected_revision }` | `insights episode-clear-assessment EPISODE_ID --expected-revision N` |
| `EpisodeDelete { id, expected_revision }` | `insights episode-delete EPISODE_ID --expected-revision N` |

Resolve the supplied snapshot IDs to `(ID, digest)` members under the lock;
snapshot IDs are already content-addressed and no mutable source path participates.
Reject empty/missing/duplicate inputs. Generate the UUID only for a valid create;
return it and revision 1 after persistence. All mutation responses return the
authoritative committed episode or deleted ID/revision, not speculative UI state.
Reuse global `--store-dir` and JSON behavior, and preserve stable error codes.

`service::dispatch_json` currently masks execution errors as
`insights-operation-failed`. Add an explicit whitelist mapping typed episode
errors to fixed public codes for revision conflict, episode not found, missing
members, and size/cap violations through the existing generic FFI response.
Native conflict handling must use those public codes, not hidden Rust errors or
message matching. Keep all other failures behind the generic error; never
forward arbitrary `anyhow`, filesystem-path, parser, or source-content details.
Test each allowed code and a sensitive unexpected error through `dispatch_json`
and `tc_insights_call`, including that unknown errors remain masked.

## Stacked implementation sequence

1. **Types and store lifecycle.** Add `episodes.rs`, v4 validation/migration,
   revisions, read-time evidence resolution, overlap, caps, atomic alias/deletion
   cleanup, and mutation-effects types. Keep CLI/native behavior unavailable until
   its shared operations are present. Exercise the store's public entry points.
2. **Shared service, CLI, and generic FFI.** Add the operation family, absence
   handling, JSON/human renderers, cleanup notices, bounded responses, shared copy,
   and end-to-end synthetic tests. Build the FFI and maintain its existing ABI.
   Snapshot summaries remain unchanged. This layer is reviewable and usable
   before adding native controls.
3. **macOS episode UI.** Within Insights, select saved snapshots, confirm their
   whole-snapshot scope, inspect members/overlap, and edit membership/assessment.
   Use shared operations, copy, and expected revisions; no independent reducer.
4. **Windows episode UI.** Apply the same flow and native lifecycle checks against
   the freshly built FFI, with exact platform CI evidence.
5. **GTK episode UI.** Apply the same flow through the direct off-thread service;
   extend the Linux display regression and preserve first-run state-free entry.

Each is a separate stacked PR after this plan; implementations may be prepared
in isolated worktrees once their contracts are fixed. Do not claim native or
episode capability from a documentation-only PR or shared Rust tests alone.

Native selection and edit drafts carry episode UUID/revision and a presentation
generation. Switching, closing, cancelling, or leaving/re-entering invalidates
pending selection callbacks; accepted writes may finish. A stale revision forces
refresh and explicit review, without overwriting another client's edits. After
membership changes clear an assessment, the UI explains why it is unassessed.
Refresh episode list/detail after member import/deletion and episode edits; clear
stale detail on invalidation/failure. Keep normal snapshot analyze/save controls
accessible, and keep contribution, enrollment, discovery, and upload independent.

## Required verification

- Canonical membership and assessment bindings: one/many/mixed-format members,
  unknown versus unassessed, stable UUID, membership/general revisions, no-op
  behavior, revision overflow, timestamp validation, and tampered cached digests.
- v1/v2/v3 migration: empty default, no rewrite on read, reject legacy nonempty
  episode fields and unsupported future versions, preserve existing annotations
  and links, refuse corrupt/dangling members, and retain private/symlink checks.
- Atomicity and concurrency: two edits from one expected revision yield one
  success and one conflict; stale delete cannot erase a newly edited group;
  write failure changes neither snapshot data nor episodes. Busy-lock errors
  remain distinct from successful no-op edits.
- Import/deletion matrix above, including two aliases of identical bytes,
  last-alias replacement, several affected overlapping groups, successful cleanup
  effects, failed cleanup with unchanged index, and no automatic resurrection.
- Resolved evidence: unlink/relink Git or test evidence and edit a member's
  assessment; next explain reflects the change while episode membership,
  revisions, and independent assessment remain unchanged. Remove original source
  files and verify episode reads need only the saved index.
- Overlap/caps: identical groups, partial overlap, disjoint groups, duplicate
  input rejection, exact boundary/overflow, maximal overlap, bounded response
  failure, and no episode metrics or changes to snapshot summary fixtures.
- CLI/service/FFI: create/list/explain/replace/annotate/clear/delete round trips,
  expected-revision errors, malformed JSON/UUID, absent-store no-creation, visible
  cleanup notices, stable JSON shapes, and freshly linked native compatibility.
- Each native shell: whole-snapshot confirmation, members/overlap/authority,
  independent assessments, edit conflict, source replacement invalidation,
  selection-generation races, hidden/re-entry cancellation, no stale success,
  and first-run operation without enrollment or contributor state.

Run warning-denied focused and relevant workspace tests, formatting, contributor
and FFI Clippy, and the permissive license boundary. Use the GTK workspace's own
target for GTK checks. No dependency or lockfile change is planned; Linux display
and Windows/macOS native CI remain separate qualification evidence. Model
comparisons and partial-session boundaries require a later evaluation design.
