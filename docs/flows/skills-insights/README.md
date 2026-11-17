# Skills insights flow

Review [the design surface](mocks/index.html), [inventory chart](FLOW.svg),
[usage chart](USAGE.svg), or [detail chart](DETAIL.svg), [delegated traces](DELEGATED.svg), or [OpenCode normalization](OPENCODE.svg). Each image has adjacent
Mermaid source. All charts are generated from TLC's complete transition dump,
then rendered by the flow-promises skill's Pele renderer. No chart is hand drawn.

## What was checked

This is a bounded **design model**, not verification of the Rust or Swift binary.
The user selected these five promises. TLC exhaustively explored **74,288 states**
and **1,578,878 transitions** with no constraint or symmetry reduction. All five
invariants hold; the helper's additional NoDeadlock check also holds.

| Selected promise | Model result | Representation |
| --- | --- | --- |
| Disabling Skills hides its UI and prevents new work. | Holds | Feature gate removes visible inventory/preview; admission records whether work started while disabled. |
| Excluded folders never appear in results. | Holds | One representative folder permission gates inventory, definition and usage provenance. |
| Results from before a clear or permission change cannot restore removed data. | Holds | Separate request lanes become permanently stale on invalidation; publication records its request validity. |
| Analysis off still permits inventory but produces no usage. | Holds, with reachability witness | Safety invariant plus initial → Refresh → scan finds skills, with analysis false throughout. |
| Failed reads and catalog listings never count as use. | Holds | `evidence` is the provenance of the representative counted output; failed reads/catalog observations do not write it. |

The [raw checker summary](check-report.md), [full-exploration output](exploration-report.txt),
[model hashes](model-hashes.json), and [projection counts](projection.json)
record this run. No liveness promise was selected. Work may hang indefinitely;
`TheWorldWaits` makes that explicit rather than imposing fairness. A UI-only
control that starts no work does not count as a new job.

## Model scope and abstraction

`FLOW.qnt` represents one parent folder, one child trace, one installed definition, and one
representative direct, delegated and normalized OpenCode event. Inventory, extraction and detail have independent request lanes,
so completions can interleave. Each lane is idle/current/stale. This is a
relative-generation abstraction, **not** a two-value numeric generation counter:
once invalidated, a request remains stale through any number of re-enables or
permission changes until its response is discarded. Clear invalidates usage;
permission changes invalidate all lanes; disabling analysis leaves inventory
available. File changes invalidate the preview and pending definition read.

`startedDisabled` and `restoredStale` are observer variables, recording admission
and publication provenance. Their safety properties depend on implementation
faithfully setting/checking generation guards; they do not establish those guards
exist in product code. Failed-read attribution is checked on event provenance,
not on exact numeric totals. Negative controls deliberately mutate those
transitions and must produce counterexamples (see mutation results below).

The model omits concrete paths, symlink resolution, provider parsing, persistent
store crashes, transport reordering beyond these lanes, multiple simultaneous
requests within a lane, process restart, general numeric totals and multi-event deduplication, time
windows/timezones, retention, catalog/follow operations, sorting and search.
Read failures and unavailable definitions are merged; the model does not claim
the UI currently exposes every illustrated state. Evidence detail reads share
the conceptual detail lane; their payload and separate storage permission need
code-level tests. Inventory refresh preserves the representative inventory
until new results arrive (except on exclusion/feature disable).

## Rendering merges

The checker budget is 500 states, so the ordinary check produces no chart.
The full model was retained and a second TLC run dumped **all** states/actions.
`render-graph.mts` uses the skill's DOT parser and projects that dump five ways:

- Inventory: feature/permission state, scan activity, empty/partial/available result.
- Usage: feature/permission and analysis state, extraction activity, complete/partial/unsupported result.
- Detail: feature/permission state, current/stale pending read, shown/closed preview.
- Delegated: parent/child exclusions, independent extraction, missing/partial child evidence.
- OpenCode: running/failed calls versus one accepted invocation/load; repeated database/export observations are visible self-loops.

