# Windows one-pass checklist (from the Linux GTK parity rounds)

Owner, 2026-11-18: "make sure to apply everything that took multiple rounds in linux to 1 pass on windows".
Every Windows slice (W1 onward, spec draft PR #2192) meets every item that applies to it in its FIRST pass.
Reviews check this list before anything else. Do not plan a "round 2" for anything listed here.

## Brief block (paste into every Windows slice prompt)

```
ONE-PASS RULES (Linux took 2-4 rounds on each). Full list: this file
1. Mac truth = macos/Sources at the base SHA, FLAT theme. Re-read the Swift before porting any quoted hex/icon (heroes are solid
   #081538/#07152F/#0A2546 since fc4ecc1c6; stat-tile icons gone; accent #0091FD, not #8A3DFF). Port conditions, not just shapes.
2. Decoration and motion are parity: every glow, halo, gradient, mark, shine, spring, transition in the slice's Mac files is
   ported with the Mac's numbers or listed under "Not ported" with a reason. Silent omission = defect.
   Motion: each animation is a row of design.md "Animation coverage" for the slice: the Mac's curve, duration or spring, the Mac's
   frame rate (no one 30 fps clock), direction pinned by a test (dashPhase sign), none skipped.
3. Tests pin Swift-probe numbers or Mac-render pixels (file:line). No source/XAML-grep tests, no expectation computed by the
   port. Each new test is run red against old or mutated behaviour first; paste the red line. Test the wiring, not the helper.
4. No literal colours: generated tokens through the skin. Measure DRAWN pixels >= 4.5:1 on every ground (light, dark, inactive,
   High Contrast, under washes). Each new brush lands with its dark, inactive and High Contrast variants.
5. Generated tokens are regenerated (generate.py) after every merge, never hand-merged; generate.py --check is clean.
6. AutomationProperties.Name = visible title from core words (never accelerator/tooltip/concatenation). Tab walks every screen
   in visual order, no traps; decorative grids are not tab stops. Verified with a UIA dump on the box.
7. A new tc_call method is added by name to every C# fake and to sample coverage in the same commit.
8. Every new UI/render test is in the runner CI or the box executes; the log shows it ran; a list check catches missing names.
9. No shell-authored words; no new notice/card/toast without the owner's yes on it AND its words. List new and DRAFT strings.
   Never "Trace Commons", "Near.ai" literals, "daemon" or "passkey" on screen; name from brand.rs; "this computer" variants.
10. Consent no looser than the Mac: settle replies against the action pressed (id + time), account-generation guard, a card
    leaves only on a confirmed reply, read every CallAsync error, a refusal line clears when its cause does, no preselection.
11. Windows: never match windows by title; tool windows out of Alt-Tab; flyout on the clicked monitor (origin, per-monitor DPI);
    hover never takes the keyboard; focus returns without activate/restore. Test 100/125/150% and two monitors.
12. Commands wrap only between shell tokens (or after '/' in a URL), no hyphenation, copy byte-exact; test fit AND breaks at
    the narrowest width. Screens fit the 440 pane with real sample data; overflow impossible, not scrollable.
13. Animations off still redraws every state change (test never calls draw). Nothing ticks hidden/minimised; timers stop on
    Unloaded; no per-frame parsing; refresh callbacks redraw only their region and keep scroll and focus.
14. Renders diffed against the Mac's (mean |dRGB|, max, IoU), saying what the diff cannot see; captures exist on disk, are
    from the head, and are taken by HWND, never full screen.
15. Merges: after a squash landing merge on the real fork base (merge-tree --merge-base) and verify; never union-resolve copy
    modules or the header; retarget stacked children to main first; a cancelled run is not a failure; check mergeable locally.
16. Interop: every DllImport checked against trace_commons.h; rebuild the FFI in this worktree and set TC_FFI_LIB_DIR before
    trusting any result, green or red; record the binding/export count and check it moved.
17. Dependencies: Win2D version from spike S1, transitive count, licence, approved-dependencies.md; cite sources for licence terms.
18. Box: az auth, disk, session and clock preflight; no sign-in/enroll/share; restore state, clean up, deallocate.
    Builds and suites through ~/.claude/turn-locks/turn.sh. Report the head SHA, the baseline, and what did not run.
```

## How to read each item

- **(a)** What went wrong on Linux and how many rounds it took, with PR and commit evidence.
- **(b)** The Windows rule, as a definition-of-done line.
- **(c)** The Windows slices it applies to (W1 samples/gallery/binding test, W2 window shell, W3 map and tree, W4 transfers and motion, W5 look and motion foundations, W6 map art, W7 first-run steps, W8 first-run stage, W9 consent, W10 brand, W11 Home, W12 tray mark, W13 tray flyout and edge rail, W14 terminals, W15 background service, W16-W25 surfaces no slice covered, W25 keyboard and accessibility audit).
- **(d)** How the review verifies it.

"Round" counts: builder claimed done, then a reviewer, CI or box run found it, then a fix. Commit SHAs are on PR branches in TraceCommons/trace-commons.

---

## A. Decoration found late

### A1. Decoration inventoried and ported with the slice, not after
- **(a)** The GTK port moved flows, words and layout but left out the station, the first-run stage, logo coins, hero cards, tool tiles, notice motion and the tray ring. The owner noticed each only side by side ("decorative elements in Mac built have been under emphasized in the port", 2026-11-18). That forced a decoration inventory (56 items, raised to 67 by a critic) and four extra PRs: #2182, #2185, #2186 and #2187. Within those, more was found late:
  - #2185's review found Welcome's rising words, the marker check, the thread glow and selected state, and the 44 pt title all missing. None of them were on the "Not ported" list. Fixed in 2612c582e and 3fcbb821e (2 rounds).
  - #2181 round 1 had "Nothing was checked by eye". Its review found five Mac-parity defects: the tab dot drew as a 7x15 capsule, the legend halo, the focus pill fill, the unfocused dim and the range capsule.
- **(b)** Before writing code, the slice lists every decorative element in its Mac files (marks, logos, glows, halos, gradients, orbs, shines, breathing, pulses, springs, step transitions, hover washes, press feedback), each with its Mac numbers. Each one is ported in this PR, or appears under "Not ported" with a reason and an owner question. Nothing is silently simplified: a "simpler first" version, like #2164's short transfer column before #2187's peeking stack, is recorded as a deliberate difference.
- **(c)** All drawing slices: W2, W3, W4, W5, W6, W8, W11, W12, W13, and W16-W24 where they draw.
- **(d)** The reviewer diffs the slice's decoration rows in the spec's W0 inventory (`w0-decoration.md`, 80 items) against the PR's "Ported" and "Not ported" lists. Every row is accounted for. The reviewer also opens the paired gallery captures and checks each listed item by eye.

### A2. Fixed-size decorative shapes use fixed-size elements
- **(a)** The Private AI tab dot was an empty `gtk::Label`, which measures one text line high. It drew as a 7x15 capsule in the wrong colour, and its test asserted the wrong 7 px and `@tc_blue`. Fixed in #2181 round 2 as a fixed 6 px DrawingArea in `statusOff` (2 rounds).
- **(b)** Dots, pills, capsules and rings use an `Ellipse`, a `Border` or a Win2D draw with explicit Width and Height, never an empty TextBlock. Sizes and colours are pinned to Swift literals: `GlassStatusDot(size: 6)`, with neutral and held both `statusOff`.
- **(c)** W2, W5, W11, W13.
- **(d)** A UI or render test measures the element's ActualWidth and ActualHeight and its drawn colour.

---

## B. Tests that restated the implementation, passed on a wrong port, or were never red

### B1. Pin to the Mac, never to the port
- **(a)** Several suites pinned the port to itself:
  - #2181 round 1 had five tests that matched CSS substrings or `include_str!("shell.rs")`, and "one asserts a constant equals its own literal". All five were replaced in round 2 by pixel tests on the real paint functions plus Swift literals with file:line.
  - #2182 round 1's `private_ai_inputs_draw_the_private_ai_map` built its expectation with the same `Scene::private_ai(..)` call the code uses. `each_tool_has_the_macs_logo_tint_and_initials` compared `logo(..)` to itself. Fixed in e9716769b, "Pin the map's tests to the Mac's values, not the port's own calls".
  - #2170's argv test built its expectation from `terminal_args`, the function under test. It now uses literal argv per terminal (ae6a5944a).
  - The good pattern: a throwaway SwiftPM probe that compiles the Mac's Swift and prints numbers, used for the station (506 faces, 1e-6), springs and curves (`Spring(0.36,0.86)` at 0.1 s = 0.567733), and the halo (inset 2.5).
- **(b)** Every value test cites `<File>.swift:<line>` and pins either a number printed by a Swift probe compiled from the Mac sources at the base SHA, or pixels sampled from a Mac ImageRenderer render. Each test's expected value comes from the Mac, never from calling the C# code under test or a helper that reimplements its decision. A helper may transform output, never reproduce the decision.
- **(c)** All slices with ported logic or drawing: W2-W8, W11-W13.
- **(d)** For each new test, the reviewer asks where the expected value came from and rejects any expectation computed by the port.

### B2. No source-grep or XAML-text tests as coverage
- **(a)** Source-text tests kept passing while the code was wrong:
  - #2181's landing review: the dismiss test "only checks that the function body contains the text `confirm_dismiss(`".
  - #2188 review finding 7: two tests grepped `titleFor(role) !== title` and `status::private_ai_card(`. Round 3 replaced them with mounted tests; three source-text tests remain.
  - The eligibility source scans broke on a harmless refactor (322df4c13).
  - A chevron test's needle matched the wrong `.frame(width:` (#2181 G12 finding 4).
  - Windows already has XAML-as-text tests (`TraceCommons.Interop.Tests.csproj:163-178`).
- **(b)** Behaviour is tested by driving the real control, element or renderer. A source or XAML scan is allowed only for a property nothing else can witness (for example, "this file calls the shared ABI"). It is then backed by an inverse-answering fake or a recorder (see `feedback_inverse_answering_fakes`).
- **(c)** All.
- **(d)** The reviewer greps the PR's new tests for `File.ReadAllText`, `include_str!`, `Contains("` on source or XAML, and regexes over `.cs` or `.xaml`. Each hit needs a written justification.

### B3. Red first, with the red line in the PR
- **(a)** Tests were seen green but never red:
  - #2179: "Mounted tests: none were run red ... first ran on weston after their code existed". Its reds were mostly compile errors.
  - Notice fix pass 3: "No mounted test has been seen red anywhere"; a verifier had to build a pre-fix clone.
  - #2186: `a_label_with_its_own_card_is_not_unhealthy` "has no red line".
  - #2181's landing review: replacement tests "never seen failing first".
  - The good pattern was #2186's mutation table (halo inset 3 gave `left: 3.0 right: 2.5`) and #2187's red lines.
- **(b)** Each new test is run against the old behaviour or a named mutation (a wrong spring, ring order, inset or colour) and fails. The PR body shows the failing assertion line for each. A compile error does not count as red.
- **(c)** All.
- **(d)** The reviewer re-runs at least two of the listed mutations and confirms they fail.

### B4. Tests that cannot fail, or pass vacuously
- **(a)** Several tests could not fail:
  - Dialog tests used ancestor-aware `is_visible()` and "always passed whatever the widgets did" (aa98ff70c).
  - #2177's scoped dark-stage test returned early on GTK 4.14 and logged "1 passed".
  - #2185's tick test compared against the runner's animation setting, so "stops when hidden" could pass vacuously. Fixed by forcing animations on.
  - #2188's `isManaged: w => w === this._lastFocus` was always true.
  - A backdrop colour test compared against a transparent class and "passed on the wrong colour" (167d54c4e).
  - Fixtures used reply shapes the daemon never sends (`claude_source` vs the redacted `*_source_mode`). Fixed in cba36ef25, 85b1533d3 and c4dcf5718 after the box reproduced it.
- **(b)** Tests force their preconditions (animations on or off explicitly, a window mapped, a frame drawn) and assert the precondition held. Visibility checks read the element's own Visibility, not an ancestor-aware state. Daemon fixtures are recorded from a real daemon reply, never hand-shaped.
- **(c)** All; especially W7, W9 and W15 (daemon replies), and W4, W5 and W8 (motion).
- **(d)** The reviewer checks each test's setup asserts its precondition, and checks that fixtures come from recorded samples or real replies.

### B5. Test the wiring, not just the predicate
- **(a)** Some tests stopped at the helper:
  - #2171's peer check unit-tested `peer_allowed`; "No test fails if serve() stops calling peer_allowed".
  - The Launch switch-over test covered pure functions only; "that the click handler uses them was checked by reading the code".
  - #2182: dropping `set_map_style` "would pass every test".
- **(b)** For each pure decision function, one test drives the real caller (the click handler, view model or service) and fails when the call is removed.
- **(c)** All.
- **(d)** The reviewer deletes, or mentally removes, the call site and names the test that fails.

---

## C. Mac values from stale sources

### C1. Read the Mac at the base SHA, flat theme, before porting a quoted value
- **(a)** Three separate stale-source errors, each costing a round:
  - **Flat vs classic.** The first Mac renders "came out in classic by mistake" (test processes default to classic), and G17 (purple) had to be withdrawn. The decoration inventory then repeated the classic error ("Tier gradient fills: missing" is not applicable under flat).
  - **Hero grounds.** The owner decision quoted gradients (#0057B0->#0091FD and so on, plus a 0.22 radial light). Those are pre-fc4ecc1c6. The Mac ships solid #081538/#07152F/#0A2546 with no RadialGradient. The decoration critic "corrected" the survey back to the stale gradients, and #2187 corrected it again (3 rounds).
  - **Removed features.** Home stat-tile icons and the tilted watermark were removed on the Mac (HomeViews.swift:549). "Keep / undo Keep" is unused on the Mac since 2026-10-12. The override mode pill was removed in #1490, yet Linux built a clear control (7a6e483c6), review F-2 flagged it, and 78fee5144 removed it (3 rounds).
- **(b)** Every Mac value in a Windows PR is read from `macos/Sources` at the slice's base SHA, under the flat theme (`GlassTheme.resolve`; renders use `TC_GLASS_THEME=flat`), and cited by file:line. A value quoted in a decision, inventory, spec or memory is re-verified before use. If it disagrees with the source, port the source and flag the conflict to the owner. A Mac control with zero callers outside its own file and the gallery is not ported. Windows' classic accent #8A3DFF is replaced by the flat tokens (#0091FD).
- **(c)** All, especially W5 (tokens and accent), W11 (heroes and stat tiles), W9 (override and Keep) and W6.
- **(d)** The reviewer spot-checks three cited values against the Swift at the base SHA and greps the PR for any literal hex not in `glass.tokens.json`.

### C2. Port the Mac's conditions and arithmetic, not just geometry
- **(a)** Several ports matched the shape but not the rules:
  - #2186: the "unhealthy" rule was wider than `model.health != nil`, and the halo cleared by HALO_REACH where the Mac clears markOnlyGap + haloReach. Fixed in 0266ab54b.
  - #2188: F1 drew the Private AI card always (the Mac draws it only with near_ai_signin), and F2 added a state line plus an "Open {section}" row the Mac's owner decision of 2026-10-22 forbids. Both took a fix round.
  - #2160's InspectorDemand ported 3 of 8 keys.
- **(b)** Each ported view lists the Mac's show/hide conditions and demand keys with file:line, and each has a feeder and a test.
- **(c)** W2 (notices, inspector demand), W11, W12, W13, W19-W21.
- **(d)** The reviewer compares the PR's condition list with the Swift view's `if` and `switch` arms.

---

## D. Colours bypassing the skin or the contrast floor

### D1. Draw from the skinned inks, measure drawn pixels
- **(a)** The #2182 map ink regression: after merging main (b82d0f19e), every map drawing site used the Mac's raw `palette.map.*`. The skin inks and text-floor colours were "only used in tests, never in drawing". The light sublabel #8a8a90 measured 3.09:1 and dark tertiary text had 0.84 alpha. The tests "still pass, but they now check Palette fields the map no longer draws with". The cause: clean merge hunks switched the fills while the conflict hunks kept the label colours. Fixed in 3182a72b3, "Draw the map from the inks again" (3 rounds: build, merge, fix). Earlier, Omarchy contrast took 4 rounds:
  - `.dim-label` at 2.46:1;
  - the search-hit wash, where the fix over-corrected so Tokyo Night's hit "was no longer marked";
  - the fallback held but the main path not;
  - map ink at 60% alpha.
- **(b)** No Windows XAML or Win2D draw call uses a literal colour. Every colour resolves through the generated tokens and the active theme (light, dark, High Contrast). One test renders each surface and samples the pixels actually drawn for every text role. Each must be at least 4.5:1 on each ground it sits on: the pane, the card, the scene, under the field veil, under a highlight wash, and in the inactive window. A wash must also stay visible against its ground (at least 1.2:1).
- **(c)** W3, W5, W6, W8, W11, W12, W13, and every later surface.
- **(d)** The reviewer greps the PR for `#[0-9A-Fa-f]{6}` and `Color.FromArgb` outside generated files, and reads the pixel-contrast test output listing each role and ground.

### D2. Every brush has its dimmed, inactive and High Contrast variant
- **(a)** These variants were missed repeatedly:
  - #2181 round 1's unfocused dim reached only the pane, leaving dark cards at #14182a on a #15192a pane. Round 2 added a per-tier `_inactive` variant and a sweep test that fails on any fill with no unfocused twin.
  - The merge of the Omarchy skin then lacked 24 new glass names.
  - `tc_first_run_glow` was missing from the Omarchy sheet (7f0d82ff3).
  - The rail handle was a 20th colour the skin did not know (83ed09b41).
  - The heldSessions badge read 1.08:1 in its attention state.
- **(b)** Each new brush lands with its dark, light, inactive-window (`WindowActivationState.Deactivated`) and High Contrast variants in the same commit. A test fails if any theme dictionary lacks a key another defines, and checks the contrast for every state variant (attention, selected, disabled, hover).
- **(c)** W2, W5, and every slice that adds a brush.
- **(d)** The reviewer runs the theme-dictionary key-parity test and toggles High Contrast and window focus on the box for the gallery capture.

### D3. No foreground-only colour on a theme-following window
- **(a)** `.tc-brand-body` hard-coded `#000000` on a window that followed the theme, giving 1.23-1.42:1 on dark desktops. It was seen in GNOME, KDE and then Omarchy captures before #2176 fixed it with a contract test (3 sightings).
- **(b)** Any element that sets Foreground also sets, or inherits, a background from the same theme dictionary. Every first-run and dialog window is captured in light and dark.
- **(c)** W7, W8, W15, W16-W24 dialogs.
- **(d)** Light and dark captures exist for every window in the PR.

### D4. Translucency only when measured readable
- **(a)** KDE blur in #2181 took 3 rounds:
  - The shell's own text failed 4.5:1 on blurred panes, at worst 2.51:1.
  - KWin's `set_frost` turned out to be ignored, found by reading KWin source.
  - Dark now blurs by calculation; it has never been observed on a real KWin.
- **(b)** Mica or Acrylic (W5-D1) is enabled only where `MicaController.IsSupported()` or `DesktopAcrylicController.IsSupported()` is true at run time (logged). Every text role is measured over black and white desktops through the material, and the painted fallback is used otherwise. Note that the Server 2025 box may report unsupported, especially over RDP.
- **(c)** W5.
- **(d)** The PR shows the logged support values and the two-sided contrast table.

---

## E. Accessibility names and keyboard order

### E1. Names equal the visible title, from core words
- **(a)** GTK tab buttons were announced as "Ctrl+1" to "Ctrl+4". A ToggleButton with a Box child and only a tooltip falls back to its accelerator. This was found only by a real AT-SPI dump on KDE (#2160 lineage; review finding 5), fixed in af08a3e5b and merged up six PRs. #2171's own main window had the same bug ("Ctrl+1" to "Ctrl+6"). The mounted test could check only that a label existed, so the box AT-SPI dump was the real proof (3 rounds). Other names were also wrong:
  - the History "Open" name was built with an em dash;
  - the hero card's name dropped its content line;
  - the tray's accessible sentence was never ported (#2186, open through 3 rounds).

  W2-D5 gives Windows the same Ctrl+1..4 accelerators.
- **(b)** Every interactive element (tab, toolbar toggle, map node, notice, its X, list row, switch, tray flyout item) has `AutomationProperties.Name` equal to its visible title, taken whole from core words. It is never derived from an accelerator or tooltip, and never concatenated in C#. Count badges carry the core's sentence as `AutomationProperties.HelpText` or FullDescription. Composite cards expose their content line, as the Mac's `.accessibilityElement(children: .combine)` does. Re-templated stock controls keep their automation peers (W5-D3).
- **(c)** All UI slices; W25 audits.
- **(d)** A PowerShell UIA script on the box dumps Name, ControlType and HelpText for the slice's window. The dump is attached to the PR and must contain no "Ctrl+", no empty names, and no "Near.ai" or "Trace Commons".

### E2. Tab order reaches everything; decorative containers are not traps
- **(a)** The History focus trap took 4 or more rounds:
  - The combined box pass reported "Tab stops on the update notice's Close and never reaches the records".
  - The review could not verify it without a VM.
  - The cause, found on GNOME: `style::grid_append` (f6ec6ce97) made a FlowBox's children unfocusable, and FlowBox reported Tab as handled anyway.
  - Fixed in 0244b52d7, with a test (d941fbaeb) that itself needed a fix to wait for layout (abe5ff27d).
  - Verified with a real keyboard on KDE: Open was reached in 13 presses.

  A related focus fault: #2185's Welcome buttons were invisible for 1.8 s but focusable and clickable.
- **(b)** Each screen has a test that sends Tab from the window's first stop and records the visited elements. It must reach every interactive element in visual order and return to the start. A container of non-interactive figures has `IsTabStop=False` and does not swallow Tab. Elements at opacity 0 or still rising have `IsHitTestVisible=False` and `IsTabStop=False`. Rebuilding a list must not drop focus.
- **(c)** W2, W3, W7, W9, W11, W13, W16-W24; W25 audits.
- **(d)** The Tab-walk output and the number of presses to reach the last element appear in the PR, plus one real-keyboard pass on the box.

---

## F. Test doubles and the CI test list

### F1. Fakes answer or refuse every new daemon read by name
- **(a)** This recurred five times. Strict fakes in `credential_provider_tests.rs` and `funding_widget_tests.rs` panicked on each new read:
  - `tool_destinations` (94fe8b970);
  - `automatic_grant` and the Settings reads (6d5e8d30e);
  - `plan_project_modes` (676ff0551);
  - `commons_credit_summary` (11bbc4628, "The first weston run caught both");
  - `insights_glance` (8c2dc7cd2).

  Each failure cost a weston CI round, and the two files conflicted on every merge. #2171 also had a fixture that left Codex out (e4ad96492).
- **(b)** Any commit that makes the shell send a new `tc_call` method also, in the same commit:
  - adds it by name to every C# fake's answer or refuse list;
  - adds it to the recorded-sample coverage list (spike S6), or records that the sample client refuses it;
  - runs `git grep -n "<method>" windows/tests` to show every fake names it.
- **(c)** Every slice that adds a read: W2, W3, W9, W11, W13, W16-W24.
- **(d)** The reviewer lists the PR's new method names (grep `tc_call`, `CallAsync("`) and checks each against the fakes.

### F2. Every UI/render test is in the runner CI or the box executes
- **(a)** The GTK mounted tests only run under weston from a hand-kept list:
  - #2171's dialog tests were "run by no CI job" until 977a5dba4.
  - #2176 and #2177's dark and first-run checks were added later (607eeff05, 2e01c92b0).
  - #2179's consent tests came in 8cb48430f.
  - `mounted_the_memory_well_has_the_lock` and `mounted_toolbar_holds_the_toggles_and_the_gear_before_the_end_controls` are still in no list on main.
  - A renamed test matched zero tests.
  - #2182's mounted map test failed in weston at 994176808 after the builder said display tests "did not run". It was found by the follow-up review and fixed in b20622f0a.
- **(b)** Every WinUI, Win2D render and UIA test added in the PR is in the runner chosen at W1-D2 (the `windows contributor app` job or the box script). The PR adds or updates a check that fails when a test method carrying the UI/render category is absent from that runner's list, or when a listed name matches nothing. The PR links the job log showing each new test executed.
- **(c)** All; set up in W1.
- **(d)** The reviewer opens the CI or box log and finds each new test name with a pass line, and confirms no "0 tests" or "filtered out" run.

### F3. Builder runs the UI tests before claiming done
- **(a)** Compile-only claims hid real defects:
  - the toolbar was never placed (`insert_before` on the wrong widget, found on a real display; 76eb2f7bd red then green);
  - the notice column could never open;
  - the dot capsule;
  - #2182's weston failure.
- **(b)** A slice is not reported done until its UI tests have run on Windows (CI or box) at the pushed head. The report states which tests ran, where, and at which SHA.
- **(c)** All.
- **(d)** The reviewer matches the run's SHA to the PR head (`gh pr view --json headRefOid`).

---

## G. Generated design tokens

### G1. Regenerate, never hand-merge
- **(a)** #2181, #2182, #2185, #2186 and #2187 all edited `generate.py` and `brand_tokens.rs`. Every merge note said "rerun generate.py rather than hand-merging brand_tokens.rs". Shared generator functions had to be copied byte-identically between sibling branches (checked with cmp), and #2185 carried a `#[path]` module that would compile twice once #2182 landed. Other generator slips:
  - 14b252ba5 rewrote generated arrays as rustfmt keeps them;
  - the landing review's step 3 called for regenerating tokens, Cargo.lock and cargo-sources.json after each merge.
- **(b)** Generated Windows files (`Themes/BrandTokens.xaml` and any new generated C# or XAML for glass, map, logos or motion) are produced only by `scripts/design-tokens/generate.py`. After any merge or rebase touching the generator or tokens, the file is regenerated and `generate.py --check` plus `test_generate` pass. Windows-only generator changes leave the Swift and GTK outputs byte-identical (shown by `--check`). Hand-written `DesignSystem.xaml` takes no colour or size literal that a token covers.
- **(c)** W5 first, then W3, W6, W8, W11 and W12.
- **(d)** The reviewer sees `generate.py --check` output in the PR, plus `git diff --stat` showing no unexpected changes to Swift or GTK generated files.

---

## H. Merge mechanics

### H1. Merge on the real fork base after a squash landing
- **(a)** After the Linux stack was squashed into main, git's default merge base predated it, so it reported far more conflicts than were real:
  - #2182: 23 files, against 2 on fork base 020553e6c;
  - #2188: 27 against 3;
  - #2181: 30 against 9;
  - #2186: 22 against 1;
  - #2187: 15 against 4-5.

  A clean hunk still silently dropped behaviour in the map ink case (D1). Earlier, #2177 kept the branch side of five files after #2176 was squashed.
- **(b)** Before merging main into a branch whose parent was squash-merged, find the real fork base (the parent head the branch last merged). Merge each file with `git merge-file` against it, or use `git merge-tree --write-tree --merge-base=<fork>`. After the merge:
  - `git diff --name-only origin/main <merge>` equals the branch's own path set;
  - each conflicted and auto-merged file is checked for both sides' markers (count occurrences per side);
  - every ink, token or field the branch added still has a non-test consumer.
- **(c)** All stacked slices: W1-W4 is a stack; W5, W7, W9 and W11 depend on W2.
- **(d)** The reviewer recreates the merge with `git merge-tree --write-tree --merge-base=<fork>` and diffs it against the merge commit.

### H2. Never union-resolve copy modules, the C header or fake lists blindly
- **(a)** Union resolves broke builds and silently dropped behaviour:
  - In `private_inference_copy.rs`, a union resolve spliced one side's block into the middle of another function (2026-09-08).
  - The weston list and fake refusal lists conflicted on every merge and needed hand unions.
  - #2177 had to restore a closing brace both sides shared.
  - The shell_wording baselines had to be re-measured rather than picked.
- **(b)** On a conflict in a shared copy module, either `trace_commons.h` copy, `NativeMethods.cs` or a C# copy decoder: reset the file to `origin/main` and re-apply the branch's blocks by anchor from the pre-merge commit. The ShellWordingTests baseline is re-measured (set it to 0 and read the reported number), never chosen. Lists (fakes, test runner) are unioned, then checked for duplicates and dead names.
- **(c)** W1, W7, W9, W10, W16-W24 (all add copy fields or exports).
- **(d)** The build plus the header-binding test (I1) pass. The reviewer checks the baseline number against the test's own report.

### H3. Stacked PRs: retarget before the parent merges
- **(a)** `delete_branch_on_merge` is true and the queue squashes. When linux-mac-look was merged and deleted, #2187's base moved, it went CONFLICTING, and "GitHub starts no pull_request runs" (also #2186 and #2188). The landing order had to be simulated, and #2189 duplicated #2188's extension on a sibling branch.
- **(b)** Before any parent slice enters the queue, run `gh pr list --repo TraceCommons/trace-commons --state open --json number,baseRefName` and retarget every child to `main`. After the parent lands, cherry-pick the child's own commits onto fresh `origin/main` and re-run its suite. Before designing a shared interface (ABI field, extension contract), check sibling branches for the same work.
- **(c)** W1-W4 stack; W5/W7/W9/W11 on W2.
- **(d)** The reviewer runs the non-main-base check at merge time.

### H4. Read CI status correctly
- **(a)** CI status was misread several ways:
  - The "macOS app tests" job was cancelled at its 45-minute limit while its log showed all tests passed; whole cancelled runs read as red.
  - Two clients runs for one commit were both cancelled (#2164).
  - Reports quoted CI counts from an earlier commit ("33 checks") or named the wrong job.
  - GitHub's mergeable state lags (`feedback_github_mergeable_lags`).
  - A Windows timing flake (`a_witness_that_never_answers_keeps_the_approval_and_retries_hourly`) failed on #2176, #2170 and #2177.
- **(b)** Before acting on a red check:
  - `gh api repos/TraceCommons/trace-commons/actions/runs/<id> --jq '.status+"/"+.conclusion'`, where `cancelled` means re-run, not diagnose;
  - mergeability is verified locally with `git merge-tree`;
  - a timing test failing under load is re-run alone before it is reported.

  CI counts in a PR body name the job and the SHA.
- **(c)** All.
- **(d)** The reviewer checks that cited runs belong to the head SHA.

---

## I. Interop risks the Windows ABI adds

### I1. Bindings checked against the header
- **(a)** GTK links Rust directly, so Linux never had this layer; Windows does. C# binds 108 of 185 exports and nothing compares them with `trace_commons.h`. Copy fields added to Rust must reach both header copies, the Swift decode contract and C#. On Linux, the Swift export contract test failed until the decoder took the 5 new keys (253d1bcee). #2180 removed exports across core, FFI, both headers and four shells in one commit "so every tree state compiles".
- **(b)** W1 lands a test that parses `trace_commons.h` and fails when a `tc_*` DllImport names a missing export or mismatches its signature. Every later slice that adds bucket (a) or (b) ABI work updates, in one commit:
  - the Rust;
  - both header copies (identical md5);
  - the Swift export contract;
  - `NativeMethods.cs` and its decoder;
  - the C# field-level copy tests.

  The PR states the absolute binding count before and after (108 -> N).
- **(c)** W1 (test), then W2, W3, W7, W9, W10, W11, W13 and W16-W24.
- **(d)** The binding test output shows the new count, and `md5` of both header copies matches.

### I2. Stale DLL fakes results in both directions
- **(a)** This came from the Windows interop history, not Linux. A stale `trace-commons-contributor-ffi` produced 45 phantom failures (875 passed vs 920 after a rebuild). Worse, it produced a false PASS: 969 passed on a DLL that predated the new fields, "106 exported and 106 declared" agreeing with each other. The test projects copy the DLL only `Condition="Exists(...)"`. "windows contributor app" failed once only because its FFI artifact was never produced.
- **(b)** Before any interop or WinUI test run:
  - build `cargo build -p trace-commons-contributor-ffi` in this worktree, through the build lock;
  - set `TC_FFI_LIB_DIR=<worktree>/target/debug` and `DOTNET_ROOT=$HOME/.dotnet`;
  - record the export count the suite saw and confirm it moved when the slice added exports.

  On the box, the DLL's build SHA matches the PR head.
- **(c)** Every slice that runs the interop suite (all).
- **(d)** The PR's test log shows the FFI build line and the export count.

---

## J. Brand strings

### J1. The name comes from brand.rs; scan packaging too
- **(a)** The brand took a plan, a fix and a review (#2184, plus d78525563 and owner decisions on the store text).
  - Hits no test read: `publish-repo.sh` `Title=Trace Commons`, and metainfo text that wrapped "Trace⏎Commons" across two lines so both scanners missed it.
  - "Near.ai" lingered in the desktop entry.
  - Rail and panel titles first showed internal ids ("edge-rail") in Alt-Tab.
  - #2186's comment misquoted the owner's remark as licence terms.

  Windows has "Near.ai" 25 times in 14 files plus 4 in `Package.appxmanifest`, and `MainWindow.xaml:405` has an AutomationProperties.Name carrying the old brand. The spec's W10 test "manifest display names equal APP_NAME" cannot pass as written (`:92` is "Near.ai invite").
- **(b)** No user-visible "Trace Commons", "Near.ai" literal or internal id appears in XAML, C#, the manifest, tray tooltip, window titles, automation names, installer or store text. The name reaches Windows from `brand.rs` through a core export (bucket (a)). The scan joins wrapped and concatenated strings and covers `windows/packaging`, the MSIX manifest and the scripts. Any compound name ("{APP_NAME} invite") is a new string for the owner. Internal ids (`ai.tracecommons.*`, pipe names) stay.
- **(c)** W10, plus every slice that adds a window title or string (W2, W12, W13, W15).
- **(d)** `NoCommonsNameTests`, extended to packaging and multi-line, passes, plus `git grep -n -i "near\.ai\|trace commons" windows/` with every hit justified.

---

## K. Copy rule and notices

### K1. No authored words; no new notices; list new words for the owner
- **(a)** Builders kept adding words and notices:
  - DRAFT words on screen (`legend_not_watched`, `transfer_contributing`, the training sheet);
  - #2171's first draft words named the commons;
  - GTK-authored strings where shared keys existed (`SUBMIT_ALL`, `UNDO_BODY`, removed in d5f5a6fa3);
  - a scan-recovery dialog under the no-new-notices rule (removed in f6d0567dd);
  - a toast that rendered behind a modal ("nobody would see it");
  - a false sentence under an override ("Folders set to Ask or Never keep their own setting"; review F-1, fixed in 78fee5144);
  - #2188 F2's extra line;
  - test fixtures with English sentences tripping shell_wording (d620404c8, and #2171 twice via `cfg(all(test,..))`).
- **(b)** Every string on screen comes from core copy over the ABI, and ShellWordingTests' baseline goes down or stays (40 files / 337 sentences at 19b42d0d2). The PR lists:
  - every new core string, with its approval date or "needs approval";
  - every DRAFT string drawn;
  - every Mac string naming "this Mac" with its "this computer" variant.

  A card, banner, toast, InfoBar, flyout line or tray notification the Mac shows but Windows lacks is a new notice and needs the owner's yes on the notice and its words before it is built. Re-housing an existing notice is fine. Detail goes in tooltips, bodies stay at 2 short sentences or fewer, and nothing says "daemon" or "passkey". Each toast is checked to be visible (not behind a modal).
- **(c)** All.
- **(d)** The reviewer reads the ShellWordingTests delta and the PR's word list, and greps added string literals in `.cs` and `.xaml`, untruncated.

---

## L. Consent fails closed

### L1. Settle replies against the action pressed
- **(a)** #2179's undo race (review F1): the reply was checked against whatever the single undo slot held. Undo on folder A, then Submit on B, and A's reply dismissed B's card. Fixed in c856a7cb9 with UndoIdentity, settle_undo and a writing guard (build, review, fix). Earlier, the arming offer closed whatever the daemon said (82d280b45). On Windows today, `OnAcceptArming` clears the offer before the call and never reads the reply, and `CallAsync` returns errors as data (S5).
- **(b)** Each consent action keeps the identity it was pressed with (folder or entry id plus timestamp). The reply clears only that action's card, and only on a confirmed reply. A refusal keeps the card and shows the core's line. A second press while a call is out sends nothing. Every `CallAsync` result is read for its error.
- **(c)** W9, W11 (arming card), W13 (flyout actions), W21 (withdraw, publish).
- **(d)** A test interleaves two actions (undo A, submit B, A's reply lands) and asserts B survives. A refusal test asserts the card stays.

### L2. Account-generation guard on grants
- **(a)** `settleAutomaticGrantOwner` and the generation guard were missing (F2). Fixed in 7a6e483c6, then F-5 found the boundary was not taken at sign-out or unenroll. Fixed in ba0cabab7 and abe5ff27d with `an_unenroll_ends_the_writes_pressed_before_it` (3 rounds). The training offer also needed "only the latest status read may raise the sheet". History offers no Withdraw on a previous account's row (46a4976ca).
- **(b)** Windows ports the Mac's grant flow (`grant_automatic_including_existing` behind grant-owner and account generation; `withdraw_automatic_grant`). A grant or withdrawal pressed under an older account generation, or across sign-out or unenroll, is reported refused. Only the newest status read may raise a consent sheet.
- **(c)** W9; W16 (accounts).
- **(d)** A test switches account between press and reply and asserts refused.

### L3. Refusal lines leave when their cause is resolved
- **(a)** The Tools refusal ("Your folder changes couldn't be saved...") and the old-portal command stayed on screen after the command had run and the daemon watched the tool. Fixed in 215e9d32d, then for an already-held tool via `source_revisions` in 0be72b927, then 2f1a1ff94 so the row stops sending the unmappable path (3 rounds). A Sharing note stayed while false under an override (78fee5144). Other stale state: folder rows did not refresh until relaunch after a host command (focus-based refresh), and a cancelled arming card stuck when it unmapped mid-fade (4321d32f5).
- **(b)** Every failure or refusal line has a named condition that clears it, and a test drives the success path after a failure and asserts the line is gone. Every success path clears the failure state the previous attempt set. A note whose claim depends on state hides when that state changes.
- **(c)** W7, W9, W14, W15.
- **(d)** For each refusal line in the diff, the reviewer finds the test that shows it, then hides it.

### L4. Grant carry-over and path hygiene fail closed
- **(a)** A tool home declared before its store existed lost the automatic grant once the store appeared. Found in #2171 review finding 3; owner decision "carry the grant over"; fixed in f6bfa28cb, failing closed for links, files and unreadable paths. Its guard test "passed before the change too". The daemon accepted relative or empty paths (cb4a9e4f8). A declaration could be sent unmapped during a switch-over (fee503c56), and the same race in Launch was found a round later (dc71de01c): a fix in one path, the sibling still broken.
- **(b)** Windows inherits the daemon behaviour unchanged. Any Windows path that declares a folder or launches decides from live state at send time, refuses while a switch-over or reopen is pending, and sends only absolute paths. A fix to one declaring path is applied to every sibling path in the same PR, with a test each.
- **(c)** W9, W14, W15.
- **(d)** The reviewer greps every call site that sends a path or mode and checks each has the guard and a test.

### L5. Nothing more permissive than the Mac
- **(a)** Linux asked before turning a folder Automatic where the Mac writes at once: allowed, because stricter. Unpinned preview sends were blocked ("sends nothing unless this entry's preview is pinned"). S5 found Windows arms with `set_project_mode auto_upload` and no grant, and adding a grant call "would grant every folder".
- **(b)** Selection, consent and send are no more permissive than the Mac. Nothing is preselected. A failed read shows a dash, never 0. A Mac-removed control (undo-window dial, override clear) is not built. The arming card shows the shared `tc_arming_offer_copy_json` body.
- **(c)** W9, W11, W13.
- **(d)** The reviewer walks each consent path in the PR against the Mac's AppModel and lists any difference.

---

## M. Focus and window management

### M1. One title per window; tool windows out of Alt-Tab
- **(a)** The rail and panel title collision took 3 rounds:
  - The panel shared APP_NAME with the main window, matched by title (F3).
  - Then own titles "edge-rail" and "menu-panel" showed in Alt-Tab (box, g19).
  - Then brand-name titles plus a hidden marker (5704c7fca). That version's live run is still owed.
- **(b)** Windows identifies its own windows by HWND or object reference, never by title. Rail and flyout windows use `WS_EX_TOOLWINDOW` or an AppWindow presenter that keeps them out of Alt-Tab and the taskbar, and carry no internal id as their title. Only the main and Settings windows appear in Alt-Tab, titled from core brand words.
- **(c)** W2 (Settings window), W13.
- **(d)** An Alt-Tab capture on the box, plus a UIA dump of top-level windows.

### M2. Hover never takes the keyboard; focus returns without activating
- **(a)** On KDE, hovering the open rail took the keyboard from the active window, and closing never gave it back. Typing stopped reaching Konsole. The owner's rule became "Keyboard only after a click" (`RailKeyboard`). A hover during the closing slide brought the keyboard back (a0fa989a7). On GNOME, returning focus with `activate()` switched workspace and unminimised windows (HIGH in the #2189 review). Fixed with `focus()` only for a managed, current-workspace, non-minimised window (3 rounds).
- **(b)** The rail and flyout open on hover with no activation (`WS_EX_NOACTIVATE` or `ShowWindow(SW_SHOWNOACTIVATE)`). They take the keyboard only after a click inside, and give it back as the close starts. Focus goes back only to the window that had it, if it is still visible and not minimised, without restoring, raising or switching virtual desktops. Escape and Tab work while the surface holds the keyboard.
- **(c)** W13; also any flyout in W12 and W2.
- **(d)** A box test types into Notepad, hovers the rail, keeps typing, and asserts the text lands in Notepad. A click test asserts Escape closes the rail and focus returns.

### M3. Multi-monitor and DPI
- **(a)** #2188 F7: tray coordinates are desktop-wide but were treated as local to the main window's monitor, and the panel appeared on the wrong monitor. Round 2 fixed layer-shell, but the extension still used the window's monitor. Round 3 added `monitorFor`. Fractional scaling and primary-on-the-left were never run. GNOME 46's first placement put the rail at Mutter's cascade spot until first hovered (92c4c3516).
- **(b)** The flyout opens on the monitor that holds the tray click, using that monitor's work area (`MonitorFromPoint`, origin included). The rail is pinned to the Mac's choice (first or primary screen) and sized from that monitor. Positions are in physical pixels with per-monitor DPI awareness. Each surface is placed before it is first shown, so it never appears at a default position. Window growth at the screen edge (W2-D3) follows spike S2.
- **(c)** W2, W12, W13.
- **(d)** Box captures at 100%, 125% and 150%, with two monitors (primary left and right) if the box allows; otherwise the unit tests over the monitor geometry, plus a statement of what was not run.

### M4. Window and pane geometry fixed and fit
- **(a)** Several layouts broke the Mac's geometry:
  - Screens overflowed the 440 main pane (3 rounds: the scroll policy, a fit test that measured only Traces, then libadwaita's switcher floor at 448). The community strip still clips with real data.
  - Window height varied by tab (772 vs 978 vs the Mac's 614).
  - A hidden footer took 190 px (3dfea48af).
  - Equal tabs set the pane minimum (c5119f234).
  - The preview sheet was 900 wide on an 853 px screen (4c4bce862).
  - Wrapped buttons were clipped on old GTK 4.14 (4 rounds, 3a01f1c1d).
- **(b)** The main pane is 440, the map 440 (min 360) and the inspector 293. A test asserts the window's size per tab, and that every hosted screen fits the 440 pane with real sample data (including the community payload, Insights with data and History with records) at 100%, 125% and 150% text scale, with horizontal overflow impossible rather than scrollable. Collapsed elements report zero size. Dialogs and sheets request `min(design width, monitor work area)`.
- **(c)** W2, W3, W11, W22.
- **(d)** Fit-test output per screen, plus the gallery's pane-geometry comparison.

---

## N. Command and label wrapping

### N1. Commands wrap only between tokens and fit the narrowest width
- **(a)** Wrapping took 3 rounds:
  - Round 1: Pango hyphenated `trace-commons-contribut-` / `or.service` (1c7f018c8).
  - Round 2: breaks fell inside a quoted argument (6cd4f9c1b).
  - Round 3: holding every token made the 84-character URL unbreakable, so dialog A grew to about 864 px. Fixed by breaking after "/" (1290be8ac), with a test that every line fits at 360.

  The first wrap test checked only where lines start, so it passed with overflow. Elsewhere, "FOLDERS ON THIS COMPUTER" ellipsizes below 440.
- **(b)** Command text is shown in a selectable TextBlock or RichTextBlock with hyphenation off. It breaks only at spaces between shell tokens, or after "/" inside an unquoted URL token; quoted tokens and unit names stay whole. Copy returns the exact bytes. A test at the narrowest width the dialog allows asserts both that every line fits and that every break position is legal. Button and label rows wrap without clipping at 440, 380, 320 and 260.
- **(c)** W14, W15, W7 (folder hints), and any dialog with a command.
- **(d)** Narrow-width captures plus the test's fit and break output.

---

## O. First paint, staleness and per-frame work

### O1. Animations off still redraws every state change
- **(a)** #2185 review M1: with animations off the stage redrew only from its tick, so a tool watched left the old ring. The mounted test called `draw()` itself and could not see it. Fixed with `connect_changed` and a test that never calls draw (2 rounds). The Private AI canvas showed a Traces scene until the first read landed (994176808).
- **(b)** Every drawn surface invalidates on each model change it depends on, independent of any clock. A test with `AnimationsEnabled=false` changes the model through the real control and asserts the surface redrew with the new state, without calling the draw method directly. Each canvas starts in its own empty state, never another view's scene.
- **(c)** W3, W4, W6, W8, W11, W12.
- **(d)** The reviewer names the test and confirms it flips the setting and does not call draw.

### O2. Refresh callbacks redraw only what changed
- **(a)** #2181 G12 review (Medium): the inspector was rebuilt three extra times per refresh by the tool, rollup and credit callbacks, losing scroll and focus. Fixed in b883e18bf. Choosing a sharing option on Uses rebuilt the card and moved focus.
- **(b)** A read's callback re-renders only the region that shows that read. Re-rendering never resets scroll or focus. A test refreshes while a row is selected and focused and asserts both are kept.
- **(c)** W2 (inspector), W3, W9, W11.
- **(d)** The reviewer reads the test and checks the callbacks for unconditional rebuilds.

### O3. No per-frame parsing, allocation or polling
- **(a)** #2182 round 1 rebuilt the station every frame: a 506-face sort, about 8.8k trig calls, logo paths re-parsed, about 27 hex colours re-parsed. Fixed in d90bc4582. #2185 read the palette per frame. #2181's blur follower ran a per-frame plus 1 s X11 poll on every desktop until it was rewritten as `watch(...)`.
- **(b)** Geometry, paths, brushes and Win2D resources are built once per (style, scheme, DPI) and cached. A frame does only transforms and draws. Nothing polls on a timer when an event exists.
- **(c)** W3, W4, W6, W8, W12, W13.
- **(d)** The reviewer reads the per-frame path for allocations and parses, and checks a profile or frame-time note in the PR.

---

## P. Reduce motion and hidden timers

### P1. Clocks stop while hidden; reduce motion draws the final frame
- **(a)** Several clocks kept running or were tested vacuously:
  - The station's clock ticked while the map was hidden (63e556e80). Its test then failed in weston because hiding the canvas swapped the view stack (b20622f0a).
  - #2187's 10 fps pixel graphic ran on a glib timeout, so it kept firing for a minimised window, and its settings handler was never disconnected.
  - The arming card stuck forever when it unmapped mid-fade (4321d32f5).
  - A dropped pile stage left a fired timeout (0163e906a).
  - Reduce-motion assertions were dropped from intermediate stack branches.
- **(b)** Every animation runs on the composition clock or one shared frame driver that starts only while its element is loaded, visible, and its window is neither minimised nor occluded (window `VisibilityChanged` and `Activated`). It stops otherwise. Each `DispatcherTimer` has a named owner that stops it on Unloaded. With `UISettings.AnimationsEnabled` false, nothing ticks and each element shows its final state at once. An animation interrupted by hiding completes its end state. W5 lands this driver and its tests before any animated item.
- **(c)** W5 first; W4, W6, W8, W11, W12, W13.
- **(d)** A test asserts no ticks while hidden or minimised and a final frame under reduce motion. The reviewer greps the PR for `DispatcherTimer`, `Task.Delay` loops and `CompositionTarget.Rendering`, and checks each has a stop path.

### P2. Inventory existing timers correctly
- **(a)** The Windows spec's first draft said the only timer is the 10 s poll. Its review found the 1 s `_undoTick` (MainViewModel.cs:48) and a visibility debounce (MainWindow.xaml.cs:98). It also found that "nothing animates" overlooks the stock controls, which do animate.
- **(b)** Each slice touching a view lists the existing timers and stock-control animations it inherits, and states for each whether it stops when hidden and honours reduce motion.
- **(c)** W2, W5, W9.
- **(d)** The reviewer runs `git grep -n "DispatcherTimer\|Storyboard\|Timer(" windows/src` over the touched files.

---

## Q. Render-diff evidence

### Q1. Numbers, coverage stated, files present
- **(a)** Renders and captures overclaimed:
  - The reported `normalDay-desktop-third` capture did not exist.
  - #2181 round 1: "Nothing was checked by eye".
  - The Mac's capture hook renders scroll bodies blank and draws glass painted.
  - KDE blur "renders" were PIL composites, not KWin pixels.
  - Initials comparisons were by colour, which differs by font renderer (fixed to coverage, 250513e8c).
  - Reviewers viewed 7 of 20 captures.

  Good practice already exists: mean |dRGB|, max and IoU per piece (station 1.3-1.6; tiles IoU 0.98-1.00), with words and grounds masked and the limits listed.
- **(b)** Each drawn item has an offscreen Win2D render diffed against the Mac's ImageRenderer render of the same sample, flat theme, light and dark, reporting mean |dRGB|, max and IoU. The PR says what the diff masks or cannot see: text, material, wall-clock phase, blank Mac scroll views. Every capture cited exists on disk with the gallery name `<set>-<tab>-<panes>-<appearance>.png` and is from the PR head. Reviewers list which captures they opened.
- **(c)** W1 (harness), then every drawing slice.
- **(d)** The reviewer runs `find` for each cited file and opens at least the changed surfaces' pairs.

### Q2. Capture only the target window
- **(a)** Agents captured the user's own screen (`feedback_agent_blind_screen_captures`). Spectacle returned a stale frame (clock 04:36 vs 05:26), DPMS blanked captures, and a stale "Low Disk Space" banner sat in every GNOME capture.
- **(b)** `win-capture.ps1` captures the app's window by its HWND, found at capture time, and asserts the owning process. No full-screen capture. Each capture is checked against the box clock and for stray notifications.
- **(c)** W1 and all capture runs.
- **(d)** The capture log shows the process id and timestamp.

---

## R. Box testing hygiene

### R1. Preflight, isolation, cleanup
- **(a)** Box problems cost several rounds:
  - **Disks filled.** The Mac's disk hit 100% twice (a linker failure) and the GNOME box hit 100% twice while building a Flatpak. Disks were resized later.
  - **Auth expired mid-run.** gcloud auth expired twice, so the live GNOME test and the Omarchy cleanup are still owed.
  - **User data lost.** A real `~/.var/app/...` was deleted after an agent misread a truncated listing.
  - **Session state polluted.** Shell keys were written into the real dconf, the KDE box had two seat0 sessions (an autologin loop), and leftover branches and scripts sat on the boxes.
  - **Environment wrong for the test.** The KDE box had no DRM, so blur could not be verified at all.
  - **Shared scratch overwritten.** One agent overwrote another's `pr-body.md` and briefly published it over #2161.
- **(b)** Before a box run:
  - `az account show` (and gcloud if used) succeeds;
  - disk has at least 20 GB free;
  - one interactive session exists;
  - the clock matches;
  - the box is checked to have what the test needs (GPU, Mica support, the Segoe UI Variable and Segoe Fluent Icons fonts, logged).

  During the run, no sign-in, enroll or share. App state is backed up and restored. Nothing outside the app's own state is deleted on a truncated listing. Scratch files are named `<branch>-pr<N>-*`. After the run, test state, processes, installed packages and temp branches are removed, and the VM is deallocated. Builds and suites go through `~/.claude/turn-locks/turn.sh`.
- **(c)** All box work (W1 onward).
- **(d)** The PR's box section lists the preflight values and the cleanup done, and states what could not be run.

---

## S. Dependencies and licence records

### S1. Approved, pinned, recorded, audited
- **(a)** KDE blur's 18 lock packages were added "even with the feature off". `flatpak-cargo-generator.py` was denied, so cargo-sources.json was rebuilt by script, and "Nothing in PR CI validates this file". gtk4-layer-shell needed its approved-dependencies entry, a sha256-pinned C library and distro notes. `gdk4-x11` was "not approved", so a feature was dropped. The NEAR mark's licence comment quoted the owner's remark as licence terms (fixed in the landing round). Tool logos still lack a recorded trademark check.
- **(b)** Win2D (approved) lands with:
  - the exact version chosen by spike S1, not from memory;
  - its transitive NuGet count and licences recorded in the PR and in `~/.claude/approved-dependencies.md`;
  - any vulnerability audit clean.

  Any other new package is proposed with the full dependency-proposal details before it is added. Logo and mark assets carry their source file and the owner approval, quoted with its date, never paraphrased as licence terms.
- **(c)** W1/S1, W3, W5, W6, W8, W12.
- **(d)** The reviewer reads the PR's dependency section and checks the approved-dependencies.md line.

---

## T. Process items that recurred

### T1. Reports state the SHA, the baseline and only what ran
- **(a)** Reports were wrong about their own runs:
  - Reports cited a commit that did not exist (0fc27dcc3; the real one was 9af34a50f).
  - They gave test counts from earlier commits and claimed "all green" while CI was pending.
  - "Mounted tests ran" was claimed when they had not.
  - PR bodies went stale after fix commits.
- **(b)** The slice report gives:
  - the baseline counts at the base;
  - the counts at the head SHA;
  - which suites ran where;
  - which tests were written but not run.

  The PR body is updated after every fix commit.
- **(c)** All.
- **(d)** The reviewer compares the cited SHA with `gh pr view --json headRefOid` and re-runs the counts.

### T2. Fix the class, not the instance
- **(a)** Several fixes reached one site and missed its siblings:
  - Tab names were fixed in the shell but not in #2171's main window.
  - The switch-over guard went into declarations but not Launch.
  - The skin got one colour but missed the next new one.
  - `is_visible()` vs `get_visible()` recurred in #2160 and #2171.
- **(b)** When a slice fixes a defect, it greps `windows/` for the same shape and fixes every sibling in a separate commit labelled latent.
- **(c)** All.
- **(d)** The reviewer runs the same grep.
