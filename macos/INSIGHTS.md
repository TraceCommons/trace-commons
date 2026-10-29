# Local Insights on macOS

The app opens on Insights. Watcher startup, compute monitoring, update checks,
and notification setup begin only after selecting another destination, once
per app launch. Their existing roots, onboarding, and activation checks still
apply. Returning to Insights does not stop services already activated.

The window has three tabs (product owner, 2026-10-29): **Overview** (the
default) shows one local week, **Breakdown** (`patterns`) shows where that
week's tokens went, and **Benchmarks** compares models on your own traces.
The Sessions and Analyze tabs are gone: the app no longer analyzes, saves,
assesses or groups trace files itself. Snapshots saved to the local store
(for example with the CLI) still feed the saved week below.

The screen displays Rust-provided figures and coverage. It does not
calculate independent metrics, discover files, enroll contributors, or
upload traces.

Overview reads `week_overview` and, for each card's drill-down,
`card_inputs` from the same local service. It counts only snapshots saved
to the local store (feed S), dated by their own recorded timestamps, never by
when they were imported, in the local ISO week (Monday to Sunday) at the
shell's current UTC offset. Claude Code and Codex tokens are separate lines
and are never summed; Codex is the change between its first and last
counters. A session without usage counters is counted as unknown, never as
zero, and the coverage line says how many. Week-to-week figures ("vs last
week", "Your best week") need watched-folder counting and are left out
here; the feed line says so. By model keeps the core's fixed order (alphabetical, unknown last); By
project is not available for saved snapshots. Every word is the core's
`ui_copy()` analytics table, marked DRAFT pending approval; every figure is
the core's. Advice, what-if estimates and costs are not shown.

Breakdown's details are drawn in the Monitor window's inspector, not under
the chart or card that asked for them: a day picked on Tokens by day shows
that day's sessions there, and a card's "See sessions" the sessions behind
the card. Picking either opens the inspector; with nothing picked it shows
"Breakdown" over the dash.

Patterns reads `patterns` and, for each card's "See sessions", the
`pattern_sessions` list, over the same saved snapshots. Its three cards are
repeated reads, "Edit, failed command, edit" (inferred from the order of
tool calls) and long context (the core's retried tool calls card is left
out on macOS), each with the token figure
first, the count beneath it and six weekly bars; a week with no counted
Claude Code session is a gap, never a zero bar. The first two are estimates
from result size; long context is counted input. The figures overlap and do
not add up to the week's tokens. Only Claude Code sessions record tool calls,
so a week with Codex sessions says how many sessions the figures cover. The
re-read table names files only by a letter and the extension; no path is
stored or shown. Week-to-week change is left out here.

When the daemon's watched-folder counting (feed T, the
`insights_counter_pass` setting, off by default) is on and readable,
Overview and Patterns show its week instead, in the same shapes, and the feed
line says so; the two feeds are never mixed, and a failed read falls back to
the saved week with a line saying counting is unavailable. Only then are
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
and Patterns cards do not drill down to sessions.

**Benchmarks** reads the daemon's `insights_benchmarks` over the last 4 or
13 weeks of watched traces: per kind of work, the model that leads on
quality (the share of its edits kept), value (cost per trace at API
prices) and speed (time per trace), with the runner-up beneath it, and only
where each model has enough traces. A trace's kind of work is the person's
own task label when the daemon has one stored, else the task the daemon
infers. The app no longer sets task or outcome labels (the Sessions tab's
menus are gone); labels stored before are still read. No shipped screen
shows a Kept share by outcome label: Overview no longer draws the Models
card, and the daemon's `insights_model_answers` is not read by the app.

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
answers `key_unavailable`, and every tab is empty. For a design review, a
Debug build launched with `TRACE_COMMONS_INSIGHTS_SAMPLE=1` draws Insights from `InsightsSampleWeek`, a
fixed watched-folder week with goals, the lever and the weekly summary, and
no saved traces. Only the core's copy and the Overview drill-downs go to the
store. A goal or lever write is answered and kept nowhere; any other write
is refused, so none reaches the store. A release build never reads the variable.

Local operations run off the UI thread. Closing or leaving the screen discards
late results. Reopening Insights, or bringing the app to the front while it
is open, reads the saved snapshots again. A failed read of the core's copy
or of the saved snapshots shows an error line under the tab bar (the core's
own words once its copy has been read).

Verify with a freshly built contributor FFI library:

```sh
TC_FFI_LIB_DIR=/path/to/target/debug swift test
```

Bridge tests exercise the real ABI with a temporary store; model tests cover
first-run navigation, the copy-and-list read and its failure line, reading
the snapshots again, unknown/zero/partial coverage, and completion after
close. The automated suite does not qualify VoiceOver, Gatekeeper/notarization,
or all display sizes on a packaged release.

Since store version 13, a Claude Code snapshot also carries usage evidence
(per-message counters, with the 5-minute and 1-hour cache-write split),
recorded-timestamp evidence, and, when it is saved, a per-turn series
(`turn_series`): one row of counters per API response and one row per tool
call. The series holds counts, sizes, dates, declared model labels and keyed
digests only; no path, body, command, or message, tool or session ID is
stored in readable form. The digest key is kept in the OS keychain, apart
from the store. A Claude snapshot saved before version 13 has none of these
and stays unknown. A record whose counters are not all stated is unknown,
never zero.
