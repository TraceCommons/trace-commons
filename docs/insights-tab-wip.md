# Insights tab: work in progress

**Status:** work in progress from Kristi (Insights PM). Kristi will review it after she lands. Please don't merge this branch. It's for looking at and trying out.

The branch is `kristi-insights-tab`. It contains:
- origin/main;
- Zaki's open Insights PRs #1509 (list prices) and #1510 (session facts, Codex reader, model answers engine);
- the changes listed below.

## What changed and why

The goal is that Insights answers the questions developers actually ask:
- Which model is best for bug fixing?
- Which model is best for refactoring?
- Which model writes the best tests?
- Which model is cheapest for documentation?
- Which model creates the least rejected code?
- Which model saves the most time?

The answers come from the user's own sessions, on their own machine.

1. **Models card on Overview** (new; removed again by the product owner on 2026-10-28).
   - **What:** shows the six answers from the #1510 engine (`insights_model_answers`). Each row shows a leader and a runner-up, their session counts, and the basis (for example "undone edits we could see"). When there isn't enough evidence it shows "Not enough traces yet" or "No clear difference", and an (i) explains which sessions were left out and why. Money is labelled "at API prices".
   - **Why:** #1510 computed the answers, but no screen showed them.

2. **One-click labels on each Sessions row** (new).
   - **What:** each row gets a **Task** control (Bug fix, Refactor, Tests, Docs, Other), pre-filled with the automatic guess and marked as guessed or set by you. It also gets an **Outcome** control (Kept, Partly kept, Thrown away). Your task label replaces the guess in the Models answers, and the outcome adds a "kept share" per model.
   - **Where it's stored:** labels stay on the machine, in a small file of their own beside the daemon's session store. They're deleted on unenroll, never kept for Never folders, and dropped when a session leaves the 13-week window. When the daemon has to rebuild its session store (a damaged file, or one another build wrote), the labels are kept and come back with their sessions.
   - **Why:** the automatic task types are rough guesses, and "accepted" can't be measured from transcripts alone. One click from the user is the most accurate signal and needs no data to leave the machine.

3. **Correctness fixes to the #1510 answers.**
   - **What:**
     - Models are compared only on task types both models have enough sessions in.
     - A session counts as "finished" only if its last prompt was answered.
     - Slash commands and `!` bash-mode lines no longer count as prompts.
     - Undos and edits are counted in the same unit, so the undone rate can't go over 100%.
     - Codex token figures are checked against the running total.
     - Codex `shell_command` edits count as unknown, not zero.
     - Every left-out session is accounted for.
   - **Why:** without these, the first numbers people saw would be wrong. The full findings are in Kristi's review on #1510.

4. **Smaller fixes from Kristi's feedback.**
   - The divider between the main pane and the map now resizes the panes. Before, dragging it moved the whole window.
   - The rocket on the Traces map is redrawn to look like a rocket. Before, it looked like a drop of water.
   - Analyze no longer lists empty "Events · Observed total: Unknown" metrics.

## What's happening with Analyze

Analyze asks you to **pick a session file by hand** (`~/.claude/projects/<folder>/<id>.jsonl`). Terminal users never see these files, so there's now a second way in: **"Analyze & save" on a watched session in Insights > Sessions**. Open a row and press the button.
- The daemon finds that session's file among the sessions it already watches, runs the same analysis, and saves it.
- Analyze then opens the saved result, with its assessment and evidence.
- It never runs on Never folders, nothing goes over the network, and no file path is sent to the app.
- A session the daemon can't analyze (over the core's size limit, or a Codex file without exactly one session) says so in place of the button.

**"Choose file" stays.** Analyze & save needs watched-folder counting, which is on by default (product owner, 2026-10-29), a running daemon and the standard store. Without them (and in sample data mode) Choose file is the only way to add a trace, so it's always on Analyze. The line under Analyze's title names only buttons you can see: "Open a trace in Sessions and choose Analyze & save" only while Sessions offers it, otherwise the old "Choose a file to analyze".

Known gaps:
- With a custom `--insights-store` the button is hidden, because the daemon writes only the standard store.
- The button hasn't been tried in the running app with real data yet. In sample data mode the button is not offered, so use real data to try it.

## Running it

This build is meant to be tried in **sample data mode**. The Models card is then filled with sample answers, so every state can be seen without months of real sessions. Sample mode is a launch switch in debug builds, not a code change. In sample mode Sessions shows the sample's saved traces, not watched sessions, so no Task or Outcome labels are drawn. Use real data with counting on to try them.

Analyze is filled in sample mode too: five saved snapshots from the sample week (three Claude Code, two Codex) with their models, three assessed and one with a linked commit and test report, the saved history summary for them, and one group of two snapshots with its assessment. Nothing you do there is kept. Saving an assessment, linking evidence, deleting a snapshot or changing the group is answered with the sample unchanged. Making a new group, comparison tasks, saving a chosen file and Analyze & save aren't available in sample mode. Choosing a file shows its analysis without saving it, and the question cards read your real store. None of this touches your real saved analyses.

```bash
cd macos
scripts/make-app-bundle.sh debug                       # first build takes 10-15 min
TRACE_COMMONS_SAMPLE=normalDay TRACE_COMMONS_INSIGHTS_SAMPLE=1 \
  .build/Near.ai.app/Contents/MacOS/TraceCommonsApp
```

**To use your real data instead,** ask Claude to "run it with real data", or run `open .build/Near.ai.app` without the two variables.

Quit the installed Near.ai first. Both builds use the same app identity, so only one should run at a time.

## Known open items

- `every_shipped_table_is_owner_reviewed` fails until the list prices in #1509 are signed off. That failure is expected.
- The product decisions waiting on the Insights PM (rates sign-off, consent, what's shown) are listed in Kristi's reviews on #1501, #1509 and #1510.
