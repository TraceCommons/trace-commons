# Local Insights on macOS

The app opens on Insights. Watcher startup, compute monitoring, update checks,
and notification setup begin only after selecting another destination, once
per app launch. Their existing roots, onboarding, and activation checks still
apply. Returning to Insights does not stop services already activated.

The window has tabs. **Overview** (the default) shows one local week of the
snapshots you saved, **Patterns** shows where that week's tokens went,
**Sessions** shows one saved session turn by turn, and **Analyze** holds the
whole screen described below. **Spend** is shown disabled with a "Later"
chip.

Choose a Codex rollout, Claude Code session or trajectory file to analyze it
locally. Analysis is
temporary; **Re-read and save** reads the selected file again and saves derived
observations to the same local store as the CLI. Saved snapshots update only
on explicit import. Deleting a snapshot keeps the original file. Assessments
are user-reported and remain separate from verified outcomes.

The saved-history summary loads on entry and refreshes after saving, deleting,
annotating, or clearing an assessment. It shows saved-session counts, reported
categories/outcomes, and observed metric totals with both snapshot availability
and original evidence coverage. Unknown assessments and unassessed sessions are
separate; a measured zero remains distinct from an unavailable value. Dates are
analysis dates, not work dates. Summary evidence buttons open the corresponding
saved snapshot and scroll its detail into view. Separate summary and detail
reads can observe different store versions; a missing snapshot clears the old
detail and reports failure. A failed summary refresh removes the stale summary.

The screen displays Rust-provided observations, coverage, provider/rubric
attribution, and source-digest evidence. It does not calculate independent
metrics, discover files, enroll contributors, or upload traces.

