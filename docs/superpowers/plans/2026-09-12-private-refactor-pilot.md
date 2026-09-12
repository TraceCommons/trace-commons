# Private refactor pilot

Status: fresh independent session preparation authorized; original retrospective evidence retained. Three authorized Claude files have been imported into a private pilot store, and the user assessed all three final PR outcomes as accepted. Original patches for #616 and #606 match the corresponding final PR patches; #632 has unresolved later integration changes. Complete task boundaries, historical context, independence, and comparison qualification remain unverified.
Parent: [personal refactor comparison delivery plan](2026-09-12-personal-refactor-comparison.md).

Reward systems for both missions and insights are outside scope and owned by Abhishek. This pilot collects evidence and user assessments without reward eligibility, compensation, or payout features.

## Native pilot preparation checkpoint

A new isolated working copy of the original two-file pilot store was verified byte-for-byte; the original remained unchanged. Existing accepted outcomes were preserved without adding context or independence confirmations. Its private location and hash manifest stay local. The three selected tasks still share one model cohort and a root session; copying the store does not change eligibility.

The macOS date and saved-record reconciliation fixes are in [#944](https://github.com/TraceCommons/trace-commons/pull/944). Explicit store routing and the reviewed actor/shared-copy fixes are in [#945](https://github.com/TraceCommons/trace-commons/pull/945) at `15f66a62`. The combined focused Swift suite passed 27/27, including real-FFI routing; subsequent shared-copy/wording checks and warning-denied Clippy passed. The subsequent #945 head `9ac49116` passed all 26 CI checks. Astra also cleared local native-render follow-up `02de243a`: a direct Insights view with a synthetic store visibly contains the selected-store location and populated task/specification rows. That follow-up is published in #945 at `9ac49116`; it does not establish interactive workflow or human-pilot completion. The newer exact-comparison FFI passes 31 focused native checks. Draft [#947](https://github.com/TraceCommons/trace-commons/pull/947) includes a reviewed stable task-row OCR assertion that passes the render test 1/1 while separately verifying the backend pending state. Supported nonempty synthetic CLI/FFI comparison validation now passes in #947. The actual supported macOS router/model path also passes, with the full routing integration class 4/4 and Astra review. Current-head CI, separate admission, and the real human pilot remain outstanding.

Once integrated, launch the app with `--insights-store` and the absolute path to the existing working-copy directory. Verify the visible custom-store location before interacting; malformed, missing, relative, duplicate, or non-directory selections must not fall back to the normal store. Keep task context, outcome, and independence review separate. The real two-cohort app pilot remains outstanding.


## Fresh capture decision and historical discovery

A local metadata audit found that branches using the currently admitted Claude writer profile all share one root session, including the original three selected tasks. Additional roots have other writer versions. A parent session's model declaration also does not identify its subagents' declared models. This is discovery evidence only, not task attribution or outcome review; the detailed manifests remain local.

The user selected preparation of four fresh independent refactor sessions. Follow the [fresh-session preparation plan](2026-09-12-fresh-refactor-sessions.md): four disjoint tasks from one pinned checkout, separate root sessions, two matched task pairs and two declared-model cohorts. Broad historical discovery is no longer the next step. The installed writer is `2.1.269`, and direct-root sessions differ from the admitted `2.1.260` branch profile; actual-source qualification is required before comparison eligibility. Do not broaden version admission based on model metadata alone, infer independence from distinct branch files, or reuse the three accepted outcomes for other work. The human task/context/outcome/independence review remains required.

## First repository and tasks

Use Trace Commons for the first local retrospective pilot. Retain these completed refactors as the historical audit. The active comparison pilot now uses the four fresh work items described above; additional historical searching is not a preparation requirement. Both are usability evidence, not a statistical sample-size claim.

| Candidate | Review focus |
| --- | --- |
| [#616: source-root resolution](https://github.com/TraceCommons/trace-commons/pull/616) | Small shared-helper extraction; behavior preservation. |
| [#606: operator CLI plumbing](https://github.com/TraceCommons/trace-commons/pull/606) | Several callers; request/output compatibility. |
| [#632: daemon IPC handlers](https://github.com/TraceCommons/trace-commons/pull/632) | Larger extraction; handler behavior and lock lifetime. |

These PRs are merged, but neither merge status nor passing tests supplies the user's outcome assessment. Inspection found that all three selected files are child-agent branches of one parent session and retain one valid declared model cohort. They cannot supply a two-model comparison or three independent observations. A PR remains a candidate work item: complete attempt membership and final integration must be reviewed separately from patch matching.

The user authorized locating these traces under local Claude history; the three selected originals remain unchanged. Further source inspection stays within that authorization. Do not synthesize a replacement historical trace, alter model labels to create cohorts, infer an outcome from merge status, or replay completed work and call it historical evidence. Keep source files and pilot observations local.

## Prepare the local workflow

Use a build containing the task, specification, and released-source layers (#932–#936). The CLI below works independently of the macOS shell. The macOS Insights workflow offers task editing, evidence review, specification saving, evaluation, and drilldown through the same shared service.

Set `TC_BIN` to the chosen contributor binary and `TC_PILOT_STORE` to a new local pilot directory. Neither variable changes enrollment or the normal Insights store. Examples use shell placeholders; replace them with reviewed values before running. JSON responses provide IDs, revisions, and digests for subsequent commands.

```sh
TC_BIN='/absolute/path/to/trace-commons-contributor'
TC_PILOT_STORE='/absolute/path/to/private-refactor-pilot/insights'
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" list
```

An empty-store read should leave the store absent. Record the application commit and source format/version in the local pilot notes. A version label alone does not qualify a source. Native Claude Code files can be imported descriptively with `--source claude-code` (#939). The model-observation layer (#940) adds source-declared model labels and physical record references; it does not establish serving identity, task authorship, usage, time, or comparison attribution. The [Codex source profile](../specs/2026-09-12-codex-comparison-source-profile.md) admits bounded Codex 0.154.0 exec traces for comparisons. The [Claude agent-branch source profile](../specs/2026-09-12-claude-comparison-source-profile.md) is implemented in draft [#941](https://github.com/TraceCommons/trace-commons/pull/941). Independent source review, bounded acceptance checks, and local Rust/Clippy/GTK/macOS checks passed; all 26 remote checks passed at head `8d018dc1`. Older, interactive, delegated, or unsupported traces may still import generically when their selected source parser accepts them, but remain unavailable for comparison; record that as coverage feedback.

## Current local evidence checkpoint

The original pilot store retains three accepted assessments, no independence confirmations, and zero fully qualified comparison tasks. Historical context and complete task boundaries still need review. No workflow timing, abandonment, or comprehension result has been collected.

A disposable copy exercised the pending Claude attribution implementation: legacy reads preserved the store bytes, explicit reimport did not silently update frozen task bindings, and an explicit same-evidence binding refresh preserved the accepted value and assessment timestamp. Source attribution became available for #616 and #606; #632 remained unavailable at an unsupported terminal record. All three shared the frozen parent-session overlap, which remained after deleting a source from the copy. These checks passed again on the build corresponding to #941 commit `1744f822`, after independent review fixes. Additional CLI checks confirmed typed unavailability for malformed Monitor metadata and uppercase session IDs, plus rejection of cached evidence missing a required Monitor reference. Astra cleared the implementation and its test-only follow-up; local Rust/Clippy/GTK/macOS checks passed, and all 26 remote checks passed at head `8d018dc1`. This is a source implementation checkpoint, not a completed human comparison pilot.

Keep raw files, local paths, identifiers, and detailed pilot artifacts local. Publish only the bounded status and implementation changes here.

## Import and review one task

1. Import only an authorized original trace. Set `TC_SOURCE_FORMAT=claude-code` for the selected Claude pilot files (`codex` remains available for Codex sources), then create an episode and task using the returned identifiers:

```sh
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" analyze \
  --source "$TC_SOURCE_FORMAT" --file "$TC_TRACE_FILE" --save
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" episode-create \
  --snapshot "$TC_SNAPSHOT_ID"
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" comparison-task create \
  --episode "$TC_EPISODE_ID"
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" comparison-task explain "$TC_TASK_ID"
```

Claude import reads exactly the selected file, without discovering its parent or sibling agents. Distinct content sequences sharing a message ID are preserved; exact repeated content sequences under that ID are collapsed. Metrics describe normalized record/block observations, not unique tool executions. An agent-written opening instruction is not proof of a human prompt. Preserve the original file while reviewing any missing task membership.

2. Review the whole task boundary and every attempt. Multiple turns never count as multiple independent tasks. Reuse one user-selected project UUID across worktrees of this repository. Set only known context fields:

```sh
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" comparison-task set-context "$TC_TASK_ID" \
  --expected-revision "$TC_REVISION" --project-id "$TC_PROJECT_ID" \
  --task-date "$TC_TASK_DATE" --language "$TC_LANGUAGE" \
  --harness-id "$TC_HARNESS_ID" --harness-version "$TC_HARNESS_VERSION" \
  --reasoning-effort "$TC_REASONING_EFFORT" \
  --tool-policy-id "$TC_TOOL_POLICY_ID" --tool-policy-version "$TC_TOOL_POLICY_VERSION" \
  --prompt-template-digest "$TC_PROMPT_TEMPLATE_DIGEST"
```

Omit unknown optional flags; reasoning effort defaults to `unknown`, which is different from known `none`. A prompt-template digest is SHA-256 of the exact UTF-8 template before task-specific substitution. A policy identity must describe the actual tool permissions; do not copy fixture settings or invent missing historical context to admit a task. See the [context contract](../specs/2026-09-12-comparison-context-v1.md).

3. Ask the user to assess the bound work as `accepted`, `partial`, `rejected`, `pending`, or `unknown`. Record substantial human rework separately in local pilot notes; it is not currently a quantified result field. Use the latest returned task revision for each mutation:

```sh
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" comparison-task set-outcome "$TC_TASK_ID" \
  --expected-revision "$TC_REVISION" --outcome "$TC_OUTCOME"
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" comparison-task explain "$TC_TASK_ID"
```

4. After the user reviews the displayed frozen/current evidence and overlap information and confirms independence, use that displayed task revision and material digest:

```sh
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" comparison-task reconfirm "$TC_TASK_ID" \
  --expected-revision "$TC_REVISION" --material-digest "$TC_MATERIAL_DIGEST"
```

Do not automate confirmation or retry a conflict with a freshly fetched digest without another review. Changing context or evidence requires reviewing stale outcome/confirmation bindings. Unsupported attribution cannot be repaired through confirmation.

## Save and inspect a comparison

Proceed when two recorded declaration cohorts have matching reviewed context. Read cohort labels from `detail.source_qualification.declared_model_cohort` and the context fingerprint from `detail.task.context.configuration_fingerprint`. If qualification is absent or only one cohort exists, report that limitation; do not manufacture the second cohort.

After recording the intended task evidence and outcomes, choose an explicit UTC evidence cutoff. Task dates are calendar dates, while the cutoff controls when material evidence and outcomes were recorded locally. Importing old tasks today does not backdate those records.

```sh
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" comparison preview-spec \
  --evidence-cutoff "$TC_CUTOFF_UTC" --cohort "$TC_COHORT_A" "$TC_COHORT_B" \
  --date-start "$TC_DATE_START" --date-end "$TC_DATE_END" --project-id "$TC_PROJECT_ID" \
  --language "$TC_LANGUAGE" --configuration-fingerprint "$TC_CONFIGURATION_FINGERPRINT"
```

Review the preview; use the same arguments with `save-spec` to save the immutable retrospective specification. Then use the returned specification ID:

```sh
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" comparison evaluate "$TC_SPECIFICATION_ID"
"$TC_BIN" --json insights --store-dir "$TC_PILOT_STORE" comparison explain-result "$TC_SPECIFICATION_ID" \
  --audit-digest "$TC_AUDIT_DIGEST"
```

Set `TC_AUDIT_DIGEST` to `result.audit_digest` from the evaluated response before explaining it. If the audit is stale, evaluate and review the changed result again; do not silently replace the digest.

Inspect included and excluded tasks, categorical outcome denominators, and coverage/missingness. Pending, unknown, and unassessed outcomes are not rejected work. Observed tokens are not complete task usage, dollar cost, or human time saved. Descriptive or suppressed output is a valid pilot result; it does not prove a model advantage. The interval estimator remains unqualified.

## Observe the product, then make corrections

Keep a local record with one row per reviewed task and a separate specification/result record. Use task IDs and structured statuses; keep free-text outcome commentary and original traces out of PRs.

| Observation | What to record |
| --- | --- |
| Source coverage | Full reviewed-task denominator; import success, producer version, attribution availability; mixed-model, delegated, and unsupported counts/fractions with typed exclusions. |
| Task boundary | Number of attempts, user confirmation, same-session overlap, unresolved ambiguity; user corrections that split/merge tasks or restore omitted attempts. |
| Context completion | Missing fields, whether values were known or only guessed, difficulty selecting project/policy/template; time to first reviewed result and where the user abandons the workflow. |
| Outcome | User report, delayed assessment, substantial human rework noted separately. |
| Staleness | Which edit invalidated a result or binding; whether the next required review was understandable. |
| Comparison | Exact context, cohort declarations, included/assessed counts, exclusions, coverage and suppression. |
| Comprehension | Can the user distinguish recorded model labels, user-reported outcomes, task counts, and observed usage? What decision, if any, would the result change? |

On a disposable copy of the pilot store, exercise context edits, delayed outcomes, duplicate-session imports, and deletion. Confirm stale results disappear, old cutoff specifications exclude later evidence, overlapping exports do not become independent after deletion, and reconfirmation requires explicit review. Preserve the original pilot store; never delete real evidence just to perform a drill.

The pilot report should separate real-task observations, synthetic regression results, unresolved source gaps, and product changes. Apply observed corrections before Windows/GTK parity. A subsequent prospective pilot can use two models through a supported harness with consistent settings and comparable scopes; it still requires reviewed independent tasks and a qualified interval method before drawing comparison claims.

These observations address the immediate representativeness and usability concerns in [Kristi’s overall-plan review](https://github.com/TraceCommons/trace-commons/pull/870#issuecomment-5645229086). Keep every selected task in the coverage denominator, including abandoned or unsupported cases; analyzing only admitted tasks would hide whether this product serves the actual workflow.
