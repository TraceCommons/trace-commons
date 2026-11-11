# Skills insights flow

Review [the design surface](mocks/index.html), [inventory chart](FLOW.svg),
[usage chart](USAGE.svg), or [detail chart](DETAIL.svg). Each image has adjacent
Mermaid source. All charts are generated from TLC's complete transition dump,
then rendered by the flow-promises skill's Pele renderer. No chart is hand drawn.

## What was checked

This is a bounded **design model**, not verification of the Rust or Swift binary.
The user selected these five promises. TLC exhaustively explored **1,338 states**
and **18,795 transitions** with no constraint or symmetry reduction. All five
invariants hold; the helper's additional NoDeadlock check also holds.

| Selected promise | Model result | Representation |
| --- | --- | --- |
| Disabling Skills hides its UI and prevents new work. | Holds | Feature gate removes visible inventory/preview; admission records whether work started while disabled. |
| Excluded folders never appear in results. | Holds | One representative folder permission gates inventory, definition and usage provenance. |
| Results from before a clear or permission change cannot restore removed data. | Holds | Separate request lanes become permanently stale on invalidation; publication records its request validity. |
| Analysis off still permits inventory but produces no usage. | Holds, with reachability witness | Safety invariant plus initial → Refresh → scan finds skills, with analysis false throughout. |
| Failed reads and catalog listings never count as use. | Holds | `evidence` is the provenance of the representative counted output; failed reads/catalog observations do not write it. |

The [raw checker summary](check-report.md) and [projection counts](projection.json)
record this run. No liveness promise was selected. Work may hang indefinitely;
`TheWorldWaits` makes that explicit rather than imposing fairness. A UI-only
control that starts no work does not count as a new job.

## Model scope and abstraction

`FLOW.qnt` represents one folder, one installed definition and one representative
usage event. Inventory, extraction and detail have independent request lanes,
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
requests within a lane, process restart, numeric counts/deduplication, time
windows/timezones, retention, catalog/follow operations, sorting and search.
Read failures and unavailable definitions are merged; the model does not claim
the UI currently exposes every illustrated state. Evidence detail reads share
the conceptual detail lane; their payload and separate storage permission need
code-level tests. Inventory refresh preserves the representative inventory
until new results arrive (except on exclusion/feature disable).

## Rendering merges

The checker budget is 500 states, so the ordinary check produces no chart.
The full model was retained and a second TLC run dumped **all** states/actions.
`render-graph.mts` uses the skill's DOT parser and projects that dump three ways:

- Inventory: feature/permission state, scan activity, empty/partial/available result.
- Usage: feature/permission and analysis state, extraction activity, complete/partial/unsupported result.
- Detail: feature/permission state, current/stale pending read, shown/closed preview.

Each projection merges other variables and edges with identical endpoint labels;
all action labels on each resulting arrow are retained. Every source state is
represented. Same-label transitions become internal steps (counted in
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
node render-graph.mts "$FLOW_SKILL" /tmp/skills-full-new/graph.dot .
node "$FLOW_SKILL/scripts/render.mjs" FLOW.mmd FLOW.svg
node "$FLOW_SKILL/scripts/render.mjs" USAGE.mmd USAGE.svg
node "$FLOW_SKILL/scripts/render.mjs" DETAIL.mmd DETAIL.svg
```

Do not check generated compiler/cache folders into the repository. All diagrams
were inspected after Pele rendering via Quick Look PNG previews; that verifies
legible labels/edges/colors, not product UI behavior.

## Negative controls

All five deliberately broken models produced the expected TLC counterexample.
These are model mutations, not reproduced product defects:

| Invariant | Deliberate mutation | Shortest action path |
| --- | --- | --- |
| SkillsDisabled | Admit a periodic scan while disabled | Flag off → periodic scan starts (2 steps) |
| ExcludedFolders | Retain inventory when excluding its folder | Refresh → scan finds skills → exclude (3 steps) |
| OldResultsStayRemoved | Publish a stale scan and record its stale provenance | Refresh → flag off → old scan returns (3 steps) |
| AnalysisOff | Retain usage state on analysis disable | Enable analysis → disable analysis (2 steps) |
| OnlyObservedUse | Attribute a failed read as counted output | Enable analysis → analysis starts → failed read (3 steps) |

[Raw mutation reports](mutation-report.md) retain the exact checker paths.

`python3 mutation-checks.py FLOW_SKILL NEW_SCRATCH_DIR` generates these mutations outside
the repository, runs the skill checker sequentially, and requires each named
invariant to break. The compiler uses a local backend, so running these compile
jobs concurrently can contend; the recorded successful checks were sequential.