Overview reads `week_overview` and, for each card's drill-down,
`card_inputs` from the same local service. It counts only sessions you
analyzed and saved (feed S), dated by their own recorded timestamps, never by
when they were imported, in the local ISO week (Monday to Sunday) at the
shell's current UTC offset. Claude Code and Codex tokens are separate lines
and are never summed; Codex is the change between its first and last
counters. A session without usage counters is counted as unknown, never as
zero, and the coverage line says how many. Week-to-week figures ("vs last
week", "Your best week") need watched-folder counting and are left out
here; the feed line says so. By model keeps the core's fixed order (alphabetical, unknown last); By
project is not available for analyzed files. Every word is the core's
`ui_copy()` analytics table, marked DRAFT pending approval; every figure is
the core's. Advice, what-if estimates and costs are not shown.

Patterns reads `patterns` and, for each card's "See sessions", the
`pattern_sessions` list, over the same saved snapshots. Its four cards are
repeated reads, retried tool calls, "Edit, failed command, edit" (inferred
from the order of tool calls) and long context, each with the token figure
first, the count beneath it and six weekly bars; a week with no counted
Claude Code session is a gap, never a zero bar. The first three are estimates
from result size; long context is counted input. The figures overlap and do
not add up to the week's tokens. Only Claude Code sessions record tool calls,
so a week with Codex sessions says how many sessions the figures cover. The
re-read table names files only by a letter and the extension; no path is
stored or shown. Week-to-week change is left out here.

Sessions reads `session_drill` for one saved snapshot, the newest unless you
pick another, while watched-folder counting (below) is not showing. The header gives the date, harness, turns, tokens and the time
between the first and last recorded event, which is not active time. Input
sent each turn is stacked cache read, uncached and cache write, with a dashed
line at the long-context threshold. A turn with any counter unknown is a gap,
never a zero bar. Markers fire once per occurrence and are lettered in turn
order: the cache written again after a pause (inferred from counters; expiry
itself is not observable), context shrinking (inferred), context passing the
threshold, and a file read again with no edit tool call to it in between
(from the order of tool calls, named by letter and extension). Codex sessions
show that turn-by-turn usage is not recorded. No what-if estimate and no
project name are shown.

When the daemon's watched-folder counting (feed T, the
`insights_counter_pass` setting, off by default) is on and readable,
Overview and Patterns show its week instead, in the same shapes, and the feed
line says so; the two feeds are never mixed, and a failed read falls back to
the saved week with a line saying counting is unavailable. Sessions then
lists watched traces only (`insights_sessions`, newest first, at most 200),
each labelled by its folder, the day and time of its first recorded event,
the tool and its turns ("trace-commons · Mon, Oct 19 14:02 · Claude Code ·
119 turns"; a Codex trace records no turns, so its label has none). The
folder is the label the daemon shows for that folder elsewhere, matched at
answer time from the trace's stored folder digest; a folder it no longer
knows reads "Unknown folder". Picking one reads `insights_session_drill`,
the same drill a saved snapshot has, from the daemon's stored counters, so
the same file gives the same figures either way; its header's "i" names the
watched-folder feed. Saved snapshots stay under Analyze. A watched list that
cannot be read falls back to the saved snapshots with the same unavailable
line Overview shows. Only then are
weeks compared: "vs last week" and "Your best week" (a tick on the cache
bar) need comparable weeks. The daemon's kept weekly figures go unchanged to
the core's `comparisons`, which marks your goals (Patterns, "Goals": six
weekly marks and "Down from / Up from last week", never a run of weeks),
picks the lever of the week ("Higher than usual": an observation with "Show reads" and "Not
useful", no advice) and, on the first opens after a week closes, the weekly
summary card at the top of Overview. "View week" puts the closed week on
screen; "Hide" clears the daemon's `insights_recap_card_enabled`, the
card's only switch. Goals and "Not useful" are stored as rule IDs and Monday
dates only. The week's feed T session rows carry no reference, so Overview
and Patterns cards do not drill down to sessions; the Sessions tab lists
them through `insights_sessions`, whose opaque references are not the
week's.

Insights is the main window's fourth tab (Cmd-4; 2026-10-15, before that
a Home page, and Home's Insights card now switches to the tab). It draws in
the main pane: 440pt at the window's default width, 408pt inside the
pane's insets. Every
tab is laid out for that width (direction B, 2026-10-15): one column of
cards, and a second only when the pane is wider than about 690pt. Overview
puts the week and the Sessions figure on one row, then one card per harness
(Claude Code and Codex are never summed): tokens and cache share side by
side, and Claude Code's days stacked on the chart surface, read out in the
legend under the pointer. Codex records whole-trace totals, so its card says
so instead of drawing days. Charts plot on `chartSurface` in `chartSeries1`
to `chartSeries4` (the Near AI OS chart tokens), each 3:1 or more on it. A
figure that does not drill is drawn plain. Drill-down rows are two lines,
not a table. Every tab keeps one type and spacing scale
(`InsightsLayout`): each section is a card under a title at the card-title
step, groups inside it open with a body-strong heading, and explanatory
notices sit behind an "i" (`InsightsInfo`, named with the core's "More about
{title}") beside what they explain; status, failures, prerequisites and an
irreversible consequence stay on the page. `InsightsPaneWidthRenderTests` renders each tab at 408pt from
the sample week; set `TC_INSIGHTS_RENDER_DIR` (and `TC_GLASS_THEME=flat`) to
keep the PNGs.

A dev build is ad-hoc signed, so it cannot read the team keychain access
group that holds the Insights digest keys: its watched-folder counting
answers `key_unavailable` and it cannot save an analysis, and every tab is
empty. For a design review, a Debug build launched with
`TRACE_COMMONS_INSIGHTS_SAMPLE=1` draws Insights from `InsightsSampleWeek`, a
fixed watched-folder week with goals, the lever, the weekly summary and one
trace for Sessions, plus a sample Subscriptions card (ChatGPT Plus, the 5-hour
window 42% used, Weekly 13% used, one reset credit) with Settings' "Codex
usage" switch drawn on. Only the analytics answers are sampled; a goal, lever,
summary or switch write is answered and kept nowhere. A release build never
reads the variable.

The Subscriptions card on the Overview reads `subscription_usage` (PR #1608,
first slice: Codex only) and is drawn only while Settings' "Codex usage" is on.
Opening the Overview reads the daemon's held snapshot; only Refresh asks the
daemon to probe the Codex app, after which the card re-reads for up to 35
seconds until no login says `refreshing`. Nothing polls in the background. A
daemon built without the `subscription-usage` feature answers
`unknown_method` and leaves `subscription_usage` out of `get_settings`, so
neither the card nor the switch is drawn.

Local operations run off the UI thread. Closing or leaving the screen discards
late results; a save or deletion already started can still finish. Reopening
loads saved results again. Selection uses the native macOS file picker.

Verify with a freshly built contributor FFI library:

```sh
TC_FFI_LIB_DIR=/path/to/target/debug swift test
```

Bridge tests exercise the real ABI with a temporary store; model tests cover
first-run navigation, explicit persistence, summary refresh/failure, missing
evidence navigation, unknown/zero/partial coverage, and completion after close.
The automated suite does not qualify VoiceOver, the native file picker,
Gatekeeper/notarization, or all display sizes on a packaged release.

Saved details show bounded declared-model metadata with physical source
coordinates, missing/invalid/omitted counts, and its source digest. Older
snapshots remain unknown until explicitly reimported. Declarations do not
verify serving identity or allocate work to a model.

Since store version 13, a Claude Code snapshot also carries usage evidence
(per-message counters, with the 5-minute and 1-hour cache-write split),
recorded-timestamp evidence, and, when it is saved, a per-turn series
(`turn_series`): one row of counters per API response and one row per tool
call. The series holds counts, sizes, dates, declared model labels and keyed
digests only; no path, body, command, or message, tool or session ID is
stored in readable form. The digest key is kept in the OS keychain, apart
from the store. A Claude snapshot saved before version 13 has none of these
and stays unknown until it is reimported. A record whose counters are not
all stated is unknown, never zero.

Saved snapshots can link an explicitly selected local repository and full
commit object ID, or import a selected structured test-report JSON file. The
import control explains the accepted fields. Git inspection and imported
report assertions have separate authority labels; neither establishes task
acceptance or model attribution. Removing a link preserves original files.
Each picker freezes its snapshot selection (and Git commit); refresh, selection
changes, and closing invalidate pending selections. Successful link changes
refresh saved detail, history, and summary; failures clear prior detail.

Episodes group one or more whole saved snapshots selected by the user. The
native screen creates and lists groups, resolves current member evidence, shows
snapshot overlap with other groups, and supports complete membership replacement,
an independent category/outcome assessment, clearing that assessment, and group
deletion. These groups do not establish task boundaries, independent tasks,
model attribution, rankings, time saved, or cost.

An open group freezes its ID and revision. Member edits submit the complete
selection against that revision; assessment and deletion use the same conflict
check. A conflict discards the draft, refreshes the group, and requires review
instead of retrying the write. Leaving the group or Insights invalidates pending
callbacks. Episode IO has separate busy state, so the compact screen keeps its
snapshot analysis, save, detail, evidence, and assessment controls available.

Snapshot replacement or deletion can remove episode groups. The screen shows
the committed group IDs and explains the loss of grouping and assessments. This
notice survives automatic episode/history refresh, including a refresh failure,
and clears on the next user action or closing Insights. A failed or missing
episode refresh clears editable detail; a successful mutation is reconciled by
reading the episode list and current detail from the local service.