Each projection merges other variables and edges with identical endpoint labels;
all action labels on each resulting arrow are retained. Every source state is
represented. Same-label transitions become internal steps, except explicit duplicate-copy
self-loops in the OpenCode view (counted in
`projection.json`); no cross-label arrow is hidden. A chart arrow is existential:
some underlying states admit it, not every state with that label. In particular,
re-enabling can show an outstanding stale request while it drains; the result
still cannot commit. Consult the Quint guards for exact cross-region conditions.
The charts are deliberately separate and zoomable because reset/exclusion edges
are retained. Projection is for rendering only and never used for checking.

## Implementation relationship and replay

Sources: the [approved local inventory design](../../superpowers/specs/2026-11-11-local-skill-index-design.md),
`insights_skills.rs`, `insights_week.rs`, and `InsightsSkillsModel.swift` in this
worktree. Implementation was being edited concurrently during this model run.
At initial inspection the daemon compared inventory policies before returning,
and Swift detail callbacks compared selection IDs. These checks alone are not
an implementation of a monotonic invalidation generation. Subsequent code edits
must be reviewed/tested independently; this artifact does not certify them.

Replay these scenarios in code with delayed/mock completions:

1. Start inventory, disable feature, finish old work; UI stays hidden and new
   periodic/manual jobs are rejected.
2. Start inventory and detail, exclude folder, finish both; no inventory,
   definition, or usage from that folder is shown.
3. Start usage, clear, re-enable/refresh, finish old usage; old rows stay removed.
   Repeat exclusion → permission restored to catch policy-equality ABA bugs.
4. Turn analysis off, Refresh, complete scan; inventory appears, usage remains
   unavailable. Finish pre-disable extraction too.
5. Parse failed read and catalog listing fixtures; invocation/load counts stay
   unchanged. Positive invocation and successful load remain separate evidence.
6. Start definition, change file or change selection, finish read; old content
   does not reappear. Exercise the same installation ID across invalidation.

No product counterexample is claimed from a model mutation. Mutations establish
that the model's checks can detect these representative violations.

## Reproduce

Set `FLOW_SKILL` to the checked-out `flow-promises` skill directory. Requires
Quint, Java 21+, Node with TypeScript support, and the skill's TLC/Pele tools.
Set `JAVA`/`TLA2TOOLS` to existing installations if mise is unavailable.
From this directory:

```sh
"$FLOW_SKILL/scripts/check" FLOW.qnt --view view.mts --out /tmp/skills-check-new
```

The check intentionally does not reduce the model to fit the chart budget.
It deletes old FLOW SVG/Mermaid when over budget, so always regenerate afterward:

```sh
mkdir /tmp/skills-full-new
quint compile --target tlaplus --main FLOW FLOW.qnt > /tmp/skills-full-new/FLOW.tla
cp FLOW.cfg /tmp/skills-full-new/FLOW.cfg
# Extract Apalache.tla and Variants.tla from the installed apalache.jar into
# /tmp/skills-full-new (the skill's check script shows the exact unzip commands).
# In /tmp/skills-full-new:
"$JAVA" -XX:+UseParallelGC -cp "$TLA2TOOLS" tlc2.TLC -workers 1 \
  -dump dot,actionlabels graph -config FLOW.cfg FLOW
# Back in this directory:
NODE_OPTIONS=--max-old-space-size=8192 node render-graph.mts "$FLOW_SKILL" /tmp/skills-full-new/graph.dot .
node "$FLOW_SKILL/scripts/render.mjs" FLOW.mmd FLOW.svg
node "$FLOW_SKILL/scripts/render.mjs" USAGE.mmd USAGE.svg
node "$FLOW_SKILL/scripts/render.mjs" DETAIL.mmd DETAIL.svg
node "$FLOW_SKILL/scripts/render.mjs" DELEGATED.mmd DELEGATED.svg
node "$FLOW_SKILL/scripts/render.mjs" OPENCODE.mmd OPENCODE.svg
```

Do not check generated compiler/cache folders into the repository. All diagrams
were inspected after Pele rendering via Quick Look PNG previews; that verifies
legible labels/edges/colors, not product UI behavior.

## Negative controls

All five original deliberately broken models produced the expected TLC counterexample.
Two extension-specific negative controls additionally exercise the same selected
promises: counting a running OpenCode call and retaining child evidence after
parent exclusion.
These are model mutations, not reproduced product defects. Some mutation runs
reach the checker's 100,000-state post-failure exploration bound before its
additional NoDeadlock check completes; the named invariant counterexample is
already established. This bound does not apply to the successful base-model
exploration reported above:

| Invariant | Deliberate mutation | Shortest action path |
| --- | --- | --- |
| SkillsDisabled | Admit a periodic scan while disabled | Flag off → periodic scan starts (2 steps) |
| ExcludedFolders | Retain inventory when excluding its folder | Refresh → scan finds skills → exclude (3 steps) |
| OldResultsStayRemoved | Publish a stale scan and record its stale provenance | Refresh → flag off → old scan returns (3 steps) |
| AnalysisOff | Retain usage state on analysis disable | Enable analysis → disable analysis (2 steps) |
| OnlyObservedUse | Attribute a failed read as counted output | Enable analysis → analysis starts → failed read (3 steps) |
| OnlyObservedUse | Count a running OpenCode call | Enable analysis → analysis starts → running call (3 steps) |
| ExcludedFolders | Retain child evidence on parent exclusion | Child available → enable analysis → child analysis starts → child invocation → exclude parent (5 steps) |

[Raw mutation reports](mutation-report.md) retain the exact checker paths.

`python3 mutation-checks.py FLOW_SKILL NEW_SCRATCH_DIR` generates these mutations outside
the repository, runs the skill checker sequentially, and requires each named
invariant to break. The compiler uses a local backend, so running these compile
jobs concurrently can contend; the recorded successful checks were sequential.

## OpenCode and delegated extension

The same five user-selected promises cover the new variables; no new promise
selection is assumed. The complete dump additionally checks every observed
duplicate transition for unchanged `accepted`, and every running/failed/catalog
transition for unchanged `accepted`. These are explicit model consistency
checks, not proof that the production normalizer implements this abstraction.
`projection.json` records 36,288 duplicate-observation transitions and 99,792
incomplete/failed/catalog transitions; every one preserves accepted evidence.

`call` models one stable OpenCode session/event identity, seen in database and/or
export form. A final invocation or successful definition load sets `accepted=1`;
re-observing either representation keeps it at one. Running/failed calls and
catalog records produce no accepted event. An explicit invocation and a load are
alternative representative event kinds here; real traces may contain both as
separate observations. No assumption that text-only name matching is sufficient
for attribution is made. Timestamp merging, cross-version event identity,
multiple calls, export import order, parse failures and real database adapter
behavior require fixture and integration tests outside this model.

The child lane has its own current/stale generation abstraction. Parent exclusion
invalidates both lanes; child exclusion invalidates only child work. Clear and
feature/analysis disable remove delegated evidence and invalidate pending writes.
A missing child removes its representative observation and remains a distinct
coverage state. Complete and partial children may be reanalyzed; version-triggered
backfill is abstracted by another admission, without a numeric parser-version
variable. Parent and child populations are separate and never summed in the
model. The parent identity is fixed implicitly; this model cannot prove code
preserves or rejects arbitrary/malicious parent IDs.

**Current product coverage:** the delegated implementation uses existing Claude
spawned traces. Codex automatic parent attribution remains unsupported. Claude
children normally inherit the parent's project permission; the physical child
transcript directory is not itself a new project-policy identity. The model's
independent child exclusion represents filtering an independently scoped stored
child, where such a policy identity exists. It is a conservative design case,
not a claim of a new UI permission toggle or generic provider lineage support.

The first extension check found a missing model invalidation: enable analysis →
start analysis → accept database invocation → turn Skills off retained new fields.
The extension was corrected to clear those fields and stale the child lane on
feature disable; the promise was unchanged. This was a model construction defect,
not a reproduced implementation bug.

Additional replay targets: database then export then database for the same source
event; running call then successful completion; failed call/catalog listing only;
parent exclusion during child extraction; child removal during extraction; clear
then reanalysis then old child completion; missing child remains partial rather
than a zero-use claim. Keep direct/delegated counts and parent drill references
separate in every case.
